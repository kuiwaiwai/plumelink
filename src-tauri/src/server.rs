mod audio;
mod files;
mod input;
mod usb_access;
use audio::upgrade_microphone;
use files::{resumable_upload, send_file};
use input::upgrade;

use crate::{
    control::Controller,
    file_transfer::{FileHub, MAX_FILE_BYTES},
    security::{Limiter, Pairing},
};
use axum::{
    body::to_bytes,
    extract::{
        ws::{CloseFrame, Message, WebSocket},
        ConnectInfo, Request, State,
    },
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::{
    sync::{watch, Semaphore},
    task::JoinHandle,
};

struct Session {
    standby: bool,
    cancel: watch::Sender<u16>,
    device_id: String,
}
struct Limits {
    pair: Limiter,
    total: Limiter,
    upgrade: Limiter,
}
struct ControlRequest {
    id: String,
    name: String,
    created: Instant,
}
#[derive(Clone)]
pub struct Context {
    pub origin: String,
    mdns_host: Option<String>,
    shutdown: watch::Receiver<bool>,
    shared: Arc<SharedContext>,
    usb_access: Option<usb_access::UsbAccess>,
}
pub struct SharedContext {
    shutdown: watch::Receiver<bool>,
    pairing: Mutex<Pairing>,
    limits: Mutex<Limits>,
    active: Mutex<HashMap<u64, Session>>,
    next: AtomicU64,
    controller: Arc<Controller>,
    files: Arc<FileHub>,
    store: crate::preferences::SharedStore,
    file_slots: Arc<Semaphore>,
    download_slots: Arc<Semaphore>,
    slots: Arc<Semaphore>,
    audio_slots: Arc<Semaphore>,
    connection_slots: Arc<Semaphore>,
    application_slots: Arc<Semaphore>,
    application_icon_slots: Arc<Semaphore>,
    control_request: Mutex<Option<ControlRequest>>,
    recent_control: Mutex<Option<(String, Instant)>>,
    uploads: crate::resumable::Uploads,
}
impl std::ops::Deref for Context {
    type Target = SharedContext;
    fn deref(&self) -> &Self::Target { &self.shared }
}
impl Context {
    fn stopped(&self) -> bool { *self.shutdown.borrow() || *self.shared.shutdown.borrow() }
    fn while_live<T>(&self, operation: impl FnOnce() -> T) -> Option<T> {
        let local = self.shutdown.borrow(); let global = self.shared.shutdown.borrow();
        if *local || *global { None } else { Some(operation()) }
    }
    fn stop_signal(&self) -> ConnectionStop {
        ConnectionStop { local: self.shutdown.clone(), global: self.shared.shutdown.clone() }
    }
    fn window_access(&self, token: &str) -> Result<(), String> {
        if self.stopped() { return Err("连接已失效".into()); }
        let device = self.pairing.lock().unwrap().device(token).ok_or("配对已失效，请重新配对")?;
        if !device.permissions.input { return Err("此设备没有遥控权限".into()); }
        if !self.store.lock().unwrap().primary_allows(&device.id) { return Err("当前无主控权".into()); }
        Ok(())
    }
    fn files_allowed(&self, token: &str) -> bool {
        !self.stopped()
            && self
                .pairing
                .lock()
                .unwrap()
                .device(token)
                .is_some_and(|device| device.permissions.files)
    }
    pub fn connected(&self) -> bool {
        !self.active.lock().unwrap().is_empty()
    }
    pub fn connected_count(&self) -> usize {
        self.active
            .lock()
            .unwrap()
            .values()
            .map(|session| &session.device_id)
            .collect::<std::collections::HashSet<_>>()
            .len()
    }
    pub fn online_device_ids(&self) -> std::collections::HashSet<String> {
        self.active
            .lock()
            .unwrap()
            .values()
            .map(|session| session.device_id.clone())
            .collect()
    }
    fn presence(&self, device_id: &str) -> serde_json::Value {
        let preferences = self.store.lock().unwrap().get();
        let effective_primary = (preferences.control_mode
            == crate::preferences::ControlMode::Primary)
            .then_some(preferences.primary_device.as_deref())
            .flatten();
        let primary_name = effective_primary.and_then(|id| {
            preferences
                .devices
                .iter()
                .find(|device| device.id == id)
                .map(|device| device.name.as_str())
        });
        let recent_id = self
            .recent_control
            .lock()
            .unwrap()
            .as_ref()
            .filter(|(_, at)| at.elapsed() < Duration::from_secs(10))
            .map(|(id, _)| id.clone());
        let recent_name = recent_id.as_deref().and_then(|id| {
            preferences
                .devices
                .iter()
                .find(|device| device.id == id)
                .map(|device| device.name.as_str())
        });
        serde_json::json!({
            "connectedCount": self.connected_count(),
            "controlMode": preferences.control_mode,
            "primary": effective_primary == Some(device_id),
            "primaryName": primary_name,
            "controlRequestPending": self.control_request.lock().unwrap().as_ref()
                .is_some_and(|request| request.id == device_id && request.created.elapsed() < Duration::from_secs(60)),
            "adminGranted": self.controller.admin.applies_to(device_id),
            "recentController": recent_name,
        })
    }
    pub fn recent_controller(&self) -> Option<String> {
        let recent = self.recent_control.lock().unwrap();
        let (id, at) = recent.as_ref()?;
        if at.elapsed() >= Duration::from_secs(10) {
            return None;
        }
        self.store
            .lock()
            .unwrap()
            .get()
            .devices
            .iter()
            .find(|device| device.id == *id)
            .map(|device| device.name.clone())
    }
    fn record_control(&self, device_id: &str) {
        *self.recent_control.lock().unwrap() = Some((device_id.to_string(), Instant::now()));
    }
    pub fn code_remaining(&self) -> u64 {
        self.pairing_status().1
    }
    pub fn code(&self) -> String {
        self.pairing_status().0
    }
    pub fn pairing_status(&self) -> (String, u64) {
        let mut pairing = self.pairing.lock().unwrap();
        if let Err(error) = pairing.refresh_if_expired() {
            crate::logging::warn("pairing", &format!("code renewal failed: {error}"));
            return (String::new(), 0);
        }
        (pairing.code.clone(), pairing.remaining())
    }
    pub fn refresh_pairing_code(&self) -> Result<(), String> {
        self.pairing.lock().unwrap().refresh_code()
    }
    pub fn set_pairing_interval(&self, minutes: u16) -> Result<(), String> {
        self.pairing.lock().unwrap().set_interval(minutes)
    }
    fn disconnect_locked(&self) {
        for (_, session) in self.active.lock().unwrap().drain() {
            let _ = session.cancel.send(4003);
        }
        self.controller.revoke();
    }
    pub fn revoke(&self) -> Result<(), String> {
        let mut p = self.pairing.lock().unwrap();
        let result = p.rotate();
        self.disconnect_locked();
        result
    }
    pub fn revoke_device(&self, device_id: &str) -> Result<(), String> {
        self.store.lock().unwrap().update(|preferences| {
            preferences.devices.retain(|device| device.id != device_id);
            if preferences.primary_device.as_deref() == Some(device_id) {
                preferences.primary_device = None;
                preferences.control_mode = crate::preferences::ControlMode::Free;
            }
        })?;
        let sessions: Vec<_> = {
            let mut active = self.active.lock().unwrap();
            let ids: Vec<_> = active
                .iter()
                .filter(|(_, session)| session.device_id == device_id)
                .map(|(id, _)| *id)
                .collect();
            ids.into_iter()
                .filter_map(|id| active.remove(&id).map(|session| (id, session)))
                .collect()
        };
        for (id, session) in sessions {
            let _ = session.cancel.send(4003);
            self.controller.release_for(id);
        }
        self.clear_control_request_for(device_id);
        Ok(())
    }
    pub fn refresh_device(&self, device_id: &str) {
        let sessions: Vec<_> = {
            let mut active = self.active.lock().unwrap();
            let ids: Vec<_> = active
                .iter()
                .filter(|(_, session)| session.device_id == device_id)
                .map(|(id, _)| *id)
                .collect();
            ids.into_iter()
                .filter_map(|id| active.remove(&id).map(|session| (id, session)))
                .collect()
        };
        for (id, session) in sessions {
            let _ = session.cancel.send(4004);
            self.controller.release_for(id);
        }
        self.clear_control_request_for(device_id);
    }
    pub fn refresh_sessions(&self) {
        let sessions: Vec<_> = self.active.lock().unwrap().drain().collect();
        for (id, session) in sessions {
            let _ = session.cancel.send(4004);
            self.controller.release_for(id);
        }
    }
    pub fn request_control(&self, device: &crate::security::AuthorizedDevice) -> bool {
        if !self
            .active
            .lock()
            .unwrap()
            .values()
            .any(|session| session.device_id == device.id)
        {
            return false;
        }
        let mut pending = self.control_request.lock().unwrap();
        if pending.as_ref().is_some_and(|request| request.id == device.id && request.created.elapsed() < Duration::from_secs(60)) {
            return true;
        }
        *pending = Some(ControlRequest {
            id: device.id.clone(),
            name: device.name.clone(),
            created: Instant::now(),
        });
        true
    }
    pub fn pending_control_request(&self) -> Option<(String, String)> {
        let active_ids: std::collections::HashSet<String> = self
            .active
            .lock()
            .unwrap()
            .values()
            .map(|session| session.device_id.clone())
            .collect();
        let input_devices: std::collections::HashSet<String> = self
            .store
            .lock()
            .unwrap()
            .get()
            .devices
            .iter()
            .filter(|device| device.permissions.input)
            .map(|device| device.id.clone())
            .collect();
        let mut request = self.control_request.lock().unwrap();
        let stale = request.as_ref().is_some_and(|item| {
            item.created.elapsed() > Duration::from_secs(60)
                || !active_ids.contains(&item.id)
                || !input_devices.contains(&item.id)
        });
        if stale {
            *request = None;
        }
        request
            .as_ref()
            .map(|item| (item.id.clone(), item.name.clone()))
    }
    pub fn clear_control_request(&self) {
        *self.control_request.lock().unwrap() = None;
    }
    fn clear_control_request_for(&self, device_id: &str) {
        let mut request = self.control_request.lock().unwrap();
        if request.as_ref().is_some_and(|item| item.id == device_id) {
            *request = None;
        }
    }
    fn authorize(
        &self,
        token: &str,
    ) -> Result<(u64, watch::Receiver<u16>, crate::security::AuthorizedDevice), u16> {
        let local = self.shutdown.borrow(); let global = self.shared.shutdown.borrow();
        if *local || *global { return Err(4000); }
        let pairing = self.pairing.lock().unwrap();
        let device = pairing.device(token).ok_or(4003u16)?;
        pairing.mark_seen(&device.id);
        drop(pairing);
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = watch::channel(0);
        if !self.controller.activate(id) { return Err(4004); }
        let previous = {
            let mut active = self.active.lock().unwrap();
            if active.iter().any(|(other, session)| *other > id && session.standby && session.device_id == device.id) {
                drop(active);
                self.controller.release_for(id);
                return Err(4001);
            }
            let ids: Vec<_> = active.iter().filter(|(other, session)| **other < id && session.standby && session.device_id == device.id)
                .map(|(id, _)| *id).collect();
            let previous: Vec<_> = ids.into_iter().filter_map(|id| active.remove(&id).map(|session| (id, session))).collect();
            active.insert(id, Session { standby: false, cancel: tx, device_id: device.id.clone() });
            previous
        };
        for (previous_id, session) in previous {
            let _ = session.cancel.send(4001);
            self.controller.release_for(previous_id);
        }
        Ok((id, rx, device))
    }
    fn enter_standby(&self, id: u64, device_id: &str) -> bool {
        let previous = {
            let mut active = self.active.lock().unwrap();
            if !active.contains_key(&id) || active.iter().any(|(other, session)| *other > id && session.device_id == device_id) {
                return false;
            }
            if let Some(session) = active.get_mut(&id) { session.standby = true; }
            let ids: Vec<_> = active.iter().filter(|(other, session)| **other != id && session.device_id == device_id)
                .map(|(other, _)| *other).collect();
            ids.into_iter().filter_map(|other| active.remove(&other).map(|session| (other, session))).collect::<Vec<_>>()
        };
        for (previous_id, session) in previous {
            let _ = session.cancel.send(4001);
            self.controller.release_for(previous_id);
        }
        true
    }
    fn detach(&self, id: u64) {
        if self.active.lock().unwrap().remove(&id).is_some() {
            self.controller.release_for(id);
        }
    }
    fn current(&self, id: u64) -> bool {
        self.active.lock().unwrap().contains_key(&id)
    }
    fn host_ok(&self, headers: &HeaderMap) -> bool {
        !self.stopped() && header_host_ok(&self.origin, self.mdns_host.as_deref(), headers)
    }
    fn origin_ok(&self, headers: &HeaderMap) -> bool {
        !self.stopped() && header_origin_ok(&self.origin, self.mdns_host.as_deref(), headers)
    }
    fn api_origin_ok(&self, headers: &HeaderMap) -> bool {
        !self.stopped() && header_api_origin_ok(&self.origin, self.mdns_host.as_deref(), headers)
    }
}
struct ConnectionStop { local: watch::Receiver<bool>, global: watch::Receiver<bool> }
impl ConnectionStop {
    async fn changed(&mut self) {
        if *self.local.borrow() || *self.global.borrow() { return; }
        tokio::select! { _ = self.local.changed() => {}, _ = self.global.changed() => {} }
    }
}
fn allowed_host(origin: &str, mdns_host: Option<&str>, candidate: &str) -> bool {
    let primary = origin
        .split_once("://")
        .map(|(_, host)| host)
        .unwrap_or(origin);
    let default = if origin.starts_with("https://") {
        ":443"
    } else {
        ":80"
    };
    let matches = |expected: &str| {
        candidate.eq_ignore_ascii_case(expected)
            || expected
                .strip_suffix(default)
                .is_some_and(|host| candidate.eq_ignore_ascii_case(host))
    };
    matches(primary) || mdns_host.is_some_and(matches)
}
fn allowed_origin(origin: &str, mdns_host: Option<&str>, candidate: &str) -> bool {
    let Some((expected_scheme, _)) = origin.split_once("://") else {
        return false;
    };
    let Some((scheme, host)) = candidate.split_once("://") else {
        return false;
    };
    scheme.eq_ignore_ascii_case(expected_scheme) && allowed_host(origin, mdns_host, host)
}

fn header_host_ok(origin: &str, mdns_host: Option<&str>, headers: &HeaderMap) -> bool {
    headers
        .get("host")
        .and_then(|h| h.to_str().ok())
        .is_some_and(|host| allowed_host(origin, mdns_host, host))
}
fn header_origin_ok(origin: &str, mdns_host: Option<&str>, headers: &HeaderMap) -> bool {
    header_host_ok(origin, mdns_host, headers)
        && headers
            .get("origin")
            .and_then(|h| h.to_str().ok())
            .is_some_and(|candidate| allowed_origin(origin, mdns_host, candidate))
}
fn header_api_origin_ok(origin: &str, mdns_host: Option<&str>, headers: &HeaderMap) -> bool {
    header_host_ok(origin, mdns_host, headers)
        && headers
            .get("origin")
            .and_then(|h| h.to_str().ok())
            .is_none_or(|candidate| allowed_origin(origin, mdns_host, candidate))
}
struct Discovery {
    daemon: mdns_sd::ServiceDaemon,
    info: mdns_sd::ServiceInfo,
}
impl Discovery {
    fn refresh(&self) {
        let _ = self.daemon.register(self.info.clone());
    }
    fn shutdown(self) {
        let _ = self.daemon.shutdown();
    }
}
pub(crate) struct PreparedServer {
    host: std::net::Ipv4Addr,
    listener: tokio::net::TcpListener,
    address: SocketAddr,
    tls: Option<Arc<rustls::ServerConfig>>,
    pairing: Pairing,
}

pub struct Server {
    pub context: Arc<Context>,
    pub address: SocketAddr,
    shutdown: watch::Sender<bool>,
    lan: Option<ServingEndpoint>,
    usb: HashMap<String,ServingEndpoint>,
    tls: Option<Arc<rustls::ServerConfig>>,
    discovery: Option<Discovery>,
    discovery_error: Option<String>,
}
struct ServingEndpoint {
    address: SocketAddr,
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
    usb_access: Option<usb_access::UsbAccess>,
}
impl ServingEndpoint {
    fn start(listener: tokio::net::TcpListener, tls: Option<Arc<rustls::ServerConfig>>, context: &Context, mdns: bool, usb_access:Option<usb_access::UsbAccess>) -> Self {
        let address = listener.local_addr().expect("bound listener address");
        let (stop, shutdown) = watch::channel(false);
        let view = Arc::new(Context {
            origin: format!("{}://{address}", if tls.is_some() { "https" } else { "http" }),
            mdns_host: if mdns { mdns_host_for(&context.store, address.port()) } else { None },
            shutdown: shutdown.clone(), shared: context.shared.clone(),
            usb_access: usb_access.clone(),
        });
        let routes = Router::new().route("/ws", get(upgrade)).route("/microphone", get(upgrade_microphone))
            .fallback(http).layer(axum::middleware::from_fn_with_state(view.clone(),usb_access::guard)).with_state(view);
        let mut shutdown = shutdown;
        let connections=context.connection_slots.clone();
        let task = tokio::spawn(async move {
            if let Some(tls) = tls {
                let listener=crate::transport::CancelListener::new(crate::transport::TlsListener::with_slots(listener,tls,connections),shutdown.clone());
                let _ = axum::serve(listener,
                    routes.into_make_service_with_connect_info::<crate::transport::Peer>())
                    .with_graceful_shutdown(async move { let _ = shutdown.changed().await; }).await;
            } else {
                let listener=crate::transport::CancelListener::new(crate::transport::LimitedListener::with_slots(listener,connections),shutdown.clone());
                let _ = axum::serve(listener,
                    routes.into_make_service_with_connect_info::<crate::transport::Peer>())
                    .with_graceful_shutdown(async move { let _ = shutdown.changed().await; }).await;
            }
        });
        Self { address, stop, task: Some(task), usb_access }
    }
    async fn close(mut self) {
        let _ = self.stop.send(true);
        if let Some(mut task) = self.task.take() {
            if tokio::time::timeout(Duration::from_secs(3), &mut task).await.is_err() {
                task.abort(); let _ = task.await;
            }
        }
    }
}
impl Drop for ServingEndpoint {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        if let Some(task) = self.task.take() { task.abort(); }
    }
}
fn claim_usb_port(port:u16)->bool {
    static HISTORY:Mutex<[u64;1024]>=Mutex::new([0;1024]);
    let mut history=HISTORY.lock().unwrap();let word=usize::from(port)/64;let bit=1u64<<(port%64);
    if history[word]&bit!=0 {false}else{history[word]|=bit;true}
}
async fn fresh_usb_listener(mut initial:Option<tokio::net::TcpListener>)->Result<tokio::net::TcpListener,String> {
    const FAILURE:&str="暂时无法创建 USB 入口，请重启电脑端羽连后重试";
    for _ in 0..64 {
        if let Some(listener)=initial.take() {
            if claim_usb_port(listener.local_addr().map_err(|_|FAILURE)?.port()) {return Ok(listener);}
        }else{
            let socket=tokio::net::TcpSocket::new_v4().map_err(|_|FAILURE)?;
            socket.bind((std::net::Ipv4Addr::LOCALHOST,0).into()).map_err(|_|FAILURE)?;
            if claim_usb_port(socket.local_addr().map_err(|_|FAILURE)?.port()) {return socket.listen(1024).map_err(|_|FAILURE.into());}
        }
    }
    Err(FAILURE.into())
}
impl Server {
    pub async fn start(
        host: std::net::Ipv4Addr,
        port: u16,
        controller: Arc<Controller>,
    ) -> Result<Self, String> {
        Self::start_with_store(host, port, controller, crate::preferences::Store::memory()).await
    }
    pub async fn start_with_store(
        host: std::net::Ipv4Addr,
        port: u16,
        controller: Arc<Controller>,
        store: crate::preferences::SharedStore,
    ) -> Result<Self, String> {
        Self::start_inner(host, port, controller, store, FileHub::new(), false).await
    }
    pub async fn start_with_files(
        host: std::net::Ipv4Addr,
        port: u16,
        controller: Arc<Controller>,
        store: crate::preferences::SharedStore,
        files: Arc<FileHub>,
    ) -> Result<Self, String> {
        Self::start_inner(host, port, controller, store, files, false).await
    }
    pub async fn start_secure_with_store(
        host: std::net::Ipv4Addr,
        port: u16,
        controller: Arc<Controller>,
        store: crate::preferences::SharedStore,
    ) -> Result<Self, String> {
        Self::start_inner(host, port, controller, store, FileHub::new(), true).await
    }
    pub async fn start_secure_with_files(
        host: std::net::Ipv4Addr,
        port: u16,
        controller: Arc<Controller>,
        store: crate::preferences::SharedStore,
        files: Arc<FileHub>,
    ) -> Result<Self, String> {
        Self::start_inner(host, port, controller, store, files, true).await
    }
    pub async fn start_usb_only(controller: Arc<Controller>, store: crate::preferences::SharedStore, files: Arc<FileHub>) -> Result<Self, String> {
        let mut prepared=Self::prepare(std::net::Ipv4Addr::LOCALHOST,0,&store,true).await?;
        prepared.listener=fresh_usb_listener(Some(prepared.listener)).await?;
        prepared.address=prepared.listener.local_addr().map_err(|error|error.to_string())?;
        let mut server=Self::launch_with_access(prepared,controller,store,files,Some(usb_access::UsbAccess::new()?));
        if let Some(endpoint)=server.lan.take() {server.usb.insert(String::new(),endpoint);}
        Ok(server)
    }
    async fn start_inner(
        host: std::net::Ipv4Addr,
        port: u16,
        controller: Arc<Controller>,
        store: crate::preferences::SharedStore,
        files: Arc<FileHub>,
        secure: bool,
    ) -> Result<Self, String> {
        let prepared = Self::prepare(host, port, &store, secure).await?;
        Ok(Self::launch(prepared, controller, store, files))
    }
    pub(crate) async fn bind(
        host: std::net::Ipv4Addr,
        port: u16,
    ) -> Result<tokio::net::TcpListener, String> {
        let bind_error = |e: std::io::Error| match e.kind() {
            std::io::ErrorKind::AddrInUse => format!("端口 {port} 已被占用，请选择其他端口。"),
            std::io::ErrorKind::PermissionDenied => {
                format!("Windows 无法使用端口 {port}，请选择其他端口。")
            }
            _ => format!("无法监听所选地址：{e}"),
        };
        if port != 0 && !host.is_unspecified() {
            // Winsock allows a specific address to shadow an existing wildcard listener.
            // Probe without listening; only the selected adapter will accept connections.
            let probe = tokio::net::TcpSocket::new_v4().map_err(bind_error)?;
            probe
                .bind((std::net::Ipv4Addr::UNSPECIFIED, port).into())
                .map_err(bind_error)?;
        }
        tokio::net::TcpListener::bind((host, port))
            .await
            .map_err(bind_error)
    }
    pub(crate) async fn prepare(
        host: std::net::Ipv4Addr,
        port: u16,
        store: &crate::preferences::SharedStore,
        secure: bool,
    ) -> Result<PreparedServer, String> {
        {
            let mut preferences = store.lock().unwrap();
            if preferences.get().installation_id.is_empty() {
                let mut identifier = [0_u8; 12];
                getrandom::fill(&mut identifier).map_err(|error| error.to_string())?;
                preferences.update(|settings| settings.installation_id = hex::encode(identifier))?;
            }
        }
        let listener = Self::bind(host, port).await?;
        let address = listener.local_addr().map_err(|e| e.to_string())?;
        let tls = if secure {
            Some(tls_config(host, store)?)
        } else {
            None
        };
        let pairing = Pairing::with_store(store.clone())?;
        Ok(PreparedServer {
            host,
            listener,
            address,
            tls,
            pairing,
        })
    }
    pub(crate) fn launch(
        prepared: PreparedServer,
        controller: Arc<Controller>,
        store: crate::preferences::SharedStore,
        files: Arc<FileHub>,
    ) -> Self { Self::launch_with_access(prepared,controller,store,files,None) }
    fn launch_with_access(
        prepared: PreparedServer,
        controller: Arc<Controller>,
        store: crate::preferences::SharedStore,
        files: Arc<FileHub>,
        usb_access: Option<usb_access::UsbAccess>,
    ) -> Self {
        let PreparedServer {
            host,
            listener,
            address,
            tls,
            pairing,
        } = prepared;
        let (tx, rx) = watch::channel(false);
        let mdns_host = mdns_host_for(&store, address.port());
        let context = Arc::new(Context {
            origin: format!("{}://{address}", if tls.is_some() { "https" } else { "http" }),
            mdns_host,
            shutdown: rx.clone(),
            usb_access: usb_access.clone(),
            shared: Arc::new(SharedContext {
            shutdown: rx.clone(),
            pairing: Mutex::new(pairing),
            limits: Mutex::new(Limits {
                pair: Limiter::new(6, 60),
                total: Limiter::new(30, 60),
                upgrade: Limiter::new(30, 60),
            }),
            active: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            controller,
            files: files.clone(),
            store: store.clone(),
            file_slots: Arc::new(Semaphore::new(1)),
            download_slots: Arc::new(Semaphore::new(4)),
            slots: Arc::new(Semaphore::new(8)),
            audio_slots: Arc::new(Semaphore::new(2)),
            connection_slots: Arc::new(Semaphore::new(64)),
            application_slots: Arc::new(Semaphore::new(1)),
            application_icon_slots: Arc::new(Semaphore::new(1)),
            control_request: Mutex::new(None),
            recent_control: Mutex::new(None),
            uploads: crate::resumable::Uploads::with_history(files.received.clone()),
            }),
        });
        let cleanup = context.clone();
        let renew = context.clone();
        let mut renew_stop = rx.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = renew_stop.changed() => break,
                    _ = tokio::time::sleep(Duration::from_secs(1)) => { renew.pairing_status(); },
                }
            }
        });
        let mut cleanup_stop = rx.clone();
        let initial_dir = crate::file_transfer::receive_dir(&store);
        tokio::task::spawn_blocking(move || {
            crate::file_transfer::cleanup_orphaned_parts(&initial_dir, Duration::from_secs(60));
        });
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cleanup_stop.changed() => { cleanup.uploads.clear(); break; },
                    _ = tokio::time::sleep(Duration::from_secs(30)) => cleanup.uploads.prune(|owner| cleanup.store.lock().unwrap().get().devices.iter().any(|d| d.id == owner && d.permissions.files)),
                }
            }
        });
        let (discovery, discovery_error) = if host.is_loopback() { (None, None) } else { match advertise(host, address.port(), &store) {
            Ok(daemon) => (Some(daemon), None),
            Err(error) => (None, Some(error)),
        }};
        let endpoint = ServingEndpoint::start(listener, tls.clone(), &context, !host.is_loopback(),usb_access);
        let (lan, usb) = (Some(endpoint), HashMap::new());
        Self {
            context,
            address,
            shutdown: tx,
            lan, usb, tls,
            discovery,
            discovery_error,
        }
    }
    pub fn usb_address(&self) -> Option<SocketAddr> { self.usb.values().map(|e|e.address).min() }
    pub fn usb_address_for(&self,id:&str)->Option<SocketAddr> {self.usb.get(id).map(|e|e.address)}
    pub fn usb_access_for(&self,id:&str)->Option<&str> {self.usb.get(id)?.usb_access.as_ref().map(usb_access::UsbAccess::key)}
    pub fn usb_addresses(&self)->Vec<SocketAddr> {let mut addresses:Vec<_>=self.usb.values().map(|e|e.address).collect();addresses.sort();addresses}
    pub fn lan_address(&self) -> Option<SocketAddr> { self.lan.as_ref().map(|e| e.address) }
    pub async fn ensure_usb(&mut self) -> Result<SocketAddr, String> {
        self.ensure_usb_for("").await
    }
    pub async fn ensure_usb_for(&mut self,id:&str)->Result<SocketAddr,String> {
        if let Some(address)=self.usb_address_for(id) {return Ok(address);}
        if !id.is_empty() {
            if let Some(endpoint)=self.usb.remove("") {let address=endpoint.address;self.usb.insert(id.to_string(),endpoint);return Ok(address);}
        }
        if self.usb.len()>=128 {return Err("连接的 USB 手机过多，请先断开部分手机".into());}
        let listener=fresh_usb_listener(None).await?;
        let endpoint = ServingEndpoint::start(listener, self.tls.clone(), &self.context, false,Some(usb_access::UsbAccess::new()?));
        let address = endpoint.address;
        self.usb.insert(id.to_string(),endpoint);
        Ok(address)
    }
    pub async fn stop_usb(&mut self) {
        let endpoints:Vec<_>=self.usb.drain().map(|(_,endpoint)|endpoint).collect();
        futures_util::future::join_all(endpoints.into_iter().map(ServingEndpoint::close)).await;
    }
    pub async fn stop_usb_for(&mut self,id:&str) {
        if let Some(endpoint)=self.usb.remove(id) {endpoint.close().await;}
    }
    pub(crate) async fn retain_usb(&mut self,ids:&[String]) {
        let keys:Vec<_>=self.usb.keys().filter(|key|!ids.contains(key)).cloned().collect();
        let endpoints:Vec<_>=keys.into_iter().filter_map(|key|self.usb.remove(&key)).collect();
        futures_util::future::join_all(endpoints.into_iter().map(ServingEndpoint::close)).await;
    }
    pub(crate) async fn rotate_usb_for(&mut self,id:&str) -> Result<SocketAddr,String> {
        let listener=fresh_usb_listener(None).await?;
        let endpoint=ServingEndpoint::start(listener,self.tls.clone(),&self.context,false,Some(usb_access::UsbAccess::new()?));
        let address=endpoint.address;
        self.stop_usb_for(id).await;self.usb.insert(id.to_string(),endpoint);Ok(address)
    }
    pub async fn stop_lan(&mut self) {
        if let Some(discovery) = self.discovery.take() { discovery.shutdown(); }
        if let Some(endpoint) = self.lan.take() { endpoint.close().await; }
    }
    pub(crate) async fn replace_lan(&mut self, listener: tokio::net::TcpListener) {
        let address = listener.local_addr().expect("bound LAN listener");
        let endpoint = ServingEndpoint::start(listener, self.tls.clone(), &self.context, true,None);
        self.stop_lan().await;
        self.lan = Some(endpoint); self.address = address;
        self.context = Arc::new(Context { origin: format!("{}://{address}", if self.tls.is_some() { "https" } else { "http" }),
            mdns_host: mdns_host_for(&self.context.store, address.port()), usb_access:None, ..(*self.context).clone() });
        if let std::net::IpAddr::V4(host) = address.ip() {
            match advertise(host, address.port(), &self.context.store) {
                Ok(discovery) => { self.discovery = Some(discovery); self.discovery_error = None; },
                Err(error) => self.discovery_error = Some(error),
            }
        }
    }
    pub fn discovery_active(&self) -> bool {
        self.discovery.is_some()
    }
    pub fn discovery_error(&self) -> Option<&str> {
        self.discovery_error.as_deref()
    }
    pub fn refresh_discovery(&self) {
        if let Some(discovery) = &self.discovery {
            discovery.refresh();
        }
    }
    pub async fn stop(&mut self) {
        let _ = self.shutdown.send(true);
        if let Some(endpoint)=&self.lan {let _=endpoint.stop.send(true);}
        for endpoint in self.usb.values() {let _=endpoint.stop.send(true);}
        self.context.controller.audio.stop();
        // Stop the active session without forgetting trusted credentials.
        for (_, session) in self.context.active.lock().unwrap().drain() {
            let _ = session.cancel.send(4000);
        }
        self.context.controller.revoke();
        self.stop_lan().await;
        self.stop_usb().await;
    }
}

