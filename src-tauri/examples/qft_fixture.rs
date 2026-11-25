//! Local integration fixture: production server/controller, fake device only.
use plumelink::{
    control::{Controller, Native},
    microphone,
    protocol::Command,
    server::Server,
};
use std::io::{self, BufRead, Write};
struct Device { launches: Option<std::sync::Arc<plumelink::launcher::jobs::LaunchQueue>>, usb: bool }
impl Native for Device {
    fn probe_windows(&mut self) -> Result<serde_json::Value,String> {
        Ok(if self.usb { serde_json::json!([{"hwnd":111,"title":"USB fixture window","app":"Fixture","minimized":false,"icon":null}]) } else { serde_json::json!([]) })
    }
    fn switch_to_window(&mut self, hwnd:u64) -> Result<(),String> {
        if !self.usb || hwnd!=111 { return Err("窗口已关闭".into()); }
        println!("{}",serde_json::json!({"event":"window-switch","hwnd":hwnd}));let _=io::stdout().flush();Ok(())
    }
    fn launch_queue(&self) -> Option<std::sync::Arc<plumelink::launcher::jobs::LaunchQueue>> { self.launches.clone() }
    fn qft_action_for(&mut self, session: u64, device: Option<&str>, command: &Command) -> Result<(), String> {
        if let (Some(queue), Command::Launch { id, request }) = (&self.launches, command) {
            queue.request(session, device, id, request.as_deref())?;
            println!("{}", serde_json::json!({"event":"command","command":command}));
            let _ = io::stdout().flush();
            return Ok(());
        }
        self.execute(command)
    }
    fn execute(&mut self, command: &Command) -> Result<(), String> {
        if let (Some(queue), Command::Launch { id, request }) = (&self.launches, command) {
            queue.request(0, None, id, request.as_deref())?;
        }
        println!(
            "{}",
            serde_json::json!({"event":"command","command":command})
        );
        let _ = io::stdout().flush();
        Ok(())
    }
    fn qft_apply(&mut self, _session: u64, v: qfast_transmission::Vector) -> Result<(), String> {
        self.execute(&Command::Motion {
            dx: v.x,
            dy: v.y,
            scroll: v.z,
        })
    }
    fn release(&mut self) -> Result<(), String> {
        self.execute(&Command::Release {})
    }
}
#[tokio::main]
async fn main() {
    if std::env::args().any(|arg|arg=="--inspect-usb") {
        let bridge=plumelink::usb::Bridge::default();let status=bridge.refresh("",None).await;
        println!("{}",serde_json::json!({"event":"usb-component-inspection","available":status.available,"deviceCount":status.devices.len(),"states":status.devices.iter().map(|device|&device.state).collect::<Vec<_>>(),"error":status.error}));
        assert!(status.available&&status.error.is_empty());return;
    }
    let mixed = std::env::args().any(|v| v == "--mixed");
    let recovery = std::env::args().any(|v| v == "--recovery");
    let usb = std::env::args().any(|v| v == "--usb" || v == "--usb-only");
    let output = std::env::args()
        .find_map(|arg| arg.strip_prefix("--windows-audio-output=").map(str::to_owned))
        .map(|name| {
            microphone::endpoints()
                .expect("list Windows playback devices")
                .into_iter()
                .find(|endpoint| endpoint.name == name)
                .expect("the requested Windows playback device was not found")
        });
    let host = std::env::args()
        .find_map(|arg| arg.strip_prefix("--loopback=").map(str::to_owned))
        .map(|value| value.parse::<std::net::Ipv4Addr>().expect("invalid --loopback"))
        .unwrap_or_else(|| {
            if std::env::args().any(|arg| arg == "--secondary-loopback") {
                std::net::Ipv4Addr::new(127, 0, 0, 2)
            } else {
                std::net::Ipv4Addr::LOCALHOST
            }
        });
    assert!(host.is_loopback(), "the fixture only listens on loopback");
    let store = plumelink::preferences::Store::memory();
    let launch_path = std::env::args().find_map(|arg| arg.strip_prefix("--launch=").map(std::path::PathBuf::from));
    if let Some(path) = &launch_path { plumelink::launcher::add(&store, vec![path.clone()]).unwrap(); }
    let launches = launch_path.map(|_| plumelink::launcher::jobs::LaunchQueue::new(store.clone()));
    let mut controller = Controller::start(move || Ok(Box::new(Device { launches, usb }))).unwrap();
    if let Some(output) = &output {
        store.lock().unwrap().update(|p| p.audio_output = output.id.clone()).unwrap();
    }
    let files = plumelink::file_transfer::FileHub::new();
    let directory = std::env::temp_dir().join(format!("plumelink-mixed-{}", std::process::id()));
    if mixed {
        let file_mib = std::env::args()
            .find_map(|arg| arg.strip_prefix("--file-mib=").map(str::to_owned))
            .map(|value| value.parse::<u32>().expect("invalid --file-mib"))
            .unwrap_or(4);
        assert!((1..=2048).contains(&file_mib));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("concurrent-download.bin");
        let mut file = std::fs::File::create(&path).unwrap();
        let block = vec![0x5au8; 1024 * 1024];
        for _ in 0..file_mib {
            file.write_all(&block).unwrap();
        }
        drop(file);
        files.add(path).unwrap();
        for path in std::env::args()
            .filter_map(|arg| arg.strip_prefix("--share=").map(std::path::PathBuf::from))
        {
            files.add(path).expect("share additional test file");
        }
        store
            .lock()
            .unwrap()
            .update(|p| {
                if output.is_none() {
                    p.audio_output = "fake-audio".into();
                }
                p.receive_dir = directory.to_string_lossy().into_owned();
            })
            .unwrap();
    }
    if mixed && output.is_none() {
        std::sync::Arc::get_mut(&mut controller).unwrap().audio = std::sync::Arc::new(
            plumelink::microphone::AudioHub::with_renderer(std::sync::Arc::new(
                |_, stream, _, ready| {
                    let mut samples = 0u64;
                    ready.send(Ok(())).unwrap();
                    while !stream.stopped() {
                        let mut buffer = [0i16; 480];
                        stream.consume(&mut buffer);
                        samples += buffer.iter().filter(|v| **v == 1234).count() as u64;
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                    println!("{}", serde_json::json!({"event":"audio","samples":samples}));
                    let _ = io::stdout().flush();
                    Ok(())
                },
            )),
        );
    }
    let mut server = if std::env::args().any(|v|v=="--usb-only") { Server::start_usb_only(controller.clone(),store.clone(),files).await.unwrap() } else { Server::start_secure_with_files(
        host,
        0,
        controller.clone(),
        store.clone(),
        files,
    )
    .await
    .unwrap() };
    let usb_origin = if usb { Some(format!("https://{}",server.ensure_usb().await.unwrap())) } else { None };
    let mut recovery_servers = Vec::new();
    let mut recovery_controllers = Vec::new();
    let mut recovery_origins = serde_json::Map::new();
    if recovery {
        let base = store.lock().unwrap().get();
        for kind in ["same", "changedCertificate", "changedIdentity"] {
            let peer_store = if kind == "same" { store.clone() } else {
                let peer = plumelink::preferences::Store::memory();
                peer.lock().unwrap().update(|preferences| {
                    if kind == "changedCertificate" { preferences.installation_id = base.installation_id.clone(); }
                    else { preferences.tls = base.tls.clone(); }
                }).unwrap();
                peer
            };
            let peer_controller = Controller::start(|| Ok(Box::new(Device { launches: None, usb: false }))).unwrap();
            let peer = Server::start_secure_with_store(host, 0, peer_controller.clone(), peer_store).await.unwrap();
            recovery_origins.insert(kind.into(), peer.context.origin.clone().into());
            recovery_servers.push(peer); recovery_controllers.push(peer_controller);
        }
    }
    println!(
        "{}",
        serde_json::json!({"event":"ready","origin":server.context.origin,"usbOrigin":usb_origin,"usbAccess":server.usb_access_for(""),"code":server.context.code(),"recovery":recovery_origins})
    );
    io::stdout().flush().unwrap();
    let (sender,mut receiver)=tokio::sync::mpsc::unbounded_channel();
    let reader = tokio::task::spawn_blocking(move || {
        for line in io::stdin().lock().lines() {
            let Ok(line)=line else {break;};let known=matches!(line.as_str(),"code"|"lan-off"|"usb-off"|"input-suspend"|"input-resume"|"bulk-pressure");
            if sender.send(line).is_err() || !known {break;}
        }
    });
    while let Some(command)=receiver.recv().await {
        match command.as_str() {
            "code"=>println!("{}",serde_json::json!({"event":"pairing-code","code":server.context.code()})),
            "lan-off"=>{server.stop_lan().await;println!("{}",serde_json::json!({"event":"lan-stopped"}));},
            "usb-off"=>{server.stop_usb().await;println!("{}",serde_json::json!({"event":"usb-stopped"}));},
            "input-suspend"=>{controller.suspend_inputs();println!("{}",serde_json::json!({"event":"input-suspended"}));},
            "input-resume"=>{controller.resume_inputs(true);println!("{}",serde_json::json!({"event":"input-resumed"}));},
            "bulk-pressure"=>{controller.audio.note_pressure();println!("{}",serde_json::json!({"event":"bulk-pressure"}));},
            _=>break,
        }
        let _=io::stdout().flush();
    }
    drop(receiver);let _=reader.await;
    for mut peer in recovery_servers { peer.stop().await; }
    for peer_controller in recovery_controllers { peer_controller.shutdown(); }
    server.stop().await;
    controller.shutdown();
    if mixed {
        let _ = std::fs::remove_dir_all(directory);
    }
}
