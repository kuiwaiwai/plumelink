use crate::Error;

/// Application-ACK feedback, in microseconds. Queue time is measured by the receiver.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Feedback {
    /// Total observed application round-trip time, in microseconds.
    pub rtt_us: u32,
    /// Portion of `rtt_us` spent waiting in the receiver's local queue.
    pub queue_us: u32,
}

/// Token-bucket rate limiter and dual RTT/queue adaptive application pacer.
///
/// Separates network RTT (`rtt_us - queue_us`) for pacing decisions from full
/// application-ACK RTT (`ack_srtt_us + 4 * ack_variation_us`) for retry timeouts.
#[derive(Clone, Copy, Debug)]
pub struct Pacer {
    interval_us: u32,
    srtt_us: u32,
    variation_us: u32,
    queue_us: u32,
    ack_srtt_us: u32,
    ack_variation_us: u32,
    base_us: u32,
    samples: [(u64, u32); 32],
    last_feedback: u64,
    baseline_us: u32,
    last_adjustment: Option<u64>,
    next_send: u64,
    tokens: u32,
    reserve: u32,
    refill_at: u64,
}
/// Read-only snapshot; modifying this value cannot affect pacing invariants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacerMetrics {
    /// Minimum retained network sample at the latest valid feedback time.
    /// One minimum is retained per second, for at most 30 seconds.
    pub baseline_us: u32,
    /// Current target interval between normal checkpoints, in microseconds.
    pub interval_us: u32,
    /// Smoothed network RTT (excluding receiver queue), in microseconds.
    pub srtt_us: u32,
    /// Mean deviation estimate of network RTT, in microseconds.
    pub variation_us: u32,
    /// Most recently observed receiver queue delay, in microseconds.
    pub queue_us: u32,
    /// Smoothed full application-ACK RTT (including receiver queue), in microseconds.
    pub ack_srtt_us: u32,
    /// Mean deviation estimate of full application-ACK RTT, in microseconds.
    pub ack_variation_us: u32,
}
impl Pacer {
    /// Return a read-only telemetry snapshot of current estimators and pacing interval.
    #[must_use]
    pub fn metrics(&self) -> PacerMetrics {
        PacerMetrics {
            baseline_us: self.baseline_us,
            interval_us: self.interval_us,
            srtt_us: self.srtt_us,
            variation_us: self.variation_us,
            queue_us: self.queue_us,
            ack_srtt_us: self.ack_srtt_us,
            ack_variation_us: self.ack_variation_us,
        }
    }
    pub(crate) fn set_base_interval(&mut self, interval: u32) {
        // Keep any congestion backoff and in-flight accounting across a preference change.
        let backoff = self.interval_us.saturating_sub(self.base_us);
        self.base_us = interval;
        self.interval_us = (interval + backoff).min(64_000);
    }

