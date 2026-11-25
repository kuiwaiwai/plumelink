use serde::Serialize;
use std::{collections::HashMap, path::{Path, PathBuf}, process::Stdio, sync::{atomic::{AtomicU64, Ordering}, Mutex}, time::Duration};
use tokio::{io::{AsyncRead, AsyncReadExt, AsyncWriteExt}, net::TcpStream};
use futures_util::{future::BoxFuture, stream::{FuturesUnordered,StreamExt}};

const TIMEOUT: Duration = Duration::from_secs(5);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
const ADB_PORT: u16 = 5037;
const SERVER_START_FAILED: &str = "USB 后台服务启动失败，请检查安全软件是否拦截后重新检测";
const SERVER_START_TIMEOUT: &str = "USB 后台服务启动较慢，请稍后重新检测";
const SERVER_RESPONSE_FAILED: &str = "USB 后台服务未正常响应，请先退出其它手机助手或模拟器，再重新检测";
const DEVICE_LIST_FAILED: &str = "无法读取 USB 手机列表，请重新插接手机后重新检测";
const OUTPUT_LIMIT: usize = 65536;
const CLEANUP_BATCH: usize = 4;
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(3);
const COMPONENT_TIMEOUT: Duration = Duration::from_millis(1200);
const CONNECT_LIMIT:usize=8;
const READ_LIMIT:usize=2;
const NAME_TIMEOUT: Duration = Duration::from_millis(1200);
const NAME_CACHE_TTL: Duration = Duration::from_secs(60);
const NAME_COMMAND: &str = "settings get global device_name 2>/dev/null; settings get system device_name 2>/dev/null; getprop ro.product.marketname";
const PACKAGE: &str = "com.kuiwaiwai.plumelink";
const LEGACY_USB_UNAVAILABLE: &str = "电脑后台的 USB 组件较旧。请仅连接一台 USB 手机并允许调试，或重启为新版 ADB 服务后重试。";
pub(crate) const PORT_IN_USE: &str = "USB 入口被占用，请断开其它 USB 遥控工具后重试";
const CLEANUP_PENDING: &str = "USB 连接清理未完成，将自动重试";
pub(crate) const RESULT_UNCONFIRMED: &str = "USB 连接结果尚未确认，请重新检测后重试";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub connection_unverified: bool,
    pub id: String,
    pub name: String,
    pub state: String,
    pub linked: bool,
    pub selected: bool,
    pub error: String,
    pub endpoint: String,
    #[serde(skip)]
    serial: String,
    #[serde(skip)]
    transport: String,
}
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub available: bool,
    pub devices: Vec<Device>,
    pub error: String,
    pub busy: bool,
    pub active_count: usize,
    pub endpoint: String,
    pub endpoints: Vec<String>,
    pub fingerprint: String,
    pub code: String,
}
#[derive(Clone)]
struct Mapping { id: String, name: String, serial: String, transport: String, port: u16, path: PathBuf }
#[derive(Default)]
struct State { status: Status, mappings: HashMap<String, Mapping>, cleanup: HashMap<(String,u16), Mapping>, quarantine:HashMap<(String,u16),Mapping>, cleanup_cursor:usize, quarantine_cursor:usize, committed_ticket:Option<u64>, connect_ticket:Option<u64>, reconcile:bool, verified_component: Option<(PathBuf,std::time::SystemTime)>, device_names: HashMap<(String,String),(String,std::time::Instant)> }
pub struct Bridge {
    io: std::sync::Arc<dyn AdbIo>,
    operation: tokio::sync::Mutex<()>,
    state: Mutex<State>,
    generation: AtomicU64,
    connect_slots:std::sync::Arc<tokio::sync::Semaphore>,
    read_slots:std::sync::Arc<tokio::sync::Semaphore>,
    component_slots:std::sync::Arc<tokio::sync::Semaphore>,
}
struct Busy<'a>(&'a Bridge);
impl Drop for Busy<'_> {
    fn drop(&mut self) { self.0.state.lock().unwrap().status.busy=false; }
}
trait AdbIo: Send + Sync {
    fn device_name<'a>(&'a self, _path: &'a Path, _transport: &'a str) -> BoxFuture<'a, Option<String>> {
        Box::pin(async { None })
    }
    fn run<'a>(&'a self, path: &'a Path, args: &'a [&'a str]) -> BoxFuture<'a,Result<String,String>>;
    fn create<'a>(&'a self,path:&'a Path,args:&'a [&'a str])->BoxFuture<'a,Result<(),CreateFailure>> {
        Box::pin(async move {self.run(path,args).await.map(|_|()).map_err(|error|if error==PORT_IN_USE {CreateFailure::Conflict}else{CreateFailure::Unconfirmed})})
    }
    fn devices(&self) -> BoxFuture<'_,Result<Vec<Device>,String>>;
    fn ensure_server<'a>(&'a self,path:&'a Path) -> BoxFuture<'a,Result<(),String>>;
}
enum CreateFailure {NotStarted(String),Conflict,Unconfirmed}
enum CommandFailure {NotStarted(String),Conflict,Unconfirmed(String)}
impl CommandFailure {
    fn message(self)->String {match self {Self::NotStarted(error)|Self::Unconfirmed(error)=>error,Self::Conflict=>PORT_IN_USE.into()}}
}
struct SystemAdb;
impl AdbIo for SystemAdb {
    fn device_name<'a>(&'a self, path: &'a Path, transport: &'a str) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move {
            let output = run(path, &["-t", transport, "shell", NAME_COMMAND]).await.ok()?;
            readable_device_name(&output)
        })
    }
    fn run<'a>(&'a self,path:&'a Path,args:&'a [&'a str])->BoxFuture<'a,Result<String,String>> { Box::pin(run(path,args)) }
    fn create<'a>(&'a self,path:&'a Path,args:&'a [&'a str])->BoxFuture<'a,Result<(),CreateFailure>> {Box::pin(async move {
        run_checked(path,args).await.map(|_|()).map_err(|error|match error {CommandFailure::NotStarted(error)=>CreateFailure::NotStarted(error),CommandFailure::Conflict=>CreateFailure::Conflict,CommandFailure::Unconfirmed(_)=>CreateFailure::Unconfirmed})
    })}
    fn devices(&self)->BoxFuture<'_,Result<Vec<Device>,String>> { Box::pin(devices()) }
    fn ensure_server<'a>(&'a self,path:&'a Path)->BoxFuture<'a,Result<(),String>> { Box::pin(ensure_server_at(path, ADB_PORT)) }
}
impl Default for Bridge {
    fn default() -> Self { Self { io:std::sync::Arc::new(SystemAdb),operation:tokio::sync::Mutex::new(()),state:Mutex::new(State::default()),generation:AtomicU64::new(0),connect_slots:std::sync::Arc::new(tokio::sync::Semaphore::new(CONNECT_LIMIT)),read_slots:std::sync::Arc::new(tokio::sync::Semaphore::new(READ_LIMIT)),component_slots:std::sync::Arc::new(tokio::sync::Semaphore::new(1)) } }
}

const COMPONENT_MISSING: &str = "内置 USB 组件缺失，请重新安装电脑端羽连";
const COMPONENT_FAILED: &str = "无法运行 USB 组件，请重新安装电脑端羽连";

fn explicit_component(configured: &str) -> Option<PathBuf> {
    let path = PathBuf::from(configured);
    (path.is_absolute() && path.is_file() && path.file_name().is_some_and(|n| n.eq_ignore_ascii_case("adb.exe"))).then_some(path)
}

fn bundled_component_at(directory: &Path) -> Option<PathBuf> {
    let directory = directory.join("usb");
    (directory.is_absolute() && ["adb.exe", "AdbWinApi.dll", "AdbWinUsbApi.dll", "NOTICE.txt"].iter()
        .all(|name| directory.join(name).is_file())).then(|| directory.join("adb.exe"))
}

fn bundled_component() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|exe| exe.parent().and_then(bundled_component_at))
}

fn component(configured: &str) -> Option<PathBuf> {
    if let Some(path) = bundled_component() { return Some(path); }
    if !configured.is_empty() { return explicit_component(configured); }
    let mut candidates = Vec::new();
    for key in ["ANDROID_SDK_ROOT", "ANDROID_HOME"] {
        if let Some(root) = std::env::var_os(key) { candidates.push(PathBuf::from(root).join("platform-tools/adb.exe")); }
    }
    if let Some(root) = std::env::var_os("LOCALAPPDATA") { candidates.push(PathBuf::from(root).join("Android/Sdk/platform-tools/adb.exe")); }
    if let Some(paths) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&paths).filter(|p| p.is_absolute()).map(|p| p.join("adb.exe")));
    }
    candidates.into_iter().find_map(|p| explicit_component(p.to_str()?))
}

