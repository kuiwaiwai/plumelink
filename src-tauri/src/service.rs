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
    #[serde(rename = "lanRunning")]
    pub lan_running: bool,
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
    #[serde(rename = "receivedFiles")]
    pub received_files: Vec<crate::receive_history::ReceivedFile>,
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
    Wirelessstop,
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
    usb_lifecycle: Mutex<()>,
    pub controller: Arc<Controller>,
    pub exiting: AtomicBool,
    quitting: AtomicBool,
    pub store: crate::preferences::SharedStore,
    pub files: Arc<FileHub>,
    pub desired_running: AtomicBool,
    pub usb: Arc<crate::usb::Bridge>,
}
struct UsbAttempt<'a> {bridge:&'a crate::usb::Bridge,ticket:u64,armed:bool}
impl Drop for UsbAttempt<'_> {
    fn drop(&mut self) {if self.armed {self.bridge.cancel_ticket(self.ticket);}}
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
            quitting: AtomicBool::new(false),
            store,
            files: FileHub::new(),
            desired_running: AtomicBool::new(true),
            usb: Arc::new(crate::usb::Bridge::default()),
            usb_lifecycle: Mutex::new(()),
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
            if state.server.as_ref().is_some_and(|server| server.lan_address().is_some()) {
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
                        if let Some(server) = state.server.as_mut() {
                            server.stop_lan().await;
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
            running: state.server.as_ref().is_some_and(|server| server.lan_address().is_some() || server.usb_address().is_some()),
            lan_running: state.server.as_ref().is_some_and(|server| server.lan_address().is_some()),
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
            port: preferences.service_port,
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
            received_files: self.files.received.list(),
            admin: self.controller.admin.status(),
            microphone: self.controller.audio.status(),
        }
    }
    pub async fn action(self: &Arc<Self>, action: Action, address: Option<String>) -> DesktopState {
        let stopping=matches!(action,Action::Stop|Action::Quit);
        if !stopping {return self.action_inner(action,address,false).await;}
        self.usb.cancel();
        self.desired_running.store(false,Ordering::SeqCst);
        if matches!(action,Action::Quit) {self.quitting.store(true,Ordering::SeqCst);}
        let service=self.clone();
        let mut work=Box::pin(async move {service.stop_owned(action,address).await});
        // Preserve the original lock position before handing a pending stop to its owner.
        let ready=std::future::poll_fn(|context|std::task::Poll::Ready(match std::future::Future::poll(work.as_mut(),context) {
            std::task::Poll::Ready(state)=>Some(state),
            std::task::Poll::Pending=>None,
        })).await;
        if let Some(state)=ready {return state;}
        match tokio::spawn(work).await {
            Ok(state)=>state,
            Err(_)=>{
                self.state.lock().await.error="停止服务未完成，请重试".into();
                self.snapshot().await
            }
        }
    }
    async fn stop_owned(&self,action:Action,address:Option<String>)->DesktopState {
        let _usb=self.usb_lifecycle.lock().await;
        self.usb.require_reconcile();
        let _=self.usb.retire_selection(None);
        let quitting=matches!(action,Action::Quit);
        let result=self.action_inner(action,address,false).await;
        let configured=self.store.lock().unwrap().get().adb_path;
        self.usb.cleanup_retired(&configured).await;
        self.usb.finish_reconcile();
        if quitting {self.controller.shutdown();self.exiting.store(true,Ordering::SeqCst);}
        result
    }
    pub async fn usb_status(self:&Arc<Self>)->crate::usb::Status {
        let Some(slot)=self.usb.read_slot() else {return self.decorate_usb_status(self.usb.status()).await;};
        let service=self.clone();
        match tokio::spawn(async move {let _slot=slot;service.usb_status_inner().await}).await {
            Ok(status)=>status,
            Err(_)=>self.decorate_usb_status(self.usb.status()).await,
        }
    }
    async fn usb_status_inner(&self) -> crate::usb::Status {
        let settings = self.store.lock().unwrap().get();
        let ports:Vec<_> = self.state.lock().await.server.as_ref().map(|server|server.usb_addresses().into_iter().map(|address|address.port()).collect()).unwrap_or_default();
        let mut status = self.usb.refresh_ports(&settings.adb_path,&ports).await;
        status.busy|=self.usb.connect_busy();
        if let Ok(_usb)=self.usb_lifecycle.try_lock() {
            if !status.busy {self.stop_unused_usb().await;}
        }
        self.decorate_usb_status(status).await
    }
    async fn decorate_usb_status(&self,mut status:crate::usb::Status)->crate::usb::Status {
        use base64::Engine;
        use sha2::Digest;
        let settings=self.store.lock().unwrap().get();
        let state = self.state.lock().await;
        if let Some(server) = state.server.as_ref().filter(|s|s.usb_address().is_some()) {
            status.endpoints=server.usb_addresses().iter().map(ToString::to_string).collect();
            status.endpoint=status.endpoints.first().cloned().unwrap_or_default();
            for device in &mut status.devices {device.endpoint=server.usb_address_for(&device.id).map(|address|address.to_string()).unwrap_or_default();}
            status.code = server.context.code();
            if let Some(tls) = settings.tls {
                if let Ok(certificate) = base64::engine::general_purpose::STANDARD.decode(tls.certificate) {
                    status.fingerprint = sha2::Sha256::digest(certificate).iter().map(|b|format!("{b:02X}")).collect::<Vec<_>>().join(":");
                }
            }
        }
        status
    }
    pub async fn set_usb_component(&self,path:&str,_slot:tokio::sync::OwnedSemaphorePermit)->Result<(),String> {
        self.usb.configure(path).await?;
        self.store.lock().unwrap().update(|settings|settings.adb_path=path.into())
    }
    pub async fn usb_connect(self:&Arc<Self>, id: &str) -> Result<(), String> {
        if self.quitting.load(Ordering::SeqCst) || self.exiting.load(Ordering::SeqCst) {return Err("服务正在退出".into());}
        let slot=self.usb.connect_slot()?;
        let ticket = self.usb.connection_ticket();
        self.usb.require_reconcile();
        let mut cancellation=UsbAttempt{bridge:&self.usb,ticket,armed:true};
        let service=self.clone();let id=id.to_string();
        let worker=tokio::spawn(async move {let _slot=slot;service.usb_connect_owned(&id,ticket).await});
        let result=worker.await.map_err(|_|crate::usb::RESULT_UNCONFIRMED.to_string());
        cancellation.armed=false;
        result?
    }
    async fn usb_connect_owned(&self,id:&str,ticket:u64)->Result<(),String> {
        let _usb=self.usb_lifecycle.lock().await;
        self.usb.require_reconcile();
        let result=self.usb_connect_locked(id,ticket).await;
        if result.is_err() {self.stop_unused_usb().await;}
        self.usb.finish_connection(ticket);
        self.usb.finish_reconcile();
        result
    }
    async fn usb_connect_locked(&self,id:&str,ticket:u64)->Result<(),String> {
        let configured = self.store.lock().unwrap().get().adb_path;
        let mut port = {
            let mut state = self.state.lock().await;
            if self.quitting.load(Ordering::SeqCst) || self.exiting.load(Ordering::SeqCst) { return Err("服务正在退出".into()); }
            if !self.usb.is_current(ticket) { return Err("USB 连接已取消".into()); }
            if state.server.is_none() {
                state.server = Some(Server::start_usb_only(self.controller.clone(),self.store.clone(),self.files.clone()).await?);
                state.error.clear();
            }
            if state.server.as_ref().is_some_and(|server|server.lan_address().is_none()) {
                self.desired_running.store(false,Ordering::SeqCst); state.error.clear();
            }
            state.server.as_mut().ok_or("服务尚未启动")?.ensure_usb_for(id).await?.port()
        };
        for attempt in 0..3 {
            let access={let state=self.state.lock().await;state.server.as_ref().and_then(|server|server.usb_access_for(id)).ok_or("USB 连接已取消")?.to_string()};
            let result=self.usb.connect_with_access(&configured,id,port,ticket,&access).await;
            if result.as_ref().is_err_and(|message|message==crate::usb::PORT_IN_USE) && attempt<2 && !self.usb.selected_ids().iter().any(|selected|selected==id) && self.usb.is_current(ticket) {
                let mut state=self.state.lock().await;
                let server=state.server.as_mut().ok_or("USB 连接已取消")?;
                if server.usb_address_for(id).map(|a|a.port())!=Some(port) { return Err("USB 连接已取消".into()); }
                port=server.rotate_usb_for(id).await?.port();
            } else { return result; }
        }
        Err(crate::usb::PORT_IN_USE.into())
    }
    pub async fn usb_disconnect(&self, id: Option<&str>) -> Result<(), String> {
        self.usb.validate_disconnect(id)?;self.usb.cancel();
        let _usb=self.usb_lifecycle.lock().await;
        self.usb.require_reconcile();self.usb.retire_selection(id)?;
        {let mut state=self.state.lock().await;if let Some(server)=state.server.as_mut() {if let Some(id)=id {server.stop_usb_for(id).await;}else{server.stop_usb().await;}}}
        let configured = self.store.lock().unwrap().get().adb_path;
        self.usb.cleanup_retired(&configured).await;
        self.stop_unused_usb().await;
        Ok(())
    }
    async fn stop_unused_usb(&self) {
        let selected=self.usb.selected_ids();
        {
            let mut state = self.state.lock().await;
            if let Some(server) = state.server.as_mut() { server.retain_usb(&selected).await; }
            if state.server.as_ref().is_some_and(|server| server.lan_address().is_none()&&server.usb_address().is_none()) {
                if let Some(mut server) = state.server.take() { server.stop().await; }
            }
        }
        self.usb.finish_reconcile();
    }
    pub async fn set_port(
        &self,
        port: u16,
        address: Option<String>,
    ) -> Result<DesktopState, String> {
        crate::preferences::validate_port(port)?;
        {
            let mut state = self.state.lock().await;
            if self.quitting.load(Ordering::SeqCst) || self.exiting.load(Ordering::SeqCst) {
                return Err("服务正在退出".into());
            }
            let changed = self.store.lock().unwrap().get().service_port != port;
            if changed {
                if state.server.as_ref().is_some_and(|server| server.lan_address().is_some()) {
                    let host = state
                        .selected
                        .parse::<Ipv4Addr>()
                        .map_err(|e| e.to_string())?;
                    // Keep the old service intact through every fallible step, including disk I/O.
                    let listener = Server::bind(host, port).await?;
                    self.store
                        .lock()
                        .unwrap()
                        .update(|p| p.service_port = port)?;
                    if let Some(server) = state.server.as_mut() { server.replace_lan(listener).await; }
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
                Action::Wirelessstop => {
                    self.desired_running.store(false,Ordering::SeqCst);
                    if let Some(server) = state.server.as_mut() { server.stop_lan().await; }
                    state.error.clear(); state.missing_address_polls=0;
                }
                Action::Start => {
                    if !retry && !self.quitting.load(Ordering::SeqCst) && !self.exiting.load(Ordering::SeqCst) {
                        self.desired_running.store(true, Ordering::SeqCst);
                    }
                    if state.server.as_ref().is_none_or(|server| server.lan_address().is_none())
                        && self.desired_running.load(Ordering::SeqCst)
                        && !self.quitting.load(Ordering::SeqCst)
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
                            let start = if let Some(server) = state.server.as_mut() {
                                match Server::bind(host,port).await {
                                    Ok(listener) => { server.replace_lan(listener).await; Ok(None) },
                                    Err(error) => Err(error),
                                }
                            } else { Server::start_secure_with_files(
                                host,
                                port,
                                self.controller.clone(),
                                self.store.clone(),
                                self.files.clone(),
                            )
                            .await.map(Some) };
                            match start {
                                Ok(server) => {
                                    crate::logging::info(
                                        "service",
                                        "LAN listener started",
                                    );
                                    if let Some(server) = server { state.server = Some(server); }
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
    pub async fn shutdown(self: &Arc<Self>) {
        self.action(Action::Quit, None).await;
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
                device.name_customized = true;
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

#[cfg(test)]
mod usb_lifecycle_tests {
    use super::*;
    struct NoInput;
    impl crate::control::Native for NoInput {
        fn execute(&mut self,_command:&crate::protocol::Command)->Result<(),String> {Err("test input disabled".into())}
        fn release(&mut self)->Result<(),String> {Ok(())}
    }
    #[tokio::test]
    async fn disconnect_or_stop_cannot_close_a_new_connection_listener() {
        for stop in [false,true] {
            let store=crate::preferences::Store::memory();
            let controller=Controller::start(||Ok(Box::new(NoInput))).unwrap();let files=FileHub::new();
            let mut server=Server::start_usb_only(controller.clone(),store.clone(),files.clone()).await.unwrap();let port=server.usb_address().unwrap().port();
            let fixture=crate::usb::tests::LifecycleFixture::new(port).await;server.ensure_usb_for(&fixture.id).await.unwrap();store.lock().unwrap().update(|preferences|preferences.adb_path=fixture.path.clone()).unwrap();
            let service=Arc::new(Service {state:Mutex::new(State{server:Some(server),selected:String::new(),error:String::new(),missing_address_polls:0,last_snapshot:None}),usb_lifecycle:Mutex::new(()),controller:controller.clone(),exiting:AtomicBool::new(false),quitting:AtomicBool::new(false),store,files,desired_running:AtomicBool::new(false),usb:fixture.bridge.clone()});
            fixture.pause_cleanup_and_creation();
            let closing={let service=service.clone();let id=fixture.id.clone();tokio::spawn(async move {if stop {service.action(Action::Stop,None).await;}else{service.usb_disconnect(Some(&id)).await.unwrap();}})};
            fixture.entered().await;
            let opening={let service=service.clone();let id=fixture.id.clone();tokio::spawn(async move{service.usb_connect(&id).await})};tokio::task::yield_now().await;
            fixture.resume();closing.await.unwrap();fixture.entered().await;
            let address=service.state.lock().await.server.as_ref().and_then(Server::usb_address).expect("new USB listener was closed by the earlier stop");
            assert!(tokio::net::TcpStream::connect(address).await.is_ok());
            fixture.resume();opening.await.unwrap().unwrap();assert!(service.usb.has_mappings());
            service.usb_disconnect(None).await.unwrap();controller.shutdown();
        }
    }

    #[tokio::test]
    async fn a_failed_first_usb_connection_does_not_leave_a_listener_running() {
        for lan in [false,true] {
            let store=crate::preferences::Store::memory();let controller=Controller::start(||Ok(Box::new(NoInput))).unwrap();let files=FileHub::new();
            let server=if lan {Some(Server::start_secure_with_files(Ipv4Addr::LOCALHOST,0,controller.clone(),store.clone(),files.clone()).await.unwrap())}else{None};
            let lan_address=server.as_ref().and_then(Server::lan_address);
            let fixture=crate::usb::tests::LifecycleFixture::new(49123).await;
            fixture.bridge.disconnect(&fixture.path,None).await.unwrap();fixture.fail_open();
            store.lock().unwrap().update(|preferences|preferences.adb_path=fixture.path.clone()).unwrap();
            let service=Arc::new(Service{state:Mutex::new(State{server,selected:String::new(),error:String::new(),missing_address_polls:0,last_snapshot:None}),usb_lifecycle:Mutex::new(()),controller:controller.clone(),exiting:AtomicBool::new(false),quitting:AtomicBool::new(false),store,files,desired_running:AtomicBool::new(lan),usb:fixture.bridge.clone()});
            assert!(service.usb_connect(&fixture.id).await.is_err());
            {let state=service.state.lock().await;assert_eq!(state.server.as_ref().and_then(Server::lan_address),lan_address);assert!(state.server.as_ref().and_then(Server::usb_address).is_none());assert_eq!(state.server.is_some(),lan);}
            assert!(!service.usb.has_mappings());assert!(!service.usb.needs_refresh());
            if let Some(address)=lan_address {assert!(tokio::net::TcpStream::connect(address).await.is_ok());}
            service.action(Action::Stop,None).await;controller.shutdown();
        }
    }
    #[tokio::test]
    async fn an_aborted_usb_request_is_retired_by_status_without_a_leftover_listener() {
        for before_creation in [true,false] {
        let store=crate::preferences::Store::memory();let controller=Controller::start(||Ok(Box::new(NoInput))).unwrap();let files=FileHub::new();
        let fixture=crate::usb::tests::LifecycleFixture::new(49123).await;fixture.bridge.disconnect(&fixture.path,None).await.unwrap();
        if before_creation {fixture.pause_devices();}else{fixture.pause_open();}
        store.lock().unwrap().update(|preferences|preferences.adb_path=fixture.path.clone()).unwrap();
        let service=Arc::new(Service{state:Mutex::new(State{server:None,selected:String::new(),error:String::new(),missing_address_polls:0,last_snapshot:None}),usb_lifecycle:Mutex::new(()),controller:controller.clone(),exiting:AtomicBool::new(false),quitting:AtomicBool::new(false),store,files,desired_running:AtomicBool::new(false),usb:fixture.bridge.clone()});
        let task={let service=service.clone();let id=fixture.id.clone();tokio::spawn(async move {service.usb_connect(&id).await})};
        fixture.entered().await;task.abort();assert!(task.await.unwrap_err().is_cancelled());
        assert!(service.usb.needs_refresh());assert!(!service.usb.has_mappings());
        fixture.resume();
        tokio::time::timeout(std::time::Duration::from_secs(5),async {while fixture.connect_count()!=0 {tokio::task::yield_now().await;}}).await.unwrap();
        let status=service.usb_status().await;assert!(status.endpoint.is_empty());assert!(!status.busy);assert!(!service.usb.needs_refresh());assert!(service.state.lock().await.server.is_none());controller.shutdown();
        }
    }
    #[tokio::test]
    async fn disconnect_stops_only_its_phone_before_adb_cleanup_finishes() {
        use futures_util::{SinkExt,StreamExt};
        use tokio_tungstenite::tungstenite::{client::IntoClientRequest,Message};
        let store=crate::preferences::Store::memory();let controller=Controller::start(||Ok(Box::new(NoInput))).unwrap();let files=FileHub::new();
        let mut server=Server::start_with_files(Ipv4Addr::LOCALHOST,0,controller.clone(),store.clone(),files.clone()).await.unwrap();
        let port=server.ensure_usb().await.unwrap().port();let fixture=crate::usb::tests::LifecycleFixture::new(port).await;server.ensure_usb_for(&fixture.id).await.unwrap();let second=fixture.second_phone().await;
        store.lock().unwrap().update(|preferences|preferences.adb_path=fixture.path.clone()).unwrap();
        let lan=server.context.origin.clone();let code=server.context.code();
        let service=Arc::new(Service{state:Mutex::new(State{server:Some(server),selected:String::new(),error:String::new(),missing_address_polls:0,last_snapshot:None}),usb_lifecycle:Mutex::new(()),controller:controller.clone(),exiting:AtomicBool::new(false),quitting:AtomicBool::new(false),store,files,desired_running:AtomicBool::new(true),usb:fixture.bridge.clone()});
        service.usb_connect(&second).await.unwrap();let status=service.usb_status().await;assert_eq!(status.endpoints.len(),2);
        let first_endpoint=status.devices.iter().find(|device|device.id==fixture.id).unwrap().endpoint.clone();
        let second_endpoint=status.devices.iter().find(|device|device.id==second).unwrap().endpoint.clone();assert_ne!(first_endpoint,second_endpoint);
        let (first_access,second_access)={let state=service.state.lock().await;let server=state.server.as_ref().unwrap();(server.usb_access_for(&fixture.id).unwrap().to_string(),server.usb_access_for(&second).unwrap().to_string())};
        let client=reqwest::Client::builder().no_proxy().build().unwrap();
        let paired=client.post(format!("{lan}/api/pair")).json(&serde_json::json!({"code":code})).send().await.unwrap().json::<serde_json::Value>().await.unwrap();let token=paired["token"].as_str().unwrap();
        let mut request=format!("ws://{first_endpoint}/ws").into_client_request().unwrap();request.headers_mut().insert("origin",format!("http://{first_endpoint}").parse().unwrap());
        request.headers_mut().insert("x-plumelink-usb",first_access.parse().unwrap());
        let mut socket=tokio_tungstenite::connect_async(request).await.unwrap().0;
        socket.send(Message::Text(serde_json::json!({"kind":"auth","token":token}).to_string().into())).await.unwrap();assert!(socket.next().await.unwrap().unwrap().into_text().unwrap().contains("ready"));
        fixture.pause_cleanup();
        let closing={let service=service.clone();let id=fixture.id.clone();tokio::spawn(async move {service.usb_disconnect(Some(&id)).await})};fixture.entered().await;
        let frame=tokio::time::timeout(std::time::Duration::from_secs(1),socket.next()).await.unwrap().unwrap().unwrap();assert!(matches!(frame,Message::Close(Some(frame)) if frame.code==4000u16.into()));
        assert!(!closing.is_finished());assert!(tokio::net::TcpStream::connect(&first_endpoint).await.is_err());
        for (origin,access) in [(format!("http://{second_endpoint}"),second_access),(lan,String::new())] {assert_eq!(client.get(format!("{origin}/api/status")).header("x-plumelink-usb",access).bearer_auth(token).send().await.unwrap().status(),200);}
        fixture.resume();closing.await.unwrap().unwrap();assert_eq!(service.usb_status().await.active_count,1);
        service.action(Action::Stop,None).await;controller.shutdown();
    }
    #[tokio::test]
    async fn retirement_survives_disconnect_or_stop_aborted_before_closing_the_listener() {
        for stop in [false,true] {
            let store=crate::preferences::Store::memory();let controller=Controller::start(||Ok(Box::new(NoInput))).unwrap();let files=FileHub::new();
            let mut server=Server::start_usb_only(controller.clone(),store.clone(),files.clone()).await.unwrap();let port=server.usb_address().unwrap().port();let fixture=crate::usb::tests::LifecycleFixture::new(port).await;server.ensure_usb_for(&fixture.id).await.unwrap();
            store.lock().unwrap().update(|preferences|preferences.adb_path=fixture.path.clone()).unwrap();
            let service=Arc::new(Service{state:Mutex::new(State{server:Some(server),selected:String::new(),error:String::new(),missing_address_polls:0,last_snapshot:None}),usb_lifecycle:Mutex::new(()),controller:controller.clone(),exiting:AtomicBool::new(false),quitting:AtomicBool::new(false),store,files,desired_running:AtomicBool::new(false),usb:fixture.bridge.clone()});
            let locked=service.state.lock().await;
            let task={let service=service.clone();let id=fixture.id.clone();tokio::spawn(async move {if stop {service.action(Action::Stop,None).await;Ok(())}else{service.usb_disconnect(Some(&id)).await}})};
            tokio::time::timeout(std::time::Duration::from_secs(1),async {while service.usb.has_mappings() {tokio::task::yield_now().await;}}).await.unwrap();
            task.abort();assert!(task.await.unwrap_err().is_cancelled());assert!(service.usb.needs_refresh());drop(locked);
            let status=service.usb_status().await;assert_eq!(status.active_count,0);assert!(status.endpoint.is_empty());assert!(!service.usb.needs_refresh());assert!(service.state.lock().await.server.is_none());controller.shutdown();
        }
    }
    async fn owned_fixture(connected:bool)->(Arc<Service>,crate::usb::tests::LifecycleFixture,Arc<Controller>) {
        let store=crate::preferences::Store::memory();let controller=Controller::start(||Ok(Box::new(NoInput))).unwrap();let files=FileHub::new();
        let mut server=if connected {Some(Server::start_usb_only(controller.clone(),store.clone(),files.clone()).await.unwrap())}else{None};
        let port=server.as_ref().and_then(Server::usb_address).map(|address|address.port()).unwrap_or(49123);
        let fixture=crate::usb::tests::LifecycleFixture::new(port).await;
        if let Some(server)=server.as_mut(){server.ensure_usb_for(&fixture.id).await.unwrap();}else{fixture.bridge.disconnect(&fixture.path,None).await.unwrap();}
        store.lock().unwrap().update(|preferences|preferences.adb_path=fixture.path.clone()).unwrap();
        let service=Arc::new(Service{state:Mutex::new(State{server,selected:String::new(),error:String::new(),missing_address_polls:0,last_snapshot:None}),usb_lifecycle:Mutex::new(()),controller:controller.clone(),exiting:AtomicBool::new(false),quitting:AtomicBool::new(false),store,files,desired_running:AtomicBool::new(false),usb:fixture.bridge.clone()});
        fixture.clear_calls();(service,fixture,controller)
    }
    struct CountedRelease(Arc<std::sync::atomic::AtomicUsize>);
    impl crate::control::Native for CountedRelease {
        fn execute(&mut self,_:&crate::protocol::Command)->Result<(),String>{Err("test input disabled".into())}
        fn release(&mut self)->Result<(),String>{self.0.fetch_add(1,Ordering::SeqCst);Ok(())}
    }
    async fn accepted_shutdown_survives_caller_abort(quit:bool) {
        let releases=Arc::new(std::sync::atomic::AtomicUsize::new(0));let observed=releases.clone();
        let controller=Controller::start(move ||Ok(Box::new(CountedRelease(observed)))).unwrap();
        let store=crate::preferences::Store::memory();let files=FileHub::new();
        let mut server=Server::start_usb_only(controller.clone(),store.clone(),files.clone()).await.unwrap();
        let fixture=crate::usb::tests::LifecycleFixture::new(server.usb_address().unwrap().port()).await;
        server.ensure_usb_for(&fixture.id).await.unwrap();
        let listener=Server::bind(Ipv4Addr::LOCALHOST,0).await.unwrap();let lan=listener.local_addr().unwrap();
        server.replace_lan(listener).await;store.lock().unwrap().update(|settings|settings.adb_path=fixture.path.clone()).unwrap();
        let service=Arc::new(Service{state:Mutex::new(State{server:Some(server),selected:Ipv4Addr::LOCALHOST.to_string(),error:String::new(),missing_address_polls:0,last_snapshot:None}),
            usb_lifecycle:Mutex::new(()),controller:controller.clone(),exiting:AtomicBool::new(false),quitting:AtomicBool::new(false),store,files,desired_running:AtomicBool::new(true),usb:fixture.bridge.clone()});
        assert!(controller.activate(41));let before=releases.load(Ordering::SeqCst);
        if quit {fixture.pause_cleanup();}
        let locked=service.state.lock().await;
        let caller={let service=service.clone();tokio::spawn(async move {if quit {service.shutdown().await;}else{service.action(Action::Stop,None).await;}})};
        tokio::time::timeout(std::time::Duration::from_secs(1),async{while service.usb.has_mappings(){tokio::task::yield_now().await;}}).await.unwrap();
        if quit {
            assert!(service.quitting.load(Ordering::SeqCst));assert!(!service.exiting.load(Ordering::SeqCst));
            assert_eq!(service.usb_connect(&fixture.id).await.err().unwrap(),"服务正在退出");
        }
        caller.abort();assert!(caller.await.unwrap_err().is_cancelled());drop(locked);
        if quit {
            fixture.entered().await;
            assert!(service.quitting.load(Ordering::SeqCst));assert!(!service.exiting.load(Ordering::SeqCst));
            let port=service.store.lock().unwrap().get().service_port;
            assert_eq!(service.set_port(17841,None).await.err().unwrap(),"服务正在退出");
            assert_eq!(service.store.lock().unwrap().get().service_port,port);
            service.action(Action::Start,None).await;assert!(!service.desired_running.load(Ordering::SeqCst));
            assert!(!service.exiting.load(Ordering::SeqCst));fixture.resume();
        }
        let completed=tokio::time::timeout(std::time::Duration::from_secs(1),async{
            loop {
                let closed=service.state.lock().await.server.is_none();
                if closed && fixture.mapping_count()==0 && releases.load(Ordering::SeqCst)>before
                    && !service.desired_running.load(Ordering::SeqCst)
                    && (!quit || (service.exiting.load(Ordering::SeqCst)&&!controller.input_available())) {break;}
                tokio::task::yield_now().await;
            }
        }).await.is_ok();
        let lan_open=tokio::net::TcpStream::connect(lan).await.is_ok();
        if quit {service.action(Action::Start,None).await;assert!(!service.desired_running.load(Ordering::SeqCst));}
        service.action(Action::Stop,None).await;controller.shutdown();
        assert!(completed,"accepted {} did not finish after its caller was aborted; LAN still open: {lan_open}",if quit{"Quit"}else{"Stop"});
        assert!(!lan_open);
    }
    #[tokio::test] async fn accepted_stop_finishes_lan_usb_and_input_cleanup_after_caller_abort(){accepted_shutdown_survives_caller_abort(false).await;}
    #[tokio::test] async fn accepted_quit_finishes_lan_usb_and_controller_cleanup_after_caller_abort(){accepted_shutdown_survives_caller_abort(true).await;}
    #[tokio::test] async fn accepted_stop_keeps_its_lock_position_and_preserves_a_later_start_intent() {
        let (service,fixture,controller)=owned_fixture(true).await;
        let listener=Server::bind(Ipv4Addr::LOCALHOST,0).await.unwrap();
        service.state.lock().await.server.as_mut().unwrap().replace_lan(listener).await;
        let occupied=std::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED,0)).unwrap();
        let port=occupied.local_addr().unwrap().port();
        service.store.lock().unwrap().update(|settings|settings.service_port=port).unwrap();
        service.desired_running.store(true,Ordering::SeqCst);fixture.pause_cleanup();
        let mut stopping=Box::pin(service.action(Action::Stop,None));
        let first=std::future::poll_fn(|context|std::task::Poll::Ready(std::future::Future::poll(stopping.as_mut(),context))).await;
        assert!(first.is_pending());
        service.action(Action::Start,None).await;
        fixture.entered().await;fixture.resume();stopping.await;
        let start_preserved=service.desired_running.load(Ordering::SeqCst);
        let stopped=service.state.lock().await.server.is_none();
        service.action(Action::Stop,None).await;controller.shutdown();drop(occupied);
        assert!(start_preserved,"an accepted older Stop overwrote the later explicit Start intent");
        assert!(stopped,"the held port should prevent opening a real LAN listener");
    }
    #[tokio::test]
    async fn configuring_a_component_keeps_current_usb_connections_and_failed_validation_keeps_settings() {
        let (service,fixture,controller)=owned_fixture(true).await;let before=service.usb_status().await;fixture.clear_calls();let generation=fixture.generation();
        service.set_usb_component(&fixture.path,service.usb.component_slot().unwrap()).await.unwrap();
        let folder=tempfile::tempdir().unwrap();let replacement=folder.path().join("adb.exe");std::fs::write(&replacement,b"fixture").unwrap();
        service.set_usb_component(replacement.to_str().unwrap(),service.usb.component_slot().unwrap()).await.unwrap();
        let failed=folder.path().join("missing/adb.exe");assert!(service.set_usb_component(failed.to_str().unwrap(),service.usb.component_slot().unwrap()).await.is_err());
        assert_eq!(service.store.lock().unwrap().get().adb_path,replacement.to_str().unwrap());assert_eq!(fixture.generation(),generation);assert_eq!(fixture.mapping_count(),1);
        let after=service.usb_status().await;assert_eq!(after.endpoints,before.endpoints);assert_eq!(after.active_count,1);assert!(after.devices[0].linked);
        assert!(!fixture.calls().iter().any(|call|call.iter().any(|arg|arg=="--remove"||arg=="--no-rebind"||arg=="shell")));
        service.action(Action::Stop,None).await;controller.shutdown();
    }
    #[tokio::test]
    async fn a_component_save_failure_keeps_the_previous_path_and_live_listener() {
        let (original,fixture,controller)=owned_fixture(true).await;let directory=tempfile::tempdir().unwrap();let path=directory.path().join("settings.json");let store=crate::preferences::Store::open(path.clone()).unwrap();
        store.lock().unwrap().update(|settings|settings.adb_path=fixture.path.clone()).unwrap();std::fs::remove_file(&path).unwrap();std::fs::create_dir(&path).unwrap();
        let server=original.state.lock().await.server.take();let service=Arc::new(Service{state:Mutex::new(State{server,selected:String::new(),error:String::new(),missing_address_polls:0,last_snapshot:None}),usb_lifecycle:Mutex::new(()),controller:controller.clone(),exiting:AtomicBool::new(false),quitting:AtomicBool::new(false),store,files:original.files.clone(),desired_running:AtomicBool::new(false),usb:fixture.bridge.clone()});
        let replacement=directory.path().join("adb.exe");std::fs::write(&replacement,b"fixture").unwrap();
        assert!(service.set_usb_component(replacement.to_str().unwrap(),service.usb.component_slot().unwrap()).await.is_err());assert_eq!(service.store.lock().unwrap().get().adb_path,fixture.path);assert_eq!(fixture.mapping_count(),1);
        let address=service.state.lock().await.server.as_ref().unwrap().usb_address().unwrap();assert!(tokio::net::TcpStream::connect(address).await.is_ok());assert!(fixture.calls().iter().all(|call|call==&["version"]));
        service.action(Action::Stop,None).await;controller.shutdown();
    }
    #[tokio::test]
    async fn caller_abort_keeps_the_creation_receipt_and_stop_waits_for_its_cleanup() {
        let (service,fixture,controller)=owned_fixture(false).await;fixture.pause_create_reply();
        let caller={let service=service.clone();let id=fixture.id.clone();tokio::spawn(async move {service.usb_connect(&id).await})};fixture.entered().await;assert_eq!(fixture.mapping_count(),1);let ticket=fixture.generation();
        caller.abort();assert!(caller.await.unwrap_err().is_cancelled());assert_eq!(fixture.generation(),ticket+1);assert_eq!(fixture.connect_count(),1);
        let stopping={let service=service.clone();tokio::spawn(async move {service.action(Action::Stop,None).await})};
        tokio::time::timeout(std::time::Duration::from_secs(1),async {while fixture.generation()==ticket+1 {tokio::task::yield_now().await;}}).await.unwrap();assert!(!stopping.is_finished());
        fixture.resume();assert!(!stopping.await.unwrap().running);assert_eq!(fixture.connect_count(),0);assert_eq!(fixture.mapping_count(),0);
        assert!(!fixture.calls().iter().any(|call|call.iter().any(|arg|arg=="shell")));assert!(service.state.lock().await.server.is_none());controller.shutdown();
    }
    #[tokio::test]
    async fn an_old_caller_drop_does_not_cancel_the_newer_queued_connection() {
        let (service,fixture,controller)=owned_fixture(false).await;fixture.pause_create_reply();
        let first={let service=service.clone();let id=fixture.id.clone();tokio::spawn(async move {service.usb_connect(&id).await})};fixture.entered().await;
        let second={let service=service.clone();let id=fixture.id.clone();tokio::spawn(async move {service.usb_connect(&id).await})};
        tokio::time::timeout(std::time::Duration::from_secs(1),async {while fixture.connect_count()!=2 {tokio::task::yield_now().await;}}).await.unwrap();let newest=fixture.generation();
        first.abort();assert!(first.await.unwrap_err().is_cancelled());assert_eq!(fixture.generation(),newest);fixture.resume();second.await.unwrap().unwrap();
        assert_eq!(fixture.mapping_count(),1);assert_eq!(fixture.calls().iter().filter(|call|call.iter().any(|arg|arg=="shell")).count(),1);
        service.action(Action::Stop,None).await;controller.shutdown();
    }
    #[tokio::test]
    async fn rejected_capacity_never_cancels_an_admitted_request_and_permits_recover() {
        let (service,fixture,controller)=owned_fixture(false).await;let locked=service.usb_lifecycle.lock().await;let mut callers=Vec::new();
        for _ in 0..8 {let service=service.clone();let id=fixture.id.clone();callers.push(tokio::spawn(async move {service.usb_connect(&id).await}));}
        tokio::time::timeout(std::time::Duration::from_secs(1),async {while fixture.connect_count()!=8 {tokio::task::yield_now().await;}}).await.unwrap();let newest=fixture.generation();
        assert_eq!(service.usb_connect(&fixture.id).await.unwrap_err(),"USB 操作较多，请稍后重试");assert_eq!(fixture.generation(),newest);
        for caller in callers {caller.abort();assert!(caller.await.unwrap_err().is_cancelled());}assert_eq!(fixture.connect_count(),8);drop(locked);
        tokio::time::timeout(std::time::Duration::from_secs(5),async {while fixture.connect_count()!=0 {tokio::task::yield_now().await;}}).await.unwrap();assert_eq!(fixture.mapping_count(),0);
        service.usb_connect(&fixture.id).await.unwrap();service.action(Action::Stop,None).await;controller.shutdown();
    }
    #[tokio::test]
    async fn a_cancelled_status_waiter_does_not_abandon_a_restoration_receipt() {
        let (service,fixture,controller)=owned_fixture(true).await;fixture.clear_first_mapping();fixture.pause_create_reply();
        let caller={let service=service.clone();tokio::spawn(async move {service.usb_status().await})};fixture.entered().await;
        caller.abort();assert!(matches!(caller.await,Err(error)if error.is_cancelled()));assert_eq!(fixture.read_count(),1);fixture.resume();
        tokio::time::timeout(std::time::Duration::from_secs(5),async {while fixture.read_count()!=0 {tokio::task::yield_now().await;}}).await.unwrap();
        let status=service.usb_status().await;assert_eq!(status.active_count,1);assert!(status.devices[0].linked);service.action(Action::Stop,None).await;assert_eq!(fixture.mapping_count(),0);controller.shutdown();
    }
    #[tokio::test]
    async fn an_unknown_creation_closes_its_endpoint_and_explicit_retry_uses_a_new_port() {
        let (service,fixture,controller)=owned_fixture(false).await;fixture.lose_create_reply();
        assert_eq!(service.usb_connect(&fixture.id).await.unwrap_err(),crate::usb::RESULT_UNCONFIRMED);assert!(!service.usb.has_mappings());assert!(service.state.lock().await.server.is_none());
        let failed_port=fixture.calls().into_iter().find(|call|call.iter().any(|arg|arg=="--no-rebind")).unwrap()[4].clone();
        for _ in 0..2 {let status=service.usb_status().await;assert!(status.endpoint.is_empty());assert!(!status.devices[0].selected);assert_eq!(status.devices[0].error,crate::usb::RESULT_UNCONFIRMED);}
        assert_eq!(fixture.mapping_count(),1);assert!(!fixture.calls().iter().any(|call|call.iter().any(|arg|arg=="--remove"||arg=="shell")));
        service.usb_connect(&fixture.id).await.unwrap();let status=service.usb_status().await;assert_eq!(status.active_count,1);assert_ne!(status.endpoint,failed_port.replace("tcp:","127.0.0.1:"));assert!(status.devices[0].linked);assert!(status.devices[0].error.is_empty());
        service.usb_disconnect(None).await.unwrap();assert_eq!(fixture.mapping_count(),1);fixture.clear_remote();service.usb_status().await;assert!(!service.usb.needs_refresh());controller.shutdown();
    }
    #[tokio::test]
    async fn an_uncertain_restoration_is_not_reclaimed_and_keeps_another_phone_healthy() {
        let (service,fixture,controller)=owned_fixture(true).await;let second=fixture.second_phone().await;service.usb_connect(&second).await.unwrap();
        fixture.clear_first_mapping();fixture.lose_create_reply();fixture.clear_calls();
        for _ in 0..3 {let status=service.usb_status().await;assert_eq!(status.active_count,1);let first=status.devices.iter().find(|device|device.id==fixture.id).unwrap();assert!(!first.selected);assert!(first.endpoint.is_empty());assert_eq!(first.error,crate::usb::RESULT_UNCONFIRMED);assert!(status.devices.iter().find(|device|device.id==second).unwrap().linked);}
        assert!(!fixture.calls().iter().any(|call|call.get(1).is_some_and(|serial|serial=="fixture")&&call.iter().any(|arg|arg=="shell"||arg=="--remove")));
        service.usb_connect(&fixture.id).await.unwrap();assert_eq!(service.usb_status().await.active_count,2);service.action(Action::Stop,None).await;assert_eq!(fixture.mapping_count(),1);fixture.clear_remote();service.usb_status().await;assert!(!service.usb.needs_refresh());controller.shutdown();
    }
}
