//! Aggregate file pacing while a live audio stream shares the network.
use std::time::{Duration, Instant};

pub(crate) struct BulkClock {
    next: Instant,
    pressured_until: Instant,
}
impl BulkClock {
    pub fn new(now: Instant) -> Self {
        Self {
            next: now,
            pressured_until: now,
        }
    }
    pub fn pressure(&mut self, now: Instant) {
        self.pressured_until = now + Duration::from_secs(2);
    }
    pub fn reserve(&mut self, running: bool, bytes: usize, now: Instant) -> Option<Instant> {
        if !running {
            self.next = now;
            return None;
        }
        let rate = if now < self.pressured_until {
            256 * 1024
        } else {
            32 * 1024 * 1024
        };
        let step = Duration::from_secs_f64(bytes as f64 / rate as f64);
        // Bound accumulated pacing debt so concurrent streams cannot starve after audio pressure ends.
        self.next = (self.next.max(now) + step).min(now + Duration::from_millis(500));
        Some(self.next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_bulk_streams_share_budget_and_pressure_expires() {
        let now = Instant::now();
        let mut clock = BulkClock::new(now);
        assert_eq!(
            clock.reserve(true, 1024 * 1024, now),
            Some(now + Duration::from_micros(31250))
        );
        assert_eq!(
            clock.reserve(true, 1024 * 1024, now),
            Some(now + Duration::from_micros(62500))
        );
        clock.pressure(now);
        assert_eq!(
            clock.reserve(true, 64 * 1024, now),
            Some(now + Duration::from_micros(312500))
        );
        let later = now + Duration::from_secs(3);
        assert_eq!(
            clock.reserve(true, 1024 * 1024, later),
            Some(later + Duration::from_micros(31250))
        );
        assert_eq!(clock.reserve(false, 64 * 1024, later), None);
    }
}
