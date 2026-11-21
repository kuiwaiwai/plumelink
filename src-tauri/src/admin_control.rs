//! Routes authorized device input to a local elevated helper.
use crate::{
    control::Native,
    protocol::{Button, Command},
    windows_control::WindowsControl,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::windows::{
        ffi::OsStrExt,
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant},
};
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{ERROR_PIPE_CONNECTED, ERROR_PIPE_LISTENING, HANDLE, WAIT_OBJECT_0},
        Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY},
        Storage::FileSystem::{
            FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX, SECURITY_IDENTIFICATION,
        },
        System::{
            Pipes::{
                ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId,
                GetNamedPipeServerProcessId, SetNamedPipeHandleState, PIPE_NOWAIT,
                PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
            },
            Threading::{
                GetCurrentProcess, GetExitCodeProcess, GetProcessId, OpenProcessToken,
                TerminateProcess, WaitForSingleObject,
            },
            IO::CancelIoEx,
        },
        UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
    },
};

pub const MAX_TEMP_MINUTES: u16 = 24 * 60;
const MAX_FRAME: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdminGrant {
    Persistent,
    Temporary { minutes: u16 },
}
impl AdminGrant {
    pub fn validate(self) -> Result<Self, String> {
        match self {
            Self::Temporary { minutes } if minutes == 0 || minutes > MAX_TEMP_MINUTES => {
                Err(format!("临时授权时长须为 1–{MAX_TEMP_MINUTES} 分钟"))
            }
            _ => Ok(self),
        }
    }
    fn expires(self) -> Option<Instant> {
        match self {
            Self::Persistent => None,
            Self::Temporary { minutes } => {
                Some(Instant::now() + Duration::from_secs(u64::from(minutes) * 60))
            }
        }
    }
}

