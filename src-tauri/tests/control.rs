use plumelink::{
    control::{Controller, Native},
    protocol::{Button, Command, Key},
};
use serde_json::{json, Value};
use std::sync::{mpsc, Arc, Mutex};
struct Blocker {
    calls: Arc<Mutex<Vec<Command>>>,
    started: mpsc::SyncSender<()>,
    resume: mpsc::Receiver<()>,
    first: bool,
}
impl Native for Blocker {
    fn qft_apply(&mut self, _session: u64, v: qfast_transmission::Vector) -> Result<(), String> {
        self.execute(&Command::Motion {
            dx: v.x,
            dy: v.y,
            scroll: v.z,
        })
    }
    fn execute(&mut self, c: &Command) -> Result<(), String> {
        self.calls.lock().unwrap().push(c.clone());
        if self.first {
            self.first = false;
            self.started.send(()).unwrap();
            self.resume.recv().unwrap();
        }
        Ok(())
    }
    fn release(&mut self) -> Result<(), String> {
        self.calls.lock().unwrap().push(Command::Release {});
        Ok(())
    }
    fn probe(&mut self) -> Result<Value, String> {
        Ok(json!(true))
    }
}
type Fixture = (
    Arc<Controller>,
    Arc<Mutex<Vec<Command>>>,
    mpsc::Receiver<()>,
    mpsc::SyncSender<()>,
);
fn qft_decoded(bytes: Option<[u8; qfast_transmission::WIRE_BYTES]>) -> qfast_transmission::Frame {
    qfast_transmission::decode(&bytes.expect("expected encoded QFT frame")).unwrap()
}
#[tokio::test]
async fn qft_priority_cancel_settles_discarded_action_and_preserves_other_session() {
    use qfast_transmission::*;
    let (c, calls, started, resume) = fixture();
    c.activate(2);
    let grant = qft_decoded(c.qft_open(1).await.unwrap().unwrap());
    let mut sender = Sender::new(grant, 0, 0, plumelink::qft::config()).unwrap();
    let block = c.submit(2, Command::Key { key: Key::Space });
    started.recv().unwrap();
    sender
        .queue_action(
            Action {
                code: 1,
                argument: 1,
            },
            0,
        )
        .unwrap();
    let stage = c.qft_stage(1, 1, Command::Key { key: Key::Enter });
    let action = c.qft_frame(1, sender.poll(0, SendBudget::AVAILABLE).unwrap().unwrap());
    let other = c.submit(2, Command::Key { key: Key::Right });
    sender.cancel(10_000).unwrap();
    let cancel = c.qft_frame(
        1,
        sender.poll(10_000, SendBudget::AVAILABLE).unwrap().unwrap(),
    );
    assert!(stage.await.unwrap().unwrap().is_null());
    assert!(action.await.unwrap().unwrap().is_none());
    resume.send(()).unwrap();
    block.await.unwrap().unwrap();
    sender
        .receive(qft_decoded(cancel.await.unwrap().unwrap()), 20_000)
        .unwrap();
    assert_eq!(sender.take_outcome(), Some(Outcome::NotExecuted(1)));
    other.await.unwrap().unwrap();
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            Command::Key { key: Key::Space },
            Command::Release {},
            Command::Key { key: Key::Right }
        ]
    );
    c.shutdown();
}
#[tokio::test]
async fn qft_worker_rejects_a_frame_that_expired_while_waiting_for_the_executor() {
    use qfast_transmission::*;
    let (c, calls, started, resume) = fixture();
    c.activate(2);
    let cfg = Config {
        motion_ttl_ms: 100,
        action_ttl_ms: 100,
        ..plumelink::qft::config()
    };
    let grant = qft_decoded(c.qft_open_with_config(1, cfg).await.unwrap().unwrap());
    let mut sender = Sender::new(grant, 0, 0, cfg).unwrap();
    let block = c.submit(2, Command::Key { key: Key::Space });
    started.recv().unwrap();
    sender.offer(Vector::new(12, 0, 0), 0).unwrap();
    let motion = c.qft_frame(1, sender.poll(0, SendBudget::AVAILABLE).unwrap().unwrap());
    std::thread::sleep(std::time::Duration::from_millis(160));
    resume.send(()).unwrap();
    block.await.unwrap().unwrap();
    let reply = qft_decoded(motion.await.unwrap().unwrap());
    assert!(matches!(
        reply.body,
        Body::Ack {
            status: Status::Expired,
            ..
        }
    ));
    assert!(!calls
        .lock()
        .unwrap()
        .iter()
        .any(|c| matches!(c, Command::Motion { .. })));
    c.shutdown();
}
#[tokio::test]
async fn qft_silent_lease_expiry_releases_only_its_owner() {
    struct Owned(Arc<Mutex<Vec<u64>>>);
    impl Native for Owned {
        fn execute(&mut self, _: &Command) -> Result<(), String> {
            Ok(())
        }
        fn release(&mut self) -> Result<(), String> {
            self.0.lock().unwrap().push(0);
            Ok(())
        }
        fn release_for(&mut self, session: u64) -> Result<(), String> {
            self.0.lock().unwrap().push(session);
            Ok(())
        }
    }
    let released = Arc::new(Mutex::new(vec![]));
    let copy = released.clone();
    let c = Controller::start(move || Ok(Box::new(Owned(copy)))).unwrap();
    c.activate(1);
    c.activate(2);
    let short_lease = qfast_transmission::Config {
        lease_ms: 500,
        motion_ttl_ms: 120,
        action_ttl_ms: 200,
        ..plumelink::qft::config()
    };
    c.qft_open_with_config(1, short_lease)
        .await
        .unwrap()
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    assert_eq!(*released.lock().unwrap(), vec![1]);
    c.shutdown();
}
#[tokio::test]
async fn qfast_expiry_does_not_execute_or_release_and_fresh_motion_survives() {
    let (c, calls, started, resume) = fixture();
    let first = c.submit(1, Command::Key { key: Key::Space });
    started.recv().unwrap();
    let stale = c.submit_with_budget(
        1,
        Command::Motion {
            dx: 100,
            dy: 0,
            scroll: 0,
        },
        None,
        Some(1),
    );
    std::thread::sleep(std::time::Duration::from_millis(10));
    let fresh = c.submit_with_budget(
        1,
        Command::Motion {
            dx: 2,
            dy: 3,
            scroll: 120,
        },
        None,
        Some(120),
    );
    resume.send(()).unwrap();
    first.await.unwrap().unwrap();
    assert!(stale.await.unwrap().unwrap_err().contains("过期"));
    let timing = fresh.await.unwrap().unwrap();
    assert!(timing["queueUs"].is_u64() && timing["executionUs"].is_u64());
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            Command::Key { key: Key::Space },
            Command::Motion {
                dx: 2,
                dy: 3,
                scroll: 120
            }
        ]
    );
    c.shutdown();
}
#[tokio::test]
async fn qfast_coalescing_preserves_click_session_and_elevation_boundaries() {
    let (c, calls, started, resume) = fixture();
    c.activate(2);
    let first = c.submit(1, Command::Key { key: Key::Space });
    started.recv().unwrap();
    let m = |dx| Command::Motion {
        dx,
        dy: 0,
        scroll: 1,
    };
    let click = Command::Click {
        button: Button::Left,
        count: 1,
    };
    let replies = vec![
        c.submit(1, m(2)),
        c.submit(1, m(3)),
        c.submit(1, click.clone()),
        c.submit(1, m(4)),
        c.submit(2, m(5)),
        c.submit(1, m(6)),
        c.submit_routed(1, m(7), Some("elevated-device".into())),
    ];
    resume.send(()).unwrap();
    first.await.unwrap().unwrap();
    for reply in replies {
        reply.await.unwrap().unwrap();
    }
    assert_eq!(
        &calls.lock().unwrap()[1..],
        &[
            Command::Motion {
                dx: 5,
                dy: 0,
                scroll: 2
            },
            click,
            m(4),
            m(5),
            m(6),
            m(7)
        ]
    );
    c.shutdown();
}
#[tokio::test]
async fn release_cancels_motion_and_commands_when_native_queue_is_full() {
    let (c, calls, started, resume) = fixture();
    let first = c.submit(1, Command::Key { key: Key::Space });
    started.recv().unwrap();
    let motion = c.submit(
        1,
        Command::Motion {
            dx: 1,
            dy: 0,
            scroll: 0,
        },
    );
    let queued: Vec<_> = (0..15)
        .map(|_| c.submit(1, Command::Key { key: Key::Left }))
        .collect();
    let release = c.submit(1, Command::Release {});
    assert!(motion.await.unwrap().is_err());
    for reply in queued {
        assert!(reply.await.unwrap().is_err());
    }
    resume.send(()).unwrap();
    first.await.unwrap().unwrap();
    release.await.unwrap().unwrap();
    assert_eq!(
        *calls.lock().unwrap(),
        vec![Command::Key { key: Key::Space }, Command::Release {}]
    );
    c.shutdown();
}
fn fixture() -> Fixture {
    let calls = Arc::new(Mutex::new(vec![]));
    let clone = calls.clone();
    let (started, started_rx) = mpsc::sync_channel(1);
    let (resume, resume_rx) = mpsc::sync_channel(1);
    let c = Controller::start(move || {
        Ok(Box::new(Blocker {
            calls: clone,
            started,
            resume: resume_rx,
            first: true,
        }))
    })
    .unwrap();
    c.activate(1);
    (c, calls, started_rx, resume)
}
#[tokio::test]
async fn merged_motion_keeps_earliest_deadline_and_expiry_blocks_the_next_click() {
    let (c, calls, started, resume) = fixture();
    let first = c.submit(1, Command::Key { key: Key::Space });
    started.recv().unwrap();
    let motion = Command::Motion {
        dx: 1,
        dy: 0,
        scroll: 0,
    };
    let a = c.submit_with_budget(1, motion.clone(), None, Some(20));
    let b = c.submit_with_budget(1, motion, None, Some(120));
    let click = Command::Click {
        button: Button::Left,
        count: 1,
    };
    let blocked = c.submit(1, click.clone());
    std::thread::sleep(std::time::Duration::from_millis(30));
    resume.send(()).unwrap();
    first.await.unwrap().unwrap();
    assert!(a.await.unwrap().unwrap_err().contains("过期"));
    assert!(b.await.unwrap().unwrap_err().contains("过期"));
    assert!(blocked
        .await
        .unwrap()
        .unwrap_err()
        .contains("位置可能未同步"));
    assert_eq!(calls.lock().unwrap().len(), 1);
    c.submit(1, click).await.unwrap().unwrap();
    assert_eq!(calls.lock().unwrap().len(), 2);
    c.shutdown();
}
#[tokio::test]
async fn disconnect_cancels_queued_clicks_without_releasing_another_session() {
    let (c, calls, started, resume) = fixture();
    let first = c.submit(1, Command::Key { key: Key::Space });
    started.recv().unwrap();
    let click = c.submit(
        1,
        Command::Click {
            button: Button::Left,
            count: 1,
        },
    );
    let shutdown = c.submit(1, Command::Shutdown { confirmed: true });
    let lens = c.submit(1, Command::Magnifier { enabled: true });
    c.release_for(1);
    assert!(click.await.unwrap().is_err());
    assert!(shutdown.await.unwrap().is_err());
    assert!(lens.await.unwrap().is_err());
    c.activate(2);
    c.release_for(1);
    let new = c.submit(2, Command::Key { key: Key::Enter });
    resume.send(()).unwrap();
    first.await.unwrap().unwrap();
    new.await.unwrap().unwrap();
    c.probe().await.unwrap();
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            Command::Key { key: Key::Space },
            Command::Release {},
            Command::Key { key: Key::Enter }
        ]
    );
    c.shutdown();
    assert!(c.submit(2, Command::Release {}).await.unwrap().is_err());
}
#[tokio::test]
async fn coalesces_bounded_moves_and_rejects_overflow() {
    let (c, calls, started, resume) = fixture();
    let first = c.submit(1, Command::Key { key: Key::Space });
    started.recv().unwrap();
    let a = c.submit(1, Command::Move { dx: 300, dy: 0 });
    let b = c.submit(1, Command::Move { dx: 300, dy: 1 });
    let mut replies = vec![];
    for _ in 0..40 {
        replies.push(c.submit(1, Command::Key { key: Key::Left }));
    }
    resume.send(()).unwrap();
    first.await.unwrap().unwrap();
    a.await.unwrap().unwrap();
    b.await.unwrap().unwrap();
    let mut rejected = 0;
    for reply in replies {
        if reply.await.unwrap().is_err() {
            rejected += 1;
        }
    }
    assert_eq!(rejected, 25);
    assert_eq!(calls.lock().unwrap()[1], Command::Move { dx: 500, dy: 1 });
    c.shutdown();
}