fn tls_config(
    host: std::net::Ipv4Addr,
    store: &crate::preferences::SharedStore,
) -> Result<Arc<rustls::ServerConfig>, String> {
    use crate::preferences::TlsIdentity;
    // Multiple HTTP dependencies may compile both rustls providers. Select one
    // explicitly so TLS initialization never depends on Cargo feature unification.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let address = host.to_string();
    let identity = {
        let current = store.lock().unwrap().get().tls;
        // Keep the same certificate when DHCP changes the listening address.
        // The Android client pins its fingerprint; regenerating it on every IP
        // change would force an unnecessary trust reset.
        if let Some(identity) = current {
            identity
        } else {
            let rcgen::CertifiedKey { cert, signing_key } =
                rcgen::generate_simple_self_signed(vec![address.clone(), "localhost".into()])
                    .map_err(|e| format!("无法生成加密连接证书：{e}"))?;
            let identity = TlsIdentity {
                address: address.clone(),
                certificate: BASE64.encode(cert.der()),
                private_key: BASE64.encode(signing_key.serialize_der()),
            };
            store
                .lock()
                .unwrap()
                .update(|p| p.tls = Some(identity.clone()))?;
            identity
        }
    };
    let certificate = BASE64
        .decode(identity.certificate)
        .map_err(|_| "加密连接证书已损坏".to_string())?;
    let private_key = BASE64
        .decode(identity.private_key)
        .map_err(|_| "加密连接私钥已损坏".to_string())?;
    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(certificate)],
            PrivatePkcs8KeyDer::from(private_key).into(),
        )
        .map_err(|e| format!("无法加载加密连接证书：{e}"))?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        self.context.controller.revoke();
        if let Some(discovery) = self.discovery.take() {
            discovery.shutdown();
        }
        self.lan.take(); self.usb.clear();
    }
}
fn mdns_host_for(store: &crate::preferences::SharedStore, port: u16) -> Option<String> {
    let installation_id = store.lock().unwrap().get().installation_id;
    if installation_id.is_empty() {
        return None;
    }
    let suffix = &installation_id[..installation_id.len().min(12)];
    Some(format!("plumelink-{suffix}.local:{port}"))
}
fn advertise(
    host: std::net::Ipv4Addr,
    port: u16,
    store: &crate::preferences::SharedStore,
) -> Result<Discovery, String> {
    let installation_id = store.lock().unwrap().get().installation_id;
    if installation_id.is_empty() {
        return Err("电脑身份缺失，无法启动自动发现".into());
    }
    let daemon = mdns_sd::ServiceDaemon::new().map_err(|e| format!("自动发现初始化失败：{e}"))?;
    let suffix = &installation_id[..installation_id.len().min(12)];
    let hostname = format!("plumelink-{suffix}.local.");
    let instance = format!("PlumeLink-{suffix}");
    let properties = [
        ("id", installation_id.as_str()),
        ("scheme", "https"),
        ("version", env!("CARGO_PKG_VERSION")),
    ];
    let info = mdns_sd::ServiceInfo::new(
        "_plumelink._tcp.local.",
        &instance,
        &hostname,
        host.to_string(),
        port,
        &properties[..],
    )
    .map_err(|e| format!("自动发现服务信息无效：{e}"))?;
    daemon
        .register(info.clone())
        .map_err(|e| format!("自动发现广播失败：{e}"))?;
    Ok(Discovery { daemon, info })
}
fn json(status: StatusCode, value: serde_json::Value) -> Response {
    let mut response = (status, axum::Json(value)).into_response();
    let headers = response.headers_mut();
    headers.insert("cache-control", "no-store".parse().unwrap());
    headers.insert("x-content-type-options", "nosniff".parse().unwrap());
    response
}
fn error(status: StatusCode, message: &str) -> Response {
    json(status, serde_json::json!({"error":message}))
}
async fn http(
    State(ctx): State<Arc<Context>>,
    ConnectInfo(peer): ConnectInfo<crate::transport::Peer>,
    request: Request,
) -> Response {
    if !ctx.host_ok(request.headers()) {
        return error(StatusCode::FORBIDDEN, "Host 不匹配");
    }
    let path = request.uri().path().to_owned();
    let method = request.method().clone();
    if path.starts_with("/api/") {
        if !ctx.api_origin_ok(request.headers()) {
            return error(StatusCode::FORBIDDEN, "来源不匹配");
        }
        if path == "/api/identity" && method == Method::GET {
            let mut identity=serde_json::json!({
                "service": "plumelink", "version": 1, "id": ctx.store.lock().unwrap().get().installation_id
            });
            if let Some(access)=&ctx.usb_access {identity["usbSession"]=access.tag.clone().into();}
            return json(StatusCode::OK,identity);
        }
        if path == "/api/pair" && method == Method::POST {
            {
                let mut limits = ctx.limits.lock().unwrap();
                if !limits.pair.allow(&peer.0.ip().to_string()) || !limits.total.allow("all") {
                    return error(StatusCode::TOO_MANY_REQUESTS, "尝试过于频繁，请等待一分钟");
                }
            }
            if request
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                != Some("application/json")
            {
                return error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "需要 JSON");
            }
            let bytes = match tokio::time::timeout(
                Duration::from_secs(5),
                to_bytes(request.into_body(), 2048),
            )
            .await
            {
                Ok(Ok(body)) => body,
                _ => return error(StatusCode::BAD_REQUEST, "请求过大或读取超时"),
            };
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct PairRequest {
                code: String,
                #[serde(default, rename = "deviceId")]
                device_id: Option<String>,
                #[serde(default, rename = "deviceName")]
                device_name: String,
            }
            let body: PairRequest = match serde_json::from_slice(&bytes) {
                Ok(b) => b,
                Err(_) => return error(StatusCode::BAD_REQUEST, "请求无效"),
            };
            let attempt=ctx.while_live(||{
                let mut pairing=ctx.pairing.lock().unwrap();
                let result=pairing.pair_device(&body.code,body.device_id.as_deref(),&body.device_name);
                (result,pairing.expired())
            });
            return match attempt {
                Some((Ok(Some(result)),_)) => json(
                    StatusCode::OK,
                    serde_json::json!({"token":result.token,"deviceId":result.device_id}),
                ),
                Some((Ok(None),expired)) => error(
                    StatusCode::UNAUTHORIZED,
                    if expired {
                        "配对码已过期，请在电脑端重新配对"
                    } else {
                        "配对码不正确，请核对电脑上显示的 6 位数字"
                    },
                ),
                Some((Err(_),_)) => error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "随机数生成失败，服务拒绝配对",
                ),
                None=>error(StatusCode::SERVICE_UNAVAILABLE,"连接已失效"),
            };
        }
        let token = request
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");
        let Some(device) = ctx.pairing.lock().unwrap().device(token) else {
            return error(StatusCode::UNAUTHORIZED, "需要配对");
        };
        return if path == "/api/status" && method == Method::GET {
            json(
                StatusCode::OK,
                serde_json::json!({"connected":ctx.connected(),"connectedCount":ctx.connected_count(),"deviceId":device.id,"deviceName":device.name,"permissions":device.permissions}),
            )
        } else if path == "/api/device/name" && method == Method::POST {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct DeviceName { name: String, #[serde(default)] model: String }
            let credential = token.to_owned();
            let bytes = match tokio::time::timeout(Duration::from_secs(5), to_bytes(request.into_body(), 1024)).await {
                Ok(Ok(bytes)) => bytes,
                _ => return error(StatusCode::BAD_REQUEST, "请求过大或读取超时"),
            };
            let Ok(body) = serde_json::from_slice::<DeviceName>(&bytes) else { return error(StatusCode::BAD_REQUEST, "请求无效"); };
            let pairing = ctx.pairing.lock().unwrap();
            let Some(device) = pairing.device(&credential) else { return error(StatusCode::UNAUTHORIZED, "需要配对"); };
            match pairing.report_name(&device.id, &body.name, &body.model) {
                Ok(name) => json(StatusCode::OK, serde_json::json!({"deviceName":name})),
                Err(message) => error(StatusCode::BAD_REQUEST, &message),
            }
        } else if path == "/api/control/request" && method == Method::POST {
            if !device.permissions.input {
                return error(StatusCode::FORBIDDEN, "此设备没有遥控权限");
            }
            if !ctx.request_control(&device) {
                return error(StatusCode::CONFLICT, "设备当前未连接，无法请求主控");
            }
            json(StatusCode::OK, serde_json::json!({"requested":true}))
        } else if path == "/api/microphone/status" && method == Method::GET {
            if !device.permissions.microphone {
                return error(StatusCode::FORBIDDEN, "此设备没有麦克风权限");
            }
            let settings = ctx.store.lock().unwrap().get();
            let current = ctx.controller.audio.device_status(&device.id);
            json(
                StatusCode::OK,
                serde_json::json!({"configured":!settings.audio_output.is_empty(),"output":settings.audio_output_name,
                "allowed":true,"gainDb":settings.microphone_gain_db,
                "owned":current.owned,"busy":current.busy,"status":current.status}),
            )
        } else if path == "/api/files" && method == Method::GET {
            if !device.permissions.files {
                return error(StatusCode::FORBIDDEN, "此设备没有文件权限");
            }
            json(
                StatusCode::OK,
                serde_json::json!({"files":ctx.files.list(),"maxBytes":MAX_FILE_BYTES}),
            )
        } else if path == "/api/uploads" || path.starts_with("/api/uploads/") {
            if !device.permissions.files {
                return error(StatusCode::FORBIDDEN, "此设备没有文件权限");
            }
            let token = token.to_owned();
            resumable_upload(ctx.clone(), request, token, device.id).await
        } else if path.starts_with("/api/applications/launches/") && method == Method::GET {
            if !device.permissions.input { return error(StatusCode::FORBIDDEN, "此设备没有遥控权限"); }
            let request = path.strip_prefix("/api/applications/launches/").unwrap_or_default();
            if !crate::launcher::valid_id(request) { return error(StatusCode::BAD_REQUEST, "启动请求无效"); }
            match ctx.controller.launch_queue.as_ref().and_then(|queue| queue.status(&device.id, request)) {
                Some(report) => json(StatusCode::OK, serde_json::json!(report)),
                None => error(StatusCode::NOT_FOUND, "未找到启动结果，请检查电脑；不会自动重发"),
            }
        } else if path == "/api/applications/icons" && method == Method::GET {
            if !device.permissions.input {
                return error(StatusCode::FORBIDDEN, "此设备没有遥控权限");
            }
            let icons = crate::launcher::icons(ctx.store.clone(), ctx.application_icon_slots.clone()).await;
            if !ctx.pairing.lock().unwrap().device(token).is_some_and(|current| current.permissions.input) {
                return error(StatusCode::FORBIDDEN, "此设备没有遥控权限");
            }
            match icons {
                Ok(icons) => json(StatusCode::OK, serde_json::json!({"icons":icons})),
                Err(message) => error(StatusCode::SERVICE_UNAVAILABLE, &message),
            }
        } else if path == "/api/applications" && method == Method::GET {
            if !device.permissions.input {
                return error(StatusCode::FORBIDDEN, "此设备没有遥控权限");
            }
            let entries = match crate::launcher::catalog(ctx.store.clone(), ctx.application_slots.clone()).await {
                Ok(entries) => entries,
                Err(message) => return error(StatusCode::SERVICE_UNAVAILABLE, &message),
            };
            if !ctx.pairing.lock().unwrap().device(token).is_some_and(|current| current.permissions.input) {
                return error(StatusCode::FORBIDDEN, "此设备没有遥控权限");
            }
            json(StatusCode::OK, serde_json::json!({"applications": entries, "launchJobs":ctx.controller.launch_queue.is_some()}))
        } else if path == "/api/windows" && method == Method::GET {
            if !device.permissions.input {
                return error(StatusCode::FORBIDDEN, "此设备没有遥控权限");
            }
            if !ctx.store.lock().unwrap().primary_allows(&device.id) {
                return error(StatusCode::FORBIDDEN, "当前无主控权");
            }
            let windows = ctx.controller.probe_windows().await;
            let current = ctx.pairing.lock().unwrap().device(token);
            let Some(current) = current else {
                return error(StatusCode::UNAUTHORIZED, "配对已失效，请重新配对");
            };
            if !current.permissions.input || !ctx.store.lock().unwrap().primary_allows(&current.id) {
                return error(StatusCode::FORBIDDEN, "此设备没有遥控权限");
            }
            return match windows {
                Ok(windows) => json(StatusCode::OK, serde_json::json!({"windows":windows})),
                Err(message) => error(StatusCode::INTERNAL_SERVER_ERROR, &message),
            };
        } else if path == "/api/windows/switch" && method == Method::POST {
            if !device.permissions.input {
                return error(StatusCode::FORBIDDEN, "此设备没有遥控权限");
            }
            let credential = token.to_owned();
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct SwitchRequest {
                hwnd: u64,
            }
            let bytes = match tokio::time::timeout(
                Duration::from_secs(5),
                to_bytes(request.into_body(), 512),
            )
            .await
            {
                Ok(Ok(body)) => body,
                _ => return error(StatusCode::BAD_REQUEST, "请求过大或读取超时"),
            };
            let Ok(body) = serde_json::from_slice::<SwitchRequest>(&bytes) else {
                return error(StatusCode::BAD_REQUEST, "请求无效");
            };
            if !ctx.store.lock().unwrap().primary_allows(&device.id) {
                return error(StatusCode::FORBIDDEN, "当前无主控权");
            }
            let current = ctx.clone();
            return match ctx.controller.switch_window(body.hwnd, move || current.window_access(&credential)).await {
                Ok(()) => json(StatusCode::OK, serde_json::json!({"switched":true})),
                Err(message) if message == "配对已失效，请重新配对" => error(StatusCode::UNAUTHORIZED, &message),
                Err(message) if matches!(message.as_str(), "此设备没有遥控权限" | "当前无主控权") => error(StatusCode::FORBIDDEN, &message),
                Err(message) => error(StatusCode::CONFLICT, &message),
            };
        } else if method == Method::GET {
            if !device.permissions.files {
                return error(StatusCode::FORBIDDEN, "此设备没有文件权限");
            }
            match path
                .strip_prefix("/api/files/")
                .and_then(|v| v.parse::<u64>().ok())
            {
                Some(id) => send_file(ctx.clone(), id, token.to_owned(), request.headers()).await,
                None => error(StatusCode::NOT_FOUND, "没有此接口"),
            }
        } else {
            error(StatusCode::NOT_FOUND, "没有此接口")
        };
    }
    error(StatusCode::NOT_FOUND, "此地址仅供羽连 Android 应用连接")
}

