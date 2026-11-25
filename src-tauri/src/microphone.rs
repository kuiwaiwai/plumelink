//! Live PCM into an explicitly selected Windows render endpoint (virtual cable input).
mod frame;
mod levels;
mod playout;
mod route;
mod wasapi;
pub use frame::FrameGuard;
pub use levels::{read as endpoint_volumes, EndpointVolume, EndpointVolumes};
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
    pub gain_clipped_samples: u64,
    pub clipping: bool,
    #[serde(skip)]
    last_gain_clip_at: Option<Instant>,
    pub source_gap_frames: u64,
    pub queued_ms: u32,
    pub target_ms: u32,
    pub jitter_ms: f64,
    pub drift_ppm: i32,
}
impl Status {
    fn snapshot(&self, now: Instant) -> Self {
        let mut state = self.clone();
        state.clipping = self.active && self.last_gain_clip_at
            .and_then(|at| now.checked_duration_since(at))
            .is_some_and(|age| age < Duration::from_secs(2));
        state
    }
}
pub struct DeviceStatus {
    pub owned: bool,
    pub busy: bool,
    pub status: Status,
}
pub type Renderer = dyn Fn(&str, &Stream, &str, std::sync::mpsc::SyncSender<Result<(), String>>) -> Result<(), String>
    + Send
    + Sync;
