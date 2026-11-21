use crate::{
    control::Controller,
    file_transfer::{self, FileHub, SharedFileInfo},
    network::{self, Address},
    server::Server,
    windows_control::WindowsControl,
};
use serde::{Deserialize, Serialize};
use std::{
    net::Ipv4Addr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::sync::Mutex;
#[derive(Clone, Serialize)]
pub struct DesktopState {
    pub running: bool,
    pub discovery: DiscoveryState,
    pub connected: bool,
    #[serde(rename = "connectedCount")]
    pub connected_count: usize,
    #[serde(rename = "recentController")]
    pub recent_controller: Option<String>,
    pub code: String,
    #[serde(rename = "codeRemaining")]
    pub code_remaining: u64,
    #[serde(rename = "pairingIntervalMinutes")]
    pub pairing_interval_minutes: u16,
    pub addresses: Vec<Address>,
    pub selected: String,
    pub port: u16,
    pub error: String,
    pub autostart: bool,
    pub trusted: bool,
    #[serde(rename = "pairedCount")]
    pub paired_count: usize,
    pub devices: Vec<DeviceInfo>,
    #[serde(rename = "controlMode")]
    pub control_mode: crate::preferences::ControlMode,
    #[serde(rename = "primaryDevice")]
    pub primary_device: Option<String>,
    #[serde(rename = "controlRequest")]
    pub control_request: Option<ControlRequestInfo>,
    pub halo: crate::preferences::HaloSettings,
    #[serde(rename = "receivePath")]
    pub receive_path: String,
    #[serde(rename = "sharedFiles")]
    pub shared_files: Vec<SharedFileInfo>,
    pub admin: crate::admin_control::AdminStatus,
    pub microphone: crate::microphone::Status,
}
#[derive(Clone, Serialize)]
pub struct DiscoveryState {
    pub active: bool,
    pub error: String,
}
#[derive(Clone, Serialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub online: bool,
    pub permissions: crate::preferences::DevicePermissions,
    #[serde(rename = "pairedAt")]
    pub paired_at: u64,
    #[serde(rename = "lastSeen")]
    pub last_seen: u64,
}
#[derive(Clone, Serialize)]
pub struct ControlRequestInfo {
    pub id: String,
    pub name: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Hide,
    Start,
    Stop,
    Disconnect,
    Pair,
    Pairinginterval0,
    Pairinginterval5,
    Pairinginterval10,
    Pairinginterval15,
    Pairinginterval30,
    Quit,
    Autostarton,
    Autostartoff,
    Haloon,
    Halooff,
    Halosmall,
    Halomedium,
    Halolarge,
}
struct State {
    server: Option<Server>,
    selected: String,
    error: String,
    missing_address_polls: u8,
    last_snapshot: Option<(std::time::Instant, std::time::SystemTime)>,
}
pub struct Service {
    state: Mutex<State>,
    pub controller: Arc<Controller>,
    pub exiting: AtomicBool,
    pub store: crate::preferences::SharedStore,
    pub files: Arc<FileHub>,
    pub desired_running: AtomicBool,
}
impl Service {
    pub fn new(store: crate::preferences::SharedStore) -> Result<Arc<Self>, String> {
        let input_store = store.clone();
        let admin = Arc::new(crate::admin_control::AdminManager::with_store(store.clone()));
        let input_admin = admin.clone();
        let controller = Controller::start_with_admin(
            move || {
                Ok(Box::new(WindowsControl::with_store_and_admin(
                    input_store,
                    Some(input_admin),
                )?))
            },
            admin,
        )?;
        let addresses = network::addresses().unwrap_or_default();
        let saved = store.lock().unwrap().get().address;
        let selected = addresses
            .iter()
            .find(|a| a.address == saved)
            .or_else(|| addresses.iter().find(|a| !a.virtual_adapter))
            .or_else(|| addresses.first())
            .map(|a| a.address.clone())
            .unwrap_or_default();
        Ok(Arc::new(Self {
            state: Mutex::new(State {
                server: None,
                selected,
                error: String::new(),
                missing_address_polls: 0,
                last_snapshot: None,
            }),
            controller,
            exiting: AtomicBool::new(false),
            store,
            files: FileHub::new(),
            desired_running: AtomicBool::new(true),
        }))
    }
    pub async fn snapshot(&self) -> DesktopState {
        let addresses = network::addresses();
        let now_mono = std::time::Instant::now();
        let now_wall = std::time::SystemTime::now();
        let addresses = {
            let mut state = self.state.lock().await;
            let woke_from_sleep = state.last_snapshot.is_some_and(|(prev_mono, prev_wall)| {
                let wall_delta = now_wall.duration_since(prev_wall).unwrap_or_default();
                let mono_delta = now_mono.saturating_duration_since(prev_mono);
                wall_delta.saturating_sub(mono_delta) >= std::time::Duration::from_secs(5)
            });
            state.last_snapshot = Some((now_mono, now_wall));
            let addresses = match addresses {
                Ok(a) => a,
                Err(e) => {
                    state.error = e;
                    vec![]
                }
            };
            if state.server.is_some() {
                if addresses.iter().any(|a| a.address == state.selected) {
                    state.missing_address_polls = 0;
                    if woke_from_sleep {
                        if let Some(server) = state.server.as_ref() {
                            server.refresh_discovery();
                        }
                    }
                } else {
                    state.missing_address_polls = state.missing_address_polls.saturating_add(1);
                    if state.missing_address_polls >= 3 {
                        state.missing_address_polls = 0;
                        if let Some(mut server) = state.server.take() {
                            server.stop().await;
                        }
                        state.error = "连接地址已变化，请重新选择网卡并启动遥控".into();
                    }
                }
            } else {
                state.missing_address_polls = 0;
            }
            addresses
        };
        let state = self.state.lock().await;
        let (preferences, halo_error) = {
            let store = self.store.lock().unwrap();
            (store.get(), store.halo_error.clone())
        };
        let connected_count = state
            .server
            .as_ref()
            .map(|server| server.context.connected_count())
            .unwrap_or(0);
        let online = state
            .server
            .as_ref()
            .map(|server| server.context.online_device_ids())
            .unwrap_or_default();
        let devices = preferences
            .devices
            .iter()
            .map(|device| DeviceInfo {
                id: device.id.clone(),
                name: device.name.clone(),
                online: online.contains(&device.id),
                permissions: device.permissions,
                paired_at: device.paired_at,
                last_seen: device.last_seen,
            })
            .collect();
        let control_request = state
            .server
            .as_ref()
            .and_then(|server| server.context.pending_control_request())
            .map(|(id, name)| ControlRequestInfo { id, name });
        let (code, code_remaining) = state
            .server
            .as_ref()
            .map(|server| server.context.pairing_status())
            .unwrap_or_default();
        DesktopState {
            running: state.server.is_some(),
            discovery: DiscoveryState {
                active: state.server.as_ref().is_some_and(Server::discovery_active),
                error: state
                    .server
                    .as_ref()
                    .and_then(Server::discovery_error)
                    .unwrap_or_default()
                    .to_string(),
            },
            connected: connected_count > 0,
            connected_count,
            recent_controller: state
                .server
                .as_ref()
                .and_then(|server| server.context.recent_controller()),
            code,
            code_remaining,
            pairing_interval_minutes: preferences.pairing_interval_minutes,
            addresses,
            selected: state.selected.clone(),
            port: state.server.as_ref().map_or(preferences.service_port, |s| s.address.port()),
            error: if state.error.is_empty() {
                halo_error
            } else {
                state.error.clone()
            },
            autostart: false,
            trusted: !preferences.devices.is_empty(),
            paired_count: preferences.devices.len(),
            devices,
            control_mode: preferences.control_mode,
            primary_device: preferences.primary_device,
            control_request,
            halo: preferences.halo,
            receive_path: file_transfer::receive_dir(&self.store)
                .to_string_lossy()
                .into_owned(),
            shared_files: self.files.list(),
            admin: self.controller.admin.status(),
            microphone: self.controller.audio.status(),
        }
    }
    pub async fn action(&self, action: Action, address: Option<String>) -> DesktopState {
        self.action_inner(action, address, false).await
    }
    pub async fn set_port(
        &self,
        port: u16,
        address: Option<String>,
    ) -> Result<DesktopState, String> {
        crate::preferences::validate_port(port)?;
        {
            let mut state = self.state.lock().await;
            if self.exiting.load(Ordering::SeqCst) {
                return Err("服务正在退出".into());
            }
            let changed = self.store.lock().unwrap().get().service_port != port;
            if changed {
                if state.server.is_some() {
                    let host = state
                        .selected
                        .parse::<Ipv4Addr>()
                        .map_err(|e| e.to_string())?;
                    // Keep the old service intact through every fallible step, including disk I/O.
                    let prepared = Server::prepare(host, port, &self.store, true).await?;
                    self.store
                        .lock()
                        .unwrap()
                        .update(|p| p.service_port = port)?;
                    if let Some(mut old) = state.server.take() {
                        old.stop().await;
                    }
                    state.server = Some(Server::launch(
                        prepared,
                        self.controller.clone(),
                        self.store.clone(),
                        self.files.clone(),
                    ));
                } else {
                    let selected = address
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .unwrap_or(&state.selected);
                    let addresses = network::addresses()?;
                    // With no LAN attached the real bind check happens at the next start.
                    let _reservation = if addresses.is_empty() {
                        None
                    } else {
                        if !addresses.iter().any(|a| a.address == selected) {
                            return Err("没有可用的局域网 IPv4 地址，请选择网卡".into());
                        }
                        let host = selected.parse::<Ipv4Addr>().map_err(|e| e.to_string())?;
                        Some(Server::bind(host, port).await?)
                    };
                    self.store
                        .lock()
                        .unwrap()
                        .update(|p| p.service_port = port)?;
                }
                state.error.clear();
                crate::logging::info("service", &format!("service port changed to {port}"));
            }
        }
        Ok(self.snapshot().await)
    }
    pub async fn retry_start(&self) -> DesktopState {
        self.action_inner(Action::Start, None, true).await
    }
    async fn action_inner(
        &self,
        action: Action,
        address: Option<String>,
        retry: bool,
    ) -> DesktopState {
        {
            let mut state = self.state.lock().await;
            match action {
                Action::Start => {
                    if !retry {
                        self.desired_running.store(true, Ordering::SeqCst);
                    }
                    if state.server.is_none()
                        && self.desired_running.load(Ordering::SeqCst)
                        && !self.exiting.load(Ordering::SeqCst)
                    {
                        let addresses = network::addresses().unwrap_or_default();
                        if let Some(address) = address {
                            if addresses.iter().any(|a| a.address == address) {
                                state.selected = address;
                            }
                        }
                        if !addresses.iter().any(|a| a.address == state.selected) {
                            state.selected = addresses
                                .iter()
                                .find(|a| !a.virtual_adapter)
                                .or_else(|| addresses.first())
                                .map(|a| a.address.clone())
                                .unwrap_or_default();
                        }
                        if !addresses.iter().any(|a| a.address == state.selected) {
                            state.error = "没有可用的局域网 IPv4 地址，请选择网卡".into();
                            crate::logging::warn("service", "start skipped: no LAN IPv4 address");
                        } else {
                            let host = state.selected.parse::<Ipv4Addr>().unwrap();
                            let port = self.store.lock().unwrap().get().service_port;
                            match Server::start_secure_with_files(
                                host,
                                port,
                                self.controller.clone(),
                                self.store.clone(),
                                self.files.clone(),
                            )
                            .await
                            {
                                Ok(server) => {
                                    crate::logging::info(
                                        "service",
                                        &format!("server started on {}", server.address),
                                    );
                                    state.server = Some(server);
                                    state.missing_address_polls = 0;
                                    state.error.clear();
                                    if let Err(e) = self
                                        .store
                                        .lock()
                                        .unwrap()
                                        .update(|p| p.address = state.selected.clone())
                                    {
                                        state.error = e;
                                    }
                                }
                                Err(error) => {
                                    crate::logging::error(
                                        "service",
                                        &format!("server start failed: {error}"),
                                    );
                                    state.error = error;
                                }
                            }
                        }
                    }
                }
                Action::Stop | Action::Quit => {
                    crate::logging::info(
                        "service",
                        if matches!(action, Action::Quit) {
                            "service quitting"
                        } else {
                            "service stopped"
                        },
                    );
                    self.desired_running.store(false, Ordering::SeqCst);
                    if matches!(action, Action::Quit) {
                        self.controller.admin.stop();
                        self.exiting.store(true, Ordering::SeqCst);
                    }
                    if let Some(mut server) = state.server.take() { server.stop().await; }
                    state.missing_address_polls = 0;
                    self.controller.revoke();
                }
                Action::Haloon
                | Action::Halooff
                | Action::Halosmall
                | Action::Halomedium
                | Action::Halolarge => {
                    use crate::preferences::HaloSize;
                    let result = self.store.lock().unwrap().update(|p| match action {
                        Action::Haloon => p.halo.enabled = true,
                        Action::Halooff => p.halo.enabled = false,
                        Action::Halosmall => p.halo.size = HaloSize::Small,
                        Action::Halomedium => p.halo.size = HaloSize::Medium,
                        Action::Halolarge => p.halo.size = HaloSize::Large,
                        _ => unreachable!(),
                    });
                    match result {
                        Ok(()) => state.error.clear(),
                        Err(e) => state.error = e,
                    }
                }
                Action::Hide | Action::Autostarton | Action::Autostartoff => {}
                Action::Pair => {
                    let result = if let Some(server) = state.server.as_ref() {
                        server.context.refresh_pairing_code()
                    } else {
                        Ok(())
                    };
                    if let Err(error) = result {
                        state.error = error;
                    }
                }
                Action::Pairinginterval0 | Action::Pairinginterval5 | Action::Pairinginterval10 | Action::Pairinginterval15 | Action::Pairinginterval30 => {
                    let minutes = match action {
                        Action::Pairinginterval0 => 0,
                        Action::Pairinginterval5 => 5,
                        Action::Pairinginterval10 => 10,
                        Action::Pairinginterval15 => 15,
                        Action::Pairinginterval30 => 30,
                        _ => unreachable!(),
                    };
                    let result = if let Some(server) = state.server.as_ref() {
                        server.context.set_pairing_interval(minutes)
                    } else {
                        self.store.lock().unwrap().update(|p| p.pairing_interval_minutes = minutes)
                    };
                    match result {
                        Ok(()) => state.error.clear(),
                        Err(error) => state.error = error,
                    }
                }
                Action::Disconnect => {
                    let result = if let Some(server) = state.server.as_ref() {
                        server.context.revoke()
                    } else {
                        self.store.lock().unwrap().update(|p| {
                            p.token_digest = None;
                            p.token_digests.clear();
                            p.devices.clear();
                        })
                    };
                    if let Err(error) = result {
                        state.error = error;
                    }
                }
            }
        }
        self.snapshot().await
    }
    pub async fn shutdown(&self) {
        self.action(Action::Quit, None).await;
        self.controller.shutdown();
    }
    pub async fn prepare_admin(&self) -> Result<DesktopState, String> {
        let admin = self.controller.admin.clone();
        tokio::task::spawn_blocking(move || admin.prepare()).await.map_err(|e| e.to_string())??;
        Ok(self.snapshot().await)
    }
    pub async fn stop_admin(&self) -> DesktopState {
        self.controller.admin.stop();
        self.snapshot().await
    }
    pub async fn rename_device(&self, device_id: &str, name: &str) -> Result<DesktopState, String> {
        let clean: String = name
            .trim()
            .chars()
            .filter(|character| !character.is_control())
            .take(40)
            .collect();
        if clean.is_empty() {
            return Err("设备名称不能为空".into());
        }
        let found = self
            .store
            .lock()
            .unwrap()
            .get()
            .devices
            .iter()
            .any(|device| device.id == device_id);
        if !found {
            return Err("设备不存在".into());
        }
        self.store.lock().unwrap().update(|preferences| {
            if let Some(device) = preferences
                .devices
                .iter_mut()
                .find(|device| device.id == device_id)
            {
                device.name = clean;
            }
        })?;
        Ok(self.snapshot().await)
    }
    pub async fn set_device_permissions(
        &self,
        device_id: &str,
        permissions: crate::preferences::DevicePermissions,
    ) -> Result<DesktopState, String> {
        let found = self
            .store
            .lock()
            .unwrap()
            .get()
            .devices
            .iter()
            .any(|device| device.id == device_id);
        if !found {
            return Err("设备不存在".into());
        }
        self.store.lock().unwrap().update(|preferences| {
            if let Some(device) = preferences
                .devices
                .iter_mut()
                .find(|device| device.id == device_id)
            {
                device.permissions = permissions;
            }
        })?;
        if let Some(server) = self.state.lock().await.server.as_ref() {
            server.context.refresh_device(device_id);
        }
        Ok(self.snapshot().await)
    }
    pub async fn revoke_device(&self, device_id: &str) -> Result<DesktopState, String> {
        let state = self.state.lock().await;
        if let Some(server) = state.server.as_ref() {
            server.context.revoke_device(device_id)?;
        } else {
            self.store.lock().unwrap().update(|preferences| {
                preferences.devices.retain(|device| device.id != device_id);
                if preferences.primary_device.as_deref() == Some(device_id) {
                    preferences.primary_device = None;
                    preferences.control_mode = crate::preferences::ControlMode::Free;
                }
            })?;
        }
        drop(state);
        Ok(self.snapshot().await)
    }
    pub async fn set_primary_device(
        &self,
        device_id: Option<&str>,
    ) -> Result<DesktopState, String> {
        if let Some(id) = device_id {
            if !self
                .store
                .lock()
                .unwrap()
                .get()
                .devices
                .iter()
                .any(|device| device.id == id)
            {
                return Err("设备不存在".into());
            }
        }
        self.store.lock().unwrap().update(|preferences| {
            preferences.control_mode = if device_id.is_some() {
                crate::preferences::ControlMode::Primary
            } else {
                crate::preferences::ControlMode::Free
            };
            preferences.primary_device = device_id.map(str::to_string);
        })?;
        if let Some(server) = self.state.lock().await.server.as_ref() {
            server.context.clear_control_request();
            server.context.refresh_sessions();
        }
        Ok(self.snapshot().await)
    }
    pub async fn deny_control_request(&self) -> DesktopState {
        if let Some(server) = self.state.lock().await.server.as_ref() {
            server.context.clear_control_request();
        }
        self.snapshot().await
    }
    pub async fn approve_control_request(&self, device_id: &str) -> Result<DesktopState, String> {
        let matches = self
            .state
            .lock()
            .await
            .server
            .as_ref()
            .and_then(|server| server.context.pending_control_request())
            .is_some_and(|(id, _)| id == device_id);
        if !matches {
            return Err("主控请求已失效，请刷新后重试".into());
        }
        self.set_primary_device(Some(device_id)).await
    }
}