async fn send(socket: &mut WebSocket, value: serde_json::Value) -> bool {
    send_frame(socket, Message::Text(value.to_string().into())).await
}
async fn send_frame(socket: &mut WebSocket, frame: Message) -> bool {
    matches!(
        tokio::time::timeout(Duration::from_secs(1), socket.send(frame)).await,
        Ok(Ok(()))
    )
}
async fn close(socket: &mut WebSocket, code: u16) {
    if tokio::time::timeout(
        Duration::from_millis(500),
        socket.send(Message::Close(Some(CloseFrame {
            code,
            reason: "PlumeLink connection closed".into(),
        }))),
    )
    .await
    .is_ok()
    {
        let _ = tokio::time::timeout(Duration::from_millis(100), async {
            while let Some(Ok(frame)) = socket.recv().await {
                if matches!(frame, Message::Close(_)) {
                    break;
                }
            }
        })
        .await;
    }
}

#[cfg(test)]
mod endpoint_tests {
    use super::*;
    struct NoInput;
    impl crate::control::Native for NoInput {
        fn execute(&mut self,_:&crate::protocol::Command)->Result<(),String> {Err("input disabled".into())}
        fn release(&mut self)->Result<(),String> {Ok(())}
    }
    #[tokio::test]
    async fn usb_port_history_rejects_forced_old_candidates_and_survives_server_recreation() {
        let candidate=tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST,0)).await.unwrap();let old=candidate.local_addr().unwrap().port();let _=claim_usb_port(old);
        let fresh=fresh_usb_listener(Some(candidate)).await.unwrap();let fresh_port=fresh.local_addr().unwrap().port();assert_ne!(fresh_port,old);assert!(!claim_usb_port(fresh_port));drop(fresh);
        let controller=Controller::start(||Ok(Box::new(NoInput))).unwrap();let store=crate::preferences::Store::memory();let files=FileHub::new();let mut ports=std::collections::HashSet::from([old,fresh_port]);
        for _ in 0..3 {let mut server=Server::start_usb_only(controller.clone(),store.clone(),files.clone()).await.unwrap();assert!(ports.insert(server.usb_address().unwrap().port()));server.ensure_usb_for("fixture").await.unwrap();assert!(ports.insert(server.rotate_usb_for("fixture").await.unwrap().port()));server.stop().await;}
        controller.shutdown();
    }
    #[tokio::test]
    async fn a_stopped_endpoint_cannot_commit_a_pairing_body_already_being_read() {
        let store=crate::preferences::Store::memory();let controller=Controller::start(||Ok(Box::new(NoInput))).unwrap();
        let mut server=Server::start_with_store(std::net::Ipv4Addr::LOCALHOST,0,controller.clone(),store.clone()).await.unwrap();
        let (stop,signal)=watch::channel(false);let view=Arc::new(Context{shutdown:signal,..(*server.context).clone()});
        let (body_sender,body_receiver)=tokio::sync::oneshot::channel::<Vec<u8>>();let entered=Arc::new(tokio::sync::Notify::new());let reading=entered.clone();
        let body=axum::body::Body::from_stream(futures_util::stream::once(async move {reading.notify_one();body_receiver.await.map(axum::body::Bytes::from).map_err(|_|std::io::Error::other("fixture closed"))}));
        let request=Request::builder().method(Method::POST).uri("/api/pair").header("host",server.address.to_string()).header("origin",&server.context.origin).header("content-type","application/json").body(body).unwrap();
        let pending=tokio::spawn(http(State(view),ConnectInfo(crate::transport::Peer("127.0.0.1:1".parse().unwrap(),None)),request));
        tokio::time::timeout(Duration::from_secs(1),entered.notified()).await.unwrap();stop.send(true).unwrap();
        body_sender.send(serde_json::to_vec(&serde_json::json!({"code":server.context.code()})).unwrap()).unwrap();
        assert_eq!(pending.await.unwrap().status(),StatusCode::SERVICE_UNAVAILABLE);assert!(store.lock().unwrap().get().devices.is_empty());
        let response=reqwest::Client::builder().no_proxy().build().unwrap().post(format!("{}/api/pair",server.context.origin)).json(&serde_json::json!({"code":server.context.code()})).send().await.unwrap();
        assert_eq!(response.status(),200);assert_eq!(store.lock().unwrap().get().devices.len(),1);server.stop().await;controller.shutdown();
    }
}