pub struct AudioHub {
    gain_db: std::sync::Arc<std::sync::atomic::AtomicU8>,
    session_number: AtomicU64,
    active: Mutex<Option<Arc<Stream>>>,
    renderer: Arc<Renderer>,
    render_slots: Arc<tokio::sync::Semaphore>,
    bulk: Mutex<super::bulk::BulkClock>,
    bulk_gate: tokio::sync::Mutex<()>,
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
pub(crate) struct AudioSession {
    pub stream: Arc<Stream>,
    pub number: u64,
    stop_on_drop: bool,
}
impl AudioSession {
    fn into_stream(mut self) -> (Arc<Stream>, u64) {
        self.stop_on_drop = false;
        (self.stream.clone(), self.number)
    }
}
impl Drop for AudioSession {
    fn drop(&mut self) {
        if self.stop_on_drop { self.stream.stop(); }
    }
}
pub(crate) struct AudioReady {
    stream: Arc<Stream>,
    receiver: std::sync::mpsc::Receiver<Result<(), String>>,
    deadline: Instant,
}
impl AudioReady {
    pub(crate) fn wait(self) -> Result<(), String> {
        loop {
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            match self.receiver.recv_timeout(remaining.min(Duration::from_millis(50))) {
                Ok(Ok(())) if !self.stream.stopped() => return Ok(()),
                Ok(Ok(())) => return Err("麦克风启动已取消".into()),
                Ok(Err(error)) => {
                    {
                        let mut status = self.stream.status.lock().unwrap();
                        if !self.stream.stopped() { status.error = error.clone(); }
                    }
                    self.stream.stop();
                    return Err(error);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                    if !self.stream.stopped() && Instant::now() < self.deadline => {}
                Err(_) if self.stream.stopped() => return Err("麦克风启动已取消".into()),
                Err(_) => { self.stream.stop(); return Err("音频设备启动超时".into()); }
            }
        }
    }
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
            render_slots: Arc::new(tokio::sync::Semaphore::new(2)),
            bulk: Mutex::new(super::bulk::BulkClock::new(now)),
            bulk_gate: tokio::sync::Mutex::new(()),
        }
    }
    pub fn note_pressure(&self) {
        self.bulk.lock().unwrap().pressure(Instant::now());
    }
    pub fn bulk_quantum(&self, remaining: usize) -> usize {
        if !self.active.lock().unwrap().as_ref().is_some_and(|s|!s.stopped()) {return remaining.min(64*1024);}
        self.bulk.lock().unwrap().quantum(remaining,Instant::now())
    }
    pub async fn pace_bulk(&self, bytes: usize) {
        let mut remaining=bytes;
        while remaining>0 {
            let _turn=self.bulk_gate.lock().await;
            let now=Instant::now();
            let (count,anchor)={let clock=self.bulk.lock().unwrap();(clock.quantum(remaining,now),clock.anchor(now))};
            loop {
                let now=Instant::now();
                let running=self.active.lock().unwrap().as_ref().is_some_and(|s|!s.stopped());
                let until=self.bulk.lock().unwrap().deadline(running,count,anchor,now);
                match until {
                    None=>break,
                    Some(until) if now>=until=>{self.bulk.lock().unwrap().complete(until);break;},
                    Some(until)=>tokio::time::sleep_until(until.min(now+Duration::from_millis(16)).into()).await,
                }
            }
            remaining-=count;
        }
    }
    pub fn status(&self) -> Status {
        self.active
            .lock()
            .unwrap()
            .as_ref()
            .map(|s| s.status.lock().unwrap().snapshot(Instant::now()))
            .unwrap_or_default()
    }
    pub fn device_status(&self, device: &str) -> DeviceStatus {
        let active = self.active.lock().unwrap();
        match active.as_ref() {
            Some(stream) if stream.device == device => DeviceStatus {
                owned: true, busy: false,
                status: stream.status.lock().unwrap().snapshot(Instant::now()),
            },
            other => DeviceStatus {
                owned: false, busy: other.is_some_and(|stream| !stream.stopped()), status: Status::default(),
            },
        }
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
        let (session, ready) = self.prepare_session(endpoint, device, name, resume)?;
        ready.wait()?;
        Ok(session.into_stream())
    }
    pub(crate) fn prepare_session(
        &self,
        endpoint: &str,
        device: &str,
        name: &str,
        resume: Option<u64>,
    ) -> Result<(AudioSession, AudioReady), String> {
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
        let render_slot = self.render_slots.clone().try_acquire_owned()
            .map_err(|_| "电脑音频设备正在停止，请稍后重试".to_string())?;
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
        let deadline = Instant::now() + Duration::from_secs(4);
        std::thread::Builder::new()
            .name("PlumeLink-audio".into())
            .spawn(move || {
                let _render_slot = render_slot;
                if source.stopped() { return; }
                let result = renderer(&endpoint, &source, &name, tx.clone());
                let mut status = source.status.lock().unwrap();
                status.active = false;
                match result {
                    Err(error) if !source.stopped() => {
                        status.error = error.clone();
                        let _ = tx.try_send(Err(error));
                    }
                    _ => {}
                }
                source.stopped.store(true, Ordering::Release);
            })
            .map_err(|e| {
                stream.stop();
                e.to_string()
            })?;
        Ok((
            AudioSession { stream: stream.clone(), number: session, stop_on_drop: true },
            AudioReady { stream, receiver: rx, deadline },
        ))
    }
}
impl Stream {
    pub fn apply_gain(&self, samples: &mut [i16]) {
        let db = self.gain_db.load(Ordering::Relaxed).min(36);
        let multiplier = 10.0_f32.powf(f32::from(db) / 20.0);
        let mut clipped = 0;
        for sample in samples {
            let amplified = (f32::from(*sample) * multiplier).round();
            let bounded = amplified.clamp(f32::from(i16::MIN), f32::from(i16::MAX));
            clipped += u64::from(amplified != bounded);
            *sample = bounded as i16;
        }
        if clipped > 0 {
            let mut status = self.status.lock().unwrap();
            if !self.stopped() {
                status.gain_clipped_samples = status.gain_clipped_samples.saturating_add(clipped);
                status.last_gain_clip_at = Some(Instant::now());
            }
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
        status.last_gain_clip_at = None;
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
    fn pacing_hub() -> Arc<AudioHub> {
        let hub=Arc::new(AudioHub::with_renderer(Arc::new(|_,stream,_,ready| {
            let _=ready.send(Ok(()));
            while !stream.stopped() {std::thread::sleep(Duration::from_millis(1));}
            Ok(())
        })));
        hub.start("generated","owner","fixture").unwrap();hub.note_pressure();hub
    }
    #[tokio::test]
    async fn bulk_waiters_are_fifo_and_pay_separate_quanta() {
        use futures_util::FutureExt;
        let hub=pacing_hub();let order=Arc::new(Mutex::new(Vec::new()));
        let work=|id| {let hub=hub.clone();let order=order.clone();async move {hub.pace_bulk(16384).await;order.lock().unwrap().push(id);}};
        let mut first=Box::pin(work(1));let mut second=Box::pin(work(2));let mut third=Box::pin(work(3));
        let began=Instant::now();
        assert!(first.as_mut().now_or_never().is_none());assert!(second.as_mut().now_or_never().is_none());assert!(third.as_mut().now_or_never().is_none());
        tokio::time::timeout(Duration::from_secs(3),async {tokio::join!(first,second,third);}).await.unwrap();
        assert_eq!(*order.lock().unwrap(),vec![1,2,3]);assert!(began.elapsed()>=Duration::from_millis(180));hub.stop();
    }
    #[tokio::test]
    async fn cancelled_bulk_waits_release_the_turn_without_accumulating_debt() {
        use futures_util::FutureExt;
        let hub=pacing_hub();
        for _ in 0..20 {
            let mut pending=Box::pin(hub.pace_bulk(16384));assert!(pending.as_mut().now_or_never().is_none());drop(pending);
            assert!(hub.bulk_gate.try_lock().is_ok());
        }
        let now=Instant::now();assert_eq!(hub.bulk.lock().unwrap().anchor(now),now);
        hub.stop();
    }
    #[tokio::test]
    async fn stopping_audio_unblocks_the_active_bulk_turn_and_queued_waiters() {
        use futures_util::FutureExt;
        let hub=pacing_hub();let mut first=Box::pin(hub.pace_bulk(65536));let mut second=Box::pin(hub.pace_bulk(65536));
        assert!(first.as_mut().now_or_never().is_none());assert!(second.as_mut().now_or_never().is_none());
        hub.stop();
        tokio::time::timeout(Duration::from_millis(150),async {tokio::join!(first,second);}).await.unwrap();
    }
    fn packet(sequence: u32, time: u32) -> Vec<u8> {
        let mut bytes = vec![0; FRAME_BYTES];
        bytes[..4].copy_from_slice(b"QFM1");
        bytes[4..8].copy_from_slice(&sequence.to_le_bytes());
        bytes[8..12].copy_from_slice(&time.to_le_bytes());
        bytes
    }
    #[test]
    fn device_diagnostics_hide_other_streams_and_preserve_the_owners_stopped_error() {
        let hub = AudioHub::with_renderer(Arc::new(|_, stream, _, ready| {
            ready.send(Ok(())).unwrap();
            while !stream.stopped() { std::thread::sleep(Duration::from_millis(5)); }
            Ok(())
        }));
        assert!(!hub.device_status("owner").owned);
        let stream = hub.start("test", "owner", "Owner").unwrap();
        {
            let mut status = stream.status.lock().unwrap();
            status.active = true; status.device_name = "Private phone".into(); status.error = "Private driver error".into();
            status.received_frames = 9; status.input_level = 0.5;
        }
        let own = hub.device_status("owner");
        assert!(own.owned); assert_eq!(own.status.received_frames, 9);
        let other = hub.device_status("other");
        assert!(!other.owned); assert!(other.busy);
        assert_eq!(serde_json::to_value(other.status).unwrap(), serde_json::to_value(Status::default()).unwrap());
        stream.stop();
        assert!(!hub.device_status("other").busy);
        assert!(!hub.device_status("owner").status.active);
        assert_eq!(hub.device_status("owner").status.error, "Private driver error");
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
        assert_eq!(stream.status.lock().unwrap().gain_clipped_samples, 2);
        stream.gain_db.store(0, Ordering::Relaxed);
        let mut full_scale = [i16::MIN, i16::MAX, 0];
        stream.apply_gain(&mut full_scale);
        assert_eq!(full_scale, [i16::MIN, i16::MAX, 0]);
        assert_eq!(stream.status.lock().unwrap().gain_clipped_samples, 2);
    }
    #[test]
    fn gain_clipping_hint_expires_and_does_not_serialize_internal_timing() {
        let now = Instant::now();
        let mut state = Status {
            active: true,
            gain_clipped_samples: 3,
            last_gain_clip_at: Some(now),
            ..Status::default()
        };
        assert!(state.snapshot(now).clipping);
        assert!(state.snapshot(now + Duration::from_millis(1999)).clipping);
        assert!(!state.snapshot(now + Duration::from_secs(2)).clipping);
        let serialized = serde_json::to_value(state.snapshot(now)).unwrap();
        assert_eq!(serialized["gainClippedSamples"], 3);
        assert_eq!(serialized["clipping"], true);
        assert!(!serialized.as_object().unwrap().keys().any(|key| key.contains("ClipAt") || key.contains("clip_at")));
        state.active = false;
        assert!(!state.snapshot(now).clipping);
    }
    #[test]
    fn stopping_and_restarting_audio_does_not_carry_over_gain_clipping() {
        let hub = AudioHub::with_renderer(Arc::new(|_, stream, _, ready| {
            stream.status.lock().unwrap().active = true;
            ready.send(Ok(())).unwrap();
            while !stream.stopped() { std::thread::sleep(Duration::from_millis(5)); }
            Ok(())
        }));
        hub.set_gain_db(36);
        let first = hub.start("test", "phone", "Phone").unwrap();
        first.apply_gain(&mut [30000, -30000]);
        assert_eq!(hub.status().gain_clipped_samples, 2);
        assert!(hub.status().clipping);
        first.stop();
        assert!(!hub.status().clipping);
        let next = hub.start("test", "phone", "Phone").unwrap();
        first.apply_gain(&mut [30000]);
        next.apply_gain(&mut [0]);
        assert_eq!(hub.status().gain_clipped_samples, 0);
        assert!(!hub.status().clipping);
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

    #[test]
    fn cancelled_initializers_are_bounded_until_the_native_resources_finish() {
        let release = Arc::new(AtomicBool::new(false));
        struct Unblock(Arc<AtomicBool>);
        impl Drop for Unblock { fn drop(&mut self) { self.0.store(true, Ordering::SeqCst); } }
        let _unblock = Unblock(release.clone());
        let gate = release.clone();
        let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(2);
        let hub = AudioHub::with_renderer(Arc::new(move |_, stream, _, ready| {
            entered_tx.send(()).unwrap();
            let began = Instant::now();
            while !gate.load(Ordering::SeqCst) && began.elapsed() < Duration::from_secs(3) {
                std::thread::sleep(Duration::from_millis(1));
            }
            let _ = ready.send(Ok(()));
            while !stream.stopped() { std::thread::sleep(Duration::from_millis(1)); }
            Err("late cancelled renderer".into())
        }));
        let mut retired = Vec::new();
        for _ in 0..2 {
            let (session, ready) = hub.prepare_session("endpoint", "phone", "Phone", None).unwrap();
            entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            retired.push(session.stream.clone());
            drop(session);
            let began = Instant::now();
            assert!(ready.wait().is_err());
            assert!(began.elapsed() < Duration::from_millis(500));
        }
        let error = match hub.prepare_session("endpoint", "phone", "Phone", None) {
            Ok(_) => panic!("a third native initializer must not be allocated"),
            Err(error) => error,
        };
        assert_eq!(error, "电脑音频设备正在停止，请稍后重试");
        release.store(true, Ordering::SeqCst);
        let began = Instant::now();
        while hub.render_slots.available_permits() != 2 && began.elapsed() < Duration::from_secs(1) {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(hub.render_slots.available_permits(), 2);
        assert!(retired.iter().all(|stream| stream.status.lock().unwrap().error.is_empty()));
        let stream = hub.start("endpoint", "phone", "Phone").unwrap();
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(!stream.stopped());
    }

    #[test]
    fn dropping_an_old_session_guard_does_not_stop_its_replacement() {
        let hub = AudioHub::with_renderer(Arc::new(|_, stream, _, ready| {
            let _ = ready.send(Ok(()));
            while !stream.stopped() { std::thread::sleep(Duration::from_millis(1)); }
            Ok(())
        }));
        let (old, ready) = hub.prepare_session("endpoint", "phone", "Phone", None).unwrap();
        ready.wait().unwrap();
        hub.stop();
        let (current, ready) = hub.prepare_session("endpoint", "phone", "Phone", None).unwrap();
        ready.wait().unwrap();
        drop(old);
        assert!(!current.stream.stopped());
    }

    #[test]
    fn an_initializer_error_keeps_its_cause_after_the_renderer_exits() {
        for reports_ready in [true, false] {
            let hub = AudioHub::with_renderer(Arc::new(move |_, _, _, ready| {
                if reports_ready { let _ = ready.send(Err("driver startup test".into())); }
                Err("driver startup test".into())
            }));
            let (session, ready) = hub.prepare_session("endpoint", "phone", "Phone", None).unwrap();
            assert_eq!(ready.wait().unwrap_err(), "driver startup test");
            assert!(session.stream.stopped());
            assert_eq!(session.stream.status.lock().unwrap().error, "driver startup test");
        }
    }
}