    /// Construct a pacer with `base_interval_us` (clamped to `1_000..=64_000`) at `now_us`.
    #[must_use]
    pub fn new(base_us: u32, now_us: u64) -> Self {
        let base_us = base_us.clamp(1000, 64_000);
        Self {
            interval_us: base_us,
            base_us,
            srtt_us: 0,
            variation_us: 0,
            queue_us: 0,
            ack_srtt_us: 0,
            ack_variation_us: 0,
            samples: [(0, 0); 32],
            last_feedback: now_us,
            baseline_us: 0,
            last_adjustment: None,
            next_send: now_us,
            tokens: 4_000_000,
            reserve: 2_000_000,
            refill_at: now_us,
        }
    }
    /// Clear retained network baseline and RTT estimates after an underlying path
    /// change (such as a Wi-Fi to cellular handoff) while preserving token-bucket
    /// rate limits and pacing schedule (`next_send`) so rate bounds cannot be bypassed.
    pub fn reset_path(&mut self, now_us: u64) -> Result<(), Error> {
        if now_us < self.last_feedback || now_us < self.refill_at {
            return Err(Error::ClockReversed);
        }
        self.samples = [(0, 0); 32];
        self.baseline_us = 0;
        self.srtt_us = 0;
        self.variation_us = 0;
        self.queue_us = 0;
        self.ack_srtt_us = 0;
        self.ack_variation_us = 0;
        self.interval_us = self.base_us;
        self.last_adjustment = None;
        self.last_feedback = now_us;
        Ok(())
    }
    /// Incorporate one valid application-ACK delay observation with adaptive pacing enabled.
    pub fn observe(&mut self, feedback: Feedback, now_us: u64) {
        self.observe_with_adaptation(feedback, now_us, true);
    }
    pub(crate) fn observe_with_adaptation(
        &mut self,
        feedback: Feedback,
        now_us: u64,
        adaptive: bool,
    ) {
        if feedback.rtt_us == 0
            || feedback.rtt_us > 30_000_000
            || feedback.queue_us > feedback.rtt_us
            || now_us < self.last_feedback
            || now_us < self.refill_at
        {
            return;
        }
        let network = feedback.rtt_us.saturating_sub(feedback.queue_us).max(1);
        self.last_feedback = now_us;
        if self.ack_srtt_us == 0 {
            self.ack_srtt_us = feedback.rtt_us;
            self.ack_variation_us = feedback.rtt_us / 2;
        } else {
            self.ack_variation_us =
                (3 * self.ack_variation_us + self.ack_srtt_us.abs_diff(feedback.rtt_us)) / 4;
            self.ack_srtt_us = (7 * self.ack_srtt_us + feedback.rtt_us) / 8;
        }
        // Feedback density must not evict a recent low-delay reference. Bucket
        // by elapsed time, retaining the actual sample time so higher samples
        // cannot rejuvenate an old minimum. Slots wrap only after 32 seconds.
        let bucket = now_us / 1_000_000;
        let slot = &mut self.samples[(bucket % self.samples.len() as u64) as usize];
        if slot.1 == 0 || slot.0 / 1_000_000 != bucket || network <= slot.1 {
            *slot = (now_us, network);
        }
        let baseline = self
            .samples
            .iter()
            .filter(|(at, value)| *value != 0 && now_us.saturating_sub(*at) <= 30_000_000)
            .map(|(_, value)| *value)
            .min()
            .unwrap_or(network);
        self.baseline_us = baseline;
        if self.srtt_us == 0 {
            self.srtt_us = network;
            self.variation_us = network / 2;
        } else {
            self.variation_us = (3 * self.variation_us + self.srtt_us.abs_diff(network)) / 4;
            self.srtt_us = (7 * self.srtt_us + network) / 8;
        }
        self.queue_us = feedback.queue_us;
        let congested = network.saturating_sub(baseline) > 20_000.max(baseline / 2)
            || feedback.queue_us > 24_000;
        if adaptive
            && self
                .last_adjustment
                .map_or(true, |at| now_us.saturating_sub(at) >= 250_000)
        {
            self.last_adjustment = Some(now_us);
            self.interval_us = if congested {
                (self.interval_us * 3 / 2).min(64_000)
            } else {
                self.interval_us.saturating_sub(2000).max(self.base_us)
            };
        }
    }
    /// A heuristic estimate, NOT a latency guarantee or a synchronized timestamp.
    pub fn estimated_arrival_us(&self) -> u32 {
        if self.srtt_us == 0 {
            0
        } else {
            self.srtt_us / 2 + self.variation_us * 2 + self.queue_us
        }
    }
    /// Current retry timeout (`clamp(ACK_SRTT + 4*ACK_var + 10ms, 20ms, 200ms)`), defaulting to 100 ms.
    #[must_use]
    pub fn retry_us(&self) -> u32 {
        if self.ack_srtt_us == 0 {
            100_000
        } else {
            (self.ack_srtt_us + self.ack_variation_us * 4 + 10_000).clamp(20_000, 200_000)
        }
    }
    /// Earliest token/pacing opportunity without consuming a token.
    pub(crate) fn next_permit_at(&self, now_us: u64, urgent: bool) -> Option<u64> {
        let (tokens, rate) = if urgent {
            (self.reserve, 8_u64)
        } else {
            (self.tokens, 80_u64)
        };
        let missing = 1_000_000_u64.saturating_sub(tokens as u64);
        let token_at = self.refill_at.checked_add(missing.div_ceil(rate))?;
        let pace_at = if urgent { now_us } else { self.next_send };
        Some(now_us.max(token_at).max(pace_at))
    }
    fn refill(&mut self, now_us: u64) {
        if now_us <= self.refill_at {
            return;
        }
        let elapsed = now_us.saturating_sub(self.refill_at).min(1_000_000) as u32;
        self.tokens = (self.tokens + elapsed * 80).min(4_000_000);
        self.reserve = (self.reserve + elapsed * 8).min(2_000_000);
        self.refill_at = now_us;
    }
    /// Consume a normal checkpoint permit or an urgent control permit if available at `now_us`.
    pub fn permit(&mut self, now_us: u64, urgent: bool) -> bool {
        self.refill(now_us);
        if urgent {
            if self.reserve < 1_000_000 {
                return false;
            }
            self.reserve -= 1_000_000;
        } else {
            if now_us < self.next_send || self.tokens < 1_000_000 {
                return false;
            }
            self.tokens -= 1_000_000;
            self.next_send = now_us.saturating_add(self.interval_us as u64);
        }
        true
    }
}
