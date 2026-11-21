//! Local integration fixture: production server/controller, fake device only.
use plumelink::{
    control::{Controller, Native},
    microphone,
    protocol::Command,
    server::Server,
};
use std::io::{self, BufRead, Write};
struct Device;
impl Native for Device {
    fn execute(&mut self, command: &Command) -> Result<(), String> {
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
    let mixed = std::env::args().any(|v| v == "--mixed");
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
    let mut controller = Controller::start(|| Ok(Box::new(Device))).unwrap();
    let store = plumelink::preferences::Store::memory();
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
    let mut server = Server::start_secure_with_files(
        host,
        0,
        controller.clone(),
        store,
        files,
    )
    .await
    .unwrap();
    println!(
        "{}",
        serde_json::json!({"event":"ready","origin":server.context.origin,"code":server.context.code()})
    );
    io::stdout().flush().unwrap();
    let context = server.context.clone();
    let _ = tokio::task::spawn_blocking(move || {
        for line in io::stdin().lock().lines() {
            if line.as_deref().ok() != Some("code") {
                break;
            }
            println!(
                "{}",
                serde_json::json!({"event":"pairing-code","code":context.code()})
            );
            let _ = io::stdout().flush();
        }
    })
    .await;
    server.stop().await;
    controller.shutdown();
    if mixed {
        let _ = std::fs::remove_dir_all(directory);
    }
}
