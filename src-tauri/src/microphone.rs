//! Live PCM into an explicitly selected Windows render endpoint (virtual cable input).
mod frame;
mod playout;
mod route;
mod wasapi;
pub use frame::FrameGuard;
use playout::Playout;
pub use route::{
    check as check_route, default_recordings, restore_defaults,
    set_default as set_default_recording, Check as RouteCheck,
};
use serde::Serialize;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use wasapi::render;

pub const RATE: u32 = 24_000;
pub const SAMPLES: usize = 480;
pub const FRAME_BYTES: usize = 12 + SAMPLES * 2;
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Endpoint {
    pub id: String,
    pub name: String,
    pub virtual_hint: bool,
    pub plume_link: bool,
}
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub active: bool,
    pub device_name: String,
    pub endpoint_name: String,
    pub error: String,
    pub dropped_frames: u64,
    pub underruns: u64,
    pub concealed_samples: u64,
    pub received_frames: u64,
    pub received_signal_frames: u64,
    pub rendered_samples: u64,
    pub rendered_signal_samples: u64,
    pub input_level: f32,
    pub output_level: f32,
    pub source_gap_frames: u64,
    pub queued_ms: u32,
    pub target_ms: u32,
    pub jitter_ms: f64,
    pub drift_ppm: i32,
}
pub type Renderer = dyn Fn(&str, &Stream, &str, std::sync::mpsc::SyncSender<Result<(), String>>) -> Result<(), String>
    + Send
    + Sync;
pub struct AudioHub {
    gain_db: std::sync::Arc<std::sync::atomic::AtomicU8>,
    session_number: AtomicU64,
    active: Mutex<Option<Arc<Stream>>>,
    renderer: Arc<Renderer>,
    bulk: Mutex<super::bulk::BulkClock>,
}

impl Default for AudioHub {
    fn default() -> Self {
        Self::with_renderer(Arc::new(render))
    }
}
pub struct Stream {
    gain_db: std::sync::Arc<std::sync::atomic::AtomicU8>,
    stopped: AtomicBool,
    samples: Mutex<Playout>,
    pub device: String,
    status: Arc<Mutex<Status>>,
}
pub fn endpoints() -> Result<Vec<Endpoint>, String> {
    wasapi::endpoints()
}
pub fn recording_endpoints() -> Result<Vec<Endpoint>, String> {
    wasapi::recording_endpoints()
}
impl Drop for AudioHub {
    fn drop(&mut self) {
        self.stop();
    }
}
impl AudioHub {
    pub fn set_gain_db(&self, db: u8) {
        self.gain_db.store(db.min(36), Ordering::Relaxed);
    }
    /// Host injection boundary for non-device tests. The production host uses WASAPI.
    pub fn with_renderer(renderer: Arc<Renderer>) -> Self {
        let now = Instant::now();
        Self {
            gain_db: Arc::new(std::sync::atomic::AtomicU8::new(6)),
            active: Mutex::new(None),
            session_number: AtomicU64::new(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_micros() as u64),
            renderer,
            bulk: Mutex::new(super::bulk::BulkClock::new(now)),
        }
    }
    pub fn note_pressure(&self) {
        self.bulk.lock().unwrap().pressure(Instant::now());
    }
    pub async fn pace_bulk(&self, bytes: usize) {
        let running = self
            .active
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|s| !s.stopped());
        let until = self
            .bulk
            .lock()
            .unwrap()
            .reserve(running, bytes, Instant::now());
        if let Some(until) = until {
            tokio::time::sleep_until(until.into()).await;
        }
    }
    pub fn status(&self) -> Status {
        self.active
            .lock()
            .unwrap()
            .as_ref()
            .map(|s| s.status.lock().unwrap().clone())
            .unwrap_or_default()
    }
    pub fn stop(&self) {
        let active = self.active.lock().unwrap();
        self.session_number.fetch_add(1, Ordering::Relaxed);
        if let Some(stream) = active.as_ref() {
            stream.stop();
        }
    }
    pub fn start(&self, endpoint: &str, device: &str, name: &str) -> Result<Arc<Stream>, String> {
        self.start_session(endpoint, device, name, None)
            .map(|(stream, _)| stream)
    }
    pub fn start_session(
        &self,
        endpoint: &str,
        device: &str,
        name: &str,
        resume: Option<u64>,
    ) -> Result<(Arc<Stream>, u64), String> {
        if endpoint.is_empty() {
            return Err("请在电脑端选择虚拟音频设备".into());
        }
        let mut active = self.active.lock().unwrap();
        if let Some(previous) = resume {
            if self.session_number.load(Ordering::Relaxed) != previous
                || !active
                    .as_ref()
                    .is_some_and(|stream| stream.device == device)
            {
                return Err("上次麦克风会话已结束，请重新开启".into());
            }
        }
        if active
            .as_ref()
            .is_some_and(|stream| !stream.stopped.load(Ordering::Acquire))
        {
            return Err("另一台手机正在使用电脑麦克风通道".into());
        }
        let stream = Arc::new(Stream {
            gain_db: self.gain_db.clone(),
            stopped: AtomicBool::new(false),
            samples: Mutex::new(Playout::default()),
            device: device.into(),
            status: Arc::new(Mutex::new(Status::default())),
        });
        let session = self.session_number.fetch_add(1, Ordering::Relaxed) + 1;
        *active = Some(stream.clone());
        drop(active); // Status, stop and competing starts stay responsive while WASAPI initializes.
        let source = stream.clone();
        let endpoint = endpoint.to_owned();
        let name = name.to_owned();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let renderer = self.renderer.clone();
        std::thread::Builder::new()
            .name("PlumeLink-audio".into())
            .spawn(move || {
                let result = renderer(&endpoint, &source, &name, tx);
                let mut status = source.status.lock().unwrap();
                status.active = false;
                if let Err(e) = result {
                    status.error = e;
                }
                source.stopped.store(true, Ordering::Release);
            })
            .map_err(|e| {
                stream.stop();
                e.to_string()
            })?;
        // Endpoint initialization is bounded by the caller's blocking task, never the input worker.
        match rx.recv_timeout(Duration::from_secs(4)) {
            Ok(Ok(())) if !stream.stopped() => Ok((stream, session)),
            Ok(Ok(())) => Err("麦克风启动已取消".into()),
            Ok(Err(e)) => {
                stream.stop();
                Err(e)
            }
            Err(_) if stream.stopped() => Err("麦克风启动已取消".into()),
            Err(_) => {
                stream.stop();
                Err("音频设备启动超时".into())
            }
        }
    }
}
impl Stream {
    pub fn apply_gain(&self, samples: &mut [i16]) {
        let db = self.gain_db.load(Ordering::Relaxed).min(36);
        let multiplier = 10.0_f32.powf(f32::from(db) / 20.0);
        for sample in samples {
            *sample = (f32::from(*sample) * multiplier)
                .round()
                .clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16;
        }
    }
    pub fn consume(&self, output: &mut [i16]) {
        let mut queue = self.samples.lock().unwrap();
        queue.consume(output);
        self.update_audio_status(&queue);
    }
    fn update_audio_status(&self, queue: &Playout) {
        let mut status = self.status.lock().unwrap();
        status.dropped_frames = queue.dropped_frames;
        status.underruns = queue.underruns;
        status.concealed_samples = queue.concealed_samples;
        status.received_frames = queue.received_frames;
        status.source_gap_frames = queue.source_gap_frames;
        status.queued_ms = queue.queued_ms();
        status.target_ms = queue.target_ms();
        status.jitter_ms = queue.jitter_ms();
        status.drift_ppm = queue.drift_ppm;
    }

