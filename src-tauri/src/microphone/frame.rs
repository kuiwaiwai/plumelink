use super::FRAME_BYTES;
use std::time::Instant;

/// Strict ordering and freshness guard. Frames never accumulate without a bound.
#[derive(Default)]
pub struct FrameGuard {
    sequence: u32,
    timestamp: u32,
    anchor: Option<(Instant, u32)>,
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
        if elapsed > 30_000 {
            self.anchor = Some(
                self.window_best
                    .take()
                    .map(|(best_now, best_ts, _)| (best_now, best_ts))
                    .unwrap_or((now, timestamp)),
            );
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
}
