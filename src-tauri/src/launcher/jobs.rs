use super::{launch_entry_before, Entry};
use crate::preferences::SharedStore;
use serde::Serialize;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

const LIMIT: usize = 64;
const RETAIN: Duration = Duration::from_secs(120);
const DEADLINE: Duration = Duration::from_secs(4);
const CANCELED: &str = "启动请求已取消，未执行";
const TIMEOUT: &str = "准备启动超时，未执行，请检查电脑后重试";
const UNCERTAIN: &str = "启动结果未确认，请检查电脑；不会自动重发";

enum Phase {
    Preparing,
    Starting,
    Complete(Result<(), String>),
}
struct Job {
    request: String,
    target: String,
    session: u64,
    device: Option<String>,
    created: Instant,
    deadline: Instant,
    phase: Mutex<Phase>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub state: &'static str,
    pub message: String,
}
impl Job {
    fn report(&self) -> Report {
        let mut phase = self.phase.lock().unwrap();
        if matches!(*phase, Phase::Preparing) && Instant::now() >= self.deadline {
            *phase = Phase::Complete(Err(TIMEOUT.into()));
        }
        match &*phase {
            Phase::Preparing => Report {
                state: "preparing",
                message: String::new(),
            },
            Phase::Starting if Instant::now() >= self.deadline => Report {
                state: "uncertain",
                message: UNCERTAIN.into(),
            },
            Phase::Starting => Report {
                state: "starting",
                message: String::new(),
            },
            Phase::Complete(Ok(())) => Report {
                state: "succeeded",
                message: String::new(),
            },
            Phase::Complete(Err(message)) => Report {
                state: "failed",
                message: message.clone(),
            },
        }
    }
    fn enter(&self) -> Result<(), String> {
        let mut phase = self.phase.lock().unwrap();
        if !matches!(*phase, Phase::Preparing) {
            return Err(CANCELED.into());
        }
        if Instant::now() >= self.deadline {
            *phase = Phase::Complete(Err(TIMEOUT.into()));
            return Err(TIMEOUT.into());
        }
        *phase = Phase::Starting;
        Ok(())
    }
    fn finish(&self, result: Result<(), String>) {
        let mut phase = self.phase.lock().unwrap();
        if !matches!(*phase, Phase::Complete(_)) {
            *phase = Phase::Complete(result);
        }
    }
    fn cancel(&self) {
        let mut phase = self.phase.lock().unwrap();
        if matches!(*phase, Phase::Preparing) {
            *phase = Phase::Complete(Err(CANCELED.into()));
        }
    }
}
type Runner =
    Arc<dyn Fn(Entry, &dyn Fn() -> Result<(), String>) -> Result<(), String> + Send + Sync>;
