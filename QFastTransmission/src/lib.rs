//! QFastTransmission: a bounded, transport-independent interaction kernel.
//!
//! This crate is `no_std`, allocation-free and has no runtime dependencies.
//! The transport adapter supplies authentication and congestion control.
#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

use core::fmt;

mod pacing;
mod receiver;
mod sender;
mod wire;

pub use pacing::{Feedback, Pacer, PacerMetrics};
pub use receiver::{Executor, Receiver};
pub use sender::{Outcome, SendBudget, Sender, SenderStats};
pub use wire::{decode, encode, encode_into, WIRE_BYTES};

/// Maximum number of distinct unsent/unconfirmed history boundary records held by [`Sender`].
pub const HISTORY_CAPACITY: usize = 64;
/// Maximum number of send attempts tracked concurrently by [`Sender`].
pub const FLIGHT_CAPACITY: usize = 8;
/// Maximum absolute coordinate magnitude per axis accepted on the `QFT2` wire format (`2^28`).
pub const MAX_TOTAL: i32 = 1 << 28;

const LEASE_MS: core::ops::RangeInclusive<u16> = 20..=60_000;

/// Additive application coordinates, not necessarily raw OS pointer events.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Vector {
    /// Logical X axis coordinate or delta.
    pub x: i32,
    /// Logical Y axis coordinate or delta.
    pub y: i32,
    /// Logical Z axis coordinate or delta (for example, scroll axis).
    pub z: i32,
}
impl Vector {
    /// Zero vector `(0, 0, 0)`.
    pub const ZERO: Self = Self { x: 0, y: 0, z: 0 };
    /// Construct a 3D additive vector.
    #[must_use]
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }
    /// Returns `true` when all three components are zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.x == 0 && self.y == 0 && self.z == 0
    }
    /// Returns `true` if `limit >= 0` and `|axis| <= limit` for every axis.
    #[must_use]
    pub fn bounded(self, limit: i32) -> bool {
        limit >= 0
            && [self.x, self.y, self.z]
                .iter()
                .all(|v| v.unsigned_abs() <= limit as u32)
    }
    /// Checked component-wise addition returning `None` on signed 32-bit overflow.
    #[must_use]
    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        Some(Self::new(
            self.x.checked_add(rhs.x)?,
            self.y.checked_add(rhs.y)?,
            self.z.checked_add(rhs.z)?,
        ))
    }
    /// Checked component-wise subtraction returning `None` on signed 32-bit overflow.
    #[must_use]
    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        Some(Self::new(
            self.x.checked_sub(rhs.x)?,
            self.y.checked_sub(rhs.y)?,
            self.z.checked_sub(rhs.z)?,
        ))
    }
}

/// Application-defined operation. The adapter validates its code and argument before execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Action {
    /// Application-defined operation opcode.
    pub code: u16,
    /// Fixed 32-bit signed operation argument carried in the wire fence.
    pub argument: i32,
}