#[derive(Clone, Serialize)]
pub struct AdminStatus {
    pub active: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
enum Request {
    Hello { secret: String },
    Activate { grant: AdminGrant },
    Execute { session: u64, command: Command },
    Position { x: i32, y: i32 },
    Release { session: u64 },
    Stop,
}

struct Channel {
    file: TimedPipe,
    process: Arc<OwnedHandle>,
}

// Release a held button through the helper that pressed it, even after a grant change.
#[derive(Clone)]
pub(crate) struct AdminButtonRoute(Arc<OwnedHandle>);

#[derive(Default)]
pub struct AdminManager {
    launch: Mutex<()>,
    channel: Mutex<Option<Channel>>,
    ready: Mutex<Option<Arc<OwnedHandle>>>,
    store: Option<crate::preferences::SharedStore>,
    generation: AtomicU64,
}

fn handle(file: &File) -> HANDLE {
    HANDLE(file.as_raw_handle())
}

fn process_running(process: &OwnedHandle) -> bool {
    let mut code = 0;
    unsafe { GetExitCodeProcess(HANDLE(process.as_raw_handle()), &mut code).is_ok() && code == 259 }
}

#[derive(Default)]
struct PipeTimer {
    deadline: Option<Instant>,
    closed: bool,
}

// Join the pipe watchdog before Windows can reuse the handle.
struct TimedPipe {
    file: File,
    timer: Arc<(Mutex<PipeTimer>, Condvar)>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl TimedPipe {
    fn new(file: File) -> Result<Self, String> {
        let raw = file.as_raw_handle() as usize;
        let timer = Arc::new((Mutex::new(PipeTimer::default()), Condvar::new()));
        let watch = timer.clone();
        let worker = std::thread::Builder::new()
            .name("admin-pipe-timer".into())
            .spawn(move || {
                let mut state = watch.0.lock().unwrap();
                while !state.closed {
                    state = match state.deadline {
                        None => watch.1.wait(state).unwrap(),
                        Some(end) if Instant::now() >= end => {
                            unsafe {
                                let _ = CancelIoEx(HANDLE(raw as *mut std::ffi::c_void), None);
                            }
                            // Retry if cancellation raced with the next blocking read.
                            state.deadline = Some(Instant::now() + Duration::from_millis(50));
                            state
                        }
                        Some(end) => {
                            watch
                                .1
                                .wait_timeout(state, end.saturating_duration_since(Instant::now()))
                                .unwrap()
                                .0
                        }
                    };
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            file,
            timer,
            worker: Some(worker),
        })
    }
}
impl Drop for TimedPipe {
    fn drop(&mut self) {
        self.timer.0.lock().unwrap().closed = true;
        self.timer.1.notify_one();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn exchange(pipe: &mut TimedPipe, request: &Request) -> Result<(), String> {
    exchange_response(pipe, request)?
}

fn exchange_response(
    pipe: &mut TimedPipe,
    request: &Request,
) -> Result<Result<(), String>, String> {
    pipe.timer.0.lock().unwrap().deadline = Some(Instant::now() + Duration::from_millis(1500));
    pipe.timer.1.notify_one();
    let file = &mut pipe.file;
    let result = (|| -> Result<Result<(), String>, String> {
        let data = serde_json::to_vec(request).map_err(|e| e.to_string())?;
        if data.len() > MAX_FRAME {
            return Err("管理员控制消息过大".into());
        }
        file.write_all(&(data.len() as u32).to_le_bytes())
            .and_then(|_| file.write_all(&data))
            .map_err(|e| format!("管理员控制通道写入失败：{e}"))?;
        let data = read_frame(file)?;
        serde_json::from_slice::<Result<(), String>>(&data)
            .map_err(|_| "管理员控制响应无效".to_string())
    })();
    pipe.timer.0.lock().unwrap().deadline = None;
    result
}

fn read_frame(file: &mut File) -> Result<Vec<u8>, String> {
    let mut size = [0u8; 4];
    file.read_exact(&mut size).map_err(|e| e.to_string())?;
    let size = u32::from_le_bytes(size) as usize;
    if size == 0 || size > MAX_FRAME {
        return Err("管理员控制消息长度无效".into());
    }
    let mut data = vec![0; size];
    file.read_exact(&mut data).map_err(|e| e.to_string())?;
    Ok(data)
}

fn reply(file: &mut File, result: Result<(), String>) -> Result<(), String> {
    let data = serde_json::to_vec(&result).map_err(|e| e.to_string())?;
    file.write_all(&(data.len() as u32).to_le_bytes())
        .and_then(|_| file.write_all(&data))
        .map_err(|e| e.to_string())
}

fn random_hex<const N: usize>() -> Result<String, String> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
    Ok(hex::encode(bytes))
}

// The ordinary desktop owns the server endpoint. A high-integrity helper can open
// this pipe after UAC approval, including when alternate admin credentials are used.
fn create_desktop_pipe(name: &str) -> Result<File, String> {
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let raw = unsafe {
        CreateNamedPipeW(
            PCWSTR(wide.as_ptr()),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            16 * 1024,
            16 * 1024,
            0,
            None,
        )
    };
    if raw.is_invalid() {
        return Err(format!(
            "无法创建本机管理员控制通道：{}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(unsafe { File::from_raw_handle(raw.0) })
}

fn accept_helper(
    file: &File,
    pid: u32,
    still_starting: impl Fn() -> Result<(), String>,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        still_starting()?;
        match unsafe { ConnectNamedPipe(handle(file), None) } {
            Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => break,
            Err(e) if e.code() != ERROR_PIPE_LISTENING.to_hresult() => return Err(e.to_string()),
            _ => {}
        }
        if Instant::now() >= deadline {
            return Err("管理员辅助进程连接超时".into());
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    let mut actual_pid = 0;
    unsafe { GetNamedPipeClientProcessId(handle(file), &mut actual_pid) }
        .map_err(|e| e.to_string())?;
    if actual_pid != pid {
        return Err("管理员辅助进程身份不匹配".into());
    }
    unsafe { SetNamedPipeHandleState(handle(file), Some(&PIPE_WAIT), None, None) }
        .map_err(|e| e.to_string())
}

pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let token = OwnedHandle::from_raw_handle(token.0);
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0;
        GetTokenInformation(
            HANDLE(token.as_raw_handle()),
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
        .is_ok()
            && elevation.TokenIsElevated != 0
    }
}

impl AdminManager {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_store(store: crate::preferences::SharedStore) -> Self {
        Self {
            store: Some(store),
            ..Self::default()
        }
    }
    fn clean(&self, slot: &mut Option<Channel>) {
        if slot
            .as_ref()
            .is_some_and(|channel| !process_running(&channel.process))
        {
            self.stop_locked(slot);
        }
    }
    fn stop_locked(&self, slot: &mut Option<Channel>) {
        *self.ready.lock().unwrap() = None;
        if let Some(mut channel) = slot.take() {
            let _ = exchange(&mut channel.file, &Request::Stop);
            drop(channel.file);
            unsafe {
                let handle = HANDLE(channel.process.as_raw_handle());
                if WaitForSingleObject(handle, 500) != WAIT_OBJECT_0 {
                    let _ = TerminateProcess(handle, 1);
                }
            }
        }
    }
    pub fn stop(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        *self.ready.lock().unwrap() = None;
        self.stop_locked(&mut self.channel.lock().unwrap());
    }
    pub fn status(&self) -> AdminStatus {
        let active = self
            .ready
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|process| process_running(process));
        AdminStatus { active }
    }

    fn permissions(&self, device: &str) -> Option<crate::preferences::DevicePermissions> {
        self.store
            .as_ref()?
            .lock()
            .unwrap()
            .admin_permissions(device)
    }

    pub fn applies_to(&self, device: &str) -> bool {
        self.permissions(device).is_some() && self.status().active
    }
    fn launch_standby(&self, ticket: u64) -> Result<Channel, String> {
        let pipe_name = format!(r"\\.\pipe\PlumeLinkAdmin-{}", random_hex::<16>()?);
        let file = create_desktop_pipe(&pipe_name)?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe_wide: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
        let params = format!(
            "--admin-helper {} {} standby 0",
            pipe_name,
            std::process::id()
        );
        let params_wide: Vec<u16> = params.encode_utf16().chain(Some(0)).collect();
        let mut launch = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            lpVerb: w!("runas"),
            lpFile: PCWSTR(exe_wide.as_ptr()),
            lpParameters: PCWSTR(params_wide.as_ptr()),
            nShow: 0,
            ..Default::default()
        };
        unsafe { ShellExecuteExW(&mut launch) }
            .map_err(|e| format!("未能获得本机管理员批准：{e}"))?;
        if launch.hProcess.is_invalid() {
            return Err("管理员辅助进程未启动".into());
        }
        let process = unsafe { OwnedHandle::from_raw_handle(launch.hProcess.0) };
        let pid = unsafe { GetProcessId(HANDLE(process.as_raw_handle())) };
        if pid == 0 {
            return Err("无法验证管理员辅助进程".into());
        }
        accept_helper(&file, pid, || {
            if self.generation.load(Ordering::SeqCst) != ticket {
                return Err("管理员控制启动已取消".into());
            }
            if unsafe { WaitForSingleObject(HANDLE(process.as_raw_handle()), 0) } == WAIT_OBJECT_0 {
                return Err("管理员辅助进程未能启动；请确认已在本机批准 UAC".into());
            }
            Ok(())
        })?;
        let mut file = TimedPipe::new(file)?;
        exchange(
            &mut file,
            &Request::Hello {
                secret: String::new(),
            },
        )?;
        Ok(Channel {
            file,
            process: Arc::new(process),
        })
    }

    pub fn prepare(&self) -> Result<(), String> {
        let ticket = self.generation.load(Ordering::SeqCst);
        let _launch = self.launch.lock().unwrap();
        if self.generation.load(Ordering::SeqCst) != ticket {
            return Err("管理员控制启动已取消".into());
        }
        if self.status().active {
            return Ok(());
        }
        let mut channel = self.launch_standby(ticket)?;
        let mut slot = self.channel.lock().unwrap();
        if self.generation.load(Ordering::SeqCst) != ticket {
            return Err("管理员控制启动已取消".into());
        }
        exchange(
            &mut channel.file,
            &Request::Activate {
                grant: AdminGrant::Persistent,
            },
        )?;
        *self.ready.lock().unwrap() = Some(channel.process.clone());
        *slot = Some(channel);
        Ok(())
    }
    pub fn execute(&self, device_id: &str, session: u64, command: &Command) -> Result<(), String> {
        if !elevated_command(command) {
            return Err("该指令不允许提升权限".into());
        }
        self.authorized_request(
            device_id,
            &Request::Execute {
                session,
                command: command.clone(),
            },
        )
    }

    pub fn position(&self, device_id: &str, x: i32, y: i32) -> Result<(), String> {
        self.authorized_request(device_id, &Request::Position { x, y })
    }

    fn authorized_request(&self, device_id: &str, request: &Request) -> Result<(), String> {
        let mut slot = self.channel.lock().unwrap();
        self.clean(&mut slot);
        let permitted = self.permissions(device_id).is_some_and(|p| match request {
            Request::Execute {
                command: Command::Text { .. },
                ..
            } => p.text,
            Request::Execute { .. } | Request::Position { .. } => p.input,
            _ => false,
        });
        if !permitted {
            return Err("此设备没有管理员控制权限".into());
        }
        self.send_live(&mut slot, request)
    }

    fn send_live(&self, slot: &mut Option<Channel>, request: &Request) -> Result<(), String> {
        let channel = slot.as_mut().ok_or("管理员控制未启用")?;
        match exchange_response(&mut channel.file, request) {
            Ok(result) => result,
            Err(error) => {
                self.stop_locked(slot);
                Err(error)
            }
        }
    }

    pub(crate) fn press_button(
        &self,
        device: &str,
        button: Button,
    ) -> Result<AdminButtonRoute, String> {
        let mut slot = self.channel.lock().unwrap();
        self.clean(&mut slot);
        let channel = slot
            .as_mut()
            .filter(|_| self.permissions(device).is_some_and(|p| p.input))
            .ok_or("此设备没有管理员控制权限")?;
        let route = AdminButtonRoute(channel.process.clone());
        // Session zero is reserved for the desktop's aggregate physical button.
        let result = self.send_live(
            &mut slot,
            &Request::Execute {
                session: 0,
                command: Command::Buttonstate { button, down: true },
            },
        );
        result.map(|()| route)
    }

    pub(crate) fn holds_route(&self, route: &AdminButtonRoute) -> bool {
        self.ready
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|process| Arc::ptr_eq(process, &route.0) && process_running(process))
    }

    pub(crate) fn lift_button(
        &self,
        route: &AdminButtonRoute,
        button: Button,
    ) -> Result<(), String> {
        let mut slot = self.channel.lock().unwrap();
        if !slot
            .as_ref()
            .is_some_and(|c| Arc::ptr_eq(&c.process, &route.0))
        {
            return Ok(()); // The previous helper releases its inputs on shutdown.
        }
        self.send_live(
            &mut slot,
            &Request::Execute {
                session: 0,
                command: Command::Buttonstate {
                    button,
                    down: false,
                },
            },
        )
    }

    pub fn release(&self, session: u64) -> Result<(), String> {
        let mut slot = self.channel.lock().unwrap();
        if slot.is_some() {
            return self.send_live(&mut slot, &Request::Release { session });
        }
        Ok(())
    }
}

pub fn elevated_command(command: &Command) -> bool {
    matches!(
        command,
        Command::Move { .. }
            | Command::Motion { .. }
            | Command::Click { .. }
            | Command::Buttonstate { .. }
            | Command::Scroll { .. }
            | Command::Key { .. }
            | Command::Combo { .. }
            | Command::Switch { .. }
            | Command::Desktop { .. }
            | Command::Taskview { .. }
            | Command::Appswitch { .. }
            | Command::Virtualdesktop { .. }
            | Command::Notifications { .. }
            | Command::Zoom { .. }
            | Command::Text { .. }
    )
}

pub fn run_helper(args: &[String]) -> Result<(), String> {
    let (pipe_arg, pid_arg, expected_secret, mode_arg, minutes_arg) = match args.len() {
        4 => (&args[0], &args[1], None, &args[2], &args[3]),
        5 => {
            if args[2].len() != 64 || !args[2].bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("管理员辅助进程参数无效".into());
            }
            (
                &args[0],
                &args[1],
                Some(args[2].as_str()),
                &args[3],
                &args[4],
            )
        }
        _ => return Err("管理员辅助进程参数无效".into()),
    };
    let suffix = pipe_arg.strip_prefix(r"\\.\pipe\PlumeLinkAdmin-");
    if !suffix.is_some_and(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit())) {
        return Err("管理员辅助进程参数无效".into());
    }
    let grant = match (mode_arg.as_str(), minutes_arg.parse::<u16>()) {
        ("standby", Ok(0)) => None,
        ("persistent", Ok(0)) => Some(AdminGrant::Persistent),
        ("temporary", Ok(minutes)) => Some(AdminGrant::Temporary { minutes }.validate()?),
        _ => return Err("管理员授权模式或时长无效".into()),
    };
    let parent_pid = pid_arg.parse::<u32>().map_err(|e| e.to_string())?;
    // An elevated client must never offer an impersonation token to the pipe server.
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .security_qos_flags(SECURITY_IDENTIFICATION.0)
        .open(pipe_arg)
        .map_err(|e| format!("管理员控制通道连接失败：{e}"))?;
    let mut server_pid = 0;
    unsafe { GetNamedPipeServerProcessId(handle(&file), &mut server_pid) }
        .map_err(|e| e.to_string())?;
    if server_pid != parent_pid {
        return Err("管理员控制请求进程身份不匹配".into());
    }
    let first = read_frame(&mut file)?;
    let hello: Request = serde_json::from_slice(&first).map_err(|_| "握手无效")?;
    let secret_ok = match (&hello, expected_secret) {
        (Request::Hello { .. }, None) => true,
        (Request::Hello { secret }, Some(expected)) => secret == expected,
        _ => false,
    };
    if !secret_ok {
        return Err("管理员控制握手失败".into());
    }
    // The desktop activates the authenticated pipe after local UAC approval.
    // It checks the paired device permissions for each input request.
    let grant = match grant {
        Some(grant) => grant,
        None => {
            reply(&mut file, Ok(()))?;
            let data = match read_frame(&mut file) {
                Ok(data) => data,
                Err(_) => return Ok(()),
            };
            match serde_json::from_slice::<Request>(&data) {
                Ok(Request::Activate { grant }) => grant.validate()?,
                Ok(Request::Stop) => return Ok(()),
                _ => return Err("请先在电脑上授权设备".into()),
            }
        }
    };
    let init = WindowsControl::new();
    reply(&mut file, init.as_ref().map(|_| ()).map_err(Clone::clone))?;
    let mut control = init?;
    let expiry = grant.expires();
    let expiry_signal = Arc::new((Mutex::new(false), Condvar::new()));
    let expiry_thread = expiry.map(|expires| {
        let signal = expiry_signal.clone();
        let raw_ptr = handle(&file).0 as usize;
        std::thread::spawn(move || {
            let mut exited = signal.0.lock().unwrap();
            while !*exited {
                let now = Instant::now();
                if now >= expires {
                    unsafe {
                        let _ = CancelIoEx(HANDLE(raw_ptr as *mut std::ffi::c_void), None);
                    }
                    break;
                }
                exited = signal.1.wait_timeout(exited, expires - now).unwrap().0;
            }
        })
    });
    loop {
        if expiry.is_some_and(|expires| Instant::now() >= expires) {
            break;
        }
        let data = match read_frame(&mut file) {
            Ok(data) => data,
            Err(_) => break,
        };
        if expiry.is_some_and(|expires| Instant::now() >= expires) {
            break;
        }
        let result = match serde_json::from_slice::<Request>(&data) {
            Ok(Request::Execute { session, command }) if elevated_command(&command) => command
                .validate()
                .and_then(|_| control.execute_for(session, &command)),
            Ok(Request::Position { x, y }) => WindowsControl::position_cursor(x, y),
            Ok(Request::Release { session: 0 }) => control.release(),
            Ok(Request::Release { session }) => control.release_for(session),
            Ok(Request::Stop) => break,
            _ => Err("管理员控制指令无效".into()),
        };
        if reply(&mut file, result).is_err() {
            break;
        }
    }
    {
        let mut exited = expiry_signal.0.lock().unwrap();
        *exited = true;
        expiry_signal.1.notify_all();
    }
    if let Some(watcher) = expiry_thread {
        let _ = watcher.join();
    }
    control.end_session()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standby_requires_an_explicit_grant_before_input() {
        for (activate, position) in [(false, false), (false, true), (true, false)] {
            let pipe = format!(r"\\.\pipe\PlumeLinkAdmin-{}", random_hex::<16>().unwrap());
            let file = create_desktop_pipe(&pipe).unwrap();
            let args = vec![
                pipe,
                std::process::id().to_string(),
                "standby".into(),
                "0".into(),
            ];
            let helper = std::thread::spawn(move || run_helper(&args));
            accept_helper(&file, std::process::id(), || Ok(())).unwrap();
            let mut file = TimedPipe::new(file).unwrap();
            exchange(
                &mut file,
                &Request::Hello {
                    secret: String::new(),
                },
            )
            .unwrap();
            if activate {
                exchange(
                    &mut file,
                    &Request::Activate {
                        grant: AdminGrant::Temporary { minutes: 1 },
                    },
                )
                .unwrap();
            }
            let result = if activate {
                exchange(&mut file, &Request::Release { session: 1 })
            } else if position {
                exchange(&mut file, &Request::Position { x: 0, y: 0 })
            } else {
                exchange(
                    &mut file,
                    &Request::Execute {
                        session: 1,
                        command: Command::Move { dx: 0, dy: 0 },
                    },
                )
            };
            assert_eq!(result.is_ok(), activate);
            drop(file);
            assert_eq!(helper.join().unwrap().is_ok(), activate);
        }
    }

    #[test]
    fn administrator_input_and_text_are_checked_against_current_device_permissions() {
        use crate::preferences::{PairedDevice, Store};
        let store = Store::memory();
        store
            .lock()
            .unwrap()
            .update(|p| {
                p.devices.push(PairedDevice {
                    id: "phone".into(),
                    ..Default::default()
                });
            })
            .unwrap();
        let manager = AdminManager::with_store(store.clone());
        let text = Command::Text {
            text: "permission check".into(),
        };
        let denied = "此设备没有管理员控制权限";
        let no_helper = "管理员控制未启用";
        assert_eq!(manager.execute("unknown", 1, &text).unwrap_err(), denied);
        assert_eq!(manager.execute("phone", 1, &text).unwrap_err(), no_helper);
        store
            .lock()
            .unwrap()
            .update(|p| p.devices[0].permissions.input = false)
            .unwrap();
        assert_eq!(manager.position("phone", 1, 1).unwrap_err(), denied);
        assert_eq!(manager.execute("phone", 1, &text).unwrap_err(), no_helper);
        store
            .lock()
            .unwrap()
            .update(|p| {
                p.devices[0].permissions.input = true;
                p.devices[0].permissions.text = false;
            })
            .unwrap();
        assert_eq!(manager.execute("phone", 1, &text).unwrap_err(), denied);
        assert_eq!(manager.position("phone", 1, 1).unwrap_err(), no_helper);
        store
            .lock()
            .unwrap()
            .update(|p| p.devices[0].permissions.admin = false)
            .unwrap();
        assert_eq!(manager.position("phone", 1, 1).unwrap_err(), denied);
        assert!(!manager.applies_to("phone"));
    }

    #[test]
    fn high_integrity_command_allowlist() {
        assert!(elevated_command(&Command::Move { dx: 1, dy: 0 }));
        assert!(elevated_command(&Command::Text { text: "ok".into() }));
        assert!(!elevated_command(&Command::Shutdown { confirmed: true }));
        assert!(!elevated_command(&Command::Magnifier { enabled: true }));
        assert!(!elevated_command(&Command::Volume {
            action: crate::protocol::Volume::Up
        }));
    }
    #[test]
    fn grant_modes_and_time_bounds() {
        assert!(AdminGrant::Persistent.validate().is_ok());
        assert!(AdminGrant::Temporary { minutes: 1 }.validate().is_ok());
        assert!(AdminGrant::Temporary {
            minutes: MAX_TEMP_MINUTES
        }
        .validate()
        .is_ok());
        assert!(AdminGrant::Temporary { minutes: 0 }.validate().is_err());
        assert!(AdminGrant::Temporary {
            minutes: MAX_TEMP_MINUTES + 1
        }
        .validate()
        .is_err());
        assert!(AdminGrant::Persistent.expires().is_none());
    }
    #[test]
    fn pipe_accepts_only_parent_handshake() {
        for (mode, minutes, legacy) in [
            ("temporary", "15", true),
            ("persistent", "0", true),
            ("temporary", "1", false),
        ] {
            let pipe = format!(r"\\.\pipe\PlumeLinkAdmin-{}", random_hex::<16>().unwrap());
            let file = create_desktop_pipe(&pipe).unwrap();
            let secret = if legacy {
                random_hex::<32>().unwrap()
            } else {
                String::new()
            };
            let mut args = vec![pipe, std::process::id().to_string()];
            if legacy {
                args.push(secret.clone());
            }
            args.extend([mode.into(), minutes.into()]);
            let helper = std::thread::spawn(move || run_helper(&args));
            accept_helper(&file, std::process::id(), || Ok(())).unwrap();
            let mut file = TimedPipe::new(file).unwrap();
            exchange(&mut file, &Request::Hello { secret }).unwrap();
            drop(file);
            assert!(helper.join().unwrap().is_ok());
        }
    }

    #[test]
    fn rejects_mismatched_peer_ids_before_handshake() {
        let name = format!(r"\\.\pipe\PlumeLinkAdmin-{}", random_hex::<16>().unwrap());
        let server = create_desktop_pipe(&name).unwrap();
        let args = vec![name.clone(), "0".into(), "temporary".into(), "1".into()];
        assert!(run_helper(&args).unwrap_err().contains("身份不匹配"));
        drop(server);
        let server = create_desktop_pipe(&name).unwrap();
        let _client = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&name)
            .unwrap();
        assert!(accept_helper(&server, 0, || Ok(()))
            .unwrap_err()
            .contains("身份不匹配"));
    }
}
