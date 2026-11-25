use super::FRAME_BYTES;
use std::time::Instant;

/// Strict ordering and freshness guard. Frames never accumulate without a bound.
#[derive(Default)]
pub struct FrameGuard {
    sequence: u32,
    timestamp: u32,
    anchor: Option<(Instant, u32)>,
    window_started: Option<Instant>,
    window_best: Option<(Instant, u32, i64)>,
}
impl FrameGuard {
    pub fn accept<'a>(
        &mut self,
        bytes: &'a [u8],
        now: Instant,
    ) -> Result<Option<&'a [u8]>, &'static str> {
        if bytes.len() != FRAME_BYTES || &bytes[..4] != b"QFM1" {
            return Err("音频帧格式不匹配");
        }
        let sequence = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let timestamp = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        if sequence <= self.sequence || (self.sequence > 0 && timestamp <= self.timestamp) {
            return Err("音频序号无效");
        }
        self.sequence = sequence;
        self.timestamp = timestamp;
        let (anchor, origin) = *self.anchor.get_or_insert((now, timestamp));
        let elapsed = now.duration_since(anchor).as_millis() as i64;
        let age = elapsed - i64::from(timestamp.wrapping_sub(origin));
        if age < -200 {
            return Err("音频时钟无效");
        }
        if age > 150 {
            return Ok(None);
        }
        // Track the packet with lowest one-way transit delay in this 30 s window
        // so re-anchoring never locks onto a transiently delayed packet.
        if self
            .window_best
            .is_none_or(|(_, _, best_age)| age <= best_age)
        {
            self.window_best = Some((now, timestamp, age));
        }
        let window_started = *self.window_started.get_or_insert(now);
        if now.duration_since(window_started).as_millis() > 30_000 {
            self.anchor = Some(
                self.window_best
                    .take()
                    .map(|(best_now, best_ts, _)| (best_now, best_ts))
                    .unwrap_or((now, timestamp)),
            );
            self.window_started = Some(now);
        }
        Ok(Some(&bytes[12..]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn packet(sequence: u32, timestamp: u32) -> Vec<u8> {
        let mut frame = vec![0u8; FRAME_BYTES];
        frame[..4].copy_from_slice(b"QFM1");
        frame[4..8].copy_from_slice(&sequence.to_le_bytes());
        frame[8..12].copy_from_slice(&timestamp.to_le_bytes());
        frame
    }

    #[test]
    fn reanchoring_uses_minimum_delay_packet_in_window() {
        let mut guard = FrameGuard::default();
        let start = Instant::now();
        assert!(guard.accept(&packet(1, 0), start).unwrap().is_some());
        assert!(guard
            .accept(&packet(2, 10_000), start + Duration::from_millis(10_005))
            .unwrap()
            .is_some());
        // Packet right after 30 s arrives with 100 ms transit spike; next packet at 40 s
        // arrives normally at +100 ms relative to start and must remain acceptable.
        assert!(guard
            .accept(&packet(3, 30_020), start + Duration::from_millis(30_120))
            .unwrap()
            .is_some());
        assert!(guard
            .accept(&packet(4, 40_000), start + Duration::from_millis(40_010))
            .unwrap()
            .is_some());
    }

    #[test]
    fn a_late_packet_after_a_window_boundary_cannot_reset_the_freshness_budget() {
        let mut guard = FrameGuard::default();
        let start = Instant::now();
        assert!(guard.accept(&packet(1, 0), start).unwrap().is_some());
        for index in 1..=1500 {
            let timestamp = index * 20;
            assert!(guard.accept(&packet(index + 1, timestamp), start + Duration::from_millis(u64::from(timestamp) + 10)).unwrap().is_some());
        }
        assert!(guard.accept(&packet(1502, 30_020), start + Duration::from_millis(30_160)).unwrap().is_some());
        assert!(guard.accept(&packet(1503, 30_040), start + Duration::from_millis(30_320)).unwrap().is_none(), "280 ms late audio must not fit a 150 ms freshness budget");
        assert!(guard.accept(&packet(1504, 30_340), start + Duration::from_millis(30_350)).unwrap().is_some(), "fresh capture should recover without replaying old audio");
    }

    #[test]
    fn sustained_capture_tracks_clock_drift_in_both_directions() {
        let start = Instant::now();
        for drift_ppm in [-500i64, 500] {
            let mut guard = FrameGuard::default();
            for index in 0..=30_000 {
                let timestamp = index * 20;
                let arrival_ms = i64::from(timestamp) + i64::from(timestamp) * drift_ppm / 1_000_000 + 10;
                let arrival = start + Duration::from_millis(arrival_ms as u64);
                assert!(guard.accept(&packet(index + 1, timestamp), arrival).unwrap().is_some(), "capture at {timestamp} ms with {drift_ppm} ppm skew must remain fresh");
            }
        }
    }
}
