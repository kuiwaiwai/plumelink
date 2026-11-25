use futures_util::FutureExt;
use plumelink::{control::{Controller, Native}, protocol::{Button, Command, Key}};
use std::{sync::{Arc, Mutex, mpsc}, time::Duration};

async fn ready(controller: &Controller) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !controller.input_available() { tokio::time::sleep(Duration::from_millis(5)).await; }
    }).await.expect("input must recover after release");
}

#[tokio::test]
async fn suspend_discards_both_devices_queued_effects_and_releases_before_new_admission() {
    struct Input {
        calls: Arc<Mutex<Vec<Command>>>, started: Option<mpsc::SyncSender<()>>, resume: mpsc::Receiver<()>,
    }
    impl Native for Input {
        fn execute(&mut self, command: &Command) -> Result<(), String> {
            self.calls.lock().unwrap().push(command.clone());
            if let Some(started) = self.started.take() {
                started.send(()).unwrap();
                self.resume.recv_timeout(Duration::from_secs(3)).map_err(|_| "test gate expired")?;
            }
            Ok(())
        }
        fn release(&mut self) -> Result<(), String> { self.calls.lock().unwrap().push(Command::Release {}); Ok(()) }
        fn switch_to_window(&mut self, _: u64) -> Result<(), String> { panic!("cancelled window must not execute") }
        fn restore_volume(&mut self, _: f32) -> Result<(), String> { panic!("cancelled volume must not execute") }
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    let (started, held) = mpsc::sync_channel(1);
    let (resume, gate) = mpsc::channel();
    let effects = calls.clone();
    let c = Controller::start(move || Ok(Box::new(Input { calls: effects, started: Some(started), resume: gate }))).unwrap();
    assert!(c.activate(1)); assert!(c.activate(2));
    let held_input = c.submit(1, Command::Buttonstate { button: Button::Left, down: true });
    held.recv_timeout(Duration::from_secs(2)).unwrap();
    let click = c.submit(1, Command::Click { button: Button::Left, count: 1 });
    let text = c.submit(2, Command::Text { text: "obsolete".into() });
    let mut window = Box::pin(c.switch_window(7, || Ok(())));
    let mut volume = Box::pin(c.restore_volume(0.5));
    assert!(window.as_mut().now_or_never().is_none());
    assert!(volume.as_mut().now_or_never().is_none());
    c.suspend_inputs(); c.resume_inputs(true);
    let blocked = !c.activate(3);
    resume.send(()).unwrap();
    held_input.await.unwrap().unwrap();
    assert!(blocked);
    assert!(click.await.unwrap().is_err()); assert!(text.await.unwrap().is_err());
    assert!(window.await.is_err()); assert!(volume.await.is_err());
    ready(&c).await;
    assert!(c.submit(1, Command::Key { key: Key::Enter }).await.unwrap().is_err());
    assert!(c.activate(3));
    c.submit(3, Command::Key { key: Key::Enter }).await.unwrap().unwrap();
    assert_eq!(*calls.lock().unwrap(), vec![Command::Buttonstate { button: Button::Left, down: true }, Command::Release {}, Command::Key { key: Key::Enter }]);
    c.shutdown();
}

#[tokio::test]
async fn old_qft_grants_and_staged_text_cannot_cross_resume() {
    use qfast_transmission::*;
    struct Input(Arc<Mutex<usize>>);
    impl Native for Input {
        fn execute(&mut self, _: &Command) -> Result<(), String> { *self.0.lock().unwrap() += 1; Ok(()) }
        fn release(&mut self) -> Result<(), String> { Ok(()) }
    }
    let calls = Arc::new(Mutex::new(0)); let effects = calls.clone();
    let c = Controller::start(move || Ok(Box::new(Input(effects)))).unwrap();
    assert!(c.activate(1));
    let grant = decode(&c.qft_open(1).await.unwrap().unwrap().unwrap()).unwrap();
    c.qft_stage(1, 1, Command::Text { text: "old".into() }).await.unwrap().unwrap();
    let old_action = Frame { serial: 1, body: Body::Action { id: 1, version: 0, total: Vector::ZERO, action: Action { code: 1, argument: 1 } }, ..grant };
    c.suspend_inputs();
    assert!(!c.activate(2));
    c.resume_inputs(true); ready(&c).await;
    assert!(c.qft_frame(1, old_action).await.unwrap().is_err());
    assert!(c.activate(2));
    let fresh = decode(&c.qft_open(2).await.unwrap().unwrap().unwrap()).unwrap();
    let reply = c.qft_frame(2, Frame { session: fresh.session, lease: fresh.lease, ..old_action }).await.unwrap().unwrap().unwrap();
    assert!(matches!(decode(&reply).unwrap().body, Body::Ack { status: Status::ExecutionUncertain, .. }));
    assert_eq!(*calls.lock().unwrap(), 0);
    c.shutdown();
}

#[tokio::test]
async fn failed_release_keeps_new_input_blocked_until_retry_succeeds() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Input(Arc<AtomicUsize>);
    impl Native for Input {
        fn execute(&mut self, _: &Command) -> Result<(), String> { Ok(()) }
        fn release(&mut self) -> Result<(), String> { Ok(()) }
        fn end_session(&mut self) -> Result<(), String> {
            if self.0.fetch_add(1, Ordering::SeqCst) < 2 { Err("release busy".into()) } else { Ok(()) }
        }
    }
    let attempts = Arc::new(AtomicUsize::new(0)); let effects = attempts.clone();
    let c = Controller::start(move || Ok(Box::new(Input(effects)))).unwrap();
    c.suspend_inputs(); c.resume_inputs(true);
    assert!(!c.activate(1));
    tokio::time::timeout(Duration::from_secs(2), async {
        while attempts.load(Ordering::SeqCst) == 0 { tokio::task::yield_now().await; }
    }).await.unwrap();
    assert!(!c.activate(1));
    ready(&c).await;
    assert!(attempts.load(Ordering::SeqCst) >= 3);
    assert!(c.activate(2));
    c.shutdown();
}

#[tokio::test]
async fn window_still_authorizing_is_cancelled_at_resume_boundary() {
    struct Input;
    impl Native for Input {
        fn execute(&mut self, _: &Command) -> Result<(), String> { Ok(()) }
        fn release(&mut self) -> Result<(), String> { Ok(()) }
        fn switch_to_window(&mut self, _: u64) -> Result<(), String> { panic!("stale prepared switch") }
    }
    let c = Controller::start(|| Ok(Box::new(Input))).unwrap();
    let (started, held) = mpsc::sync_channel(1); let (resume, gate) = mpsc::channel();
    let mut switch = Box::pin(c.switch_window(7, move || {
        started.send(()).unwrap();
        gate.recv_timeout(Duration::from_secs(3)).map_err(|_| "test gate expired".into())
    }));
    assert!(switch.as_mut().now_or_never().is_none());
    held.recv_timeout(Duration::from_secs(2)).unwrap();
    c.suspend_inputs(); c.resume_inputs(true); resume.send(()).unwrap();
    assert!(switch.await.unwrap_err().contains("取消"));
    ready(&c).await;
    let mut boundary = c.input_boundary(); let generation = *boundary.borrow_and_update();
    c.resume_inputs(false);
    assert_eq!(*boundary.borrow_and_update(), generation);
    c.shutdown();
}
