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
#[tokio::test]
async fn qft_failure_details_are_session_scoped_bounded_and_do_not_change_outcomes() {
    use qfast_transmission::*;
    struct Fails(Arc<Mutex<usize>>, &'static str);
    impl Native for Fails {
        fn execute(&mut self, _: &Command) -> Result<(), String> {
            *self.0.lock().unwrap() += 1;
            Err(self.1.into())
        }
        fn release(&mut self) -> Result<(), String> { Ok(()) }
    }
    for message in ["此应用需要管理员权限，请在电脑端启动", "private application path"] {
        let calls = Arc::new(Mutex::new(0));
        let effects = calls.clone();
        let controller = Controller::start(move || Ok(Box::new(Fails(effects, message)))).unwrap();
        controller.activate(7);
        controller.activate(8);
        let grant = qft_decoded(controller.qft_open(7).await.unwrap().unwrap());
        controller.qft_stage(7, 19, Command::Launch { id: "a".repeat(32), request: None }).await.unwrap().unwrap();
        let action = Frame { body: Body::Action { id: 1, version: 0, total: Vector::ZERO,
            action: Action { code: 1, argument: 19 } }, serial: 1, ..grant };
        let bytes = controller.qft_frame(7, action).await.unwrap().unwrap().unwrap();
        assert!(matches!(decode(&bytes).unwrap().body, Body::Ack { status: Status::ExecutionUncertain, .. }));
        let detail = controller.qft_failure(7, &bytes);
        assert_eq!(detail.is_some(), message != "private application path");
        if let Some(detail) = detail {
            assert_eq!((detail.lease, detail.action_id, detail.stage, detail.message), (1, 1, 19, message));
        }
        assert!(controller.qft_failure(8, &bytes).is_none());
        let mut unrelated = decode(&bytes).unwrap();
        unrelated.lease += 1;
        assert!(controller.qft_failure(7, &encode(unrelated).unwrap()).is_none());
        unrelated = decode(&bytes).unwrap();
        if let Body::Ack { ref mut action_id, .. } = unrelated.body { *action_id += 1; }
        assert!(controller.qft_failure(7, &encode(unrelated).unwrap()).is_none());
        let replay = controller.qft_frame(7, Frame { serial: 2, ..action }).await.unwrap().unwrap().unwrap();
        assert!(matches!(decode(&replay).unwrap().body, Body::Ack { status: Status::ExecutionUncertain, .. }));
        assert_eq!(*calls.lock().unwrap(), 1);
        controller.release_for(7);
        assert!(controller.qft_failure(7, &bytes).is_none());
        controller.shutdown();
    }
}
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
async fn queued_cumulative_motion_respects_actions_and_other_devices() {
    use qfast_transmission::*;
    let (c, calls, started, resume) = fixture();
    c.activate(2);
    c.activate(3);
    let first = qft_decoded(c.qft_open(1).await.unwrap().unwrap());
    let other = qft_decoded(c.qft_open(3).await.unwrap().unwrap());
    let block = c.submit(2, Command::Key { key: Key::Space });
    started.recv().unwrap();
    let checkpoint = |grant: Frame, version, x| Frame {
        serial: version, body: Body::Checkpoint { version, total: Vector::new(x, 0, 0) }, ..grant
    };
    let obsolete = c.qft_frame(1, checkpoint(first, 1, 10));
    let before_click = c.qft_frame(1, checkpoint(first, 2, 20));
    let stage = c.qft_stage(1, 1, Command::Click { button: Button::Left, count: 1 });
    let click = c.qft_frame(1, Frame { serial: 3, body: Body::Action {
        id: 1, version: 2, total: Vector::new(20, 0, 0), action: Action { code: 1, argument: 1 }
    }, ..first });
    let after_click = c.qft_frame(1, Frame { serial: 4, ..checkpoint(first, 3, 30) });
    let intervening = c.qft_frame(3, checkpoint(other, 1, 5));
    let after_other = c.qft_frame(1, Frame { serial: 5, ..checkpoint(first, 4, 40) });
    assert!(obsolete.await.unwrap().unwrap().is_none());
    resume.send(()).unwrap();
    block.await.unwrap().unwrap();
    stage.await.unwrap().unwrap();
    for reply in [before_click, click, after_click, intervening, after_other] {
        assert!(reply.await.unwrap().unwrap().is_some());
    }
    assert_eq!(*calls.lock().unwrap(), vec![
        Command::Key { key: Key::Space },
        Command::Motion { dx: 20, dy: 0, scroll: 0 },
        Command::Click { button: Button::Left, count: 1 },
        Command::Motion { dx: 10, dy: 0, scroll: 0 },
        Command::Motion { dx: 5, dy: 0, scroll: 0 },
        Command::Motion { dx: 10, dy: 0, scroll: 0 },
    ]);
    c.shutdown();
}
#[tokio::test]
async fn newer_motion_cannot_revive_an_expired_queued_prefix_or_click() {
    use qfast_transmission::*;
    let (c, calls, started, resume) = fixture();
    c.activate(2);
    let cfg = Config { motion_ttl_ms: 100, ..plumelink::qft::config() };
    let grant = qft_decoded(c.qft_open_with_config(1, cfg).await.unwrap().unwrap());
    let block = c.submit(2, Command::Key { key: Key::Space });
    started.recv().unwrap();
    let obsolete = c.qft_frame(1, Frame { serial: 1, body: Body::Checkpoint {
        version: 1, total: Vector::new(10, 0, 0)
    }, ..grant });
    std::thread::sleep(std::time::Duration::from_millis(160));
    let latest = c.qft_frame(1, Frame { serial: 2, body: Body::Checkpoint {
        version: 2, total: Vector::new(20, 0, 0)
    }, ..grant });
    let stage = c.qft_stage(1, 1, Command::Click { button: Button::Left, count: 1 });
    let click = c.qft_frame(1, Frame { serial: 3, body: Body::Action {
        id: 1, version: 2, total: Vector::new(20, 0, 0), action: Action { code: 1, argument: 1 }
    }, ..grant });
    assert!(obsolete.await.unwrap().unwrap().is_none());
    resume.send(()).unwrap();
    block.await.unwrap().unwrap();
    assert!(matches!(qft_decoded(latest.await.unwrap().unwrap()).body,
        Body::Ack { status: Status::Expired, .. }));
    assert!(stage.await.unwrap().is_err());
    assert!(matches!(qft_decoded(click.await.unwrap().unwrap()).body,
        Body::Ack { status: Status::Expired, .. }));
    assert!(calls.lock().unwrap().iter().all(|command|
        matches!(command, Command::Key { key: Key::Space } | Command::Release {})));
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
async fn authorization_wait_cannot_run_a_window_switch_after_timeout_or_cancellation() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Windows(Arc<AtomicUsize>);
    impl Native for Windows {
        fn execute(&mut self, _: &Command) -> Result<(), String> { Ok(()) }
        fn release(&mut self) -> Result<(), String> { Ok(()) }
        fn switch_to_window(&mut self, _: u64) -> Result<(), String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn probe(&mut self) -> Result<Value, String> { Ok(Value::Null) }
    }
    for cancel in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let backend = Windows(calls.clone());
        let controller = Controller::start(move || Ok(Box::new(backend))).unwrap();
        let other = controller.clone();
        let (started, ready) = mpsc::sync_channel(1);
        let (resume, wait) = mpsc::channel();
        let operation = tokio::spawn(async move {
            other.switch_window(42, move || {
                started.send(()).unwrap();
                wait.recv_timeout(std::time::Duration::from_secs(8)).unwrap();
                Ok(())
            }).await
        });
        tokio::task::spawn_blocking(move || ready.recv().unwrap()).await.unwrap();
        if cancel {
            operation.abort();
            assert!(operation.await.unwrap_err().is_cancelled());
        } else {
            assert!(operation.await.unwrap().unwrap_err().contains("超时"));
        }
        resume.send(()).unwrap();
        controller.probe().await.unwrap();
        controller.shutdown();
        assert_eq!(calls.load(Ordering::SeqCst), 0, "authorization completing after timeout/cancel must not switch a window");
    }
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
    let result = controller.switch_window(42, || Ok(())).await;
    resume.send(()).unwrap();
    controller.probe().await.unwrap();
    let _ = listing.await;
    controller.shutdown();
    assert!(result.unwrap_err().contains("超时"));
    assert!(!called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn queued_volume_restore_and_window_query_are_abandoned_after_timeout() {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Input {
        first: bool,
        started: mpsc::SyncSender<()>,
        resume: mpsc::Receiver<()>,
        restored: Arc<AtomicBool>,
        listed: Arc<AtomicBool>,
    }
    impl Native for Input {
        fn execute(&mut self, _: &Command) -> Result<(), String> {
            if self.first {
                self.first = false;
                let _ = self.started.send(());
                self.resume.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
            }
            Ok(())
        }
        fn release(&mut self) -> Result<(), String> { Ok(()) }
        fn probe(&mut self) -> Result<Value, String> { Ok(Value::Null) }
        fn probe_windows(&mut self) -> Result<Value, String> {
            self.listed.store(true, Ordering::SeqCst);
            Ok(json!([]))
        }
        fn restore_volume(&mut self, _: f32) -> Result<(), String> {
            self.restored.store(true, Ordering::SeqCst);
            Ok(())
        }
    }
    let restored = Arc::new(AtomicBool::new(false));
    let listed = Arc::new(AtomicBool::new(false));
    let (started, ready) = mpsc::sync_channel(1);
    let (resume, wait) = mpsc::channel();
    let input = Input { first: true, started, resume: wait, restored: restored.clone(), listed: listed.clone() };
    let c = Controller::start(move || Ok(Box::new(input))).unwrap();
    c.activate(1);
    let held = c.submit(1, Command::Key { key: Key::Enter });
    tokio::task::spawn_blocking(move || ready.recv().unwrap()).await.unwrap();
    let restoring = c.restore_volume(0.42);
    let listing = c.probe_windows();
    let (restore, list) = tokio::join!(restoring, listing);
    assert!(restore.unwrap_err().contains("超时"));
    assert!(list.unwrap_err().contains("超时"));
    resume.send(()).unwrap();
    held.await.unwrap().unwrap();
    c.probe().await.unwrap();
    c.shutdown();
    assert!(!restored.load(Ordering::SeqCst), "expired restore must never change volume later");
    assert!(!listed.load(Ordering::SeqCst), "abandoned query must not delay later input");
}

#[tokio::test]
async fn started_volume_restore_reports_the_actual_result() {
    struct Restore(mpsc::SyncSender<()>, mpsc::Receiver<()>);
    impl Native for Restore {
        fn execute(&mut self, _: &Command) -> Result<(), String> { Ok(()) }
        fn release(&mut self) -> Result<(), String> { Ok(()) }
        fn restore_volume(&mut self, _: f32) -> Result<(), String> {
            let _ = self.0.send(());
            self.1.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
            Ok(())
        }
    }
    let (started, ready) = mpsc::sync_channel(1);
    let (resume, wait) = mpsc::channel();
    let c = Controller::start(move || Ok(Box::new(Restore(started, wait)))).unwrap();
    let other = c.clone();
    let result = tokio::spawn(async move { other.restore_volume(0.42).await });
    tokio::task::spawn_blocking(move || ready.recv().unwrap()).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(4100)).await;
    assert!(!result.is_finished(), "an operation already executing must not report a false timeout");
    resume.send(()).unwrap();
    result.await.unwrap().unwrap();
    c.shutdown();
}

#[tokio::test]
async fn background_window_query_does_not_delay_two_device_input_or_drag_release() {
    use plumelink::control::WindowReader;
    use qfast_transmission::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Input(Arc<Mutex<Vec<(u64, Command)>>>, WindowReader);
    impl Native for Input {
        fn window_reader(&self) -> Option<WindowReader> { Some(self.1.clone()) }
        fn execute(&mut self, command: &Command) -> Result<(), String> { self.execute_for(0, command) }
        fn execute_for(&mut self, session: u64, command: &Command) -> Result<(), String> {
            self.0.lock().unwrap().push((session, command.clone()));
            Ok(())
        }
        fn qft_apply(&mut self, session: u64, v: Vector) -> Result<(), String> {
            self.execute_for(session, &Command::Motion { dx: v.x, dy: v.y, scroll: v.z })
        }
        fn release(&mut self) -> Result<(), String> { Ok(()) }
        fn probe_windows(&mut self) -> Result<Value, String> { panic!("read-only query ran on input thread") }
    }
    let calls = Arc::new(Mutex::new(vec![]));
    let recorded = calls.clone();
    let (started, ready) = mpsc::sync_channel(1);
    let (resume, wait) = mpsc::channel();
    let wait = Mutex::new(wait);
    let attempts = AtomicUsize::new(0);
    let read: WindowReader = Arc::new(move || {
        if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
            let _ = started.send(());
            wait.lock().unwrap().recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        }
        Ok(json!([]))
    });
    let c = Controller::start(move || Ok(Box::new(Input(recorded, read)))).unwrap();
    c.activate(1); c.activate(2);
    let grant = qft_decoded(c.qft_open(1).await.unwrap().unwrap());
    let mut first = Sender::new(grant, 0, 0, plumelink::qft::config()).unwrap();
    let grant = qft_decoded(c.qft_open(2).await.unwrap().unwrap());
    let mut second = Sender::new(grant, 0, 0, plumelink::qft::config()).unwrap();
    c.qft_stage(1, 1, Command::Buttonstate { button: Button::Left, down: true }).await.unwrap().unwrap();
    first.queue_action(Action { code: 1, argument: 1 }, 0).unwrap();
    let frame = first.poll(0, SendBudget::AVAILABLE).unwrap().unwrap();
    first.receive(qft_decoded(c.qft_frame(1, frame).await.unwrap().unwrap()), 0).unwrap();
    assert_eq!(first.take_outcome(), Some(Outcome::Committed(1)));
    let other = c.clone();
    let listing = tokio::spawn(async move { other.probe_windows().await });
    tokio::task::spawn_blocking(move || ready.recv().unwrap()).await.unwrap();
    assert!(c.probe_windows().await.unwrap_err().contains("正在读取"));
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        first.offer(Vector::new(9, 0, 0), 20_000).unwrap();
        first.queue_action(Action { code: 1, argument: 2 }, 20_000).unwrap();
        c.qft_stage(1, 2, Command::Buttonstate { button: Button::Left, down: false }).await.unwrap().unwrap();
        let frame = first.poll(20_000, SendBudget::AVAILABLE).unwrap().unwrap();
        first.receive(qft_decoded(c.qft_frame(1, frame).await.unwrap().unwrap()), 20_000).unwrap();
        assert_eq!(first.take_outcome(), Some(Outcome::Committed(2)));
        second.queue_action(Action { code: 1, argument: 1 }, 20_000).unwrap();
        c.qft_stage(2, 1, Command::Key { key: Key::Enter }).await.unwrap().unwrap();
        let frame = second.poll(20_000, SendBudget::AVAILABLE).unwrap().unwrap();
        second.receive(qft_decoded(c.qft_frame(2, frame).await.unwrap().unwrap()), 20_000).unwrap();
        assert_eq!(second.take_outcome(), Some(Outcome::Committed(1)));
    }).await.expect("window query blocked remote input or drag release");
    let effects = calls.lock().unwrap().clone();
    assert!(effects.contains(&(1, Command::Motion { dx: 9, dy: 0, scroll: 0 })));
    assert!(effects.contains(&(1, Command::Buttonstate { button: Button::Left, down: false })));
    assert!(effects.contains(&(2, Command::Key { key: Key::Enter })));
    assert!(listing.await.unwrap().unwrap_err().contains("超时"));
    assert!(c.probe_windows().await.unwrap_err().contains("正在读取"), "timed-out reader must retain its slot");
    resume.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            match c.probe_windows().await {
                Ok(value) => { assert_eq!(value, json!([])); break; }
                Err(error) if error.contains("正在读取") => tokio::task::yield_now().await,
                Err(error) => panic!("{error}"),
            }
        }
    }).await.unwrap();
    c.shutdown();
}
