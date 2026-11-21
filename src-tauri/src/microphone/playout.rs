use super::{RATE, SAMPLES};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const MAX_SAMPLES: usize = SAMPLES * 6;
const FADE: usize = RATE as usize / 200;
const MAX_QUEUE_AGE: Duration = Duration::from_millis(150);

/// Bounded PCM playout: arrival jitter controls priming, slow occupancy feedback corrects clocks.
const HISTORY_SAMPLES: usize = (RATE as usize * 15) / 1000;
const PLC_SAMPLES: usize = RATE as usize / 100;

pub(super) struct Playout {
    queue: VecDeque<i16>,
    history: VecDeque<i16>,
    plc: VecDeque<i16>,
    arrivals: VecDeque<(Instant, usize)>,
    previous: Option<(Instant, u32)>,
    jitter_ms: f64,
    target: usize,
    playing: bool,
    fraction: f64,
    prev_sample: f64,
    occupancy: f64,
    last: f64,
    fade: usize,
    recovery_target: usize,
    stable_samples: usize,
    pub dropped_frames: u64,
    pub underruns: u64,
    pub concealed_samples: u64,
    pub received_frames: u64,
    pub source_gap_frames: u64,
    pub drift_ppm: i32,
}
impl Default for Playout {
    fn default() -> Self {
        Self {
            queue: VecDeque::with_capacity(MAX_SAMPLES),
            history: VecDeque::with_capacity(HISTORY_SAMPLES),
            plc: VecDeque::with_capacity(PLC_SAMPLES),
            arrivals: VecDeque::with_capacity(7),
            previous: None,
            jitter_ms: 0.0,
            target: SAMPLES * 2,
            playing: false,
            fraction: 0.0,
            prev_sample: 0.0,
            occupancy: (SAMPLES * 2) as f64,
            last: 0.0,
            fade: FADE,
            recovery_target: 0,
            stable_samples: 0,
            dropped_frames: 0,
            underruns: 0,
            concealed_samples: 0,
            received_frames: 0,
            source_gap_frames: 0,
            drift_ppm: 0,
        }
    }
}
impl Playout {
    pub fn queued_ms(&self) -> u32 {
        (self.queue.len() * 1000 / RATE as usize) as u32
    }
    pub fn target_ms(&self) -> u32 {
        (self.target * 1000 / RATE as usize) as u32
    }
    pub fn jitter_ms(&self) -> f64 {
        self.jitter_ms
    }
    pub fn clear(&mut self) {
        self.queue.clear();
        self.history.clear();
        self.plc.clear();
        self.arrivals.clear();
        self.previous = None;
        self.jitter_ms = 0.0;
        self.target = SAMPLES * 2;
        self.playing = false;
        self.occupancy = self.target as f64;
        self.last = 0.0;
        self.prev_sample = 0.0;
        self.fraction = 0.0;
        self.fade = FADE;
        self.recovery_target = 0;
        self.stable_samples = 0;
        self.drift_ppm = 0;
    }
    fn discard(&mut self, count: usize) {
        let mut remaining = count.min(self.queue.len());
        for sample in self.queue.drain(..remaining) {
            if self.history.len() == HISTORY_SAMPLES {
                self.history.pop_front();
            }
            self.history.push_back(sample);
            self.prev_sample = f64::from(sample);
        }
        while remaining > 0 {
            let (_, samples) = self.arrivals.front_mut().unwrap();
            let removed = remaining.min(*samples);
            *samples -= removed;
            remaining -= removed;
            if *samples == 0 {
                self.arrivals.pop_front();
            }
        }
    }
    fn expire(&mut self, now: Instant) {
        let mut expired = 0;
        while let Some(&(arrival, samples)) = self.arrivals.front() {
            if now.saturating_duration_since(arrival) < MAX_QUEUE_AGE {
                break;
            }
            self.discard(samples);
            expired += samples;
        }
        if expired > 0 {
            self.dropped_frames += expired.div_ceil(SAMPLES) as u64;
            self.playing = false;
            self.stable_samples = 0;
            self.drift_ppm = 0;
            self.fraction = 0.0;
            self.fade = FADE;
            self.plc.clear();
        }
    }
    pub fn push(&mut self, pcm: &[u8], timestamp: u32, now: Instant) -> bool {
        let before = self.dropped_frames;
        self.expire(now);
        if let Some((arrival, captured)) = self.previous {
            let spacing = timestamp.saturating_sub(captured);
            let deviation = (now.saturating_duration_since(arrival).as_secs_f64() * 1000.0
                - f64::from(spacing))
            .abs();
            self.jitter_ms += (deviation.min(200.0) - self.jitter_ms) / 16.0;
            let wanted = ((40.0 + 4.0 * self.jitter_ms).clamp(40.0, 80.0) * f64::from(RATE)
                / 1000.0) as usize;
            let wanted = wanted.max(self.recovery_target);
            // A falling target moves slowly; a jitter spike can increase priming immediately.
            self.target = if wanted > self.target {
                wanted
            } else {
                self.target.saturating_sub(1).max(wanted)
            };
            self.source_gap_frames += u64::from(spacing.saturating_sub(20) / 20);
        }
        self.previous = Some((now, timestamp));
        self.received_frames += 1;
        let excess = (self.queue.len() + SAMPLES).saturating_sub(MAX_SAMPLES);
        let dropped = excess.div_ceil(SAMPLES);
        if dropped > 0 {
            self.discard(dropped * SAMPLES);
            self.dropped_frames += dropped as u64;
            self.stable_samples = 0;
            self.fade = FADE;
            self.fraction = 0.0;
        }
        self.queue.extend(
            pcm.as_chunks::<2>()
                .0
                .iter()
                .map(|v| i16::from_le_bytes(*v)),
        );
        self.arrivals.push_back((now, SAMPLES));
        self.dropped_frames > before
    }
    pub fn consume(&mut self, output: &mut [i16]) {
        self.consume_at(output, Instant::now());
    }
    fn consume_at(&mut self, output: &mut [i16], now: Instant) {
        self.expire(now);
        if !self.playing && self.queue.len() >= self.target {
            self.playing = true;
            self.plc.clear();
            self.fade = FADE;
            self.fraction = 0.0;
            self.occupancy = self.queue.len() as f64;
        }
        // Average across the packet sawtooth (~20 ms); adjust at most 0.2% in either direction.
        let alpha = (output.len() as f64 / (f64::from(RATE) * 4.0)).min(1.0);
        self.occupancy += alpha * (self.queue.len() as f64 - self.occupancy);
        let error = self.occupancy - self.target as f64;
        let correction = if !self.playing || error.abs() < SAMPLES as f64 / 2.0 {
            0.0
        } else {
            (error / (f64::from(RATE) * 10.0)).clamp(-0.002, 0.002)
        };
        self.drift_ppm = (correction * 1_000_000.0) as i32;
        for sample in output {
            if self.playing && self.queue.is_empty() {
                self.playing = false;
                self.underruns += 1;
                self.drift_ppm = 0;
                // Remember sparse stalls that an arrival EWMA would forget before the next spike.
                self.recovery_target = SAMPLES * 4;
                self.target = self.recovery_target;
                self.stable_samples = 0;
                self.discard(self.queue.len());
                self.fraction = 0.0;
                self.fill_plc();
                if self.plc.is_empty() {
                    self.fade = FADE;
                } else {
                    self.fade = 0;
                }
            }
            if !self.playing {
                if let Some(concealed) = self.plc.pop_front() {
                    self.last = f64::from(concealed);
                    *sample = concealed;
                } else {
                    // Decay the last value over 5 ms; sustained loss becomes silence, never repeated old speech.
                    if self.fade > 0 {
                        self.last *= (self.fade - 1) as f64 / self.fade as f64;
                        self.fade -= 1;
                    } else {
                        self.last = 0.0;
                    }
                    *sample = self.last.round() as i16;
                }
                if self.underruns > 0 {
                    self.concealed_samples += 1;
                }
                continue;
            }
            let y_1 = self.prev_sample;
            let y0 = f64::from(self.queue[0]);
            let y1 = f64::from(self.queue.get(1).copied().unwrap_or(self.queue[0]));
            let y2 = f64::from(
                self.queue
                    .get(2)
                    .copied()
                    .unwrap_or_else(|| self.queue.get(1).copied().unwrap_or(self.queue[0])),
            );
            let mut value = cubic_hermite(y_1, y0, y1, y2, self.fraction);
            if self.fade > 0 {
                value = self.last + (value - self.last) / self.fade as f64;
                self.fade -= 1;
            }
            value = soft_limit(value);
            self.last = value;
            *sample = value.round() as i16;
            self.fraction += 1.0 + correction;
            let advance = self.fraction.floor() as usize;
            self.fraction -= advance as f64;
            self.discard(advance);
            self.stable_samples = (self.stable_samples + 1).min(RATE as usize * 10);
            if self.stable_samples == RATE as usize * 10 {
                self.recovery_target = 0;
            }
        }
    }
    fn fill_plc(&mut self) {
        self.plc.clear();
        let len = self.history.len();
        if len < 120 {
            return;
        }
        let max_lag = 240.min(len / 2);
        let min_lag = 60.min(max_lag);
        let mut best_lag = min_lag;
        let mut best_score = f64::NEG_INFINITY;
        for lag in min_lag..=max_lag {
            let mut num = 0.0;
            let mut denom_a = 0.0;
            let mut denom_b = 0.0;
            let window = lag.min(120);
            let start = len - window;
            for i in 0..window {
                let a = f64::from(self.history[start + i]);
                let b = f64::from(self.history[start - lag + i]);
                num += a * b;
                denom_a += a * a;
                denom_b += b * b;
            }
            let norm = (denom_a * denom_b).sqrt();
            if norm > 1.0 {
                let score = num / norm;
                if score > best_score {
                    best_score = score;
                    best_lag = lag;
                }
            }
        }
        let cycle_start = len - best_lag;
        for i in 0..PLC_SAMPLES {
            let progress = (i + 1) as f64 / PLC_SAMPLES as f64;
            let envelope = 0.5 * (1.0 + (std::f64::consts::PI * progress).cos());
            let raw = f64::from(self.history[cycle_start + (i % best_lag)]);
            self.plc.push_back((raw * envelope).round() as i16);
        }
    }
}