    fn note_rendered(&self, count: u32, signal: u64, level: f32) {
        let mut status = self.status.lock().unwrap();
        status.rendered_samples += u64::from(count);
        status.rendered_signal_samples += signal;
        status.output_level = if self.stopped() { 0.0 } else { level };
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.samples.lock().unwrap().clear();
        let mut status = self.status.lock().unwrap();
        status.active = false;
        status.input_level = 0.0;
        status.output_level = 0.0;
    }
    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    pub fn push(&self, pcm: &[u8], timestamp: u32, now: Instant) -> bool {
        let mut queue = self.samples.lock().unwrap();
        if self.stopped() || pcm.len() != SAMPLES * 2 {
            return false;
        }
        let pressure = queue.push(pcm, timestamp, now);
        self.update_audio_status(&queue);
        let (signal, level) = signal_level(
            pcm.as_chunks::<2>()
                .0
                .iter()
                .map(|v| i16::from_le_bytes(*v)),
        );
        let mut status = self.status.lock().unwrap();
        status.received_signal_frames += u64::from(signal > 0);
        status.input_level = level;
        pressure
    }
}
fn signal_level(samples: impl Iterator<Item = i16>) -> (u64, f32) {
    let mut signal = 0;
    let mut peak = 0;
    for sample in samples {
        if sample != 0 {
            signal += 1;
        }
        peak = peak.max(sample.unsigned_abs());
    }
    (signal, f32::from(peak) / 32768.0)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn packet(sequence: u32, time: u32) -> Vec<u8> {
        let mut bytes = vec![0; FRAME_BYTES];
        bytes[..4].copy_from_slice(b"QFM1");
        bytes[4..8].copy_from_slice(&sequence.to_le_bytes());
        bytes[8..12].copy_from_slice(&time.to_le_bytes());
        bytes
    }
    #[test]
    fn only_the_same_device_can_resume_and_desktop_stop_invalidates_a_session() {
        let hub = AudioHub::with_renderer(Arc::new(|_, stream, _, ready| {
            ready.send(Ok(())).unwrap();
            while !stream.stopped() {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(())
        }));
        let (stream, session) = hub.start_session("test", "phone", "Phone", None).unwrap();
        stream.stop();
        assert!(hub
            .start_session("test", "other", "Other", Some(session))
            .is_err());
        let (_, resumed) = hub
            .start_session("test", "phone", "Phone", Some(session))
            .unwrap();
        hub.stop();
        assert!(hub
            .start_session("test", "phone", "Phone", Some(resumed))
            .is_err());
    }
    #[test]
    fn diagnostics_distinguish_silent_packets_from_output_submitted_to_windows() {
        let stream = Stream {
            gain_db: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            stopped: AtomicBool::new(false),
            samples: Mutex::new(Playout::default()),
            device: "test".into(),
            status: Arc::new(Mutex::new(Status::default())),
        };
        let now = Instant::now();
        stream.push(&vec![0; SAMPLES * 2], 0, now);
        stream.push(
            &i16::MIN.to_le_bytes().repeat(SAMPLES),
            20,
            now + Duration::from_millis(20),
        );
        let state = stream.status.lock().unwrap().clone();
        assert_eq!(state.received_frames, 2);
        assert_eq!(state.received_signal_frames, 1);
        assert_eq!(state.input_level, 1.0);
        assert_eq!(state.rendered_samples, 0);
        let mut output = [0; SAMPLES];
        stream.consume(&mut output);
        assert_eq!(stream.status.lock().unwrap().rendered_samples, 0);
        let (signal, peak) = signal_level(output.into_iter());
        stream.note_rendered(SAMPLES as u32, signal, peak);
        let state = stream.status.lock().unwrap().clone();
        assert_eq!(state.rendered_samples, SAMPLES as u64);
        assert_eq!(state.rendered_signal_samples, 0);
        stream.consume(&mut output);
        let (signal, peak) = signal_level(output.into_iter());
        stream.note_rendered(SAMPLES as u32, signal, peak);
        assert!(stream.status.lock().unwrap().rendered_signal_samples > 0);
        stream.stop();
        assert_eq!(stream.status.lock().unwrap().output_level, 0.0);
    }
    #[test]
    fn gain_boosts_quiet_samples_and_clamps_loud_samples() {
        let stream = Stream {
            gain_db: Arc::new(std::sync::atomic::AtomicU8::new(6)),
            stopped: AtomicBool::new(false),
            samples: Mutex::new(Playout::default()),
            device: "test".into(),
            status: Arc::new(Mutex::new(Status::default())),
        };
        let mut samples = [1000, 30000, -30000, 0];
        stream.apply_gain(&mut samples);
        assert!((1990..=2000).contains(&samples[0]));
        assert_eq!(samples[1], i16::MAX);
        assert_eq!(samples[2], i16::MIN);
        assert_eq!(samples[3], 0);
    }
    #[test]
    fn audio_requires_correct_format_order_and_bounded_relative_delay() {
        let now = Instant::now();
        let mut guard = FrameGuard::default();
        assert!(guard.accept(&[0; 12], now).is_err());
        assert!(guard.accept(&packet(1, 20), now).unwrap().is_some());
        assert!(guard.accept(&packet(1, 20), now).is_err());
        assert!(guard
            .accept(&packet(2, 40), now + Duration::from_millis(400))
            .unwrap()
            .is_none());
        assert!(guard
            .accept(&packet(3, 440), now + Duration::from_millis(420))
            .unwrap()
            .is_some());
        assert!(guard
            .accept(&packet(4, 2000), now + Duration::from_millis(440))
            .is_err());
    }
    #[test]
    fn stopped_stream_never_replays_samples() {
        let stream = Stream {
            gain_db: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            stopped: AtomicBool::new(false),
            samples: Mutex::new(Playout::default()),
            device: "test".into(),
            status: Arc::new(Mutex::new(Status::default())),
        };
        let pcm = 1000i16.to_le_bytes().repeat(SAMPLES);
        for n in 1..=8 {
            stream.push(&pcm, n * 20, Instant::now());
        }
        assert!(stream.status.lock().unwrap().dropped_frames > 0);
        stream.stop();
        assert!(!stream.push(&pcm, 180, Instant::now()));
        let mut output = [1; SAMPLES];
        stream.consume(&mut output);
        assert_eq!(output, [0; SAMPLES]);
    }
    #[test]
    fn initialization_reserves_channel_without_blocking_stop_or_status() {
        let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
        let hub = Arc::new(AudioHub::with_renderer(Arc::new(
            move |_, source, _, ready| {
                entered_tx.send(()).unwrap();
                while !source.stopped() {
                    std::thread::sleep(Duration::from_millis(1));
                }
                let _ = ready.send(Ok(()));
                Ok(())
            },
        )));
        let worker = hub.clone();
        let job = std::thread::spawn(move || worker.start("endpoint", "device", "phone"));
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(hub.start("endpoint", "other", "phone").is_err());
        assert!(!hub.status().active);
        hub.stop();
        assert!(job.join().unwrap().is_err());
    }
}