#[cfg(test)]
mod window_permission_tests {
    use super::*;
    use crate::{control::Native, protocol::Command};
    use std::sync::{atomic::AtomicUsize, mpsc};

    struct Input {
        switches: Arc<AtomicUsize>,
        started: mpsc::SyncSender<()>,
        resume: mpsc::Receiver<()>,
    }
    impl Native for Input {
        fn execute(&mut self, _: &Command) -> Result<(), String> { Ok(()) }
        fn release(&mut self) -> Result<(), String> { Ok(()) }
        fn probe_windows(&mut self) -> Result<serde_json::Value, String> {
            self.started.send(()).unwrap();
            self.resume.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(serde_json::json!([]))
        }
        fn switch_to_window(&mut self, _: u64) -> Result<(), String> {
            self.switches.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn queued_window_switch_rechecks_current_authorization_before_execution() {
        let mut outcomes = Vec::new();
        for change in ["none", "input", "primary", "pairing", "token"] {
            let switches = Arc::new(AtomicUsize::new(0));
            let (started, ready) = mpsc::sync_channel(1);
            let (resume, wait) = mpsc::channel();
            let backend = Input { switches: switches.clone(), started, resume: wait };
            let controller = Controller::start(move || Ok(Box::new(backend))).unwrap();
            let store = crate::preferences::Store::memory();
            let mut server = Server::start_with_store(std::net::Ipv4Addr::LOCALHOST, 0, controller.clone(), store.clone()).await.unwrap();
            let code = server.context.code();
            let token = server.context.pairing.lock().unwrap().pair(&code).unwrap().unwrap();
            let other = controller.clone();
            let blocked = tokio::spawn(async move { other.probe_windows().await });
            tokio::task::spawn_blocking(move || ready.recv().unwrap()).await.unwrap();
            let request = Request::builder().method(Method::POST).uri("/api/windows/switch")
                .header("host", server.context.origin.parse::<axum::http::Uri>().unwrap().authority().unwrap().as_str())
                .header("origin", &server.context.origin).header("authorization", format!("Bearer {token}"))
                .body(axum::body::Body::from("{\"hwnd\":42}")).unwrap();
            let mut pending = std::pin::pin!(http(State(server.context.clone()), ConnectInfo(crate::transport::Peer("127.0.0.1:1".parse().unwrap(),None)), request));
            assert!(futures_util::poll!(pending.as_mut()).is_pending(), "request should enqueue behind blocked native input");
            store.lock().unwrap().update(|prefs| match change {
                "input" => prefs.devices[0].permissions.input = false,
                "primary" => { prefs.control_mode = crate::preferences::ControlMode::Primary; prefs.primary_device = Some("other-device".into()); }
                "pairing" => prefs.devices.clear(),
                "token" => prefs.devices[0].token_digest = "0".repeat(64),
                _ => {},
            }).unwrap();
            resume.send(()).unwrap();
            let response = pending.await;
            let _ = blocked.await.unwrap();
            let count = switches.load(Ordering::SeqCst);
            server.stop().await;
            controller.shutdown();
            outcomes.push((change, response.status(), count));
        }
        for (change, status, count) in outcomes {
            assert_eq!(status, match change { "none" => StatusCode::OK, "pairing" | "token" => StatusCode::UNAUTHORIZED, _ => StatusCode::FORBIDDEN }, "{change}");
            assert_eq!(count, usize::from(change == "none"), "{change}: unauthorized request reached native window control");
        }
    }
}

#[cfg(test)]
mod certificate_tests {
    use super::*;

