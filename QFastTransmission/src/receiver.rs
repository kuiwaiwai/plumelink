use crate::{Action, Body, Config, Error, Frame, Status, Vector, MAX_TOTAL};

/// Serialized, synchronous effects. `apply` must implement additive coordinates.
/// Errors may mean partial effects; the kernel never retries a failed action.
/// `release` must be idempotent. Do not panic or block indefinitely in a callback.
pub trait Executor {
    /// Application-specific execution error returned on partial or fatal failure.
    type Error;
    /// Apply an additive coordinate catchup delta.
    fn apply(&mut self, delta: Vector) -> Result<(), Self::Error>;
    /// Execute a fenced discrete action immediately after its required prefix.
    fn action(&mut self, action: Action) -> Result<(), Self::Error>;
    /// Idempotently release any held device state (such as pressed mouse buttons or modifiers).
    fn release(&mut self) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, Debug)]
struct Completion {
    body: Body,
    status: Status,
}

/// Receiver-local leases bound acceptance age without synchronizing peer clocks.
#[derive(Debug)]
pub struct Receiver {
    config: Config,
    session: u64,
    lease: u32,
    expires_at: u64,
    clock: u64,
    version: u32,
    total: Vector,
    last_action: u32,
    completion: Option<Completion>,
    last_renewal: Option<Frame>,
    closed: bool,
    release_needed: bool,
}
impl Receiver {
    /// Create a new receiver for an authenticated `session`, initial `lease`, and `config` at `now_us`.
    pub fn new(session: u64, lease: u32, now_us: u64, config: Config) -> Result<Self, Error> {
        let config = config.validate()?;
        if session == 0 || lease == 0 {
            return Err(Error::InvalidConfig);
        }
        Ok(Self {
            config,
            session,
            lease,
            expires_at: now_us
                .checked_add(config.lease_ms as u64 * 1000)
                .ok_or(Error::Range)?,
            clock: now_us,
            version: 0,
            total: Vector::ZERO,
            last_action: 0,
            completion: None,
            last_renewal: None,
            closed: false,
            release_needed: false,
        })
    }
    /// Deliver grants over an authenticated control channel. Expiry starts at minting.
    #[must_use]
    pub fn grant(&self) -> Frame {
        Frame {
            session: self.session,
            lease: self.lease,
            serial: 0,
            body: Body::Grant {
                ttl_ms: self.config.lease_ms,
            },
        }
    }
    /// Return the receiver's authenticated session identifier.
    #[must_use]
    pub fn session(&self) -> u64 {
        self.session
    }
    /// Return the currently active lease generation.
    #[must_use]
    pub fn lease(&self) -> u32 {
        self.lease
    }
    /// Return the highest action identifier accepted in the current lease.
    #[must_use]
    pub fn last_action_id(&self) -> u32 {
        self.last_action
    }
    /// Return the validated configuration governing this receiver.
    #[must_use]
    pub fn config(&self) -> Config {
        self.config
    }
    /// Return the monotonic deadline (in microseconds) when the current lease expires.
    #[must_use]
    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }
    /// Return the highest applied checkpoint version and cumulative coordinate total.
    #[must_use]
    pub fn applied(&self) -> (u32, Vector) {
        (self.version, self.total)
    }
    /// Return `true` if the lease has been closed due to expiry, cancellation, or executor error.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed
    }
    fn advance(&mut self, now_us: u64) -> Result<(), Error> {
        if now_us < self.clock {
            return Err(Error::ClockReversed);
        }
        self.clock = now_us;
        Ok(())
    }
    fn release<E: Executor>(&mut self, executor: &mut E) -> Result<(), Error> {
        if self.release_needed {
            executor.release().map_err(|_| Error::Unconfirmed)?;
            self.release_needed = false;
        }
        Ok(())
    }
    /// Poll at [`Self::expires_at`] even if no frames arrive.
    /// Repeated polls retry a failed idempotent release, but never retry an action.
    pub fn poll<E: Executor>(&mut self, now_us: u64, executor: &mut E) -> Result<(), Error> {
        self.advance(now_us)?;
        if now_us >= self.expires_at && !self.closed {
            self.closed = true;
            self.release_needed = true;
        }
        self.release(executor)
    }
    fn ack(&self, request: Frame, status: Status, action_id: u32, queue_us: u32) -> Frame {
        Frame {
            session: self.session,
            lease: request.lease,
            serial: request.serial,
            body: Body::Ack {
                version: self.version,
                action_id,
                status,
                queue_us,
            },
        }
    }
    fn apply<E: Executor>(&mut self, version: u32, total: Vector, executor: &mut E) -> Status {
        if version < self.version {
            return Status::Stale;
        }
        if version == self.version {
            return if total == self.total {
                Status::Applied
            } else {
                Status::Conflict
            };
        }
        let Some(delta) = total.checked_sub(self.total) else {
            return Status::Rejected;
        };
        if !total.bounded(MAX_TOTAL) || !delta.bounded(self.config.max_catchup) {
            return Status::Rejected;
        }
        if delta != Vector::ZERO && executor.apply(delta).is_err() {
            self.closed = true;
            self.release_needed = true;
            let _ = self.release(executor);
            return Status::ExecutionUncertain;
        }
        self.version = version;
        self.total = total;
        Status::Applied
    }
    /// Call only after channel authentication and authorization. `queue_us` is local
    /// telemetry, not trusted remote timing. The frame itself has no authenticator.
    ///
    /// Processes prefix catchup, fenced actions, lease renewal, and cancellation/result
    /// queries (`CancelAction`) and returns the canonical [`Frame`] reply (`Ack` or `Grant`).
    pub fn receive<E: Executor>(
        &mut self,
        frame: Frame,
        now_us: u64,
        queue_us: u32,
        executor: &mut E,
    ) -> Result<Frame, Error> {
        if frame.session != self.session {
            return Err(Error::WrongSession);
        }
        self.poll(now_us, executor)?;
        if frame.lease != self.lease {
            // A lost renewal grant must not strand held input until the new lease expires.
            if matches!(frame.body, Body::Cancel)
                && self.version == 0
                && self.last_action == 0
                && self
                    .last_renewal
                    .is_some_and(|old| old.lease == frame.lease)
            {
                if !self.closed {
                    self.closed = true;
                    self.release_needed = true;
                }
                self.release(executor)?;
                return Ok(self.ack(frame, Status::Cancelled, 0, queue_us));
            }
            // Retrying a lost renewal reply must not mint a new lease or extend time.
            if !self.closed
                && self
                    .last_renewal
                    .is_some_and(|old| old.lease == frame.lease && old.body == frame.body)
            {
                return Ok(Frame {
                    serial: frame.serial,
                    ..self.grant()
                });
            }
            return Err(Error::WrongLease);
        }
        if let Body::CancelAction {
            id,
            version,
            total,
            action,
        } = frame.body
        {
            if id == 0 || !total.bounded(MAX_TOTAL) {
                return Err(Error::InvalidFrame);
            }
            if !self.closed {
                self.closed = true;
                self.release_needed = true;
            }
            self.release(executor)?;
            let original = Body::Action {
                id,
                version,
                total,
                action,
            };
            let status = if id > self.last_action {
                // Closure precedes the answer: a delayed action can no longer race it.
                Status::Cancelled
            } else if id == self.last_action {
                match self.completion {
                    Some(completion) if completion.body == original => match completion.status {
                        Status::Committed => Status::Committed,
                        Status::Rejected | Status::Stale | Status::Conflict => Status::Cancelled,
                        _ => Status::ExecutionUncertain,
                    },
                    Some(_) => Status::Conflict,
                    None => Status::ExecutionUncertain,
                }
            } else {
                Status::ExecutionUncertain
            };
            return Ok(self.ack(frame, status, id, queue_us));
        }
        if let Body::Action { id, .. } = frame.body {
            if id == self.last_action && id != 0 {
                if let Some(completion) = self.completion {
                    let status = if completion.body == frame.body {
                        completion.status
                    } else {
                        Status::Conflict
                    };
                    return Ok(self.ack(frame, status, id, queue_us));
                }
            }
        }
        if matches!(frame.body, Body::Cancel) {
            if !self.closed {
                self.closed = true;
                self.release_needed = true;
            }
            self.release(executor)?;
            return Ok(self.ack(frame, Status::Cancelled, 0, queue_us));
        }
        if self.closed {
            return Ok(self.ack(frame, Status::Expired, 0, queue_us));
        }
        match frame.body {
            Body::Checkpoint { version, total } => {
                if version == 0 {
                    return Err(Error::InvalidFrame);
                }
                let status = self.apply(version, total, executor);
                Ok(self.ack(frame, status, 0, queue_us))
            }
            Body::Action {
                id,
                version,
                total,
                action,
            } => {
                if id == 0 {
                    return Err(Error::InvalidFrame);
                }
                if id <= self.last_action {
                    return Ok(self.ack(frame, Status::Stale, id, queue_us));
                }
                if self.last_action.checked_add(1) != Some(id) {
                    return Ok(self.ack(frame, Status::Rejected, id, queue_us));
                }
                // A fence carries the cumulative prefix: missing checkpoints need no replay.
                self.last_action = id;
                self.completion = Some(Completion {
                    body: frame.body,
                    status: Status::ExecutionUncertain,
                });
                let mut status = self.apply(version, total, executor);
                if status == Status::Applied {
                    status = if executor.action(action).is_ok() {
                        Status::Committed
                    } else {
                        self.closed = true;
                        self.release_needed = true;
                        let _ = self.release(executor);
                        Status::ExecutionUncertain
                    };
                }
                self.completion = Some(Completion {
                    body: frame.body,
                    status,
                });
                Ok(self.ack(frame, status, id, queue_us))
            }
            Body::Renew {
                version,
                last_action,
                total,
            } => {
                // The renewal total covers in-flight checkpoints. Apply the missing delta before starting the new lease.
                if version < self.version || last_action != self.last_action {
                    return Err(Error::Unconfirmed);
                }
                let next = self.lease.checked_add(1).ok_or(Error::SequenceExhausted)?;
                let deadline = now_us
                    .checked_add(self.config.lease_ms as u64 * 1000)
                    .ok_or(Error::Range)?;
                match self.apply(version, total, executor) {
                    Status::Applied => {}
                    Status::ExecutionUncertain => return Err(Error::Unconfirmed),
                    Status::Rejected => return Err(Error::Range),
                    Status::Conflict
                    | Status::Stale
                    | Status::Cancelled
                    | Status::Expired
                    | Status::Committed => return Err(Error::InvalidFrame),
                }
                self.last_renewal = Some(frame);
                self.lease = next;
                self.expires_at = deadline;
                self.version = 0;
                self.total = total;
                self.last_action = 0;
                self.completion = None;
                Ok(Frame {
                    serial: frame.serial,
                    ..self.grant()
                })
            }
            _ => Err(Error::InvalidFrame),
        }
    }
}