pub async fn verify_bundled_component() -> Result<String, String> {
    let path = bundled_component().ok_or(COMPONENT_MISSING)?;
    let version = run(&path, &["version"]).await?;
    if !version.starts_with("Android Debug Bridge version 1.0.41") { return Err(COMPONENT_FAILED.into()); }
    Ok(version.lines().take(2).collect::<Vec<_>>().join("\n"))
}
async fn bounded(mut input: impl AsyncRead + Unpin) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    (&mut input).take((OUTPUT_LIMIT + 1) as u64).read_to_end(&mut bytes).await.map_err(|_| "USB 组件响应异常")?;
    if bytes.len() > OUTPUT_LIMIT { return Err("USB 组件响应过大".into()); }
    Ok(bytes)
}
async fn run(path: &Path, args: &[&str]) -> Result<String, String> {
    run_checked(path,args).await.map_err(CommandFailure::message)
}
async fn run_checked(path:&Path,args:&[&str])->Result<String,CommandFailure> {
    run_checked_at(path, args, ADB_PORT).await
}
fn adb_command(path: &Path, args: &[&str], port: u16) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(path);
    if args != ["start-server"] { command.args(["-H", "127.0.0.1"]); }
    command.arg("-P").arg(port.to_string()).args(args)
        .env_remove("ADB_SERVER_SOCKET").env_remove("ANDROID_SERIAL").env_remove("ADB_TRACE")
        .env_remove("ANDROID_ADB_SERVER_ADDRESS").env_remove("ANDROID_ADB_SERVER_PORT")
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)] command.creation_flags(0x08000000);
    command
}
async fn run_checked_at(path:&Path,args:&[&str],port:u16)->Result<String,CommandFailure> {
    let mut child = adb_command(path,args,port).spawn().map_err(|_|CommandFailure::NotStarted(COMPONENT_FAILED.into()))?;
    let stdout = child.stdout.take().ok_or_else(||CommandFailure::Unconfirmed("USB 组件响应异常".into()))?;
    let stderr = child.stderr.take().ok_or_else(||CommandFailure::Unconfirmed("USB 组件响应异常".into()))?;
    let operation = async {
        let (output, error, status) = tokio::try_join!(bounded(stdout), bounded(stderr), async { child.wait().await.map_err(|_| "USB 组件响应异常".to_string()) }).map_err(CommandFailure::Unconfirmed)?;
        if !status.success() {
            let message = String::from_utf8_lossy(&error).to_ascii_lowercase();
            return Err(if message.contains("cannot rebind") || message.contains("address already in use") || message.contains("cannot bind listener") {CommandFailure::Conflict} else {CommandFailure::Unconfirmed("USB 操作失败，请检查手机授权和数据线".into())});
        }
        decode_command_output(args,&output).map_err(CommandFailure::Unconfirmed)
    };
    let startup = args == ["start-server"];
    tokio::time::timeout(if startup { STARTUP_TIMEOUT } else { TIMEOUT }, operation).await
        .map_err(|_|CommandFailure::Unconfirmed(if startup { SERVER_START_TIMEOUT } else { "USB 操作超时，请重试" }.into()))?
}
fn decode_command_output(args:&[&str],output:&[u8])->Result<String,String> {
    let end=if args==["version"] {
        // Windows ADB prints the executable path in the local code page after the version lines.
        output.iter().enumerate().filter(|(_,byte)|**byte==b'\n').nth(1).map_or(output.len(),|(index,_)|index+1)
    } else {output.len()};
    std::str::from_utf8(&output[..end]).map(str::to_owned).map_err(|_|"USB 组件响应异常".into())
}
async fn ensure_server_at(path:&Path,port:u16)->Result<(),String> {
    match tokio::time::timeout(TIMEOUT,TcpStream::connect((std::net::Ipv4Addr::LOCALHOST,port))).await {
        Ok(Ok(_))=>{},
        Ok(Err(error)) if error.kind()==std::io::ErrorKind::ConnectionRefused=>{
            run_checked_at(path,&["start-server"],port).await.map_err(|error|match error.message().as_str() {
                SERVER_START_TIMEOUT => SERVER_START_TIMEOUT,
                COMPONENT_FAILED => COMPONENT_FAILED,
                _ => SERVER_START_FAILED,
            })?;
        },
        _=>return Err(SERVER_RESPONSE_FAILED.into()),
    }
    let version=adb_query_checked_at("host:version",port).await.map_err(|_|SERVER_RESPONSE_FAILED)?;
    if version != b"0029" { return Err("请先退出使用旧版 ADB 的程序，再重新检测 USB 连接".into()); }
    Ok(())
}
#[derive(Debug)]
enum AdbQueryFailure { UnsupportedService, Rejected, Failed(String) }
impl AdbQueryFailure {
    fn message(self) -> String {
        match self {
            Self::UnsupportedService | Self::Rejected => LEGACY_USB_UNAVAILABLE.into(),
            Self::Failed(message) => message,
        }
    }
}
impl From<&str> for AdbQueryFailure {
    fn from(message: &str) -> Self { Self::Failed(message.into()) }
}
async fn adb_query_checked_at(query: &str, port:u16) -> Result<Vec<u8>, AdbQueryFailure> {
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut socket = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).await.map_err(|_| "USB 组件尚未启动")?;
        socket.write_all(format!("{:04x}{query}", query.len()).as_bytes()).await.map_err(|_| "USB 组件响应异常")?;
        read_adb_response(&mut socket).await
    }).await.map_err(|_| AdbQueryFailure::from("USB 操作超时，请重试"))?
}
async fn read_adb_response(input: &mut (impl AsyncRead + Unpin)) -> Result<Vec<u8>, AdbQueryFailure> {
    let mut header = [0; 4];
    input.read_exact(&mut header).await.map_err(|_| "USB 组件响应异常")?;
    if &header != b"OKAY" && &header != b"FAIL" { return Err("USB 组件响应异常".into()); }
    let rejected = &header == b"FAIL";
    input.read_exact(&mut header).await.map_err(|_| "USB 组件响应异常")?;
    if !header.iter().all(u8::is_ascii_hexdigit) { return Err("USB 组件响应异常".into()); }
    let length = std::str::from_utf8(&header).ok().and_then(|s| usize::from_str_radix(s,16).ok()).ok_or("USB 组件响应异常")?;
    if length > OUTPUT_LIMIT { return Err("USB 组件响应过大".into()); }
    let mut bytes = vec![0; length]; input.read_exact(&mut bytes).await.map_err(|_| "USB 组件响应异常")?;
    if rejected { return Err(classify_adb_rejection(&bytes)); }
    Ok(bytes)
}
fn classify_adb_rejection(bytes: &[u8]) -> AdbQueryFailure {
    let Ok(reason) = std::str::from_utf8(bytes) else { return AdbQueryFailure::from("USB 组件响应异常"); };
    if reason.trim().eq_ignore_ascii_case("unknown host service") {
        AdbQueryFailure::UnsupportedService
    } else { AdbQueryFailure::Rejected }
}
trait DeviceQueries: Sync {
    fn query<'a>(&'a self, value: &'a str) -> BoxFuture<'a, Result<Vec<u8>, AdbQueryFailure>>;
}
struct SystemQueries(u16);
impl DeviceQueries for SystemQueries {
    fn query<'a>(&'a self, value: &'a str) -> BoxFuture<'a, Result<Vec<u8>, AdbQueryFailure>> {
        Box::pin(adb_query_checked_at(value,self.0))
    }
}
fn legacy_query_error(error: AdbQueryFailure) -> String {
    match error {
        AdbQueryFailure::UnsupportedService | AdbQueryFailure::Rejected => LEGACY_USB_UNAVAILABLE.into(),
        AdbQueryFailure::Failed(message) => message,
    }
}
fn legacy_usb_serial(bytes: &[u8]) -> Result<String, String> {
    let value = std::str::from_utf8(bytes).map_err(|_| "USB 组件响应异常")?.trim();
    if value.is_empty() || value.len() > 256 || value.eq_ignore_ascii_case("unknown")
        || value.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(LEGACY_USB_UNAVAILABLE.into());
    }
    Ok(value.into())
}
fn parse_legacy_devices(bytes: &[u8], usb_serial: &str) -> Result<Vec<Device>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "USB 组件响应异常")?;
    let mut serials = std::collections::HashSet::new();
    let mut transports = std::collections::HashSet::new();
    let mut selected = None;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        if serials.len() >= 128 || line.chars().any(|c| c.is_control() && c != '\t') { return Err("USB 组件响应异常".into()); }
        let mut fields = line.split_whitespace();
        let serial = fields.next().ok_or("USB 组件响应异常")?;
        let state = fields.next().ok_or("USB 组件响应异常")?;
        if serial.len() > 256 || !serials.insert(serial) { return Err("USB 设备信息重复，请重新插接手机后重试".into()); }
        let (mut transport, mut model) = (None, None);
        for field in fields {
            if let Some(value) = field.strip_prefix("transport_id:") {
                let id = value.parse::<u64>().ok().filter(|id| *id > 0 && id.to_string() == value).ok_or(LEGACY_USB_UNAVAILABLE)?;
                if transport.replace(id.to_string()).is_some() || !transports.insert(id) { return Err("USB 设备信息重复，请重新插接手机后重试".into()); }
            } else if let Some(value) = field.strip_prefix("model:") {
                if value.len() > 256 || model.replace(value).is_some() { return Err("USB 组件响应异常".into()); }
            }
        }
        if serial == usb_serial {
            let transport = transport.ok_or(LEGACY_USB_UNAVAILABLE)?;
            selected = Some(Device { connection_unverified:false, id:String::new(), serial:serial.into(), transport,
                name:model.filter(|value| !value.is_empty()).unwrap_or("Android").replace('_', " "),
                state:match state { "device" => "ready", "unauthorized" => "unauthorized", "offline" => "offline", _ => "unavailable" }.into(),
                linked:false, selected:false, error:String::new(), endpoint:String::new() });
        }
    }
    selected.map(|device| vec![device]).ok_or_else(|| LEGACY_USB_UNAVAILABLE.into())
}
async fn discover_devices(queries: &impl DeviceQueries) -> Result<Vec<Device>, String> {
    match queries.query("host:track-devices-proto-binary").await {
        Ok(bytes) => parse_devices(&bytes),
        Err(AdbQueryFailure::UnsupportedService) => {
            let serial = match queries.query("host-usb:get-serialno").await {
                Ok(bytes) => legacy_usb_serial(&bytes)?,
                Err(AdbQueryFailure::Rejected) => {
                    let list = queries.query("host:devices-l").await.map_err(legacy_query_error)?;
                    let text = std::str::from_utf8(&list).map_err(|_| "USB 组件响应异常")?;
                    let mut devices = Vec::new();
                    for line in text.lines().filter(|line| !line.trim().is_empty()) {
                        if devices.len() >= 128 { return Err("USB 组件响应过大".into()); }
                        let serial = line.split_whitespace().next().ok_or("USB 组件响应异常")?;
                        let mut device = parse_legacy_devices(&list, serial)?.remove(0);
                        device.connection_unverified = true;
                        devices.push(device);
                    }
                    if queries.query("host:devices-l").await.map_err(legacy_query_error)? != list {
                        return Err("手机列表已更新，请重新选择手机".into());
                    }
                    return Ok(devices);
                }
                Err(error) => return Err(legacy_query_error(error)),
            };
            let devices = parse_legacy_devices(&queries.query("host:devices-l").await.map_err(legacy_query_error)?, &serial)?;
            let verified = legacy_usb_serial(&queries.query("host-usb:get-serialno").await.map_err(legacy_query_error)?)?;
            if verified != serial { return Err("手机列表已更新，请重新选择手机".into()); }
            Ok(devices)
        }
        Err(error) => Err(error.message()),
    }
}
fn device_list_error(error:String)->String {
    if error == "USB 组件响应异常" { DEVICE_LIST_FAILED.into() } else { error }
}
fn readable_device_name(output: &str) -> Option<String> {
    output.lines().take(3).find_map(|line| {
        let name = line.trim();
        if name.is_empty() || name.eq_ignore_ascii_case("null") || name.eq_ignore_ascii_case("unknown")
            || name.chars().any(char::is_control) { return None; }
        Some(name.chars().take(40).collect())
    })
}
async fn devices() -> Result<Vec<Device>, String> { discover_devices(&SystemQueries(ADB_PORT)).await.map_err(device_list_error) }

pub async fn diagnose_bundled_usb(port:u16)->serde_json::Value {
    let Some(path)=bundled_component() else {return serde_json::json!({"ok":false,"stage":"component","error":COMPONENT_MISSING});};
    let version=match run_checked_at(&path,&["version"],port).await {
        Ok(version) if version.starts_with("Android Debug Bridge version 1.0.41") => version.lines().take(2).collect::<Vec<_>>().join("\n"),
        _=>return serde_json::json!({"ok":false,"stage":"component","error":COMPONENT_FAILED}),
    };
    if let Err(error)=ensure_server_at(&path,port).await {return serde_json::json!({"ok":false,"stage":"server","clientVersion":version,"error":error});}
    match discover_devices(&SystemQueries(port)).await {
        Ok(devices)=>serde_json::json!({"ok":true,"stage":"ready","clientVersion":version,"deviceCount":devices.len(),
            "ready":devices.iter().filter(|d|d.state=="ready").count(),"unauthorized":devices.iter().filter(|d|d.state=="unauthorized").count(),
            "offline":devices.iter().filter(|d|d.state=="offline").count()}),
        Err(error)=>serde_json::json!({"ok":false,"stage":"devices","clientVersion":version,"error":device_list_error(error)}),
    }
}

