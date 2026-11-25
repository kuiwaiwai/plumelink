use crate::control::{Controller, PowerInput};
use std::{collections::HashMap, ffi::c_void, sync::{atomic::{AtomicUsize, Ordering}, Arc, Mutex, OnceLock}};
use windows::Win32::{
    Foundation::HANDLE,
    System::Power::{PowerRegisterSuspendResumeNotification, PowerUnregisterSuspendResumeNotification, DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, HPOWERNOTIFY},
    UI::WindowsAndMessaging::{DEVICE_NOTIFY_CALLBACK, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND},
};

fn subscribers() -> &'static Mutex<HashMap<usize, Arc<PowerInput>>> {
    static SUBSCRIBERS: OnceLock<Mutex<HashMap<usize, Arc<PowerInput>>>> = OnceLock::new();
    SUBSCRIBERS.get_or_init(Mutex::default)
}

unsafe extern "system" fn notification(context: *const c_void, event: u32, _: *const c_void) -> u32 {
    let _ = std::panic::catch_unwind(|| {
        let target = subscribers().lock().unwrap_or_else(|e| e.into_inner())
            .get(&(context as usize)).cloned();
        if let Some(target) = target {
            match event {
                PBT_APMSUSPEND => target.transition(true, false),
                PBT_APMRESUMEAUTOMATIC => target.transition(false, true),
                PBT_APMRESUMESUSPEND => target.transition(false, false),
                _ => {},
            }
        }
    });
    0
}

pub struct PowerMonitor {
    id: usize,
    handle: isize,
    parameters: usize,
}
impl PowerMonitor {
    pub fn register(controller: &Arc<Controller>) -> Result<Self, String> {
        static NEXT: AtomicUsize = AtomicUsize::new(1);
        let id = NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| "电源通知标识已用尽")?;
        {
            let mut entries = subscribers().lock().unwrap_or_else(|e| e.into_inner());
            if entries.len() >= 16 { return Err("电源通知注册过多".into()); }
            entries.insert(id, controller.power_input());
        }
        let parameters = Box::into_raw(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
            Callback: Some(notification), Context: id as *mut c_void,
        }));
        let mut handle = std::ptr::null_mut();
        // The allocation stays fixed until unregister; callbacks resolve an opaque ID, never a borrowed pointer.
        let status = unsafe { PowerRegisterSuspendResumeNotification(DEVICE_NOTIFY_CALLBACK, HANDLE(parameters.cast()), &mut handle) };
        if status.0 != 0 {
            subscribers().lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
            unsafe { drop(Box::from_raw(parameters)); }
            return Err(format!("无法注册电源通知（{}）", status.0));
        }
        Ok(Self { id, handle: handle as isize, parameters: parameters as usize })
    }
}
impl Drop for PowerMonitor {
    fn drop(&mut self) {
        subscribers().lock().unwrap_or_else(|e| e.into_inner()).remove(&self.id);
        let status = unsafe { PowerUnregisterSuspendResumeNotification(HPOWERNOTIFY(self.handle)) };
        if status.0 == 0 {
            unsafe { drop(Box::from_raw(self.parameters as *mut DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS)); }
        } else {
            // A failed unregister may leave Windows referencing the allocation; the removed ID makes callbacks inert.
            crate::logging::warn("power", &format!("could not unregister power notification ({})", status.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct NoInput;
    impl crate::control::Native for NoInput {
        fn execute(&mut self, _: &crate::protocol::Command) -> Result<(), String> { Ok(()) }
        fn release(&mut self) -> Result<(), String> { Ok(()) }
    }
    #[test]
    fn registered_callbacks_are_scoped_and_late_callbacks_are_inert() {
        let controller = Controller::start(|| Ok(Box::new(NoInput))).unwrap();
        let monitor = PowerMonitor::register(&controller).unwrap();
        let id = monitor.id;
        let mut boundary = controller.input_boundary();
        unsafe { notification(id as *const c_void, PBT_APMSUSPEND, std::ptr::null()); }
        assert!(!controller.activate(11));
        let suspended = *boundary.borrow_and_update();
        unsafe { notification(id as *const c_void, PBT_APMRESUMEAUTOMATIC, std::ptr::null()); }
        assert_ne!(*boundary.borrow_and_update(), suspended);
        let resumed = *boundary.borrow_and_update();
        unsafe { notification(id as *const c_void, PBT_APMRESUMESUSPEND, std::ptr::null()); }
        assert_eq!(*boundary.borrow_and_update(), resumed);
        drop(monitor);
        unsafe { notification(id as *const c_void, PBT_APMSUSPEND, std::ptr::null()); }
        assert_eq!(*boundary.borrow_and_update(), resumed);
        controller.shutdown();
    }
}
