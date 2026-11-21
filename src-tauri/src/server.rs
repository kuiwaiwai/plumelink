mod audio;
mod files;
mod input;
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
pub struct Context {
    pub origin: String,
    mdns_host: Option<String>,
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
    shutdown: watch::Receiver<bool>,
    control_request: Mutex<Option<ControlRequest>>,
    recent_control: Mutex<Option<(String, Instant)>>,
    uploads: crate::resumable::Uploads,
}
impl Context {
    fn files_allowed(&self, token: &str) -> bool {
        !*self.shutdown.borrow()
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
    ) -> Option<(u64, watch::Receiver<u16>, crate::security::AuthorizedDevice)> {
        let pairing = self.pairing.lock().unwrap();
        let device = pairing.device(token)?;
        pairing.mark_seen(&device.id);
        drop(pairing);
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = watch::channel(0);
        self.controller.activate(id);
        self.active.lock().unwrap().insert(
            id,
            Session {
                cancel: tx,
                device_id: device.id.clone(),
            },
        );
        Some((id, rx, device))
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
        header_host_ok(&self.origin, self.mdns_host.as_deref(), headers)
    }
    fn origin_ok(&self, headers: &HeaderMap) -> bool {
        header_origin_ok(&self.origin, self.mdns_host.as_deref(), headers)
    }
    fn api_origin_ok(&self, headers: &HeaderMap) -> bool {
        header_api_origin_ok(&self.origin, self.mdns_host.as_deref(), headers)
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
    task: Option<JoinHandle<()>>,
    discovery: Option<Discovery>,
    discovery_error: Option<String>,
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
            pairing: Mutex::new(pairing),
            limits: Mutex::new(Limits {
                pair: Limiter::new(6, 60),
                total: Limiter::new(30, 60),
                upgrade: Limiter::new(30, 60),
            }),
            active: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            controller,
            files,
            store: store.clone(),
            file_slots: Arc::new(Semaphore::new(1)),
            download_slots: Arc::new(Semaphore::new(4)),
            slots: Arc::new(Semaphore::new(8)),
            audio_slots: Arc::new(Semaphore::new(2)),
            shutdown: rx.clone(),
            control_request: Mutex::new(None),
            recent_control: Mutex::new(None),
            uploads: crate::resumable::Uploads::default(),
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
        let (discovery, discovery_error) = match advertise(host, address.port(), &store) {
            Ok(daemon) => (Some(daemon), None),
            Err(error) => (None, Some(error)),
        };
        let routes = Router::new()
            .route("/ws", get(upgrade))
            .route("/microphone", get(upgrade_microphone))
            .fallback(http)
            .with_state(context.clone());
        let mut stop = rx;
        let task = tokio::spawn(async move {
            if let Some(config) = tls {
                let _ = axum::serve(
                    crate::transport::TlsListener::new(listener, config),
                    routes.into_make_service_with_connect_info::<crate::transport::Peer>(),
                )
                .with_graceful_shutdown(async move {
                    let _ = stop.changed().await;
                })
                .await;
            } else {
                let _ = axum::serve(
                    crate::transport::LimitedListener::new(listener),
                    routes.into_make_service_with_connect_info::<crate::transport::Peer>(),
                )
                .with_graceful_shutdown(async move {
                    let _ = stop.changed().await;
                })
                .await;
            }
        });
        Self {
            context,
            address,
            shutdown: tx,
            task: Some(task),
            discovery,
            discovery_error,
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
        self.context.controller.audio.stop();
        // Stop the active session without forgetting trusted credentials.
        for (_, session) in self.context.active.lock().unwrap().drain() {
            let _ = session.cancel.send(4000);
        }
        self.context.controller.revoke();
        if let Some(discovery) = self.discovery.take() {
            discovery.shutdown();
        }
        let _ = self.shutdown.send(true);
        if let Some(mut task) = self.task.take() {
            if tokio::time::timeout(Duration::from_secs(3), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
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
        if let Some(task) = self.task.take() {
            task.abort();
        }
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
            let mut pairing = ctx.pairing.lock().unwrap();
            return match pairing.pair_device(
                &body.code,
                body.device_id.as_deref(),
                &body.device_name,
            ) {
                Ok(Some(result)) => json(
                    StatusCode::OK,
                    serde_json::json!({"token":result.token,"deviceId":result.device_id}),
                ),
                Ok(None) => error(
                    StatusCode::UNAUTHORIZED,
                    if pairing.expired() {
                        "配对码已过期，请在电脑端重新配对"
                    } else {
                        "配对码不正确，请核对电脑上显示的 6 位数字"
                    },
                ),
                Err(_) => error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "随机数生成失败，服务拒绝配对",
                ),
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
        } else if path == "/api/control/request" && method == Method::POST {
            if !device.permissions.input {
                return error(StatusCode::FORBIDDEN, "此设备没有遥控权限");
            }
            if !ctx.request_control(&device) {
                return error(StatusCode::CONFLICT, "设备当前未连接，无法请求主控");
            }
            json(StatusCode::OK, serde_json::json!({"requested":true}))
        } else if path == "/api/microphone/status" && method == Method::GET {
            let settings = ctx.store.lock().unwrap().get();
            json(
                StatusCode::OK,
                serde_json::json!({"configured":!settings.audio_output.is_empty(),"output":settings.audio_output_name,
                "allowed":device.permissions.microphone,"status":ctx.controller.audio.status()}),
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
        } else if path == "/api/windows" && method == Method::GET {
            if !device.permissions.input {
                return error(StatusCode::FORBIDDEN, "此设备没有遥控权限");
            }
            if !ctx.store.lock().unwrap().primary_allows(&device.id) {
                return error(StatusCode::FORBIDDEN, "当前无主控权");
            }
            return match ctx.controller.probe_windows().await {
                Ok(windows) => json(StatusCode::OK, serde_json::json!({"windows":windows})),
                Err(message) => error(StatusCode::INTERNAL_SERVER_ERROR, &message),
            };
        } else if path == "/api/windows/switch" && method == Method::POST {
            if !device.permissions.input {
                return error(StatusCode::FORBIDDEN, "此设备没有遥控权限");
            }
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
            return match ctx.controller.switch_window(body.hwnd).await {
                Ok(()) => json(StatusCode::OK, serde_json::json!({"switched":true})),
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