struct Proto<'a> { bytes: &'a [u8], position: usize }
impl<'a> Proto<'a> {
    fn number(&mut self) -> Result<u64, String> {
        let mut value = 0;
        for shift in (0..70).step_by(7) {
            let byte = *self.bytes.get(self.position).ok_or("USB 组件响应异常")?; self.position += 1;
            if shift == 63 && byte > 1 { return Err("USB 组件响应异常".into()); }
            value |= u64::from(byte & 127) << shift;
            if byte & 128 == 0 { return Ok(value); }
        }
        Err("USB 组件响应异常".into())
    }
    fn block(&mut self) -> Result<&'a [u8], String> {
        let size = usize::try_from(self.number()?).map_err(|_| "USB 组件响应异常")?;
        let end = self.position.checked_add(size).filter(|end| *end <= self.bytes.len()).ok_or("USB 组件响应异常")?;
        let data = &self.bytes[self.position..end]; self.position = end; Ok(data)
    }
    fn skip(&mut self, kind: u64) -> Result<(), String> {
        match kind {
            0 => { self.number()?; },
            2 => { self.block()?; },
            1 | 5 => { self.position = self.position.checked_add(if kind == 1 {8} else {4}).filter(|p| *p <= self.bytes.len()).ok_or("USB 组件响应异常")?; },
            _ => return Err("USB 组件响应异常".into()),
        }
        Ok(())
    }
}
fn parse_devices(bytes: &[u8]) -> Result<Vec<Device>, String> {
    let mut outer = Proto { bytes, position: 0 }; let mut devices = Vec::new(); let mut count = 0;
    while outer.position < bytes.len() {
        let tag = outer.number()?;
        if tag >> 3 == 0 { return Err("USB 组件响应异常".into()); }
        if tag != 10 { outer.skip(tag & 7)?; continue; }
        count += 1; if count > 128 { return Err("USB 组件响应过大".into()); }
        let mut record = Proto { bytes: outer.block()?, position: 0 };
        let (mut serial, mut model, mut state, mut kind, mut id) = (String::new(),String::new(),0,0,0);
        while record.position < record.bytes.len() {
            let tag = record.number()?;
            if tag >> 3 == 0 { return Err("USB 组件响应异常".into()); }
            match tag {
                10 => serial = std::str::from_utf8(record.block()?).map_err(|_| "USB 组件响应异常")?.to_string(),
                42 => model = std::str::from_utf8(record.block()?).map_err(|_| "USB 组件响应异常")?.to_string(),
                16 => state = record.number()?,
                56 => kind = record.number()?,
                80 => id = record.number()?,
                _ => record.skip(tag & 7)?,
            }
        }
        if serial.len() > 256 || model.len() > 256 || id == 0 { return Err("USB 组件响应异常".into()); }
        if kind == 0 { return Err(LEGACY_USB_UNAVAILABLE.into()); }
        if kind != 1 { continue; }
        if serial.is_empty() || serial.chars().any(char::is_control) { return Err("USB 组件响应异常".into()); }
        let status = match state { 8 => "ready", 2 | 3 => "unauthorized", 6 => "offline", _ => "unavailable" };
        if devices.iter().any(|device: &Device|device.serial==serial || device.transport==id.to_string()) { return Err("USB 设备信息重复，请重新插接手机后重试".into()); }
        devices.push(Device { connection_unverified:false, id: String::new(), transport: id.to_string(), name: if model.is_empty() { "Android".into() } else { model.replace('_'," ").chars().filter(|c| !c.is_control()).collect() }, state: status.into(), linked: false, selected: false, error: String::new(), endpoint:String::new(), serial });
    }
    Ok(devices)
}
fn owns_mapping(list: &str, port: u16) -> bool {
    let endpoint = format!("tcp:{port}");
    list.lines().any(|line| { let fields: Vec<_> = line.split_whitespace().collect(); fields.len() == 3 && fields[1] == endpoint && fields[2] == endpoint })
}
impl Bridge {
    async fn resolve_device_names(&self, path: &Path, devices: &mut [Device]) {
        let cached = {
            let mut state = self.state.lock().unwrap();
            state.device_names.retain(|key, _| devices.iter().any(|device| device.serial == key.0 && device.transport == key.1));
            state.device_names.clone()
        };
        let mut pending = Vec::new();
        for (index, device) in devices.iter_mut().enumerate() {
            let key = (device.serial.clone(), device.transport.clone());
            if let Some((name, checked)) = cached.get(&key) {
                device.name.clone_from(name);
                if checked.elapsed() < NAME_CACHE_TTL { continue; }
            }
            if device.state == "ready" && pending.len() < CONNECT_LIMIT {
                pending.push((index, key, device.name.clone()));
            }
        }
        let mut queries = futures_util::stream::iter(pending).map(|(index, key, fallback)| async move {
            let name = tokio::time::timeout(NAME_TIMEOUT, self.io.device_name(path, &key.1)).await.ok().flatten().unwrap_or(fallback);
            (index, key, name)
        }).buffer_unordered(4);
        while let Some((index, key, name)) = queries.next().await {
            devices[index].name.clone_from(&name);
            self.state.lock().unwrap().device_names.insert(key, (name, std::time::Instant::now()));
        }
    }
    async fn verify_device(&self,serial:&str,generation:Option<u64>)->Result<(),String> {
        let devices = self.io.devices().await?;
        if generation.is_some_and(|generation|!self.is_current(generation)) { return Err("USB 连接已取消".into()); }
        let mut matching = devices.iter().filter(|device|device.serial==serial);
        if !matching.next().is_some_and(|device|device.state=="ready") || matching.next().is_some() {
            return Err("手机已断开或尚未授权，请重新检测".into());
        }
        Ok(())
    }
    async fn run_device(&self, path: &Path, serial: &str, args: &[&str], generation: Option<u64>) -> Result<String, String> {
        self.verify_device(serial,generation).await?;
        let mut selected = vec!["-s",serial]; selected.extend_from_slice(args);
        self.io.run(path,&selected).await
    }
    async fn create_mapping(&self,path:&Path,mapping:&Mapping,generation:u64)->Result<(),String> {
        self.verify_device(&mapping.serial,Some(generation)).await?;
        let endpoint=format!("tcp:{}",mapping.port);
        match self.io.create(path,&["-s",&mapping.serial,"reverse","--no-rebind",&endpoint,&endpoint]).await {
            Ok(())=>Ok(()),
            Err(CreateFailure::NotStarted(error))=>Err(error),
            Err(CreateFailure::Conflict)=>Err(PORT_IN_USE.into()),
            Err(CreateFailure::Unconfirmed)=>{
                let mut state=self.state.lock().unwrap();
                if state.mappings.get(&mapping.serial).is_some_and(|old|old.id==mapping.id&&old.port==mapping.port) {state.mappings.remove(&mapping.serial);}
                state.cleanup.remove(&(mapping.serial.clone(),mapping.port));
                state.quarantine.insert((mapping.serial.clone(),mapping.port),mapping.clone());
                Err(RESULT_UNCONFIRMED.into())
            }
        }
    }
    async fn remove(&self, path: &Path, mapping: &Mapping) -> bool {
        if let Ok(list) = self.run_device(path, &mapping.serial, &["reverse", "--list"],None).await {
            return !owns_mapping(&list,mapping.port) || self.run_device(path, &mapping.serial, &["reverse", "--remove", &format!("tcp:{}",mapping.port)],None).await.is_ok();
        }
        false
    }
    fn queue_cleanup(&self,mapping:&Mapping) {
        self.state.lock().unwrap().cleanup.insert((mapping.serial.clone(),mapping.port),mapping.clone());
    }
    async fn cleanup_component(&self,configured:&str,original:&Path)->Option<PathBuf> {
        if let Ok(Ok(path))=tokio::time::timeout(COMPONENT_TIMEOUT,self.available(configured)).await {return Some(path);}
        let original=original.to_str()?;
        if component(configured).as_deref()==Some(Path::new(original)) {return None;}
        tokio::time::timeout(COMPONENT_TIMEOUT,self.available(original)).await.ok()?.ok()
    }
    async fn cleanup(&self,configured:&str,serial:Option<&str>) {
        let pending={
            let mut state=self.state.lock().unwrap();
            let mut pending:Vec<_>=state.cleanup.values().filter(|mapping|serial.is_none_or(|serial|mapping.serial==serial)).cloned().collect();
            if pending.is_empty() {return;}
            pending.sort_by(|a,b|(&a.serial,a.port).cmp(&(&b.serial,b.port)));
            let start=state.cleanup_cursor%pending.len();pending.rotate_left(start);pending.truncate(CLEANUP_BATCH);
            state.cleanup_cursor=state.cleanup_cursor.wrapping_add(CLEANUP_BATCH);pending
        };
        let work=async {
            let mut running=FuturesUnordered::new();
            for mapping in pending {running.push(async move {
                if let Some(path)=self.cleanup_component(configured,&mapping.path).await {
                    if self.remove(&path,&mapping).await {self.state.lock().unwrap().cleanup.remove(&(mapping.serial,mapping.port));}
                }
            });}
            while running.next().await.is_some() {}
        };
        let _=tokio::time::timeout(CLEANUP_TIMEOUT,work).await;
    }
    async fn inspect_quarantine(&self,configured:&str) {
        let pending={
            let mut state=self.state.lock().unwrap();let mut pending:Vec<_>=state.quarantine.values().cloned().collect();
            if pending.is_empty(){return;}
            pending.sort_by(|a,b|(&a.serial,a.port).cmp(&(&b.serial,b.port)));
            let start=state.quarantine_cursor%pending.len();pending.rotate_left(start);pending.truncate(CLEANUP_BATCH);
            state.quarantine_cursor=state.quarantine_cursor.wrapping_add(CLEANUP_BATCH);pending
        };
        let work=async {
            let mut running=FuturesUnordered::new();
            for mapping in pending {running.push(async move {
                if let Some(path)=self.cleanup_component(configured,&mapping.path).await {
                    if let Ok(list)=self.run_device(&path,&mapping.serial,&["reverse","--list"],None).await {
                        if !owns_mapping(&list,mapping.port) {self.state.lock().unwrap().quarantine.remove(&(mapping.serial,mapping.port));}
                    }
                }
            });}
            while running.next().await.is_some(){}
        };
        let _=tokio::time::timeout(CLEANUP_TIMEOUT,work).await;
    }
    fn snapshot(state:&State)->Status {
        let mut status=state.status.clone();status.active_count=state.mappings.len();
        for device in &mut status.devices {
            device.selected=state.mappings.contains_key(&device.serial);device.linked&=device.selected;
            if !device.selected&&device.error.is_empty()&&state.quarantine.values().any(|mapping|mapping.serial==device.serial) {device.error=RESULT_UNCONFIRMED.into();}
        }
        if status.error.is_empty() && !state.cleanup.is_empty() {status.error=CLEANUP_PENDING.into();}
        status
    }
    pub fn status(&self) -> Status {
        let mut status=Self::snapshot(&self.state.lock().unwrap());status.busy|=self.connect_busy();status
    }
    pub(crate) fn connect_slot(&self)->Result<tokio::sync::OwnedSemaphorePermit,String> {self.connect_slots.clone().try_acquire_owned().map_err(|_|"USB 操作较多，请稍后重试".into())}
    pub(crate) fn read_slot(&self)->Option<tokio::sync::OwnedSemaphorePermit> {self.read_slots.clone().try_acquire_owned().ok()}
    pub fn component_slot(&self)->Result<tokio::sync::OwnedSemaphorePermit,String> {self.component_slots.clone().try_acquire_owned().map_err(|_|"USB 操作较多，请稍后重试".into())}
    pub(crate) fn connect_busy(&self)->bool {self.connect_slots.available_permits()<CONNECT_LIMIT}
    pub fn cancel(&self) { let _state=self.state.lock().unwrap();self.generation.fetch_add(1, Ordering::SeqCst); }
    pub fn cancel_connection(&self) {
        let state=self.state.lock().unwrap();
        if let Some(ticket)=state.connect_ticket.filter(|ticket|state.committed_ticket!=Some(*ticket)&&self.connect_busy()) {let _=self.generation.compare_exchange(ticket,ticket.wrapping_add(1),Ordering::SeqCst,Ordering::SeqCst);}
    }
    pub(crate) fn finish_connection(&self,ticket:u64) {let mut state=self.state.lock().unwrap();if state.connect_ticket==Some(ticket) {state.connect_ticket=None;}}
    pub(crate) fn cancel_ticket(&self,ticket:u64) {
        let state=self.state.lock().unwrap_or_else(|error|error.into_inner());
        if state.committed_ticket!=Some(ticket) {let _=self.generation.compare_exchange(ticket,ticket.wrapping_add(1),Ordering::SeqCst,Ordering::SeqCst);}
    }
    pub fn connection_ticket(&self) -> u64 { let mut state=self.state.lock().unwrap();let ticket=self.generation.fetch_add(1,Ordering::SeqCst) + 1;state.connect_ticket=Some(ticket);ticket }
    pub fn is_current(&self, ticket: u64) -> bool { ticket == self.generation.load(Ordering::SeqCst) }
    pub fn has_mappings(&self) -> bool { !self.state.lock().unwrap().mappings.is_empty() }
    pub fn selected_ids(&self)->Vec<String> {self.state.lock().unwrap().mappings.values().map(|mapping|mapping.id.clone()).collect()}
    pub fn needs_refresh(&self)->bool {let state=self.state.lock().unwrap();state.reconcile||!state.mappings.is_empty()||!state.cleanup.is_empty()||!state.quarantine.is_empty()}
    pub(crate) fn require_reconcile(&self) {self.state.lock().unwrap().reconcile=true;}
    pub(crate) fn finish_reconcile(&self) {self.state.lock().unwrap().reconcile=false;}
    pub async fn configure(&self, configured: &str) -> Result<(), String> {
        let path = explicit_component(configured).ok_or(COMPONENT_MISSING)?;
        let version = self.io.run(&path, &["version"]).await?;
        if !version.starts_with("Android Debug Bridge version 1.0.41") { return Err(COMPONENT_FAILED.into()); }
        Ok(())
    }
    async fn available(&self, configured: &str) -> Result<PathBuf, String> {
        let path = component(configured).ok_or(COMPONENT_MISSING)?;
        let modified = std::fs::metadata(&path).and_then(|m|m.modified()).map_err(|_| COMPONENT_FAILED)?;
        let verified = self.state.lock().unwrap().verified_component.as_ref().is_some_and(|(p,time)|p == &path && *time == modified);
        if !verified {
            let version = self.io.run(&path,&["version"]).await?;
            if !version.starts_with("Android Debug Bridge version 1.0.41") { return Err(COMPONENT_FAILED.into()); }
            self.state.lock().unwrap().verified_component=Some((path.clone(),modified));
        }
        self.io.ensure_server(&path).await?;
        Ok(path)
    }
    pub async fn refresh(&self, configured: &str, port: Option<u16>) -> Status {
        self.refresh_ports(configured,&port.into_iter().collect::<Vec<_>>()).await
    }
    pub async fn refresh_ports(&self,configured:&str,ports:&[u16])->Status {
        let Ok(_operation) = self.operation.try_lock() else { return self.status(); };
        let generation = self.generation.load(Ordering::SeqCst);
        self.cleanup(configured,None).await;
        self.inspect_quarantine(configured).await;
        let (saved, previous, quarantined) = { let state=self.state.lock().unwrap();(state.mappings.values().cloned().collect::<Vec<_>>(),state.status.devices.clone(),state.quarantine.values().cloned().collect::<Vec<_>>()) };
        let result = async {
            let path = self.available(configured).await?;
            let mut devices = self.io.devices().await?;
            self.resolve_device_names(&path, &mut devices).await;
            if !self.is_current(generation) { return Ok(None); }
            for device in &mut devices {
                device.id = saved.iter().find(|old|old.serial==device.serial).map(|old|old.id.clone())
                    .or_else(||quarantined.iter().find(|old|old.serial==device.serial).map(|old|old.id.clone()))
                    .or_else(||previous.iter().find(|old|old.serial==device.serial&&old.transport==device.transport).map(|old|old.id.clone())).unwrap_or_default();
                if device.id.is_empty() { let mut id=[0u8;16];getrandom::fill(&mut id).map_err(|_|"无法生成 USB 设备标识，请重试")?;device.id=id.iter().map(|byte|format!("{byte:02x}")).collect(); }
            }
            for old in &saved {
                if !self.is_current(generation) { return Ok(None); }
                if let Some(device) = devices.iter_mut().find(|d| d.serial == old.serial) {
                    device.selected = true;
                    if ports.contains(&old.port) && device.state=="ready" {
                        let candidate=Mapping{transport:device.transport.clone(),name:device.name.clone(),path:path.clone(),..old.clone()};
                        let mut created=false;
                        let restored = async {
                            let list = self.run_device(&path,&device.serial,&["reverse","--list"],Some(generation)).await?;
                            if !self.is_current(generation) { return Err("USB 连接已取消".to_string()); }
                            if !owns_mapping(&list,old.port) {
                                self.create_mapping(&path,&candidate,generation).await?;
                                created=true;
                            }
                            Ok::<_,String>(())
                        }.await;
                        if !self.is_current(generation) {
                            if created { self.queue_cleanup(old);self.cleanup(configured,Some(&old.serial)).await; }
                            return Ok(None);
                        }
                        match restored {
                            Ok(()) => {
                                let mapping = Mapping { transport: device.transport.clone(), name:device.name.clone(), path:path.clone(), ..old.clone() };
                                let committed={let mut state=self.state.lock().unwrap();if self.is_current(generation)&&state.mappings.get(&mapping.serial).is_some_and(|current|current.id==old.id&&current.port==old.port) {state.mappings.insert(mapping.serial.clone(),mapping);true}else{false}};
                                if !committed {if created {self.queue_cleanup(old);self.cleanup(configured,Some(&old.serial)).await;} return Ok(None);}
                                device.linked=true;
                            }
                            Err(error) => device.error=error,
                        }
                    }
                } else {
                    devices.push(Device { connection_unverified:false, id:old.id.clone(),name:old.name.clone(),serial:old.serial.clone(),transport:old.transport.clone(),state:"disconnected".into(),linked:false,selected:true,error:String::new(),endpoint:String::new() });
                }
            }
            for old in &quarantined {
                if !devices.iter().any(|device|device.serial==old.serial) {devices.push(Device{connection_unverified:false,id:old.id.clone(),name:old.name.clone(),serial:old.serial.clone(),transport:old.transport.clone(),state:"disconnected".into(),linked:false,selected:false,error:RESULT_UNCONFIRMED.into(),endpoint:String::new()});}
            }
            Ok::<_,String>(Some(devices))
        }.await;
        let mut state = self.state.lock().unwrap();
        if self.is_current(generation) {
            match result {
                Ok(Some(devices)) => state.status = Status { available:true,devices,error:String::new(),busy:false,active_count:state.mappings.len(),..Status::default() },
                Ok(None) => {},
                Err(error) => {
                    state.status.error=error;state.status.available=component(configured).is_some();
                    for device in &mut state.status.devices {device.state="unavailable".into();device.linked=false;device.selected=saved.iter().any(|old|old.serial==device.serial);}
                    for old in &saved {
                        if !state.status.devices.iter().any(|device|device.serial==old.serial) {state.status.devices.push(Device{connection_unverified:false,id:old.id.clone(),name:old.name.clone(),serial:old.serial.clone(),transport:old.transport.clone(),state:"unavailable".into(),linked:false,selected:true,error:String::new(),endpoint:String::new()});}
                    }
                }
            }
        }
        Self::snapshot(&state)
    }
    #[cfg(test)]
    pub async fn connect(&self, configured:&str,id:&str,port:u16,generation:u64)->Result<(),String> {self.connect_with_access(configured,id,port,generation,&"a".repeat(64)).await}
    pub async fn connect_with_access(&self, configured: &str, id: &str, port: u16, generation: u64, access:&str) -> Result<(), String> {
        if access.len()!=64||!access.bytes().all(|byte|byte.is_ascii_hexdigit()&&!byte.is_ascii_uppercase()){return Err("无法生成 USB 连接凭据，请重试".into());}
        let _operation = self.operation.lock().await;
        self.state.lock().unwrap().status.busy=true;
        let _busy=Busy(self);
        self.connect_inner(configured,id,port,generation,access).await
    }
    async fn connect_inner(&self, configured: &str, id: &str, port: u16, generation: u64, access:&str) -> Result<(), String> {
        if !self.is_current(generation) { return Err("USB 连接已取消".into()); }
        let selected=self.state.lock().unwrap().status.devices.iter().find(|device|device.id==id&&device.state=="ready").cloned().ok_or("手机列表已更新，请重新选择手机")?;
        self.cleanup(configured,Some(&selected.serial)).await;
        let path = self.available(configured).await?;
        let devices=self.io.devices().await?;
        let device = devices.iter().find(|device|device.serial==selected.serial&&device.transport==selected.transport&&device.state=="ready").ok_or("手机列表已更新，请重新选择手机")?;
        if devices.iter().filter(|other|other.serial==device.serial).count()!=1 { return Err("USB 设备信息重复，请重新插接手机后重试".into()); }
        if generation != self.generation.load(Ordering::SeqCst) { return Err("USB 连接已取消".into()); }
        let mapping = Mapping { id:selected.id,name:selected.name,serial:device.serial.clone(),transport:device.transport.clone(),port,path:path.clone() };
        if self.state.lock().unwrap().quarantine.contains_key(&(mapping.serial.clone(),port)) {return Err(PORT_IN_USE.into());}
        let existing = self.state.lock().unwrap().mappings.get(&device.serial).cloned();
        let list = self.run_device(&path,&device.serial,&["reverse","--list"],Some(generation)).await?;
        if !self.is_current(generation) { return Err("USB 连接已取消".into()); }
        let reused = existing.as_ref().is_some_and(|old| old.port == port) && owns_mapping(&list,port);
        if !reused {
            {let state=self.state.lock().unwrap();if state.mappings.len()+state.cleanup.len()+state.quarantine.len()>=128 {return Err(if state.quarantine.is_empty(){CLEANUP_PENDING}else{RESULT_UNCONFIRMED}.into());}}
            self.create_mapping(&path,&mapping,generation).await?;
            self.queue_cleanup(&mapping);
        }
        if generation != self.generation.load(Ordering::SeqCst) {
            if !reused { self.cleanup(configured,Some(&mapping.serial)).await; } return Err("USB 连接已取消".into());
        }
        let component = format!("{PACKAGE}/{PACKAGE}.MainActivity");
        let link = format!("'plumelink://connect?address=127.0.0.1:{port}&usb={access}'");
        let opened = self.run_device(&path,&device.serial,&["shell","am","start","-W","-a","android.intent.action.VIEW","-n",&component,"-d",&link],Some(generation)).await;
        if generation != self.generation.load(Ordering::SeqCst) || opened.as_ref().map_or(true, |text| text.contains("Error:") || text.contains("Exception")) {
            if !reused { self.cleanup(configured,Some(&mapping.serial)).await; }
            return Err(if generation != self.generation.load(Ordering::SeqCst) { "USB 连接已取消" } else { "无法打开手机羽连，请先安装并解锁手机" }.into());
        }
        let committed={let mut state=self.state.lock().unwrap();if self.is_current(generation) {
            state.committed_ticket=Some(generation);
            state.cleanup.remove(&(mapping.serial.clone(),mapping.port));
            if let Some(old)=state.mappings.insert(device.serial.clone(),mapping.clone()).filter(|old|old.port!=port) {state.cleanup.insert((old.serial.clone(),old.port),old);}
            true
        }else{false}};
        if !committed { if !reused {self.cleanup(configured,Some(&mapping.serial)).await;} return Err("USB 连接已取消".into()); }
        self.cleanup(configured,Some(&mapping.serial)).await;
        Ok(())
    }
    pub async fn disconnect(&self, configured: &str, id: Option<&str>) -> Result<(),String> {
        self.validate_disconnect(id)?;self.cancel();self.retire_selection(id)?;self.cleanup_retired(configured).await;Ok(())
    }
    pub(crate) fn validate_disconnect(&self,id:Option<&str>)->Result<(),String> {
        if id.is_some_and(|id|!self.state.lock().unwrap().mappings.values().any(|mapping|mapping.id==id)) {return Err("手机列表已更新，请重新选择手机".into());} Ok(())
    }
    pub(crate) fn retire_selection(&self,id:Option<&str>)->Result<(),String> {
        { let mut state = self.state.lock().unwrap();
            if id.is_some_and(|id|!state.mappings.values().any(|mapping|mapping.id==id)) {return Err("手机列表已更新，请重新选择手机".into());}
            let keys: Vec<_> = state.mappings.iter().filter(|(_,m)| id.is_none_or(|id| m.id == id)).map(|(k,_)|k.clone()).collect();
            for key in keys {if let Some(mapping)=state.mappings.remove(&key) {state.cleanup.insert((mapping.serial.clone(),mapping.port),mapping);}}
            for device in &mut state.status.devices {if id.is_none_or(|id|device.id==id) {device.linked=false;device.selected=false;}}
        }
        Ok(())
    }
    pub(crate) async fn cleanup_retired(&self,configured:&str) {
        let _operation=self.operation.lock().await;
        self.cleanup(configured,None).await;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    #[test]
    fn windows_version_checks_ignore_local_code_page_paths_without_relaxing_other_outputs() {
        let header=b"Android Debug Bridge version 1.0.41\r\nVersion 37.0.3-16887417\r\n";
        for path in [b"Installed as C:\\Profiles\\\xd3\xf0\xc1\xac\\adb.exe\r\n".as_slice(),
            b"Installed as C:\\Apps\\PlumeLink\\adb.exe\r\n".as_slice()] {
            let mut output=header.to_vec();output.extend(path);
            assert_eq!(decode_command_output(&["version"],&output).unwrap().as_bytes(),header);
        }
        assert!(decode_command_output(&["reverse","--list"],b"\xff").is_err());
        assert!(decode_command_output(&["version"],b"\xff\nVersion 37\n").is_err());
        assert_eq!(decode_command_output(&["version"],b"Android Debug Bridge version 1.0.41\n").unwrap(),"Android Debug Bridge version 1.0.41\n");
    }
    #[test]
    fn starting_a_local_server_never_supplies_a_remote_host_or_inherits_server_overrides() {
        let startup=adb_command(Path::new("adb.exe"),&["start-server"],12345);
        let args:Vec<_>=startup.as_std().get_args().map(|arg|arg.to_str().unwrap()).collect();
        assert_eq!(args,["-P","12345","start-server"]);
        for key in ["ADB_SERVER_SOCKET","ANDROID_ADB_SERVER_ADDRESS","ANDROID_ADB_SERVER_PORT","ANDROID_SERIAL","ADB_TRACE"] {
            assert!(startup.as_std().get_envs().any(|(name,value)|name==key&&value.is_none()));
        }
        let client=adb_command(Path::new("adb.exe"),&["-s","test-phone","reverse","--list"],12345);
        let args:Vec<_>=client.as_std().get_args().map(|arg|arg.to_str().unwrap()).collect();
        assert_eq!(args,["-H","127.0.0.1","-P","12345","-s","test-phone","reverse","--list"]);
    }

    #[tokio::test]
    async fn an_unused_windows_port_reaches_server_start_instead_of_timing_out_the_probe() {
        let listener=tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST,0)).await.unwrap();
        let port=listener.local_addr().unwrap().port();drop(listener);
        assert_eq!(ensure_server_at(Path::new("this-component-does-not-exist.exe"),port).await.unwrap_err(),COMPONENT_FAILED);
    }

    #[tokio::test]
    async fn an_existing_server_is_probed_without_starting_or_restarting_it() {
        for (response,expected) in [(b"OKAY00040029".as_slice(),None),(b"HTTP/1.1 200 OK".as_slice(),Some(SERVER_RESPONSE_FAILED)),
            (b"OKAY00040028".as_slice(),Some("请先退出使用旧版 ADB 的程序，再重新检测 USB 连接")),(b"OKAY0004".as_slice(),Some(SERVER_RESPONSE_FAILED))] {
            let listener=tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST,0)).await.unwrap();
            let port=listener.local_addr().unwrap().port();let response=response.to_vec();
            let server=tokio::spawn(async move {
                let (probe,_)=listener.accept().await.unwrap();drop(probe);
                let (mut socket,_)=listener.accept().await.unwrap();
                let mut request=[0;16];socket.read_exact(&mut request).await.unwrap();assert_eq!(&request,b"000chost:version");
                socket.write_all(&response).await.unwrap();
            });
            let result=ensure_server_at(Path::new("this-component-must-not-be-launched.exe"),port).await;
            assert_eq!(result.err().as_deref(),expected);server.await.unwrap();
        }
    }

    #[test]
    fn device_list_errors_keep_actionable_failures_and_identify_malformed_metadata() {
        assert_eq!(device_list_error("USB 组件响应异常".into()),DEVICE_LIST_FAILED);
        assert_eq!(device_list_error(LEGACY_USB_UNAVAILABLE.into()),LEGACY_USB_UNAVAILABLE);
        assert_eq!(device_list_error("USB 操作超时，请重试".into()),"USB 操作超时，请重试");
    }
    #[test]
    fn bundled_component_requires_complete_runtime_next_to_executable() {
        let folder = tempfile::tempdir().unwrap();
        assert!(bundled_component_at(folder.path()).is_none());
        let usb = folder.path().join("usb");
        std::fs::create_dir(&usb).unwrap();
        for name in ["adb.exe", "AdbWinApi.dll", "AdbWinUsbApi.dll", "NOTICE.txt"] {
            std::fs::write(usb.join(name), b"fixture").unwrap();
        }
        assert_eq!(bundled_component_at(folder.path()), Some(usb.join("adb.exe")));
        for name in ["adb.exe", "AdbWinApi.dll", "AdbWinUsbApi.dll", "NOTICE.txt"] {
            std::fs::remove_file(usb.join(name)).unwrap();
            assert!(bundled_component_at(folder.path()).is_none(), "missing {name}");
            std::fs::write(usb.join(name), b"fixture").unwrap();
        }
        assert!(bundled_component_at(Path::new("relative")).is_none());
    }

    #[test]
    fn configured_component_must_be_an_existing_absolute_adb_executable() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("adb.exe");
        assert!(explicit_component(path.to_str().unwrap()).is_none());
        std::fs::write(&path, b"fixture").unwrap();
        assert_eq!(explicit_component(path.to_str().unwrap()), Some(path));
        assert!(explicit_component("adb.exe").is_none());
        let other = folder.path().join("other.exe");
        std::fs::write(&other, b"fixture").unwrap();
        assert!(explicit_component(other.to_str().unwrap()).is_none());
    }
    #[derive(Default)]
    struct FakeState { names:HashMap<String,String>, name_calls:Vec<String>, devices:Vec<Device>, mappings:HashMap<(String,String),String>, calls:Vec<Vec<String>>, path_calls:Vec<(PathBuf,Vec<String>)>, invalid_versions:std::collections::HashSet<PathBuf>, stalled_versions:std::collections::HashSet<PathBuf>, server_mismatch:bool, fail_open:bool, fail_remove:bool, lose_create_reply:bool, failed_list:std::collections::HashSet<String> }
    #[derive(Default)]
    struct FakeIo { state:Mutex<FakeState>, pause_create:std::sync::atomic::AtomicBool, pause_create_reply:std::sync::atomic::AtomicBool, pause_devices:std::sync::atomic::AtomicBool, pause_open:std::sync::atomic::AtomicBool, pause_remove:std::sync::atomic::AtomicBool, entered:tokio::sync::Notify, resume:tokio::sync::Notify }
    impl AdbIo for FakeIo {
        fn device_name<'a>(&'a self,_path:&'a Path,transport:&'a str)->BoxFuture<'a,Option<String>> {Box::pin(async move {
            let name={let mut state=self.state.lock().unwrap();state.name_calls.push(transport.into());state.names.get(transport).cloned()};
            if name.as_deref()==Some("stall") {std::future::pending::<()>().await;}
            name
        })}
        fn run<'a>(&'a self,path:&'a Path,args:&'a [&'a str])->BoxFuture<'a,Result<String,String>> { Box::pin(async move {
            let stalled={let mut state=self.state.lock().unwrap();state.path_calls.push((path.into(),args.iter().map(|arg|arg.to_string()).collect()));args==["version"]&&state.stalled_versions.contains(path)};
            if stalled {std::future::pending::<()>().await;}
            if (args.contains(&"--no-rebind") && self.pause_create.swap(false,Ordering::SeqCst)) || (args.contains(&"shell") && self.pause_open.swap(false,Ordering::SeqCst)) || (args.contains(&"--remove") && self.pause_remove.swap(false,Ordering::SeqCst)) { self.entered.notify_one(); self.resume.notified().await; }
            let (result,created)={
                let mut state=self.state.lock().unwrap();state.calls.push(args.iter().map(|s|s.to_string()).collect());
                if args==["version"] {(Ok(if state.invalid_versions.contains(path) {"Android Debug Bridge version 1.0.39\n".into()}else{"Android Debug Bridge version 1.0.41\n".into()}),false)}else{
                    let id=args.get(1).copied().unwrap_or("");
                    if args.contains(&"--list") {
                        (if state.failed_list.contains(id) {Err("fixture phone failed".into())}else{Ok(state.mappings.iter().filter(|((serial,_),_)|serial==id).map(|((_,remote),local)|format!("UsbFfs {remote} {local}\n")).collect())},false)
                    }else if args.contains(&"--no-rebind") {
                        let key=(id.to_string(),args[4].to_string());
                        if let std::collections::hash_map::Entry::Vacant(entry)=state.mappings.entry(key) {
                            entry.insert(args[5].to_string());
                            (if std::mem::take(&mut state.lose_create_reply) {Err("USB 操作超时，请重试".into())}else{Ok(String::new())},true)
                        }else{(Err(PORT_IN_USE.into()),false)}
                    }else if args.contains(&"--remove") {
                        (if state.fail_remove {Err("fixture cleanup failed".into())}else{state.mappings.remove(&(id.to_string(),args[4].to_string()));Ok(String::new())},false)
                    }else{(if args.contains(&"shell")&&state.fail_open {Err("not installed".into())}else{Ok(String::new())},false)}
                }
            };
            if created&&self.pause_create_reply.swap(false,Ordering::SeqCst) {self.entered.notify_one();self.resume.notified().await;}
            result
        }) }
        fn devices(&self)->BoxFuture<'_,Result<Vec<Device>,String>> { Box::pin(async { if self.pause_devices.swap(false,Ordering::SeqCst) {self.entered.notify_one();self.resume.notified().await;} Ok(self.state.lock().unwrap().devices.clone()) }) }
        fn ensure_server<'a>(&'a self,_path:&'a Path)->BoxFuture<'a,Result<(),String>> { Box::pin(async {if self.state.lock().unwrap().server_mismatch {Err("请先退出使用旧版 ADB 的程序，再重新检测 USB 连接".into())}else{Ok(())}}) }
    }
    fn bridge() -> (std::sync::Arc<Bridge>,std::sync::Arc<FakeIo>,tempfile::TempDir,String) {
        let folder=tempfile::tempdir().unwrap();let path=folder.path().join("adb.exe");std::fs::write(&path,b"fixture only").unwrap();
        let io=std::sync::Arc::new(FakeIo::default());io.state.lock().unwrap().devices.push(phone("fixture","1"));
        let bridge=std::sync::Arc::new(Bridge { io:io.clone(), ..Bridge::default() });
        (bridge,io,folder,path.to_str().unwrap().to_string())
    }
    fn phone(serial:&str,transport:&str)->Device {Device{connection_unverified:false,id:String::new(),transport:transport.into(),serial:serial.into(),name:"Fixture".into(),state:"ready".into(),linked:false,selected:false,error:String::new(),endpoint:String::new()}}
    #[test]
    fn usb_names_ignore_missing_settings_and_bound_unicode() {
        assert_eq!(readable_device_name("null\nnull\nXiaomi 12S Pro\n").as_deref(), Some("Xiaomi 12S Pro"));
        assert_eq!(readable_device_name("我的手机\nother\n").as_deref(), Some("我的手机"));
        assert!(readable_device_name("null\nunknown\n\n").is_none());
        assert_eq!(readable_device_name(&"📱".repeat(50)).unwrap().chars().count(), 40);
    }
    #[tokio::test]
    async fn usb_names_are_cached_per_transport_and_kept_when_disconnected() {
        let (bridge,io,_folder,path)=bridge();
        io.state.lock().unwrap().names.insert("1".into(), "My phone".into());
        let first=bridge.refresh(&path,None).await;
        assert_eq!(first.devices[0].name,"My phone");
        bridge.refresh(&path,None).await;
        assert_eq!(io.state.lock().unwrap().name_calls,vec!["1"]);
        bridge.connect(&path,&first.devices[0].id,17840,bridge.connection_ticket()).await.unwrap();
        io.state.lock().unwrap().devices.clear();
        assert_eq!(bridge.refresh(&path,Some(17840)).await.devices[0].name,"My phone");
        {let mut state=io.state.lock().unwrap();state.devices.push(phone("fixture","2"));state.names.insert("2".into(),"Renamed phone".into());}
        assert_eq!(bridge.refresh(&path,Some(17840)).await.devices[0].name,"Renamed phone");
    }
    #[tokio::test]
    async fn usb_name_timeout_and_unauthorized_phone_do_not_block_discovery() {
        let (bridge,io,_folder,path)=bridge();
        {let mut state=io.state.lock().unwrap();state.names.insert("1".into(),"stall".into());let mut other=phone("other","2");other.state="unauthorized".into();state.devices.push(other);}
        let status=tokio::time::timeout(Duration::from_secs(3),bridge.refresh(&path,None)).await.unwrap();
        assert!(status.available);assert!(status.error.is_empty());assert_eq!(status.devices.len(),2);
        assert_eq!(status.devices[0].name,"Fixture");assert_eq!(io.state.lock().unwrap().name_calls,vec!["1"]);
    }
    async fn select(bridge:&Bridge,path:&str,serial:&str)->String {bridge.refresh(path,None).await.devices.into_iter().find(|device|device.serial==serial).unwrap().id}
    pub(crate) struct LifecycleFixture {pub bridge:std::sync::Arc<Bridge>,pub path:String,pub id:String,io:std::sync::Arc<FakeIo>,_folder:tempfile::TempDir}
    impl LifecycleFixture {
        pub async fn new(port:u16)->Self {let (bridge,io,folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;bridge.connect(&path,&id,port,bridge.connection_ticket()).await.unwrap();Self{bridge,path,id,io,_folder:folder}}
        pub fn pause_cleanup_and_creation(&self) {self.io.pause_remove.store(true,Ordering::SeqCst);self.io.pause_create.store(true,Ordering::SeqCst);}
        pub async fn entered(&self) {tokio::time::timeout(Duration::from_secs(5),self.io.entered.notified()).await.unwrap();}
        pub fn resume(&self) {self.io.resume.notify_one();}
        pub fn fail_open(&self) {self.io.state.lock().unwrap().fail_open=true;}
        pub fn pause_open(&self) {self.io.pause_open.store(true,Ordering::SeqCst);}
        pub fn pause_devices(&self) {self.io.pause_devices.store(true,Ordering::SeqCst);}
        pub fn pause_cleanup(&self) {self.io.pause_remove.store(true,Ordering::SeqCst);}
        pub async fn second_phone(&self)->String {self.io.state.lock().unwrap().devices.push(phone("second","2"));select(&self.bridge,&self.path,"second").await}
        pub fn pause_create_reply(&self) {self.io.pause_create_reply.store(true,Ordering::SeqCst);}
        pub fn lose_create_reply(&self) {self.io.state.lock().unwrap().lose_create_reply=true;}
        pub fn clear_remote(&self) {self.io.state.lock().unwrap().mappings.clear();}
        pub fn clear_first_mapping(&self) {self.io.state.lock().unwrap().mappings.retain(|(serial,_),_|serial!="fixture");}
        pub fn clear_calls(&self) {self.io.state.lock().unwrap().calls.clear();}
        pub fn mapping_count(&self)->usize {self.io.state.lock().unwrap().mappings.len()}
        pub fn calls(&self)->Vec<Vec<String>> {self.io.state.lock().unwrap().calls.clone()}
        pub fn connect_count(&self)->usize {CONNECT_LIMIT-self.bridge.connect_slots.available_permits()}
        pub fn read_count(&self)->usize {READ_LIMIT-self.bridge.read_slots.available_permits()}
        pub fn generation(&self)->u64 {self.bridge.generation.load(Ordering::SeqCst)}
    }
    fn device(kind:u8,state:u8,id:u8) -> Vec<u8> {
        let data = [10,4,b't',b'e',b's',b'0'+id,16,state,42,5,b'P',b'h',b'o',b'n',b'e',56,kind,80,id];
        let mut output=vec![10,data.len() as u8];output.extend(data);output
    }
    #[test] fn usb_metadata_is_authoritative_and_does_not_mistake_tcp_for_usb() {
        let mut data=device(1,8,1);data.extend(device(2,8,2));data.extend(device(1,3,3));
        let list=parse_devices(&data).unwrap();assert_eq!(list.len(),2);assert_eq!(list[0].transport,"1");assert_eq!(list[0].state,"ready");assert_eq!(list[1].state,"unauthorized");
        let json=serde_json::to_value(&list).unwrap();assert!(json[0].get("serial").is_none());assert!(json[0].get("transport").is_none());
    }
    type ScriptedReply = (&'static str, Result<Vec<u8>, AdbQueryFailure>);
    struct ScriptedQueries(Mutex<std::collections::VecDeque<ScriptedReply>>);
    impl DeviceQueries for ScriptedQueries {
        fn query<'a>(&'a self,value:&'a str)->BoxFuture<'a,Result<Vec<u8>,AdbQueryFailure>> {
            Box::pin(async move {
                let (expected,response)=self.0.lock().unwrap().pop_front().expect("unexpected metadata request");
                assert_eq!(value,expected);response
            })
        }
    }
    fn scripted(steps:Vec<(&'static str,Result<Vec<u8>,AdbQueryFailure>)>)->ScriptedQueries {ScriptedQueries(Mutex::new(steps.into()))}
    fn legacy_steps(list:&str,after:&str)->ScriptedQueries {
        scripted(vec![("host:track-devices-proto-binary",Err(AdbQueryFailure::UnsupportedService)),
            ("host-usb:get-serialno",Ok(b"phone".to_vec())),("host:devices-l",Ok(list.as_bytes().to_vec())),
            ("host-usb:get-serialno",Ok(after.as_bytes().to_vec()))])
    }
    #[tokio::test] async fn adb_fallback_requires_an_explicit_complete_unknown_service_failure() {
        let unknown=b"unknown host service";
        let mut failure=format!("FAIL{:04x}",unknown.len()).into_bytes();failure.extend(unknown);
        assert!(matches!(read_adb_response(&mut failure.as_slice()).await,Err(AdbQueryFailure::UnsupportedService)));
        let mut success=format!("OKAY{:04x}",unknown.len()).into_bytes();success.extend(unknown);
        assert_eq!(read_adb_response(&mut success.as_slice()).await.unwrap(),unknown);
        for bytes in [b"FAIL0014unknown".as_slice(),b"FAILzzzz".as_slice(),b"WHAT0000".as_slice(),b"FAIL0001\xff".as_slice()] {
            let mut input=bytes;
            assert!(matches!(read_adb_response(&mut input).await,Err(AdbQueryFailure::Failed(_))));
        }
        assert!(matches!(classify_adb_rejection(b"permission denied"),AdbQueryFailure::Rejected));
        assert!(matches!(classify_adb_rejection(b"unknown host service extra"),AdbQueryFailure::Rejected));
    }
    #[tokio::test] async fn modern_metadata_errors_and_unknown_types_never_trigger_legacy_queries() {
        for response in [Ok(device(0,8,1)),Ok(vec![10,50,0]),Err(AdbQueryFailure::from("USB 操作超时，请重试")),Err(AdbQueryFailure::Rejected)] {
            let queries=scripted(vec![("host:track-devices-proto-binary",response)]);
            assert!(discover_devices(&queries).await.is_err());assert!(queries.0.lock().unwrap().is_empty());
        }
        let queries=scripted(vec![("host:track-devices-proto-binary",Ok(device(1,8,1)))]);
        assert_eq!(discover_devices(&queries).await.unwrap().len(),1);assert!(queries.0.lock().unwrap().is_empty());
    }
    #[tokio::test] async fn legacy_metadata_uses_usb_selection_and_real_transport_without_guessing_names() {
        let queries=legacy_steps("127.0.0.1:5555 device model:USB_phone transport_id:11\nphone device model:Xiaomi_15 transport_id:12\n","phone");
        let devices=discover_devices(&queries).await.unwrap();assert_eq!(devices.len(),1);
        assert_eq!(devices[0].serial,"phone");assert_eq!(devices[0].transport,"12");assert_eq!(devices[0].name,"Xiaomi 15");assert_eq!(devices[0].state,"ready");
        assert!(queries.0.lock().unwrap().is_empty());let public=serde_json::to_value(devices).unwrap();assert!(public[0].get("serial").is_none());assert!(public[0].get("transport").is_none());
    }
    #[test] fn legacy_metadata_rejects_unproven_missing_or_duplicate_identifiers() {
        for list in ["phone device model:Xiaomi", "other device transport_id:3", "phone device transport_id:0", "phone device transport_id:x",
            "phone device transport_id:3 transport_id:4", "phone device transport_id:3\nphone device transport_id:4", "phone device transport_id:3\nother device transport_id:3"] {
            assert!(parse_legacy_devices(list.as_bytes(),"phone").is_err(),"{list}");
        }
        for serial in [b"unknown".as_slice(),b"".as_slice(),b"two phones".as_slice(),b"phone\nother".as_slice(),b"\xff".as_slice()] {assert!(legacy_usb_serial(serial).is_err());}
    }
    #[tokio::test] async fn legacy_metadata_lists_multiple_devices_without_claiming_usb_type() {
        let list=b"first device model:Xiaomi_15 transport_id:12\nsecond device model:Xiaomi_12S_Pro transport_id:13\nemulator-5554 offline transport_id:14\n".to_vec();
        let queries=scripted(vec![("host:track-devices-proto-binary",Err(AdbQueryFailure::UnsupportedService)),
            ("host-usb:get-serialno",Err(AdbQueryFailure::Rejected)),("host:devices-l",Ok(list.clone())),("host:devices-l",Ok(list))]);
        let devices=discover_devices(&queries).await.unwrap();assert_eq!(devices.len(),3);
        assert!(devices.iter().all(|device|device.connection_unverified));assert_eq!(devices[2].state,"offline");
        assert!(queries.0.lock().unwrap().is_empty());
    }
    #[tokio::test] async fn legacy_metadata_rejects_changed_selection() {
        let queries=legacy_steps("phone device transport_id:12\n","other");
        assert!(discover_devices(&queries).await.is_err());assert!(queries.0.lock().unwrap().is_empty());
    }
    #[tokio::test] async fn legacy_multi_device_snapshot_rejects_reused_transports_and_changes() {
        let before=b"phone device transport_id:12\n".to_vec();let after=b"phone device transport_id:13\n".to_vec();
        let queries=scripted(vec![("host:track-devices-proto-binary",Err(AdbQueryFailure::UnsupportedService)),
            ("host-usb:get-serialno",Err(AdbQueryFailure::Rejected)),("host:devices-l",Ok(before)),("host:devices-l",Ok(after))]);
        assert_eq!(discover_devices(&queries).await.err().unwrap(),"手机列表已更新，请重新选择手机");
        for list in [b"phone device\n".to_vec(),b"phone device transport_id:12\nother device transport_id:12\n".to_vec()] {
            let queries=scripted(vec![("host:track-devices-proto-binary",Err(AdbQueryFailure::UnsupportedService)),
                ("host-usb:get-serialno",Err(AdbQueryFailure::Rejected)),("host:devices-l",Ok(list))]);
            assert!(discover_devices(&queries).await.is_err());
        }
    }
    #[tokio::test] async fn legacy_metadata_preserves_io_failure_instead_of_claiming_an_outdated_server() {
        let queries=scripted(vec![("host:track-devices-proto-binary",Err(AdbQueryFailure::UnsupportedService)),
            ("host-usb:get-serialno",Err(AdbQueryFailure::from("USB 操作超时，请重试")))]);
        assert_eq!(discover_devices(&queries).await.err().unwrap(),"USB 操作超时，请重试");assert!(queries.0.lock().unwrap().is_empty());
    }
    #[test] fn truncated_overflow_and_excessive_device_lists_are_rejected() {
        assert!(parse_devices(&[10,50,0]).is_err());assert!(parse_devices(&[255;12]).is_err());
        let mut data=Vec::new();for _ in 0..129 { data.extend(device(1,8,1)); }assert!(parse_devices(&data).is_err());
    }
    #[test] fn mapping_cleanup_requires_both_exact_endpoints() {
        assert!(owns_mapping("UsbFfs tcp:49123 tcp:49123\n",49123));
        assert!(!owns_mapping("UsbFfs tcp:49123 tcp:17840\n",49123));
        assert!(!owns_mapping("UsbFfs tcp:491230 tcp:491230\n",49123));
    }
    #[test] fn cancelling_invalidates_an_already_queued_connection_ticket() {
        let bridge=Bridge::default();let first=bridge.connection_ticket();assert!(bridge.is_current(first));bridge.cancel();assert!(!bridge.is_current(first));
        let next=bridge.connection_ticket();assert!(bridge.is_current(next));assert!(!bridge.is_current(first));
    }
    #[tokio::test] async fn a_foreign_reverse_mapping_is_neither_rebound_nor_removed() {
        let (bridge,io,_folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;io.state.lock().unwrap().mappings.insert(("fixture".into(),"tcp:49123".into()),"tcp:17840".into());
        assert!(bridge.connect(&path,&id,49123,bridge.connection_ticket()).await.is_err());assert!(!bridge.has_mappings());
        let state=io.state.lock().unwrap();assert_eq!(state.mappings.get(&("fixture".into(),"tcp:49123".into())).unwrap(),"tcp:17840");assert!(!state.calls.iter().any(|c|c.iter().any(|s|s=="--remove"||s=="shell")));
    }
    #[tokio::test] async fn cancellation_during_mapping_creation_removes_only_its_own_mapping_and_never_opens_the_app() {
        let (bridge,io,_folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;io.pause_create.store(true,Ordering::SeqCst);
        let task={let bridge=bridge.clone();tokio::spawn(async move {bridge.connect(&path,&id,49123,bridge.connection_ticket()).await})};
        io.entered.notified().await;bridge.cancel();io.resume.notify_one();assert!(task.await.unwrap().is_err());assert!(!bridge.has_mappings());
        let state=io.state.lock().unwrap();assert!(state.mappings.is_empty());assert!(!state.calls.iter().any(|c|c.iter().any(|s|s=="shell")));
    }
    #[tokio::test] async fn disconnect_preserves_a_mapping_replaced_by_another_tool() {
        let (bridge,io,_folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;bridge.connect(&path,&id,49123,bridge.connection_ticket()).await.unwrap();
        io.state.lock().unwrap().mappings.insert(("fixture".into(),"tcp:49123".into()),"tcp:17840".into());bridge.disconnect(&path,None).await.unwrap();
        assert!(!bridge.has_mappings());assert_eq!(io.state.lock().unwrap().mappings.get(&("fixture".into(),"tcp:49123".into())).unwrap(),"tcp:17840");
    }
    #[tokio::test] async fn replug_uses_the_new_transport_and_failed_app_open_cleans_up() {
        let (bridge,io,_folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;bridge.connect(&path,&id,49123,bridge.connection_ticket()).await.unwrap();
        {let mut state=io.state.lock().unwrap();state.devices[0].transport="2".into();state.mappings.clear();}
        let status=bridge.refresh(&path,Some(49123)).await;assert!(status.devices[0].linked);assert_eq!(status.devices[0].id,id);
        bridge.disconnect(&path,None).await.unwrap();
        io.state.lock().unwrap().fail_open=true;assert!(bridge.connect(&path,&id,49123,bridge.connection_ticket()).await.is_err());assert!(!bridge.has_mappings());assert!(io.state.lock().unwrap().mappings.is_empty());
    }
    #[tokio::test] async fn component_output_is_bounded() { assert!(bounded(&vec![0u8;OUTPUT_LIMIT+1][..]).await.is_err()); }

    #[tokio::test] async fn opaque_selection_survives_polls_but_cannot_select_a_reused_transport() {
        let (bridge,io,_folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;assert_eq!(id.len(),32);assert_eq!(select(&bridge,&path,"fixture").await,id);
        io.state.lock().unwrap().devices=vec![phone("other","1")];io.state.lock().unwrap().calls.clear();
        assert!(bridge.connect(&path,&id,49123,bridge.connection_ticket()).await.is_err());assert!(io.state.lock().unwrap().calls.is_empty());
        let next=select(&bridge,&path,"other").await;assert_ne!(next,id);
        let json=serde_json::to_string(&bridge.status()).unwrap();assert!(!json.contains("other"));assert!(!json.contains("transport"));
    }
    #[tokio::test] async fn cleanup_never_targets_a_different_phone_reusing_the_transport() {
        let (bridge,io,_folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;bridge.connect(&path,&id,49123,bridge.connection_ticket()).await.unwrap();
        {let mut state=io.state.lock().unwrap();state.devices=vec![phone("other","1")];state.mappings.insert(("other".into(),"tcp:49123".into()),"tcp:49123".into());state.calls.clear();}
        bridge.disconnect(&path,Some(&id)).await.unwrap();assert!(io.state.lock().unwrap().calls.is_empty());assert_eq!(io.state.lock().unwrap().mappings.get(&("other".into(),"tcp:49123".into())).unwrap(),"tcp:49123");
    }
    #[tokio::test] async fn a_failed_phone_does_not_hide_or_disconnect_a_healthy_phone() {
        let (bridge,io,_folder,path)=bridge();io.state.lock().unwrap().devices.push(phone("other","2"));let first=select(&bridge,&path,"fixture").await;let second=select(&bridge,&path,"other").await;
        bridge.connect(&path,&first,49123,bridge.connection_ticket()).await.unwrap();bridge.connect(&path,&second,49123,bridge.connection_ticket()).await.unwrap();
        io.state.lock().unwrap().failed_list.insert("fixture".into());let status=bridge.refresh(&path,Some(49123)).await;
        assert!(status.error.is_empty());assert_eq!(status.devices.len(),2);assert!(status.devices.iter().find(|device|device.id==first).unwrap().selected);assert!(!status.devices.iter().find(|device|device.id==first).unwrap().error.is_empty());assert!(status.devices.iter().find(|device|device.id==second).unwrap().linked);
    }
    #[tokio::test] async fn disconnected_selection_can_be_cancelled_without_recovering_on_replug() {
        let (bridge,io,_folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;bridge.connect(&path,&id,49123,bridge.connection_ticket()).await.unwrap();io.state.lock().unwrap().devices.clear();
        let status=bridge.refresh(&path,Some(49123)).await;assert_eq!(status.devices[0].id,id);assert_eq!(status.devices[0].state,"disconnected");assert!(status.devices[0].selected);assert!(!status.devices[0].linked);
        bridge.disconnect(&path,Some(&id)).await.unwrap();io.state.lock().unwrap().devices=vec![phone("fixture","2")];io.state.lock().unwrap().calls.clear();
        let status=bridge.refresh(&path,Some(49123)).await;assert_eq!(status.active_count,0);assert!(!status.devices[0].selected);assert!(!bridge.needs_refresh());
        let state=io.state.lock().unwrap();assert!(state.mappings.is_empty());assert!(!state.calls.iter().any(|call|call.iter().any(|arg|arg=="shell"||arg=="--no-rebind")));
    }
    #[tokio::test] async fn cancellation_while_enumerating_does_not_start_restoration() {
        let (bridge,io,_folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;bridge.connect(&path,&id,49123,bridge.connection_ticket()).await.unwrap();io.state.lock().unwrap().calls.clear();io.pause_devices.store(true,Ordering::SeqCst);
        let task={let bridge=bridge.clone();tokio::spawn(async move {bridge.refresh(&path,Some(49123)).await})};io.entered.notified().await;bridge.cancel();io.resume.notify_one();task.await.unwrap();assert!(io.state.lock().unwrap().calls.is_empty());
    }
    #[tokio::test] async fn unknown_selectors_and_duplicate_serials_never_mutate_mappings() {
        let (bridge,io,_folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;io.state.lock().unwrap().calls.clear();
        assert!(bridge.connect(&path,"missing",49123,bridge.connection_ticket()).await.is_err());assert!(bridge.disconnect(&path,Some("missing")).await.is_err());assert!(io.state.lock().unwrap().calls.is_empty());
        io.state.lock().unwrap().devices.push(phone("fixture","2"));assert!(bridge.connect(&path,&id,49123,bridge.connection_ticket()).await.is_err());assert!(io.state.lock().unwrap().calls.is_empty());
        let mut duplicate=device(1,8,1);let mut second=device(1,8,2);second[7]=b'1';duplicate.extend(second);assert!(parse_devices(&duplicate).is_err());
    }
    #[tokio::test] async fn cancellation_during_app_open_removes_the_new_mapping() {
        let (bridge,io,_folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;io.pause_open.store(true,Ordering::SeqCst);
        let task={let bridge=bridge.clone();tokio::spawn(async move {bridge.connect(&path,&id,49123,bridge.connection_ticket()).await})};io.entered.notified().await;bridge.cancel();io.resume.notify_one();assert!(task.await.unwrap().is_err());assert!(!bridge.has_mappings());assert!(io.state.lock().unwrap().mappings.is_empty());
    }
    #[tokio::test] async fn retry_after_a_failed_replug_restores_or_reuses_the_same_port_without_removing_it() {
        let (bridge,io,_folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;bridge.connect(&path,&id,49123,bridge.connection_ticket()).await.unwrap();
        {let mut state=io.state.lock().unwrap();state.devices[0].transport="2".into();state.mappings.clear();state.failed_list.insert("fixture".into());}
        assert!(!bridge.refresh(&path,Some(49123)).await.devices[0].error.is_empty());io.state.lock().unwrap().failed_list.clear();
        bridge.connect(&path,&id,49123,bridge.connection_ticket()).await.unwrap();assert!(io.state.lock().unwrap().mappings.contains_key(&("fixture".into(),"tcp:49123".into())));
        {let mut state=io.state.lock().unwrap();state.devices[0].transport="3".into();state.failed_list.insert("fixture".into());}
        bridge.refresh(&path,Some(49123)).await;{let mut state=io.state.lock().unwrap();state.failed_list.clear();state.calls.clear();}
        bridge.connect(&path,&id,49123,bridge.connection_ticket()).await.unwrap();assert!(!io.state.lock().unwrap().calls.iter().any(|call|call.iter().any(|arg|arg=="--remove"||arg=="--no-rebind")));
    }
    #[tokio::test] async fn missing_components_keep_selected_devices_available_for_individual_disconnect() {
        let (bridge,io,_folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;bridge.connect(&path,&id,49123,bridge.connection_ticket()).await.unwrap();
        let status=bridge.refresh("not-an-installed-component",Some(49123)).await;assert!(!status.available);assert_eq!(status.devices[0].id,id);assert!(status.devices[0].selected);assert!(!status.devices[0].linked);
        bridge.disconnect("not-an-installed-component",Some(&id)).await.unwrap();assert!(!bridge.has_mappings());assert!(!bridge.needs_refresh());assert!(io.state.lock().unwrap().mappings.is_empty());
    }

    #[tokio::test] async fn failed_cleanup_is_retried_without_reopening_the_phone() {
        let (bridge,io,_folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;
        bridge.connect(&path,&id,49123,bridge.connection_ticket()).await.unwrap();
        io.state.lock().unwrap().fail_remove=true;bridge.disconnect(&path,Some(&id)).await.unwrap();
        assert!(!bridge.has_mappings());assert!(!bridge.status().error.is_empty());
        {let mut state=io.state.lock().unwrap();state.fail_remove=false;state.calls.clear();}
        let status=bridge.refresh(&path,None).await;assert!(status.error.is_empty());assert!(!status.devices[0].selected);
        let state=io.state.lock().unwrap();assert!(state.mappings.is_empty());assert!(!state.calls.iter().any(|call|call.iter().any(|arg|arg=="shell"||arg=="--no-rebind")));
    }
    #[tokio::test] async fn disconnecting_offline_retires_the_old_mapping_after_replug() {
        let (bridge,io,_folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;
        bridge.connect(&path,&id,49123,bridge.connection_ticket()).await.unwrap();io.state.lock().unwrap().devices.clear();
        bridge.disconnect(&path,Some(&id)).await.unwrap();assert!(!bridge.has_mappings());
        {let mut state=io.state.lock().unwrap();state.devices=vec![phone("fixture","2")];state.calls.clear();}
        let status=bridge.refresh(&path,None).await;assert!(!status.devices[0].selected);
        let state=io.state.lock().unwrap();assert!(state.mappings.is_empty());assert!(!state.calls.iter().any(|call|call.iter().any(|arg|arg=="shell"||arg=="--no-rebind")));
    }
    #[tokio::test] async fn an_aborted_app_open_resets_busy_and_leaves_a_cleanup_task() {
        let (bridge,io,_folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;io.pause_open.store(true,Ordering::SeqCst);
        let task={let bridge=bridge.clone();let path=path.clone();tokio::spawn(async move {bridge.connect(&path,&id,49123,bridge.connection_ticket()).await})};
        tokio::time::timeout(Duration::from_secs(5),io.entered.notified()).await.unwrap();assert!(bridge.status().busy);task.abort();assert!(task.await.unwrap_err().is_cancelled());
        assert!(!bridge.status().busy);assert!(bridge.needs_refresh());assert!(!bridge.has_mappings());
        bridge.refresh(&path,None).await;assert!(!bridge.needs_refresh());assert!(io.state.lock().unwrap().mappings.is_empty());
    }
    #[tokio::test] async fn a_foreign_replacement_of_pending_cleanup_is_left_intact() {
        let (bridge,io,_folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;bridge.connect(&path,&id,49123,bridge.connection_ticket()).await.unwrap();
        io.state.lock().unwrap().fail_remove=true;bridge.disconnect(&path,Some(&id)).await.unwrap();assert!(bridge.needs_refresh());
        {let mut state=io.state.lock().unwrap();state.mappings.insert(("fixture".into(),"tcp:49123".into()),"tcp:17840".into());state.fail_remove=false;state.calls.clear();}
        bridge.refresh(&path,None).await;assert!(!bridge.needs_refresh());
        let state=io.state.lock().unwrap();assert_eq!(state.mappings.get(&("fixture".into(),"tcp:49123".into())).unwrap(),"tcp:17840");assert!(!state.calls.iter().any(|call|call.iter().any(|arg|arg=="--remove")));
    }
    #[tokio::test] async fn one_pending_cleanup_does_not_cancel_another_phone() {
        let (bridge,io,_folder,path)=bridge();io.state.lock().unwrap().devices.push(phone("other","2"));
        let first=select(&bridge,&path,"fixture").await;let second=select(&bridge,&path,"other").await;
        bridge.connect(&path,&first,49123,bridge.connection_ticket()).await.unwrap();bridge.connect(&path,&second,49123,bridge.connection_ticket()).await.unwrap();
        io.state.lock().unwrap().fail_remove=true;bridge.disconnect(&path,Some(&first)).await.unwrap();
        let status=bridge.refresh(&path,Some(49123)).await;assert_eq!(status.active_count,1);assert!(status.devices.iter().find(|device|device.id==second).unwrap().linked);assert!(!status.devices.iter().find(|device|device.id==first).unwrap().selected);
        {let mut state=io.state.lock().unwrap();state.fail_remove=false;state.calls.clear();}
        bridge.refresh(&path,Some(49123)).await;let state=io.state.lock().unwrap();assert_eq!(state.mappings.len(),1);assert!(state.mappings.contains_key(&("other".into(),"tcp:49123".into())));
    }
    #[tokio::test] async fn a_replacement_component_can_finish_cleanup_when_the_original_was_removed() {
        let (bridge,io,folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;bridge.connect(&path,&id,49123,bridge.connection_ticket()).await.unwrap();
        io.state.lock().unwrap().fail_remove=true;bridge.disconnect(&path,Some(&id)).await.unwrap();std::fs::remove_file(&path).unwrap();
        let replacement=folder.path().join("replacement");std::fs::create_dir(&replacement).unwrap();let replacement=replacement.join("adb.exe");std::fs::write(&replacement,b"fixture only").unwrap();
        io.state.lock().unwrap().fail_remove=false;let status=bridge.refresh(replacement.to_str().unwrap(),None).await;
        assert!(status.error.is_empty());assert!(!bridge.needs_refresh());assert!(io.state.lock().unwrap().mappings.is_empty());
    }
    #[tokio::test] async fn repeated_open_and_cleanup_failures_have_a_bounded_retry_queue() {
        let (bridge,io,_folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;
        {let mut state=io.state.lock().unwrap();state.fail_open=true;state.fail_remove=true;}
        for port in 49100..49228 {assert!(bridge.connect(&path,&id,port,bridge.connection_ticket()).await.is_err());}
        {let mut state=io.state.lock().unwrap();assert_eq!(state.mappings.len(),128);state.calls.clear();}
        assert_eq!(bridge.connect(&path,&id,49228,bridge.connection_ticket()).await.unwrap_err(),CLEANUP_PENDING);
        assert!(!io.state.lock().unwrap().calls.iter().any(|call|call.iter().any(|arg|arg=="--no-rebind")));
        {let mut state=io.state.lock().unwrap();state.fail_open=false;state.fail_remove=false;}
        for _ in 0..32 {bridge.refresh(&path,None).await;if !bridge.needs_refresh() {break;}}
        assert!(!bridge.needs_refresh());assert!(io.state.lock().unwrap().mappings.is_empty());
        bridge.connect(&path,&id,49123,bridge.connection_ticket()).await.unwrap();assert!(bridge.has_mappings());
    }
    #[tokio::test] async fn stalled_cleanup_has_a_deadline_and_does_not_starve_other_items() {
        let (bridge,io,_folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;
        {let mut state=io.state.lock().unwrap();state.fail_open=true;state.fail_remove=true;}
        for port in 49123..49129 {assert!(bridge.connect(&path,&id,port,bridge.connection_ticket()).await.is_err());}
        io.state.lock().unwrap().fail_remove=false;io.pause_remove.store(true,Ordering::SeqCst);
        let started=tokio::time::Instant::now();
        tokio::time::timeout(Duration::from_secs(4),bridge.refresh(&path,None)).await.expect("cleanup blocked the status poll");
        assert!(started.elapsed()>=CLEANUP_TIMEOUT);assert!(bridge.needs_refresh());
        assert!(io.state.lock().unwrap().mappings.len()<6);
        for _ in 0..3 {bridge.refresh(&path,None).await;}
        assert!(!bridge.needs_refresh());assert!(io.state.lock().unwrap().mappings.is_empty());
    }
    #[tokio::test] async fn retirement_during_restoration_cannot_reselect_a_disconnected_phone() {
        let (bridge,io,_folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;bridge.connect(&path,&id,49123,bridge.connection_ticket()).await.unwrap();
        io.state.lock().unwrap().mappings.clear();io.pause_create.store(true,Ordering::SeqCst);
        let refreshing={let bridge=bridge.clone();let path=path.clone();tokio::spawn(async move {bridge.refresh_ports(&path,&[49123]).await})};
        tokio::time::timeout(Duration::from_secs(1),io.entered.notified()).await.unwrap();bridge.retire_selection(Some(&id)).unwrap();io.resume.notify_one();
        let status=refreshing.await.unwrap();assert_eq!(status.active_count,0);assert!(!status.devices[0].selected);assert!(!bridge.needs_refresh());assert!(io.state.lock().unwrap().mappings.is_empty());
    }
    #[tokio::test] async fn uncertain_creation_is_quarantined_and_never_adopted_or_removed() {
        let (bridge,io,_folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;io.state.lock().unwrap().lose_create_reply=true;
        assert_eq!(bridge.connect(&path,&id,49123,bridge.connection_ticket()).await.unwrap_err(),RESULT_UNCONFIRMED);
        assert!(!bridge.has_mappings());assert!(bridge.needs_refresh());
        for _ in 0..3 {let status=bridge.refresh(&path,Some(49123)).await;assert!(!status.devices[0].selected);assert_eq!(status.devices[0].error,RESULT_UNCONFIRMED);}
        bridge.disconnect(&path,None).await.unwrap();
        let state=io.state.lock().unwrap();assert_eq!(state.mappings.len(),1);assert!(!state.calls.iter().any(|call|call.iter().any(|arg|arg=="--remove"||arg=="shell")));
    }
    #[tokio::test] async fn a_failed_spawn_is_not_reported_as_an_uncertain_creation() {
        let folder=tempfile::tempdir().unwrap();let missing=folder.path().join("absent-adb.exe");
        assert!(matches!(run_checked(&missing,&["version"]).await,Err(CommandFailure::NotStarted(_))));
    }
    #[tokio::test] async fn caller_cancellation_cannot_undo_a_committed_ticket_or_cancel_a_newer_one() {
        let (bridge,_io,_folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;let committed=bridge.connection_ticket();
        bridge.connect(&path,&id,49123,committed).await.unwrap();bridge.cancel_ticket(committed);assert!(bridge.is_current(committed));assert!(bridge.has_mappings());
        let newer=bridge.connection_ticket();bridge.cancel_ticket(committed);assert!(bridge.is_current(newer));bridge.cancel_ticket(newer);assert!(!bridge.is_current(newer));assert!(bridge.has_mappings());
    }
    #[tokio::test] async fn cleanup_prefers_the_new_component_and_validates_a_fallback() {
        for invalid_current in [false,true] {
            let (bridge,io,folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;
            bridge.connect(&path,&id,49123,bridge.connection_ticket()).await.unwrap();bridge.retire_selection(None).unwrap();
            let new=folder.path().join("new");std::fs::create_dir(&new).unwrap();let new=new.join("adb.exe");std::fs::write(&new,b"new fixture").unwrap();
            {let mut state=io.state.lock().unwrap();state.path_calls.clear();state.invalid_versions.insert(if invalid_current {new.clone()}else{PathBuf::from(&path)});}
            bridge.state.lock().unwrap().verified_component=None;
            bridge.cleanup_retired(new.to_str().unwrap()).await;
            assert!(!bridge.needs_refresh());let state=io.state.lock().unwrap();assert!(state.mappings.is_empty());
            let expected=if invalid_current {PathBuf::from(&path)}else{new};
            let reverse:Vec<_>=state.path_calls.iter().filter(|(_,args)|args.iter().any(|arg|arg=="reverse")).collect();assert_eq!(reverse.len(),2);assert!(reverse.iter().all(|(path,_)|*path==expected));
            assert!(state.path_calls.iter().any(|(path,args)|*path==expected&&args==&["version"]));
        }
    }
    #[tokio::test] async fn unverified_clients_or_a_mismatched_server_cannot_clean_or_observe_mappings() {
        for server_mismatch in [false,true] {
            let (bridge,io,folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;
            bridge.connect(&path,&id,49123,bridge.connection_ticket()).await.unwrap();bridge.retire_selection(None).unwrap();
            let new=folder.path().join("new");std::fs::create_dir(&new).unwrap();let new=new.join("adb.exe");std::fs::write(&new,b"new fixture").unwrap();
            {let mut state=io.state.lock().unwrap();state.server_mismatch=server_mismatch;if !server_mismatch {state.invalid_versions.extend([new.clone(),PathBuf::from(&path)]);}state.path_calls.clear();}
            bridge.state.lock().unwrap().verified_component=None;
            bridge.cleanup_retired(new.to_str().unwrap()).await;
            {let mut state=bridge.state.lock().unwrap();let mapping=state.cleanup.values().next().unwrap().clone();state.quarantine.insert((mapping.serial.clone(),mapping.port),mapping);}
            bridge.inspect_quarantine(new.to_str().unwrap()).await;
            assert_eq!(bridge.state.lock().unwrap().quarantine.len(),1);assert_eq!(bridge.state.lock().unwrap().cleanup.len(),1);
            let state=io.state.lock().unwrap();assert_eq!(state.mappings.len(),1);assert!(state.path_calls.iter().all(|(_,args)|args==&["version"]));
        }
    }
    #[tokio::test] async fn a_stalled_current_component_has_a_bounded_validated_fallback() {
        let (bridge,io,folder,path)=bridge();let id=select(&bridge,&path,"fixture").await;
        bridge.connect(&path,&id,49123,bridge.connection_ticket()).await.unwrap();bridge.retire_selection(None).unwrap();
        let new=folder.path().join("new");std::fs::create_dir(&new).unwrap();let new=new.join("adb.exe");std::fs::write(&new,b"new fixture").unwrap();
        io.state.lock().unwrap().stalled_versions.insert(new.clone());
        tokio::time::timeout(CLEANUP_TIMEOUT,bridge.cleanup_retired(new.to_str().unwrap())).await.unwrap();
        assert!(!bridge.needs_refresh());assert!(io.state.lock().unwrap().mappings.is_empty());
    }
    #[test] fn connect_cancel_does_not_cancel_detection_or_a_committed_connection() {
        let bridge=Bridge::default();let generation=bridge.generation.load(Ordering::SeqCst);bridge.cancel_connection();assert!(bridge.is_current(generation));
        let slot=bridge.connect_slot().unwrap();let ticket=bridge.connection_ticket();bridge.cancel_connection();assert!(!bridge.is_current(ticket));
        let ticket=bridge.connection_ticket();bridge.state.lock().unwrap().committed_ticket=Some(ticket);bridge.cancel_connection();assert!(bridge.is_current(ticket));
        bridge.finish_connection(ticket);bridge.cancel_connection();assert!(bridge.is_current(ticket));drop(slot);
        let background=bridge.connection_ticket();bridge.cancel_connection();assert!(bridge.is_current(background));
    }
    #[tokio::test] async fn component_picker_keeps_its_permit_after_the_caller_leaves() {
        let bridge=std::sync::Arc::new(Bridge::default());let (entered,waiting)=tokio::sync::oneshot::channel();let (release,hold)=std::sync::mpsc::channel();
        let caller={let bridge=bridge.clone();tokio::spawn(async move {let slot=bridge.component_slot().unwrap();tokio::task::spawn_blocking(move ||{entered.send(()).unwrap();hold.recv().unwrap();slot}).await.unwrap()})};
        waiting.await.unwrap();caller.abort();assert!(caller.await.unwrap_err().is_cancelled());assert!(bridge.component_slot().is_err());release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2),async {while bridge.component_slots.available_permits()==0 {tokio::task::yield_now().await;}}).await.unwrap();assert!(bridge.component_slot().is_ok());
    }
}
