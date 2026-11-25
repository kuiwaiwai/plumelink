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
    pub fn quantum(&self, remaining: usize, now: Instant) -> usize {
        remaining.min(if now < self.pressured_until { 16 * 1024 } else { 64 * 1024 })
    }
    pub fn anchor(&self, now: Instant) -> Instant { self.next.max(now) }
    pub fn deadline(&mut self, running: bool, bytes: usize, anchor: Instant, now: Instant) -> Option<Instant> {
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
        Some(anchor + step)
    }
    pub fn complete(&mut self, deadline: Instant) { self.next = deadline; }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_bulk_streams_share_budget_and_pressure_expires() {
        let now = Instant::now();
        let mut clock = BulkClock::new(now);
        let first=clock.deadline(true,1024*1024,clock.anchor(now),now).unwrap();
        assert_eq!(first,now+Duration::from_micros(31250));clock.complete(first);
        let second=clock.deadline(true,1024*1024,clock.anchor(now),now).unwrap();
        assert_eq!(second,now+Duration::from_micros(62500));clock.complete(second);
        clock.pressure(now);
        assert_eq!(
            clock.deadline(true, 64 * 1024,clock.anchor(now), now),
            Some(now + Duration::from_micros(312500))
        );
        let later = now + Duration::from_secs(3);
        assert_eq!(
            clock.deadline(true, 1024 * 1024,clock.anchor(later), later),
            Some(later + Duration::from_micros(31250))
        );
        assert_eq!(clock.deadline(false, 64 * 1024,clock.anchor(later), later), None);
    }
    #[test]
    fn five_streams_pay_their_full_budget_instead_of_sharing_a_capped_deadline() {
        let now=Instant::now();let mut clock=BulkClock::new(now);clock.pressure(now);
        for index in 1..=5 {
            let until=clock.deadline(true,64*1024,clock.anchor(now),now).unwrap();
            assert_eq!(until,now+Duration::from_millis(index*250));clock.complete(until);
        }
        assert_eq!(clock.quantum(65536,now),16384);
        assert_eq!(clock.quantum(65536,now+Duration::from_secs(3)),65536);
    }
    #[test]
    fn abandoned_wait_does_not_charge_and_policy_changes_keep_the_same_anchor() {
        let now=Instant::now();let mut clock=BulkClock::new(now);clock.pressure(now);
        let anchor=clock.anchor(now);
        for _ in 0..100 { assert_eq!(clock.deadline(true,16384,anchor,now),Some(now+Duration::from_micros(62500))); }
        assert_eq!(clock.anchor(now),now);
        clock.pressure(now+Duration::from_millis(10));
        assert_eq!(clock.deadline(true,16384,anchor,now+Duration::from_millis(10)),Some(now+Duration::from_micros(62500)));
        assert_eq!(clock.deadline(false,16384,anchor,now+Duration::from_millis(11)),None);
        assert_eq!(clock.anchor(now+Duration::from_millis(11)),now+Duration::from_millis(11));
    }
}