fn cubic_hermite(y_1: f64, y0: f64, y1: f64, y2: f64, t: f64) -> f64 {
    if t == 0.0 {
        return y0;
    }
    let a = -0.5 * y_1 + 1.5 * y0 - 1.5 * y1 + 0.5 * y2;
    let b = y_1 - 2.5 * y0 + 2.0 * y1 - 0.5 * y2;
    let c = -0.5 * y_1 + 0.5 * y1;
    ((a * t + b) * t + c) * t + y0
}

fn soft_limit(sample: f64) -> f64 {
    const KNEE: f64 = 30_000.0;
    const CEIL: f64 = 32_767.0;
    let abs = sample.abs();
    if abs <= KNEE {
        return sample;
    }
    let excess = (abs - KNEE) / (CEIL - KNEE);
    let limited = KNEE + (CEIL - KNEE) * (excess / (1.0 + excess));
    limited.copysign(sample).clamp(-CEIL, CEIL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    fn pcm(value: i16) -> Vec<u8> {
        value.to_le_bytes().repeat(SAMPLES)
    }
    #[test]
    fn long_pause_discards_incomplete_priming_before_new_speech() {
        let now = Instant::now();
        let mut buffer = Playout::default();
        buffer.push(&pcm(-12_000), 20, now);
        buffer.consume_at(&mut [0; SAMPLES], now);
        for n in 0..4 {
            buffer.push(
                &pcm(2000),
                920 + n * 20,
                now + Duration::from_millis(900 + u64::from(n * 20)),
            );
        }
        let mut recovered = [0; SAMPLES];
        buffer.consume_at(&mut recovered, now + Duration::from_millis(960));
        assert!(
            recovered.iter().all(|sample| *sample >= 0),
            "old startup audio must not survive a network pause"
        );
        assert_eq!(buffer.dropped_frames, 1);
    }

    #[test]
    fn render_stall_expires_old_samples_even_without_an_incoming_packet() {
        let now = Instant::now();
        let mut buffer = Playout::default();
        for n in 0..3 {
            buffer.push(
                &pcm(4000),
                n * 20,
                now + Duration::from_millis(u64::from(n * 20)),
            );
        }
        buffer.consume_at(&mut [0; 120], now + Duration::from_millis(40));
        let mut resumed = [1; SAMPLES];
        buffer.consume_at(&mut resumed, now + Duration::from_millis(500));
        assert!(resumed[120..].iter().all(|sample| *sample == 0));
        assert_eq!(buffer.queued_ms(), 0);
        assert_eq!(buffer.dropped_frames, 3);
    }

    #[test]
    fn expired_audio_restarts_recovery_stability_window() {
        let now = Instant::now();
        let mut buffer = Playout::default();
        for n in 0..4 {
            buffer.push(
                &pcm(1000),
                n * 20,
                now + Duration::from_millis(u64::from(n * 20)),
            );
        }
        buffer.consume_at(&mut [0; SAMPLES * 6], now + Duration::from_millis(60));
        assert_eq!(buffer.underruns, 1);
        for n in 4..500 {
            let arrival = now + Duration::from_millis(u64::from(n * 20));
            buffer.push(&pcm(1000), n * 20, arrival);
            buffer.consume_at(&mut [0; SAMPLES], arrival);
        }
        assert!(buffer.stable_samples > RATE as usize * 9);
        buffer.consume_at(&mut [0; SAMPLES], now + Duration::from_millis(10_500));
        assert_eq!(buffer.stable_samples, 0);
        for n in 525..580 {
            let arrival = now + Duration::from_millis(u64::from(n * 20));
            buffer.push(&pcm(1000), n * 20, arrival);
            buffer.consume_at(&mut [0; SAMPLES], arrival);
        }
        assert_eq!(buffer.target_ms(), 80);
        assert_eq!(buffer.recovery_target, SAMPLES * 4);
    }

    #[test]
    fn exact_frame_boundary_does_not_count_as_an_underrun() {
        let now = Instant::now();
        let mut buffer = Playout::default();
        for n in 0..2 {
            buffer.push(
                &pcm(2000),
                n * 20,
                now + Duration::from_millis(u64::from(n * 20)),
            );
        }
        buffer.consume_at(&mut [0; SAMPLES], now + Duration::from_millis(20));
        buffer.consume_at(&mut [0; SAMPLES], now + Duration::from_millis(40));
        assert_eq!(buffer.underruns, 0);
        assert_eq!(buffer.queue.len(), 0);
        buffer.push(&pcm(2000), 60, now + Duration::from_millis(60));
        let mut output = [0; SAMPLES];
        buffer.consume_at(&mut output, now + Duration::from_millis(60));
        assert_eq!(output, [2000; SAMPLES]);
        assert_eq!(buffer.underruns, 0);
    }

    #[test]
    fn sparse_delay_spikes_reprime_with_headroom_instead_of_repeating_underruns() {
        let start = Instant::now();
        let mut buffer = Playout::default();
        let mut arrival = 0;
        let mut packets = (0..1000u32)
            .map(|n| {
                arrival = arrival.max(n * 20 + if n % 100 == 50 { 55 } else { 0 });
                (arrival, n * 20)
            })
            .peekable();
        for ms in (0..20_000).step_by(5) {
            while packets.peek().is_some_and(|(arrival, _)| *arrival <= ms) {
                let (arrival, timestamp) = packets.next().unwrap();
                buffer.push(
                    &pcm(1000),
                    timestamp,
                    start + Duration::from_millis(u64::from(arrival)),
                );
            }
            buffer.consume_at(&mut [0; 120], start + Duration::from_millis(u64::from(ms)));
            assert!(buffer.queued_ms() <= 120);
            assert!((40..=80).contains(&buffer.target_ms()));
        }
        assert_eq!(
            buffer.underruns, 1,
            "learn from the first sparse delay spike"
        );
        assert_eq!(buffer.dropped_frames, 0);
    }

    #[test]
    fn engine_consumption_in_twenty_ms_batches_keeps_steady_audio_continuous() {
        let now = Instant::now();
        let mut buffer = Playout::default();
        for n in 0..1000 {
            buffer.push(
                &pcm(1000),
                n * 20,
                now + Duration::from_millis(u64::from(n * 20)),
            );
            buffer.consume_at(
                &mut [0; SAMPLES],
                now + Duration::from_millis(u64::from(n * 20)),
            );
        }
        assert_eq!(buffer.underruns, 0);
        assert_eq!(buffer.dropped_frames, 0);
    }
    #[test]
    fn bursty_arrivals_play_without_underruns_and_latency_stays_bounded() {
        let start = Instant::now();
        let mut buffer = Playout::default();
        let mut plain = VecDeque::new();
        let mut plain_missing = 0;
        let mut packets = (0..500u32)
            .map(|n| (n * 20 + if n % 2 == 0 { 0 } else { 15 }, n * 20))
            .peekable();
        for ms in (0..10_000).step_by(5) {
            while packets.peek().is_some_and(|(arrival, _)| *arrival <= ms) {
                let (arrival, timestamp) = packets.next().unwrap();
                buffer.push(
                    &pcm(1000),
                    timestamp,
                    start + Duration::from_millis(u64::from(arrival)),
                );
                plain.extend([1000; SAMPLES]);
            }
            buffer.consume_at(&mut [0; 120], start + Duration::from_millis(u64::from(ms)));
            for _ in 0..120 {
                if plain.pop_front().is_none() {
                    plain_missing += 1;
                }
            }
            assert!(buffer.queued_ms() <= 120);
        }
        assert!(plain_missing > 0);
        assert_eq!(buffer.underruns, 0);
        assert_eq!(buffer.dropped_frames, 0);
        assert!((40..=80).contains(&buffer.target_ms()));
    }
    #[test]
    fn loss_fades_to_silence_and_recovery_rebounds_without_backlog() {
        let mut buffer = Playout::default();
        let now = Instant::now();
        for n in 0..3 {
            buffer.push(
                &pcm(4000),
                n * 20,
                now + Duration::from_millis(u64::from(n * 20)),
            );
        }
        buffer.consume_at(&mut [0; 1200], now + Duration::from_millis(40));
        let mut gap = [0; 1200];
        buffer.consume_at(&mut gap, now + Duration::from_millis(90));
        assert_eq!(buffer.underruns, 1);
        assert!(gap[480..].iter().all(|v| *v == 0));
        for n in 3..20 {
            buffer.push(
                &pcm(2000),
                n * 20,
                now + Duration::from_millis(u64::from(n * 20)),
            );
        }
        assert_eq!(buffer.queued_ms(), 120);
        assert!(buffer.dropped_frames > 0);
        let mut recovered = [0; 240];
        buffer.consume_at(&mut recovered, now + Duration::from_millis(380));
        assert!(recovered[120..].iter().all(|v| *v == 2000));
        buffer.clear();
        let mut stopped = [1; 480];
        buffer.consume(&mut stopped);
        assert_eq!(stopped, [0; 480]);
    }
    #[test]
    fn clock_skew_is_corrected_without_unbounded_growth() {
        for ppm in [-1000.0, 1000.0] {
            let mut buffer = Playout::default();
            let now = Instant::now();
            let mut next = 0.0;
            let mut seq = 0;
            for tick in 0..24_000 {
                let ms = f64::from(tick) * 5.0;
                while next <= ms {
                    buffer.push(
                        &pcm(500),
                        seq * 20,
                        now + Duration::from_micros((next * 1000.0) as u64),
                    );
                    seq += 1;
                    next += 20.0 / (1.0 + ppm / 1_000_000.0);
                }
                buffer.consume_at(&mut [0; 120], now + Duration::from_millis(tick as u64 * 5));
                assert!(buffer.queued_ms() <= 120);
            }
            assert_eq!(buffer.dropped_frames, 0);
            assert_eq!(buffer.underruns, 0);
        }
    }

    #[test]
    fn fractional_boundary_with_one_queued_sample_does_not_underrun() {
        let now = Instant::now();
        let mut buffer = Playout::default();
        buffer.push(&pcm(1200), 0, now);
        buffer.push(&pcm(1200), 20, now + Duration::from_millis(20));
        buffer.fraction = 0.25;
        let mut out = [0; SAMPLES * 2 - 1];
        buffer.consume_at(&mut out, now + Duration::from_millis(20));
        assert_eq!(buffer.underruns, 0);
        assert_eq!(buffer.target_ms(), 40);
    }

    #[test]
    fn cubic_hermite_and_waveform_plc_smoothly_extend_pitched_audio() {
        assert!((cubic_hermite(2000.0, 2000.0, 2000.0, 2000.0, 0.37) - 2000.0).abs() < 1e-6);
        assert!(soft_limit(35_000.0) <= 32_767.0);
        let now = Instant::now();
        let mut buffer = Playout::default();
        let mut wave = Vec::with_capacity(SAMPLES * 2);
        for i in 0..SAMPLES {
            let v = ((i as f64 * 2.0 * std::f64::consts::PI / 120.0).sin() * 8000.0).round() as i16;
            wave.extend_from_slice(&v.to_le_bytes());
        }
        for n in 0..2 {
            buffer.push(
                &wave,
                n * 20,
                now + Duration::from_millis(u64::from(n * 20)),
            );
        }
        buffer.consume_at(&mut [0; SAMPLES * 2], now + Duration::from_millis(20));
        let mut gap = [0; SAMPLES];
        buffer.consume_at(&mut gap, now + Duration::from_millis(40));
        assert_eq!(buffer.underruns, 1);
        assert!(gap[..240].iter().any(|&s| s != 0) && gap[240..].iter().all(|&s| s == 0));
    }
}