struct SessionNative(Arc<Mutex<Vec<&'static str>>>);
impl Native for SessionNative {
    fn execute(&mut self, _: &Command) -> Result<(), String> {
        self.0.lock().unwrap().push("command");
        Ok(())
    }
    fn release(&mut self) -> Result<(), String> {
        self.0.lock().unwrap().push("release");
        Ok(())
    }
    fn end_session(&mut self) -> Result<(), String> {
        self.0.lock().unwrap().push("end-session");
        Ok(())
    }
    fn probe(&mut self) -> Result<Value, String> {
        Ok(json!(null))
    }
}
#[tokio::test]
async fn session_end_cleans_accessibility_resources_but_gesture_release_does_not() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let copy = events.clone();
    let c = Controller::start(move || Ok(Box::new(SessionNative(copy)))).unwrap();
    c.activate(1);
    c.probe().await.unwrap();
    c.submit(1, Command::Magnifier { enabled: true })
        .await
        .unwrap()
        .unwrap();
    c.submit(1, Command::Release {}).await.unwrap().unwrap();
    assert_eq!(*events.lock().unwrap(), vec!["command", "release"]);
    c.release_for(1);
    c.probe().await.unwrap();
    assert_eq!(events.lock().unwrap().last(), Some(&"end-session"));
    c.shutdown();
    assert_eq!(
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| **e == "end-session")
            .count(),
        2
    );
}