#[cfg(test)]
mod port_tests {
    use super::*;
    use crate::preferences::Store;
    use std::net::TcpListener;

    #[tokio::test]
    async fn changing_port_preserves_the_working_service_on_failure_and_trust_on_success() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let store = Store::open(path.clone()).unwrap();
        let service = Service::new(store.clone()).unwrap();
        let host = Ipv4Addr::LOCALHOST;
        let server = Server::start_secure_with_files(
            host,
            0,
            service.controller.clone(),
            store.clone(),
            service.files.clone(),
        )
        .await
        .unwrap();
        let old = server.address;
        let code = server.context.pairing_status().0;
        {
            let mut state = service.state.lock().await;
            state.selected = host.to_string();
            state.server = Some(server);
        }
        store
            .lock()
            .unwrap()
            .update(|p| p.service_port = old.port())
            .unwrap();
        let identity = store.lock().unwrap().get().tls.unwrap().certificate;
        let client = reqwest::Client::builder()
            .no_proxy()
            .danger_accept_invalid_certs(true)
            .pool_max_idle_per_host(0)
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        let paired: serde_json::Value = client
            .post(format!("https://{old}/api/pair"))
            .json(&serde_json::json!({"code": code, "deviceName": "Port test"}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        let token = paired["token"].as_str().unwrap();

        let occupied = TcpListener::bind((host, 0)).unwrap();
        let port = occupied.local_addr().unwrap().port();
        assert!(service
            .set_port(port, None)
            .await
            .err()
            .unwrap()
            .contains(&port.to_string()));
        assert!(service.set_port(5060, None).await.is_err());
        assert_eq!(store.lock().unwrap().get().service_port, old.port());
        assert!(client
            .get(format!("https://{old}/api/status"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status()
            .is_success());
        drop(occupied);

        // Force the atomic settings write to fail after the replacement port has been bound.
        let backup = dir.path().join("backup.json");
        std::fs::rename(&path, &backup).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(service.set_port(port, None).await.is_err());
        assert_eq!(store.lock().unwrap().get().service_port, old.port());
        assert!(client
            .get(format!("https://{old}/api/status"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status()
            .is_success());
        std::fs::remove_dir(&path).unwrap();
        std::fs::rename(backup, &path).unwrap();

        let state = service.set_port(port, None).await.unwrap();
        assert!(state.running);
        assert_eq!(state.port, port);
        assert_eq!(state.paired_count, 1);
        let saved = Store::open(path).unwrap().lock().unwrap().get();
        assert_eq!(saved.service_port, port);
        assert_eq!(saved.tls.unwrap().certificate, identity);
        assert!(client
            .get(format!("https://{host}:{port}/api/status"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status()
            .is_success());
        assert!(TcpListener::bind(old).is_ok());
        service.action(Action::Stop, None).await;
    }

    #[tokio::test]
    async fn saving_while_stopped_does_not_start_the_service() {
        let service = Service::new(Store::memory()).unwrap();
        service.action(Action::Stop, None).await;
        let socket = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        let port = socket.local_addr().unwrap().port();
        drop(socket);
        let state = service.set_port(port, None).await.unwrap();
        assert_eq!(state.port, port);
        assert!(!state.running);
        assert!(!service.retry_start().await.running);
        let occupied = TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).unwrap();
        if !state.addresses.is_empty() {
            // Another program can claim the saved port before the next start.
            let failed = service.action(Action::Start, None).await;
            assert!(!failed.running);
            assert_eq!(failed.port, port);
            assert!(failed.error.contains(&port.to_string()));
            drop(occupied);
            let recovered = service.retry_start().await;
            assert!(recovered.running);
            assert_eq!(recovered.port, port);
            service.action(Action::Stop, None).await;
        }
    }
}
