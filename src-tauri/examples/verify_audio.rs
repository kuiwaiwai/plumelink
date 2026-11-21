//! Local WASAPI adapter smoke: enumerate outputs, optionally render silence to a named virtual endpoint.
use plumelink::microphone::{self, AudioHub, SAMPLES};
use std::{
    thread,
    time::{Duration, Instant},
};

fn main() -> Result<(), String> {
    let endpoints = microphone::endpoints()?;
    println!("{}", serde_json::to_string_pretty(&endpoints).unwrap());
    if let Some(name) = std::env::args().nth(1) {
        let endpoint = endpoints
            .iter()
            .find(|e| e.name == name && e.virtual_hint)
            .ok_or("Expected an exact, active virtual endpoint name")?;
        let hub = AudioHub::default();
        let stream = hub.start(&endpoint.id, "local-smoke", "Silence smoke test")?;
        for frame in 0..10 {
            if stream.stopped() {
                return Err(hub.status().error);
            }
            stream.push(&vec![0; SAMPLES * 2], frame as u32 * 20, Instant::now());
            thread::sleep(Duration::from_millis(20));
        }
        if !hub.status().active {
            return Err("WASAPI was not active".into());
        }
        hub.stop();
        if hub.status().active || !stream.stopped() {
            return Err("WASAPI did not stop".into());
        }
        println!("WASAPI initialized, consumed silence and stopped; no default endpoint changed.");
    }
    Ok(())
}