    #[test]
    fn implicit_https_port_is_only_valid_on_443() {
        let origin = "https://192.168.1.10:443";
        let mdns = Some("pc.local:443");
        for candidate in [
            "https://192.168.1.10",
            origin,
            "https://PC.local",
            "https://pc.local:443",
        ] {
            assert!(allowed_origin(origin, mdns, candidate));
        }
        for candidate in [
            "http://pc.local",
            "https://pc.local:80",
            "https://pc.local:8443",
            "https://other.local",
        ] {
            assert!(!allowed_origin(origin, mdns, candidate));
        }
        assert!(!allowed_host(
            "https://192.168.1.10:8443",
            None,
            "192.168.1.10"
        ));
    }
    #[test]
    fn address_change_keeps_trusted_certificate() {
        let store = crate::preferences::Store::memory();
        tls_config("192.168.1.10".parse().unwrap(), &store).unwrap();
        let first = store.lock().unwrap().get().tls.unwrap();
        tls_config("192.168.2.10".parse().unwrap(), &store).unwrap();
        let second = store.lock().unwrap().get().tls.unwrap();
        assert_eq!(first.certificate, second.certificate);
        assert_eq!(first.private_key, second.private_key);
    }

    #[test]
    fn mdns_local_host_and_origin_are_accepted_while_forged_hosts_fail() {
        let store = crate::preferences::Store::memory();
        store
            .lock()
            .unwrap()
            .update(|prefs| prefs.installation_id = "0123456789abcdef01234567".into())
            .unwrap();
        let origin = "https://192.168.1.10:19827";
        let mdns = mdns_host_for(&store, 19827);
        let mut ip_headers = HeaderMap::new();
        ip_headers.insert("host", "192.168.1.10:19827".parse().unwrap());
        ip_headers.insert("origin", "https://192.168.1.10:19827".parse().unwrap());
        assert!(header_origin_ok(origin, mdns.as_deref(), &ip_headers));
        assert!(header_api_origin_ok(origin, mdns.as_deref(), &ip_headers));

        let mut mdns_headers = HeaderMap::new();
        mdns_headers.insert(
            "host",
            "PlumeLink-0123456789ab.local:19827".parse().unwrap(),
        );
        mdns_headers.insert(
            "origin",
            "https://plumelink-0123456789ab.local:19827"
                .parse()
                .unwrap(),
        );
        assert!(header_origin_ok(origin, mdns.as_deref(), &mdns_headers));
        assert!(header_api_origin_ok(origin, mdns.as_deref(), &mdns_headers));

        let mut evil_headers = HeaderMap::new();
        evil_headers.insert("host", "evil.test:19827".parse().unwrap());
        evil_headers.insert("origin", "https://evil.test:19827".parse().unwrap());
        assert!(!header_host_ok(origin, mdns.as_deref(), &evil_headers));
        assert!(!header_origin_ok(origin, mdns.as_deref(), &evil_headers));
        assert!(!header_api_origin_ok(
            origin,
            mdns.as_deref(),
            &evil_headers
        ));
    }
}
