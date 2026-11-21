use crate::{Action, Body, Error, Frame, Status, Vector, LEASE_MS, MAX_TOTAL};

/// Fixed wire size, in bytes, of every encoded `QFT2` frame.
pub const WIRE_BYTES: usize = 64;

/// Serialize a frame into a fixed 64-byte `QFT2` buffer.
///
/// Framing does not provide integrity or confidentiality; callers must pair it
/// with an authenticated, congestion-controlled transport.
pub fn encode(frame: Frame) -> Result<[u8; WIRE_BYTES], Error> {
    let mut out = [0; WIRE_BYTES];
    encode_into(frame, &mut out)?;
    Ok(out)
}

/// Serialize a frame directly into a caller-supplied 64-byte `QFT2` buffer.
///
/// The destination buffer is cleared and populated in canonical little-endian
/// form. On error, `out` may contain zeroed or partially written bytes.
pub fn encode_into(frame: Frame, out: &mut [u8; WIRE_BYTES]) -> Result<(), Error> {
    if frame.session == 0 || frame.lease == 0 {
        return Err(Error::InvalidFrame);
    }
    out.fill(0);
    out[..4].copy_from_slice(b"QFT2");
    out[8..16].copy_from_slice(&frame.session.to_le_bytes());
    out[16..20].copy_from_slice(&frame.lease.to_le_bytes());
    out[48..52].copy_from_slice(&frame.serial.to_le_bytes());
    let (kind, version, id, total) = match frame.body {
        Body::Grant { ttl_ms } => {
            if !LEASE_MS.contains(&ttl_ms) {
                return Err(Error::InvalidFrame);
            }
            out[42..44].copy_from_slice(&ttl_ms.to_le_bytes());
            (1, 0, 0, Vector::ZERO)
        }
        Body::Checkpoint { version, total } => {
            if version == 0 {
                return Err(Error::InvalidFrame);
            }
            (2, version, 0, total)
        }
        Body::Action {
            id,
            version,
            total,
            action,
        }
        | Body::CancelAction {
            id,
            version,
            total,
            action,
        } => {
            if id == 0 {
                return Err(Error::InvalidFrame);
            }
            out[40..42].copy_from_slice(&action.code.to_le_bytes());
            out[44..48].copy_from_slice(&action.argument.to_le_bytes());
            let kind = if matches!(frame.body, Body::Action { .. }) {
                3
            } else {
                7
            };
            (kind, version, id, total)
        }
        Body::Cancel => (4, 0, 0, Vector::ZERO),
        Body::Renew {
            version,
            last_action,
            total,
        } => (5, version, last_action, total),
        Body::Ack {
            version,
            action_id,
            status,
            queue_us,
        } => {
            out[52..56].copy_from_slice(&(status as u32).to_le_bytes());
            out[56..60].copy_from_slice(&queue_us.to_le_bytes());
            (6, version, action_id, Vector::ZERO)
        }
    };
    if !total.bounded(MAX_TOTAL) {
        return Err(Error::Range);
    }
    out[4] = kind;
    out[20..24].copy_from_slice(&version.to_le_bytes());
    out[24..28].copy_from_slice(&id.to_le_bytes());
    out[28..32].copy_from_slice(&total.x.to_le_bytes());
    out[32..36].copy_from_slice(&total.y.to_le_bytes());
    out[36..40].copy_from_slice(&total.z.to_le_bytes());
    Ok(())
}

/// Decode a canonical 64-byte `QFT2` wire frame.
///
/// Rejects wrong lengths, unknown kinds/statuses, out-of-range vector totals,
/// and nonzero reserved or unused payload fields.
pub fn decode(bytes: &[u8]) -> Result<Frame, Error> {
    if bytes.len() != WIRE_BYTES || &bytes[..4] != b"QFT2" {
        return Err(Error::InvalidFrame);
    }
    let get32 = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let session = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
    let lease = get32(16);
    let serial = get32(48);
    let version = get32(20);
    let id = get32(24);
    let total = Vector::new(get32(28) as i32, get32(32) as i32, get32(36) as i32);
    let body = match bytes[4] {
        1 => Body::Grant {
            ttl_ms: u16::from_le_bytes(bytes[42..44].try_into().unwrap()),
        },
        2 => Body::Checkpoint { version, total },
        3 | 7 => {
            let action = Action {
                code: u16::from_le_bytes(bytes[40..42].try_into().unwrap()),
                argument: get32(44) as i32,
            };
            if bytes[4] == 7 {
                Body::CancelAction {
                    id,
                    version,
                    total,
                    action,
                }
            } else {
                Body::Action {
                    id,
                    version,
                    total,
                    action,
                }
            }
        }
        4 => Body::Cancel,
        5 => Body::Renew {
            version,
            last_action: id,
            total,
        },
        6 => Body::Ack {
            version,
            action_id: id,
            queue_us: get32(56),
            status: match get32(52) {
                0 => Status::Applied,
                1 => Status::Committed,
                2 => Status::Cancelled,
                3 => Status::Expired,
                4 => Status::Stale,
                5 => Status::Conflict,
                6 => Status::ExecutionUncertain,
                7 => Status::Rejected,
                _ => return Err(Error::InvalidFrame),
            },
        },
        _ => return Err(Error::InvalidFrame),
    };
    let frame = Frame {
        session,
        lease,
        serial,
        body,
    };
    // Canonical re-encoding rejects nonzero reserved / irrelevant fields.
    if encode(frame)?.as_slice() != bytes {
        return Err(Error::InvalidFrame);
    }
    Ok(frame)
}