pub struct LaunchQueue {
    store: SharedStore,
    jobs: Mutex<VecDeque<Arc<Job>>>,
    slots: Arc<Semaphore>,
    runner: Runner,
    deadline: Duration,
    closed: AtomicBool,
}
impl LaunchQueue {
    pub fn new(store: SharedStore) -> Arc<Self> {
        Self::with_runner(
            store,
            Arc::new(|entry, before| launch_entry_before(&entry, before)),
            DEADLINE,
        )
    }
    fn with_runner(store: SharedStore, runner: Runner, deadline: Duration) -> Arc<Self> {
        Arc::new(Self {
            store,
            jobs: Mutex::new(VecDeque::new()),
            slots: Arc::new(Semaphore::new(1)),
            runner,
            deadline,
            closed: AtomicBool::new(false),
        })
    }
    pub fn request(
        &self,
        session: u64,
        device: Option<&str>,
        target: &str,
        request: Option<&str>,
    ) -> Result<(), String> {
        let request = request.ok_or("请更新手机端以使用快捷启动")?;
        if !super::valid_id(request) || !super::valid_id(target) {
            return Err("启动请求无效".into());
        }
        let mut jobs = self.jobs.lock().unwrap();
        if self.closed.load(Ordering::Acquire) {
            return Err(CANCELED.into());
        }
        jobs.retain(|job| {
            job.created.elapsed() < RETAIN
                || !matches!(*job.phase.lock().unwrap(), Phase::Complete(_))
        });
        if let Some(job) = jobs.iter().find(|job| job.request == request) {
            return if job.device.as_deref() == device && job.target == target {
                Ok(())
            } else {
                Err("启动请求无效".into())
            };
        }
        if jobs.len() >= LIMIT {
            return Err("启动请求过多，请稍后再试".into());
        }
        let now = Instant::now();
        let job = Arc::new(Job {
            request: request.into(),
            target: target.into(),
            session,
            device: device.map(str::to_owned),
            created: now,
            deadline: now + self.deadline,
            phase: Mutex::new(Phase::Preparing),
        });
        jobs.push_back(job.clone());
        let Ok(permit) = self.slots.clone().try_acquire_owned() else {
            job.finish(Err("电脑正在处理启动请求，请稍后再试".into()));
            return Ok(());
        };
        let store = self.store.clone();
        let runner = self.runner.clone();
        let task = job.clone();
        if std::thread::Builder::new()
            .name("PlumeLink-launch".into())
            .spawn(move || {
                let _permit = permit;
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let entry = store
                        .lock()
                        .unwrap()
                        .get()
                        .applications
                        .into_iter()
                        .find(|entry| entry.id == task.target)
                        .ok_or("快捷入口已移除，请刷新列表")?;
                    runner(entry.clone(), &|| {
                        let guard = store.lock().unwrap();
                        let preferences = guard.get();
                        if !preferences.applications.contains(&entry) {
                            return Err("快捷入口列表已更新，请刷新".into());
                        }
                        if let Some(device) = task.device.as_deref() {
                            if !preferences
                                .devices
                                .iter()
                                .any(|entry| entry.id == device && entry.permissions.input)
                                || !guard.primary_allows(device)
                            {
                                return Err("此设备没有遥控权限".into());
                            }
                        }
                        task.enter()
                    })
                }))
                .unwrap_or_else(|_| Err("无法处理启动请求，请重试".into()));
                task.finish(result);
            })
            .is_err()
        {
            job.finish(Err("无法处理启动请求，请重试".into()));
        }
        Ok(())
    }
    pub fn status(&self, device: &str, request: &str) -> Option<Report> {
        self.jobs
            .lock()
            .unwrap()
            .iter()
            .find(|job| job.request == request && job.device.as_deref() == Some(device))
            .map(|job| job.report())
    }
    pub fn cancel_session(&self, session: u64) {
        for job in self
            .jobs
            .lock()
            .unwrap()
            .iter()
            .filter(|job| job.session == session)
        {
            job.cancel();
        }
    }
    pub fn cancel_all(&self) {
        for job in self.jobs.lock().unwrap().iter() {
            job.cancel();
        }
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.cancel_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        control::{Controller, Native},
        protocol::{Button, Command, Key},
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };

    fn fixture(runner: Runner, deadline: Duration) -> (SharedStore, Arc<LaunchQueue>, String) {
        let store = crate::preferences::Store::memory();
        store
            .lock()
            .unwrap()
            .update(|p| {
                p.applications.push(Entry {
                    id: "a".repeat(32),
                    name: "Owned test".into(),
                    path: "not-executed.exe".into(),
                    kind: super::super::Kind::Application,
                })
            })
            .unwrap();
        let mut pairing = crate::security::Pairing::with_store(store.clone()).unwrap();
        let device = pairing
            .pair_device(&pairing.code.clone(), None, "Test device")
            .unwrap()
            .unwrap()
            .device_id;
        let queue = LaunchQueue::with_runner(store.clone(), runner, deadline);
        (store, queue, device)
    }
    fn report(queue: &LaunchQueue, request: &str) -> Report {
        queue
            .jobs
            .lock()
            .unwrap()
            .iter()
            .find(|job| job.request == request)
            .unwrap()
            .report()
    }
    async fn idle(queue: &LaunchQueue) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while queue.slots.available_permits() == 0 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn prepared_launch_is_single_execution_bound_to_its_device_and_request() {
        let effects = Arc::new(AtomicUsize::new(0));
        let writes = effects.clone();
        let (_, queue, device) = fixture(
            Arc::new(move |_, before| {
                before()?;
                writes.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            DEADLINE,
        );
        let request = "b".repeat(32);
        queue
            .request(7, Some(&device), &"a".repeat(32), Some(&request))
            .unwrap();
        idle(&queue).await;
        queue
            .request(8, Some(&device), &"a".repeat(32), Some(&request))
            .unwrap();
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        assert_eq!(queue.status(&device, &request).unwrap().state, "succeeded");
        assert!(queue.status("other-device", &request).is_none());
        assert!(queue
            .request(8, Some("other-device"), &"a".repeat(32), Some(&request))
            .is_err());
        assert!(queue
            .request(8, Some(&device), &"c".repeat(32), Some(&request))
            .is_err());
    }
    #[tokio::test]
    async fn expired_preparation_never_launches_late_and_retains_the_worker_slot() {
        let (started, waiting) = mpsc::sync_channel(1);
        let (resume, held) = mpsc::channel();
        let held = Mutex::new(held);
        let effects = Arc::new(AtomicUsize::new(0));
        let writes = effects.clone();
        let (_, queue, device) = fixture(
            Arc::new(move |_, before| {
                started.send(()).unwrap();
                held.lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap();
                before()?;
                writes.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            Duration::from_millis(25),
        );
        queue
            .request(7, Some(&device), &"a".repeat(32), Some(&"b".repeat(32)))
            .unwrap();
        waiting.recv_timeout(Duration::from_secs(1)).unwrap();
        tokio::time::sleep(Duration::from_millis(35)).await;
        assert_eq!(report(&queue, &"b".repeat(32)).message, TIMEOUT);
        queue
            .request(8, Some(&device), &"a".repeat(32), Some(&"c".repeat(32)))
            .unwrap();
        assert!(report(&queue, &"c".repeat(32)).message.contains("正在处理"));
        resume.send(()).unwrap();
        idle(&queue).await;
        assert_eq!(effects.load(Ordering::SeqCst), 0);
        assert_eq!(report(&queue, &"b".repeat(32)).message, TIMEOUT);
    }
    #[tokio::test]
    async fn a_started_native_call_is_uncertain_on_timeout_and_never_repeated() {
        let (started, waiting) = mpsc::sync_channel(1);
        let (resume, held) = mpsc::channel();
        let held = Mutex::new(held);
        let effects = Arc::new(AtomicUsize::new(0));
        let writes = effects.clone();
        let (_, queue, device) = fixture(
            Arc::new(move |_, before| {
                before()?;
                started.send(()).unwrap();
                held.lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap();
                writes.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            Duration::from_millis(25),
        );
        let request = "b".repeat(32);
        queue
            .request(7, Some(&device), &"a".repeat(32), Some(&request))
            .unwrap();
        waiting.recv_timeout(Duration::from_secs(1)).unwrap();
        tokio::time::sleep(Duration::from_millis(35)).await;
        queue.cancel_session(7);
        assert_eq!(report(&queue, &request).state, "uncertain");
        queue
            .request(8, Some(&device), &"a".repeat(32), Some(&request))
            .unwrap();
        assert_eq!(queue.slots.available_permits(), 0);
        resume.send(()).unwrap();
        idle(&queue).await;
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        assert_eq!(report(&queue, &request).state, "succeeded");
    }
    #[tokio::test]
    async fn permission_or_configuration_changes_before_start_prevent_the_effect() {
        for remove in [false, true] {
            let (started, waiting) = mpsc::sync_channel(1);
            let (resume, held) = mpsc::channel();
            let held = Mutex::new(held);
            let effects = Arc::new(AtomicUsize::new(0));
            let writes = effects.clone();
            let (store, queue, device) = fixture(
                Arc::new(move |_, before| {
                    started.send(()).unwrap();
                    held.lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(2))
                        .unwrap();
                    before()?;
                    writes.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }),
                DEADLINE,
            );
            queue
                .request(7, Some(&device), &"a".repeat(32), Some(&"b".repeat(32)))
                .unwrap();
            waiting.recv_timeout(Duration::from_secs(1)).unwrap();
            store
                .lock()
                .unwrap()
                .update(|p| {
                    if remove {
                        p.applications.clear();
                    } else {
                        p.devices
                            .iter_mut()
                            .find(|entry| entry.id == device)
                            .unwrap()
                            .permissions
                            .input = false;
                    }
                })
                .unwrap();
            resume.send(()).unwrap();
            idle(&queue).await;
            assert_eq!(effects.load(Ordering::SeqCst), 0);
            assert_eq!(report(&queue, &"b".repeat(32)).state, "failed");
        }
    }
    #[tokio::test]
    async fn slow_launch_does_not_delay_drag_release_other_devices_or_shutdown() {
        struct Input {
            queue: Arc<LaunchQueue>,
            calls: Arc<Mutex<Vec<(u64, Command)>>>,
        }
        impl Native for Input {
            fn launch_queue(&self) -> Option<Arc<LaunchQueue>> {
                Some(self.queue.clone())
            }
            fn execute(&mut self, command: &Command) -> Result<(), String> {
                self.execute_for(0, command)
            }
            fn execute_for(&mut self, session: u64, command: &Command) -> Result<(), String> {
                self.calls.lock().unwrap().push((session, command.clone()));
                if let Command::Launch { id, request } = command {
                    self.queue.request(session, None, id, request.as_deref())?;
                }
                Ok(())
            }
            fn release(&mut self) -> Result<(), String> {
                Ok(())
            }
        }
        let (started, waiting) = mpsc::sync_channel(1);
        let (resume, held) = mpsc::channel();
        let held = Mutex::new(held);
        let effects = Arc::new(AtomicUsize::new(0));
        let writes = effects.clone();
        let (_, queue, _) = fixture(
            Arc::new(move |_, before| {
                started.send(()).unwrap();
                held.lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap();
                before()?;
                writes.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            DEADLINE,
        );
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let worker = queue.clone();
        let controller = Controller::start(move || {
            Ok(Box::new(Input {
                queue: worker,
                calls: recorded,
            }))
        })
        .unwrap();
        controller.activate(1);
        controller.activate(2);
        controller
            .submit(
                1,
                Command::Buttonstate {
                    button: Button::Left,
                    down: true,
                },
            )
            .await
            .unwrap()
            .unwrap();
        controller
            .submit(
                1,
                Command::Launch {
                    id: "a".repeat(32),
                    request: Some("b".repeat(32)),
                },
            )
            .await
            .unwrap()
            .unwrap();
        waiting.recv_timeout(Duration::from_secs(1)).unwrap();
        tokio::time::timeout(Duration::from_millis(200), async {
            controller
                .submit(
                    1,
                    Command::Buttonstate {
                        button: Button::Left,
                        down: false,
                    },
                )
                .await
                .unwrap()
                .unwrap();
            controller
                .submit(2, Command::Key { key: Key::Enter })
                .await
                .unwrap()
                .unwrap();
        })
        .await
        .expect("slow launch blocked drag release or another device");
        controller.release_for(1);
        let at = Instant::now();
        controller.shutdown();
        assert!(at.elapsed() < Duration::from_millis(200));
        resume.send(()).unwrap();
        idle(&queue).await;
        assert_eq!(effects.load(Ordering::SeqCst), 0);
        let calls = calls.lock().unwrap();
        assert!(calls.contains(&(
            1,
            Command::Buttonstate {
                button: Button::Left,
                down: false
            }
        )));
        assert!(calls.contains(&(2, Command::Key { key: Key::Enter })));
    }
    #[tokio::test]
    async fn job_history_and_workers_are_bounded_and_invalid_requests_do_not_enqueue() {
        let (started, waiting) = mpsc::sync_channel(1);
        let (resume, held) = mpsc::channel();
        let held = Mutex::new(held);
        let (_, queue, device) = fixture(
            Arc::new(move |_, before| {
                started.send(()).unwrap();
                held.lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap();
                before()
            }),
            DEADLINE,
        );
        assert!(queue
            .request(1, Some(&device), &"a".repeat(32), None)
            .is_err());
        assert!(queue
            .request(1, Some(&device), &"a".repeat(32), Some("invalid"))
            .is_err());
        for index in 0..LIMIT {
            queue
                .request(
                    1,
                    Some(&device),
                    &"a".repeat(32),
                    Some(&format!("{index:032x}")),
                )
                .unwrap();
            if index == 0 {
                waiting.recv_timeout(Duration::from_secs(1)).unwrap();
            }
        }
        assert!(queue
            .request(1, Some(&device), &"a".repeat(32), Some(&"f".repeat(32)))
            .is_err());
        assert_eq!(queue.jobs.lock().unwrap().len(), LIMIT);
        assert_eq!(queue.slots.available_permits(), 0);
        queue.close();
        resume.send(()).unwrap();
        idle(&queue).await;
        assert_eq!(report(&queue, &format!("{:032x}", 0)).message, CANCELED);
    }
}