/// Error returned by the QFT kernel or wire codec.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Error {
    /// Supplied [`Config`], session, lease, or initial grant parameters are invalid.
    InvalidConfig,
    /// Wire frame encoding, decoding, magic, kind, or reserved-field check failed.
    InvalidFrame,
    /// Incoming frame carries a different `session` ID than this endpoint.
    WrongSession,
    /// Incoming frame carries an unexpected or retired `lease` generation.
    WrongLease,
    /// Supplied monotonic microsecond timestamp moved backward relative to prior calls.
    ClockReversed,
    /// Capture timestamp is in the future relative to `now_us` or out of input order.
    InvalidTimestamp,
    /// Operation, effect channel, or lease lifetime has expired or closed.
    Expired,
    /// Operation cannot be accepted while another fenced action, renewal, or unconsumed outcome is active.
    Busy,
    /// Fixed-size tracking history or in-flight attempt table is full.
    Capacity,
    /// Vector delta, cumulative total, or timestamp arithmetic exceeded protocol bounds.
    Range,
    /// Monotonic lease, version, action ID, or attempt serial reached `u32::MAX`.
    SequenceExhausted,
    /// Peer reply contradicts recorded operation identity, version, or status invariants.
    Conflict,
    /// Synchronous executor callback failed or a renewal reply could not be correlated.
    Unconfirmed,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidConfig => "invalid QFT configuration or initial grant",
            Self::InvalidFrame => "malformed or non-canonical QFT2 frame",
            Self::WrongSession => "frame session does not match endpoint",
            Self::WrongLease => "frame lease is stale or unexpected",
            Self::ClockReversed => "monotonic clock moved backwards",
            Self::InvalidTimestamp => "input capture timestamp is reordered or in the future",
            Self::Expired => "QFT lease or input deadline expired",
            Self::Busy => "sender already has a pending action, renewal, or unconsumed outcome",
            Self::Capacity => "fixed sender history or flight table capacity reached",
            Self::Range => "vector magnitude, catchup delta, or timestamp range exceeded",
            Self::SequenceExhausted => "protocol sequence counter reached limit",
            Self::Conflict => "peer response conflicts with recorded state",
            Self::Unconfirmed => "executor failure or uncorrelatable confirmation",
        })
    }
}
impl core::error::Error for Error {}

/// Receiver acknowledgement status code carried in [`Body::Ack`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Status {
    /// Additive prefix was applied (or matched an equal idempotent checkpoint).
    Applied = 0,
    /// Fenced action succeeded and cached its terminal outcome.
    Committed = 1,
    /// Lease was closed via cancellation before the queried action could execute.
    Cancelled = 2,
    /// Lease had already closed or expired when the effect frame arrived.
    Expired = 3,
    /// Checkpoint version or action ID is strictly older than receiver state.
    Stale = 4,
    /// Same version or action ID presented a conflicting cumulative total or payload.
    Conflict = 5,
    /// Executor callback failed or execution state can no longer be proven.
    ExecutionUncertain = 6,
    /// Checkpoint delta exceeded `max_catchup`/`MAX_TOTAL` or action ID skipped ahead.
    Rejected = 7,
}
impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Applied => "Applied",
            Self::Committed => "Committed",
            Self::Cancelled => "Cancelled",
            Self::Expired => "Expired",
            Self::Stale => "Stale",
            Self::Conflict => "Conflict",
            Self::ExecutionUncertain => "ExecutionUncertain",
            Self::Rejected => "Rejected",
        })
    }
}

/// Typed payload of a decoded `QFT2` [`Frame`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Body {
    /// Authenticated lease grant minted by the receiver.
    Grant {
        /// Receiver lease duration in milliseconds.
        ttl_ms: u16,
    },
    /// Cumulative additive input checkpoint carrying the running total at `version`.
    Checkpoint {
        /// Monotonically increasing checkpoint version within the current lease.
        version: u32,
        /// Cumulative position at this version, including the position carried over on renewal.
        total: Vector,
    },
    /// Self-contained fenced action carrying the exact prefix total it depends on.
    Action {
        /// Lease-scoped sequential action identifier (`>= 1`).
        id: u32,
        /// Exact prefix version required prior to execution.
        version: u32,
        /// Exact cumulative vector total required prior to execution.
        total: Vector,
        /// Application opcode and signed argument.
        action: Action,
    },
    /// Unconditional request to close the lease and idempotently release held device state.
    Cancel,
    /// Close the lease and report the exact operation's cached result. Never executes it.
    CancelAction {
        /// Original action identifier being reconciled.
        id: u32,
        /// Original required prefix version.
        version: u32,
        /// Original required cumulative total.
        total: Vector,
        /// Original application action payload.
        action: Action,
    },
    /// Request the next lease, carrying the cumulative position across the boundary.
    Renew {
        /// Sender's cumulative checkpoint version to fast-forward on the receiver.
        version: u32,
        /// Expected highest committed action identifier on the current lease.
        last_action: u32,
        /// Sender's cumulative total at `version`.
        total: Vector,
    },
    /// Receiver acknowledgement echoing attempt serial and reporting receiver queue delay.
    Ack {
        /// Current receiver prefix version after processing.
        version: u32,
        /// Referenced action ID (or `0` for non-action ACKs / generic cancel).
        action_id: u32,
        /// Terminal or prefix status code.
        status: Status,
        /// Local queue delay measured by the receiver adapter, in microseconds.
        queue_us: u32,
    },
}

