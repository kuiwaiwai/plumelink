//! Small single-instance WASM adapter; algorithm state remains in the shared core.
use qfast_transmission::*;
use std::cell::RefCell;

struct Kernel {
    sender: Option<Sender>,
    input: [u8; WIRE_BYTES],
    output: [u8; WIRE_BYTES],
}
thread_local! {static KERNEL:RefCell<Kernel>=const {RefCell::new(Kernel {sender:None,input:[0;WIRE_BYTES],output:[0;WIRE_BYTES]})};}
fn time(value: f64) -> Result<u64, Error> {
    if !value.is_finite() || value < 0.0 || value > 9_007_199_254_740_991.0 || value.fract() != 0.0
    {
        Err(Error::InvalidTimestamp)
    } else {
        Ok(value as u64)
    }
}
fn result<T>(value: Result<T, Error>) -> i32 {
    match value {
        Ok(_) => 0,
        Err(Error::Busy) => -1,
        Err(Error::Expired) => -2,
        Err(Error::Capacity) => -3,
        Err(_) => -4,
    }
}
fn sender(f: impl FnOnce(&mut Sender) -> Result<(), Error>) -> i32 {
    KERNEL.with(|k| {
        result(
            k.borrow_mut()
                .sender
                .as_mut()
                .ok_or(Error::InvalidConfig)
                .and_then(f),
        )
    })
}
fn budget(writable: i32) -> SendBudget {
    SendBudget {
        writable_bytes: if writable != 0 { WIRE_BYTES } else { 0 },
        congestion_permitted: writable != 0,
    }
}

#[no_mangle]
pub extern "C" fn qft_input_ptr() -> *mut u8 {
    KERNEL.with(|k| k.borrow_mut().input.as_mut_ptr())
}
#[no_mangle]
pub extern "C" fn qft_output_ptr() -> *const u8 {
    KERNEL.with(|k| k.borrow().output.as_ptr())
}
#[no_mangle]
pub extern "C" fn qft_init(
    start: f64,
    now: f64,
    interval: u32,
    lease_ms: u32,
    motion_ttl_ms: u32,
    action_ttl_ms: u32,
) -> i32 {
    KERNEL.with(|k| {
        let mut k = k.borrow_mut();
        k.sender = None;
        let value = (|| {
            let config = Config {
                lease_ms: u16::try_from(lease_ms).map_err(|_| Error::InvalidConfig)?,
                motion_ttl_ms: u16::try_from(motion_ttl_ms).map_err(|_| Error::InvalidConfig)?,
                action_ttl_ms: u16::try_from(action_ttl_ms).map_err(|_| Error::InvalidConfig)?,
                // The desktop may take up to eight seconds to acknowledge execution.
                // This extends only result observation, never the lifetime of an effect.
                confirmation_grace_ms: 8000,
                base_interval_us: interval,
                max_catchup: 16384,
                ..Config::default()
            };
            Sender::new(decode(&k.input)?, time(start)?, time(now)?, config)
        })();
        match value {
            Ok(s) => {
                k.sender = Some(s);
                0
            }
            Err(e) => result::<()>(Err(e)),
        }
    })
}
#[no_mangle]
pub extern "C" fn qft_set_interval(interval: u32) -> i32 {
    sender(|s| s.set_base_interval_us(interval))
}
#[no_mangle]
pub extern "C" fn qft_offer(x: i32, y: i32, z: i32, captured: f64, now: f64) -> i32 {
    sender(|s| s.offer_at(Vector::new(x, y, z), time(captured)?, time(now)?))
}
#[no_mangle]
pub extern "C" fn qft_step(
    x: i32,
    y: i32,
    z: i32,
    motion_at: f64,
    argument: i32,
    action_at: f64,
    now: f64,
) -> i32 {
    sender(|s| {
        s.queue_step_at(
            Vector::new(x, y, z),
            time(motion_at)?,
            Action { code: 1, argument },
            time(action_at)?,
            time(now)?,
        )
        .map(|_| ())
    })
}
#[no_mangle]
pub extern "C" fn qft_action(argument: i32, captured: f64, now: f64) -> i32 {
    sender(|s| {
        s.queue_action_at(Action { code: 1, argument }, time(captured)?, time(now)?)
            .map(|_| ())
    })
}
#[no_mangle]
pub extern "C" fn qft_receive(now: f64) -> i32 {
    KERNEL.with(|k| {
        let mut k = k.borrow_mut();
        let frame = decode(&k.input);
        result((|| {
            let now = time(now)?;
            let frame = frame?;
            k.sender
                .as_mut()
                .ok_or(Error::InvalidConfig)?
                .receive(frame, now)
        })())
    })
}
#[no_mangle]
pub extern "C" fn qft_poll(now: f64, writable: i32) -> i32 {
    KERNEL.with(|k| {
        let mut k = k.borrow_mut();
        let frame = (|| {
            k.sender
                .as_mut()
                .ok_or(Error::InvalidConfig)?
                .poll(time(now)?, budget(writable))
        })();
        match frame {
            Ok(Some(f)) => match encode(f) {
                Ok(bytes) => {
                    k.output = bytes;
                    1
                }
                Err(e) => result::<()>(Err(e)),
            },
            Ok(None) => 0,
            Err(e) => result::<()>(Err(e)),
        }
    })
}
#[no_mangle]
pub extern "C" fn qft_next(now: f64, writable: i32) -> f64 {
    KERNEL.with(|k| {
        match time(now).and_then(|now| {
            k.borrow()
                .sender
                .as_ref()
                .ok_or(Error::InvalidConfig)?
                .next_poll_at(now, budget(writable))
        }) {
            Ok(Some(at)) => at as f64,
            Ok(None) => -1.0,
            Err(_) => -2.0,
        }
    })
}
#[no_mangle]
pub extern "C" fn qft_renew(now: f64) -> i32 {
    sender(|s| s.begin_renewal(time(now)?))
}
#[no_mangle]
pub extern "C" fn qft_cancel(now: f64) -> i32 {
    sender(|s| s.cancel(time(now)?))
}
#[no_mangle]
pub extern "C" fn qft_closed() -> i32 {
    KERNEL.with(|k| i32::from(k.borrow().sender.as_ref().is_none_or(Sender::is_closed)))
}
#[no_mangle]
pub extern "C" fn qft_outcome() -> i32 {
    KERNEL.with(|k| {
        match k
            .borrow_mut()
            .sender
            .as_mut()
            .and_then(Sender::take_outcome)
        {
            Some(Outcome::Committed(_)) => 1,
            Some(Outcome::NotExecuted(_)) => 2,
            Some(Outcome::Uncertain(_)) => 3,
            None => 0,
        }
    })
}
#[no_mangle]
pub extern "C" fn qft_metric(which: u32) -> f64 {
    KERNEL.with(|k| {
        let k = k.borrow();
        let Some(s) = k.sender.as_ref() else {
            return 0.0;
        };
        let p = s.pacing();
        let st = s.stats();
        match which {
            0 => s.expires_at() as f64,
            1 => p.interval_us as f64,
            2 => p.srtt_us as f64,
            3 => p.queue_us as f64,
            4 => st.sent as f64,
            5 => st.history_merges as f64,
            6 => st.expired_offers as f64,
            7 => s.pending_inputs() as f64,
            _ => 0.0,
        }
    })
}