#[tokio::test]
async fn two_sessions_can_submit_and_one_disconnect_does_not_end_the_other() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let copy = events.clone();
    let c = Controller::start(move || Ok(Box::new(SessionNative(copy)))).unwrap();
    c.activate(11);
    c.activate(12);
    c.submit(11, Command::Key { key: Key::Left })
        .await
        .unwrap()
        .unwrap();
    c.submit(12, Command::Key { key: Key::Right })
        .await
        .unwrap()
        .unwrap();
    c.release_for(11);
    c.submit(12, Command::Key { key: Key::Enter })
        .await
        .unwrap()
        .unwrap();
    c.probe().await.unwrap();
    assert_eq!(
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| **event == "command")
            .count(),
        3
    );
    assert!(!events.lock().unwrap().contains(&"end-session"));
    c.release_for(12);
    c.probe().await.unwrap();
    assert_eq!(events.lock().unwrap().last(), Some(&"end-session"));
}

#[tokio::test]
async fn window_switch_cannot_execute_after_its_timeout() {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Windows(Arc<AtomicBool>, mpsc::SyncSender<()>, mpsc::Receiver<()>);
    impl Native for Windows {
        fn execute(&mut self, _: &Command) -> Result<(), String> { Ok(()) }
        fn release(&mut self) -> Result<(), String> { Ok(()) }
        fn probe_windows(&mut self) -> Result<Value, String> {
            self.1.send(()).unwrap();
            self.2.recv().unwrap();
            Ok(json!([]))
        }
        fn switch_to_window(&mut self, _: u64) -> Result<(), String> {
            self.0.store(true, Ordering::SeqCst);
            Ok(())
        }
        fn probe(&mut self) -> Result<Value,String> { Ok(Value::Null) }
    }
    let called = Arc::new(AtomicBool::new(false));
    let (started, ready) = mpsc::sync_channel(1);
    let (resume, wait) = mpsc::channel();
    let backend = Windows(called.clone(), started, wait);
    let controller = Controller::start(move || Ok(Box::new(backend))).unwrap();
    let other = controller.clone();
    let listing = tokio::spawn(async move { other.probe_windows().await });
    tokio::task::spawn_blocking(move || ready.recv().unwrap()).await.unwrap();
    let result = controller.switch_window(42).await;
    resume.send(()).unwrap();
    controller.probe().await.unwrap();
    let _ = listing.await;
    controller.shutdown();
    assert!(result.unwrap_err().contains("超时"));
    assert!(!called.load(Ordering::SeqCst));
}