/// Session IDs must be fresh and bound to a mutually authenticated association.
/// `serial` identifies one send attempt; actions use a separate stable ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Frame {
    /// Fresh non-zero authenticated session association identifier.
    pub session: u64,
    /// Strictly increasing lease generation within `session` (`>= 1`).
    pub lease: u32,
    /// Per-send attempt sequence number echoed by peer replies.
    pub serial: u32,
    /// Typed protocol body.
    pub body: Body,
}
impl Frame {
    /// Serialize this frame into a fresh 64-byte `QFT2` wire buffer.
    pub fn encode(self) -> Result<[u8; WIRE_BYTES], Error> {
        encode(self)
    }
    /// Serialize this frame directly into an existing 64-byte buffer.
    pub fn encode_into(self, out: &mut [u8; WIRE_BYTES]) -> Result<(), Error> {
        encode_into(self, out)
    }
    /// Decode and canonically validate a 64-byte `QFT2` slice into a [`Frame`].
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        decode(bytes)
    }
}

/// Kernel configuration parameters negotiated by the host adapter during handshake.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Config {
    /// Receiver-enforced lease lifetime in milliseconds (`20..=60_000`).
    pub lease_ms: u16,
    /// Local capture-age TTL for queued additive motion checkpoints (`1..=lease_ms`).
    pub motion_ttl_ms: u16,
    /// Local capture-age TTL for queued fenced actions (`1..=lease_ms`).
    pub action_ttl_ms: u16,
    /// Result observation window after closure, in milliseconds (`0..=10_000`). Zero disables it.
    pub confirmation_grace_ms: u16,
    /// Maximum concurrent unacknowledged tracked attempt slots (`1..=FLIGHT_CAPACITY`).
    pub max_in_flight: u8,
    /// Maximum single catchup step per axis allowed on the receiver (`1..=MAX_TOTAL`).
    pub max_catchup: i32,
    /// Minimum pacing interval floor for regular checkpoints in microseconds (`1_000..=64_000`).
    pub base_interval_us: u32,
    /// Adapt application pacing to queue feedback. Transport congestion control is still required.
    pub adaptive_pacing: bool,
    /// Experimental: RTT/2 is unreliable on asymmetric paths. Disabled by default.
    pub predictive_admission: bool,
    /// Merge bookkeeping for inputs that have never crossed a send boundary.
    pub compact_history: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            lease_ms: 500,
            motion_ttl_ms: 120,
            action_ttl_ms: 200,
            confirmation_grace_ms: 250,
            max_in_flight: 4,
            max_catchup: 16_384,
            base_interval_us: 12_500,
            adaptive_pacing: true,
            predictive_admission: false,
            compact_history: true,
        }
    }
}
impl Config {
    /// Validate configuration bounds and return `Err(Error::InvalidConfig)` if inconsistent.
    #[must_use = "validation returns Err on invalid config"]
    pub fn validate(self) -> Result<Self, Error> {
        if !LEASE_MS.contains(&self.lease_ms)
            || self.motion_ttl_ms == 0
            || self.motion_ttl_ms > self.lease_ms
            || self.action_ttl_ms == 0
            || self.action_ttl_ms > self.lease_ms
            || self.confirmation_grace_ms > 10_000
            || self.max_in_flight == 0
            || self.max_in_flight as usize > FLIGHT_CAPACITY
            || !(1..=MAX_TOTAL).contains(&self.max_catchup)
            || !(1000..=64_000).contains(&self.base_interval_us)
        {
            return Err(Error::InvalidConfig);
        }
        Ok(self)
    }
}
