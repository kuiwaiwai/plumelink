//! Serialized native-input dispatch for PlumeLink sessions.
use crate::protocol::Command;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant, SystemTime},
};
use tokio::sync::oneshot;

pub type WindowReader = Arc<dyn Fn() -> Result<serde_json::Value, String> + Send + Sync>;

pub trait Native: Send + 'static {
    fn window_reader(&self) -> Option<WindowReader> { None }
    fn launch_queue(&self) -> Option<Arc<crate::launcher::jobs::LaunchQueue>> { None }
    fn qft_apply_for(
        &mut self,
        session: u64,
        _device: Option<&str>,
        delta: qfast_transmission::Vector,
    ) -> Result<(), String> {
        self.qft_apply(session, delta)
    }
    fn qft_action_for(
        &mut self,
        session: u64,
        _device: Option<&str>,
        command: &Command,
    ) -> Result<(), String> {
        self.execute_for(session, command)
    }
    fn qft_begin(&mut self, _session: u64) -> Result<(), String> {
        Ok(())
    }
    fn qft_apply(
        &mut self,
        _session: u64,
        _delta: qfast_transmission::Vector,
    ) -> Result<(), String> {
        Err("执行器不支持逻辑坐标".into())
    }
    fn qft_validate(&mut self, _session: u64) -> Result<(), String> {
        Ok(())
    }
    fn qft_release(&mut self, session: u64) -> Result<(), String> {
        self.release_for(session)
    }
    fn execute(&mut self, command: &Command) -> Result<(), String>;
    fn execute_for(&mut self, _session: u64, command: &Command) -> Result<(), String> {
        self.execute(command)
    }
    fn execute_elevated_for(
        &mut self,
        session: u64,
        _device_id: &str,
        command: &Command,
    ) -> Result<(), String> {
        self.execute_for(session, command)
    }
    fn release(&mut self) -> Result<(), String>;
    fn release_for(&mut self, _session: u64) -> Result<(), String> {
        self.release()
    }
    fn end_session(&mut self) -> Result<(), String> {
        self.release()
    }
    fn probe(&mut self) -> Result<serde_json::Value, String> {
        Err("诊断不可用".into())
    }
    fn probe_windows(&mut self) -> Result<serde_json::Value, String> {
        Err("窗口枚举不可用".into())
    }
    fn switch_to_window(&mut self, _hwnd: u64) -> Result<(), String> {
        Err("窗口切换不可用".into())
    }
    fn restore_volume(&mut self, _value: f32) -> Result<(), String> {
        Err("诊断不可用".into())
    }
}
type Reply = oneshot::Sender<Result<serde_json::Value, String>>;
pub type QftFrameBytes = [u8; qfast_transmission::WIRE_BYTES];
type QftReply = oneshot::Sender<Result<Option<QftFrameBytes>, String>>;
enum Work {
    QftOpen(u64, Option<String>, Option<qfast_transmission::Config>),
    QftStage(u64, i32, Command),
    QftFrame(u64, qfast_transmission::Frame),
    Command(u64, Command, Option<String>),
    Release(u64),
    EndSession,
    Probe,
    ProbeWindows,
    SwitchWindow {
        hwnd: u64,
        deadline: Instant,
        started: oneshot::Sender<()>,
        authorize: Box<dyn FnOnce() -> Result<(), String> + Send>,
    },
    Restore { value: f32, deadline: Instant, started: oneshot::Sender<()> },
}
enum JobReplies {
    Json(Vec<Reply>),
    Qft(QftReply),
}
impl JobReplies {
    fn send_err(self, message: &str) {
        match self {
            Self::Json(replies) => {
                for tx in replies {
                    let _ = tx.send(Err(message.into()));
                }
            }
            Self::Qft(tx) => {
                let _ = tx.send(Err(message.into()));
            }
        }
    }
    fn supersede(self) {
        match self {
            Self::Json(replies) => {
                for tx in replies {
                    let _ = tx.send(Ok(serde_json::Value::Null));
                }
            }
            Self::Qft(tx) => {
                let _ = tx.send(Ok(None));
            }
        }
    }
}
struct Job {
    work: Work,
    replies: JobReplies,
    enqueued: Instant,
    deadline: Option<Instant>,
}
impl Job {
    fn new(work: Work, replies: Vec<Reply>) -> Self {
        Self {
            work,
            replies: JobReplies::Json(replies),
            enqueued: Instant::now(),
            deadline: None,
        }
    }
    fn new_qft(work: Work, reply: QftReply) -> Self {
        Self {
            work,
            replies: JobReplies::Qft(reply),
            enqueued: Instant::now(),
            deadline: None,
        }
    }
}
struct Queue {
    jobs: VecDeque<Job>,
    sessions: HashSet<u64>,
    failures: HashMap<u64, crate::qft::Failure>,
    closing: bool,
    suspended: bool,
    reset_pending: bool,
    generation: u64,
}
struct Inner {
    queue: Mutex<Queue>,
    wake: Condvar,
    boundary: tokio::sync::watch::Sender<u64>,
}
pub(crate) struct PowerInput {
    inner: std::sync::Weak<Inner>,
    launch: Option<std::sync::Weak<crate::launcher::jobs::LaunchQueue>>,
}
impl PowerInput {
    pub(crate) fn transition(&self, suspended: bool, automatic: bool) {
        if let Some(inner) = self.inner.upgrade() {
            let launch = self.launch.as_ref().and_then(std::sync::Weak::upgrade);
            Controller::power_transition(&inner, launch.as_deref(), suspended, automatic);
        }
    }
}
struct ExecutionClock {
    monotonic: Instant,
    wall: SystemTime,
}
impl ExecutionClock {
    fn new() -> Self { Self { monotonic: Instant::now(), wall: SystemTime::now() } }
    fn interrupted(&mut self) -> bool {
        let now = Instant::now();
        let wall = SystemTime::now();
        let interrupted = now.duration_since(self.monotonic) >= Duration::from_secs(5)
            || wall.duration_since(self.wall).is_ok_and(|gap| gap >= Duration::from_secs(5));
        self.monotonic = now;
        self.wall = wall;
        interrupted
    }
}
pub struct Controller {
    inner: Arc<Inner>,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
    pub admin: Arc<crate::admin_control::AdminManager>,
    pub audio: Arc<crate::microphone::AudioHub>,
    pub launch_queue: Option<Arc<crate::launcher::jobs::LaunchQueue>>,
    windows: Option<WindowReader>,
    window_slots: Arc<tokio::sync::Semaphore>,
}
impl Controller {
    pub fn start(
        factory: impl FnOnce() -> Result<Box<dyn Native>, String> + Send + 'static,
    ) -> Result<Arc<Self>, String> {
        Self::start_with_admin(factory, Arc::new(crate::admin_control::AdminManager::new()))
    }
    pub fn start_with_admin(
        factory: impl FnOnce() -> Result<Box<dyn Native>, String> + Send + 'static,
        admin: Arc<crate::admin_control::AdminManager>,
    ) -> Result<Arc<Self>, String> {
        let inner = Arc::new(Inner {
            queue: Mutex::new(Queue {
                jobs: VecDeque::new(),
                sessions: HashSet::new(),
                failures: HashMap::new(),
                closing: false,
                suspended: false,
                reset_pending: false,
                generation: 0,
            }),
            wake: Condvar::new(),
            boundary: tokio::sync::watch::channel(0).0,
        });
        let worker_inner = inner.clone();
        let (init_tx, init_rx) = std::sync::mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let mut native = match factory() {
                Ok(n) => {
                    let _ = init_tx.send(Ok((n.window_reader(), n.launch_queue())));
                    n
                }
                Err(e) => {
                    let _ = init_tx.send(Err(e));
                    return;
                }
            };
            let mut last = Instant::now();
            let launch_queue = native.launch_queue();
            let mut clock = ExecutionClock::new();
            let mut stale_motion = HashSet::new();
            let mut pending_release = HashSet::new();
            let mut last_release_retry = Instant::now();
            let epoch = Instant::now();
            let mut qft: HashMap<u64, crate::qft::Session> = HashMap::new();
            loop {
                let mut q = worker_inner.queue.lock().unwrap();
                if clock.interrupted() && !q.reset_pending {
                    Self::invalidate_inputs(&worker_inner, &mut q, launch_queue.as_deref());
                }
                if q.reset_pending && !q.closing {
                    let generation = q.generation;
                    drop(q);
                    if let Some(launch) = &launch_queue { launch.cancel_all(); }
                    qft.clear();
                    stale_motion.clear();
                    pending_release.clear();
                    let released = native.end_session().is_ok();
                    let mut q = worker_inner.queue.lock().unwrap();
                    if released && q.generation == generation { q.reset_pending = false; }
                    if !released {
                        let (q, _) = worker_inner.wake.wait_timeout(q, Duration::from_millis(250)).unwrap();
                        drop(q);
                    }
                    continue;
                }
                drop(q);
                // Run expiry on the input thread so release completes before it is acknowledged.
                let now = epoch.elapsed().as_micros().min(u64::MAX as u128) as u64;
                for (&session, state) in &mut qft {
                    state.poll(session, now, native.as_mut());
                }
                if !pending_release.is_empty() && last_release_retry.elapsed() >= Duration::from_millis(250) {
                    pending_release.retain(|id| native.release_for(*id).is_err());
                    last_release_retry = Instant::now();
                }
                let mut q = worker_inner.queue.lock().unwrap();
                if q.closing {
                    drop(q);
                    let _ = native.end_session();
                    break;
                }
                if q.reset_pending { continue; }
                if let Some(job) = q.jobs.pop_front() {
                    let generation = q.generation;
                    drop(q);
                    let started = Instant::now();
                    // Discarding unexecuted motion must never release another session's input.
                    if job.deadline.is_some_and(|deadline| started >= deadline) {
                        if let Work::Command(session, _, _) = &job.work {
                            stale_motion.insert(*session);
                        }
                        job.replies.send_err("运动已过期，已丢弃");
                        continue;
                    }
                    if matches!(job.work, Work::Probe | Work::ProbeWindows)
                        && matches!(&job.replies, JobReplies::Json(replies) if replies.iter().all(|reply| reply.is_closed()))
                    {
                        continue;
                    }
                    // A click must not silently target the old position after discarded motion.
                    if let Work::Command(
                        session,
                        Command::Click { .. } | Command::Buttonstate { down: true, .. },
                        _,
                    ) = &job.work
                    {
                        if stale_motion.remove(session) {
                            job.replies
                                .send_err("移动已过期，位置可能未同步，请确认位置后重试");
                            continue;
                        }
                    }
                    if !Self::current_work(&worker_inner, &mut clock, generation, launch_queue.as_deref()) {
                        job.replies.send_err("连接变化，操作已取消");
                        continue;
                    }
                    enum WorkOutcome {
                        Json(Result<serde_json::Value, String>),
                        Qft(Result<Option<QftFrameBytes>, String>),
                    }
                    let outcome = match job.work {
                        Work::QftOpen(session, device, cfg) => WorkOutcome::Qft(
                            if let std::collections::hash_map::Entry::Vacant(entry) =
                                qft.entry(session)
                            {
                                crate::qft::Session::new(
                                    session,
                                    now,
                                    native.as_mut(),
                                    cfg.unwrap_or_else(crate::qft::config),
                                )
                                .map(|mut state| {
                                    state.device = device;
                                    let grant = Some(crate::qft::encoded(state.receiver.grant()));
                                    entry.insert(state);
                                    grant
                                })
                            } else {
                                Err("QFT 已经初始化".into())
                            },
                        ),
                        Work::QftStage(session, id, command) => WorkOutcome::Json(
                            qft.get_mut(&session)
                                .ok_or_else(|| "QFT 未初始化".to_string())
                                .and_then(|state| state.stage(id, command))
                                .map(|_| serde_json::Value::Null),
                        ),
                        Work::QftFrame(session, frame) => {
                            last = Instant::now();
                            WorkOutcome::Qft(
                                qft.get_mut(&session)
                                    .ok_or_else(|| "QFT 未初始化".to_string())
                                    .and_then(|state| {
                                        let result = state.receive(
                                            session,
                                            frame,
                                            epoch.elapsed().as_micros().min(u64::MAX as u128)
                                                as u64,
                                            started
                                                .duration_since(job.enqueued)
                                                .as_micros()
                                                .min(u32::MAX as u128)
                                                as u32,
                                            native.as_mut(),
                                        );
                                        if let Some(failure) = state.failure.as_ref().filter(|failure|
                                            result.as_ref().is_ok_and(|reply| failure.matches(*reply))) {
                                            let mut queue = worker_inner.queue.lock().unwrap();
                                            if queue.sessions.contains(&session) {
                                                queue.failures.insert(session, failure.clone());
                                            }
                                        }
                                        result
                                    })
                                    .map(|frame| Some(crate::qft::encoded(frame))),
                            )
                        }
                        Work::Command(session, command, elevated_device) => {
                            last = Instant::now();
                            match &command {
                                Command::ListWindows {} => {
                                    let result = native.probe_windows();
                                    if result.is_err() { let _ = native.release_for(session); }
                                    WorkOutcome::Json(result.map(|data| serde_json::json!({
                                        "queueUs": started.duration_since(job.enqueued).as_micros().min(u32::MAX as u128) as u32,
                                        "executionUs": started.elapsed().as_micros().min(u32::MAX as u128) as u32,
                                        "windows": data,
                                    })))
                                }
                                Command::SwitchToWindow { hwnd } => {
                                    let result = native.switch_to_window(*hwnd);
                                    if result.is_err() { let _ = native.release_for(session); }
                                    WorkOutcome::Json(result.map(|_| serde_json::json!({
                                        "queueUs": started.duration_since(job.enqueued).as_micros().min(u32::MAX as u128) as u32,
                                        "executionUs": started.elapsed().as_micros().min(u32::MAX as u128) as u32,
                                    })))
                                }
                                _ => {
                                    let result = if let Some(device_id) = elevated_device {
                                        native.execute_elevated_for(session, &device_id, &command)
                                    } else {
                                        native.execute_for(session, &command)
                                    };
                                    if result.is_err() && matches!(command, Command::Motion { .. }) {
                                        stale_motion.insert(session);
                                    }
                                    if result.is_err() {
                                        let _ = native.release_for(session);
                                    }
                                    WorkOutcome::Json(result.map(|_| {
                                        serde_json::json!({
                                            "queueUs": started.duration_since(job.enqueued).as_micros().min(u32::MAX as u128) as u32,
                                            "executionUs": started.elapsed().as_micros().min(u32::MAX as u128) as u32,
                                        })
                                    }))
                                }
                            }
                        }
                        Work::EndSession => {
                            qft.clear();
                            stale_motion.clear();
                            WorkOutcome::Json(native.end_session().map(|_| serde_json::Value::Null))
                        }
                        Work::Release(session) => {
                            qft.remove(&session);
                            stale_motion.remove(&session);
                            let result = native.release_for(session);
                            if result.is_err() {
                                pending_release.insert(session);
                            } else {
                                pending_release.remove(&session);
                            }
                            WorkOutcome::Json(result.map(|_| serde_json::Value::Null))
                        }
                        Work::Probe => WorkOutcome::Json(native.probe()),
                        Work::ProbeWindows => WorkOutcome::Json(native.probe_windows()),
                        Work::SwitchWindow { hwnd, deadline, started, authorize } => {
                            let result = if Instant::now() >= deadline || started.is_closed() {
                                Err("窗口切换超时".into())
                            } else {
                                authorize().and_then(|()| {
                                    if !Self::current_work(&worker_inner, &mut clock, generation, launch_queue.as_deref()) {
                                        Err("连接变化，操作已取消".into())
                                    } else if Instant::now() >= deadline || started.send(()).is_err() {
                                        Err("窗口切换超时".into())
                                    } else {
                                        native.switch_to_window(hwnd).map(|_| serde_json::Value::Null)
                                    }
                                })
                            };
                            WorkOutcome::Json(result)
                        }
                        Work::Restore { value, deadline, started } => {
                            let result = if Instant::now() >= deadline || started.send(()).is_err() {
                                Err("音量恢复超时".into())
                            } else {
                                native.restore_volume(value).map(|_| serde_json::Value::Null)
                            };
                            WorkOutcome::Json(result)
                        },
                    };
                    // Each QFT execution failure closes/releases its own lease.
                    // A malformed request must not release another device's input.
                    match (job.replies, outcome) {
                        (JobReplies::Json(replies), WorkOutcome::Json(result)) => {
                            for reply in replies {
                                let _ = reply.send(result.clone());
                            }
                        }
                        (JobReplies::Qft(reply), WorkOutcome::Qft(result)) => {
                            let _ = reply.send(result);
                        }
                        _ => unreachable!("work and reply variant must match"),
                    }
                } else {
                    let wait = if qft.is_empty() {
                        Duration::from_millis(250)
                    } else {
                        let now_us = epoch.elapsed().as_micros().min(u64::MAX as u128) as u64;
                        let next_us = qft
                            .values()
                            .filter(|s| !s.receiver.is_closed())
                            .map(|s| s.receiver.expires_at().saturating_sub(now_us))
                            .min()
                            .unwrap_or(250_000);
                        Duration::from_micros(next_us.clamp(10_000, 250_000))
                    };
                    let (q, _) = worker_inner.wake.wait_timeout(q, wait).unwrap();
                    drop(q);
                    if qft.is_empty() && last.elapsed() > Duration::from_secs(3) {
                        let _ = native.release();
                    }
                }
            }
        });
        let (windows, launch_queue) = init_rx.recv().map_err(|_| "Windows 控制线程无法启动")??;
        Ok(Arc::new(Self {
            inner,
            worker: Mutex::new(Some(worker)),
            admin,
            audio: Arc::new(crate::microphone::AudioHub::default()),
            launch_queue,
            windows,
            window_slots: Arc::new(tokio::sync::Semaphore::new(1)),
        }))
    }
    fn clear(q: &mut Queue) {
        for job in q.jobs.drain(..) {
            job.replies.send_err("连接变化，操作已取消");
        }
    }
    fn clear_session(q: &mut Queue, session: u64) {
        Self::clear_session_work(q, session, false);
    }
    fn clear_session_work(q: &mut Queue, session: u64, qft_cancel: bool) {
        let mut keep = VecDeque::new();
        for job in q.jobs.drain(..) {
            let belongs = matches!(job.work, Work::Command(id, _, _) | Work::Release(id) | Work::QftOpen(id,_,_) | Work::QftStage(id,_,_) | Work::QftFrame(id,_) if id == session);
            if belongs {
                // A priority QFT cancel supersedes queued work. Its terminal
                // response, not an artificial transport error, settles the action.
                let superseded =
                    qft_cancel && matches!(job.work, Work::QftStage(..) | Work::QftFrame(..));
                if superseded {
                    job.replies.supersede();
                } else {
                    job.replies.send_err("连接变化，操作已取消");
                }
            } else {
                keep.push_back(job);
            }
        }
        q.jobs = keep;
    }
    fn invalidate_inputs(inner: &Inner, q: &mut Queue, launch: Option<&crate::launcher::jobs::LaunchQueue>) {
        Self::clear(q);
        q.sessions.clear();
        q.failures.clear();
        q.reset_pending = true;
        q.generation = q.generation.wrapping_add(1);
        if let Some(launch) = launch { launch.cancel_all(); }
        inner.boundary.send_replace(q.generation);
        inner.wake.notify_one();
    }
    fn current_work(inner: &Inner, clock: &mut ExecutionClock, generation: u64, launch: Option<&crate::launcher::jobs::LaunchQueue>) -> bool {
        let mut q = inner.queue.lock().unwrap();
        if clock.interrupted() && !q.reset_pending {
            Self::invalidate_inputs(inner, &mut q, launch);
        }
        !q.closing && !q.suspended && !q.reset_pending && q.generation == generation
    }
    fn power_transition(inner: &Inner, launch: Option<&crate::launcher::jobs::LaunchQueue>, suspended: bool, automatic: bool) {
        let mut q = inner.queue.lock().unwrap();
        if q.closing || (suspended && q.suspended) || (!suspended && !automatic && !q.suspended) { return; }
        q.suspended = suspended;
        Self::invalidate_inputs(inner, &mut q, launch);
    }
    pub(crate) fn power_input(&self) -> Arc<PowerInput> {
        Arc::new(PowerInput { inner: Arc::downgrade(&self.inner), launch: self.launch_queue.as_ref().map(Arc::downgrade) })
    }
    pub fn suspend_inputs(&self) {
        Self::power_transition(&self.inner, self.launch_queue.as_deref(), true, false);
    }
    pub fn resume_inputs(&self, automatic: bool) {
        Self::power_transition(&self.inner, self.launch_queue.as_deref(), false, automatic);
    }
    pub fn input_boundary(&self) -> tokio::sync::watch::Receiver<u64> { self.inner.boundary.subscribe() }
    pub fn input_available(&self) -> bool {
        let q = self.inner.queue.lock().unwrap();
        !q.closing && !q.suspended && !q.reset_pending
    }
    pub fn activate(&self, session: u64) -> bool {
        if session == 0 {
            self.revoke();
            return false;
        }
        let mut q = self.inner.queue.lock().unwrap();
        if q.closing || q.suspended || q.reset_pending { return false; }
        q.sessions.insert(session);
        true
    }
    pub fn release_for(&self, session: u64) {
        if let Some(queue) = &self.launch_queue { queue.cancel_session(session); }
        let mut q = self.inner.queue.lock().unwrap();
        q.failures.remove(&session);
        if !q.sessions.remove(&session) {
            return;
        }
        Self::clear_session(&mut q, session);
        if q.sessions.is_empty() {
            Self::clear(&mut q);
            q.jobs.push_front(Job::new(Work::EndSession, vec![]));
        } else {
            q.jobs.push_front(Job::new(Work::Release(session), vec![]));
        }
        self.inner.wake.notify_one();
    }
    pub fn revoke(&self) {
        if let Some(queue) = &self.launch_queue { queue.cancel_all(); }
        let mut q = self.inner.queue.lock().unwrap();
        q.failures.clear();
        Self::clear(&mut q);
        q.sessions.clear();
        q.jobs.push_front(Job::new(Work::EndSession, vec![]));
        self.inner.wake.notify_one();
    }
    pub fn qft_failure(&self, session: u64, bytes: &QftFrameBytes) -> Option<crate::qft::Failure> {
        let frame = qfast_transmission::decode(bytes).ok()?;
        if frame.session != session || !matches!(frame.body, qfast_transmission::Body::Ack {
            action_id: 1.., status: qfast_transmission::Status::ExecutionUncertain, ..
        }) { return None; }
        let queue = self.inner.queue.lock().unwrap();
        queue.failures.get(&session).filter(|failure| failure.matches(frame)).cloned()
    }
    pub fn submit(
        &self,
        session: u64,
        command: Command,
    ) -> oneshot::Receiver<Result<serde_json::Value, String>> {
        self.submit_routed(session, command, None)
    }
    fn submit_qft_frame_work(
        &self,
        session: u64,
        work: Work,
        priority: bool,
    ) -> oneshot::Receiver<Result<Option<QftFrameBytes>, String>> {
        let (tx, rx) = oneshot::channel();
        let mut q = self.inner.queue.lock().unwrap();
        if q.closing || !q.sessions.contains(&session) {
            let _ = tx.send(Err("连接已失效".into()));
            return rx;
        }
        if !priority {
            let replace_motion = match (q.jobs.back(), &work) {
                (Some(Job { work: Work::QftFrame(owner, previous), .. }), Work::QftFrame(_, next))
                    if *owner == session && previous.session == next.session
                        && previous.lease == next.lease && previous.serial < next.serial =>
                {
                    matches!((previous.body, next.body),
                        (qfast_transmission::Body::Checkpoint { version: old, .. },
                         qfast_transmission::Body::Checkpoint { version: new, .. }) if old < new)
                }
                _ => false,
            };
            if replace_motion {
                let previous = q.jobs.back_mut().unwrap();
                previous.work = work;
                // Keep the oldest enqueue time; new motion cannot freshen an expired prefix.
                std::mem::replace(&mut previous.replies, JobReplies::Qft(tx)).supersede();
                return rx;
            }
        }
        if priority {
            Self::clear_session_work(&mut q, session, true);
        } else if q.jobs.len() >= 16 {
            let _ = tx.send(Err("控制队列已满".into()));
            return rx;
        }
        let job = Job::new_qft(work, tx);
        if priority {
            q.jobs.push_front(job);
        } else {
            q.jobs.push_back(job);
        }
        self.inner.wake.notify_one();
        rx
    }
    pub fn qft_open(
        &self,
        session: u64,
    ) -> oneshot::Receiver<Result<Option<QftFrameBytes>, String>> {
        self.submit_qft_frame_work(session, Work::QftOpen(session, None, None), false)
    }
    pub fn qft_open_with_config(
        &self,
        session: u64,
        config: qfast_transmission::Config,
    ) -> oneshot::Receiver<Result<Option<QftFrameBytes>, String>> {
        self.submit_qft_frame_work(session, Work::QftOpen(session, None, Some(config)), false)
    }
    pub fn qft_open_for(
        &self,
        session: u64,
        device: String,
    ) -> oneshot::Receiver<Result<Option<QftFrameBytes>, String>> {
        self.submit_qft_frame_work(session, Work::QftOpen(session, Some(device), None), false)
    }
    pub fn qft_stage(
        &self,
        session: u64,
        id: i32,
        command: Command,
    ) -> oneshot::Receiver<Result<serde_json::Value, String>> {
        let (tx, rx) = oneshot::channel();
        let mut q = self.inner.queue.lock().unwrap();
        if q.closing || !q.sessions.contains(&session) {
            let _ = tx.send(Err("连接已失效".into()));
            return rx;
        }
        if q.jobs.len() >= 16 {
            let _ = tx.send(Err("控制队列已满".into()));
            return rx;
        }
        q.jobs
            .push_back(Job::new(Work::QftStage(session, id, command), vec![tx]));
        self.inner.wake.notify_one();
        rx
    }
    pub fn qft_frame(
        &self,
        session: u64,
        frame: qfast_transmission::Frame,
    ) -> oneshot::Receiver<Result<Option<QftFrameBytes>, String>> {
        let priority = matches!(
            frame.body,
            qfast_transmission::Body::Cancel | qfast_transmission::Body::CancelAction { .. }
        );
        self.submit_qft_frame_work(session, Work::QftFrame(session, frame), priority)
    }
    pub fn submit_routed(
        &self,
        session: u64,
        command: Command,
        elevated_device: Option<String>,
    ) -> oneshot::Receiver<Result<serde_json::Value, String>> {
        self.submit_with_budget(session, command, elevated_device, None)
    }
    pub fn submit_with_budget(
        &self,
        session: u64,
        command: Command,
        elevated_device: Option<String>,
        budget_ms: Option<u16>,
    ) -> oneshot::Receiver<Result<serde_json::Value, String>> {
        let (tx, rx) = oneshot::channel();
        let enqueued = Instant::now();
        let deadline = if matches!(command, Command::Motion { .. }) {
            Some(enqueued + Duration::from_millis(budget_ms.unwrap_or(120).clamp(1, 120).into()))
        } else {
            None
        };
        let mut q = self.inner.queue.lock().unwrap();
        if q.closing || session == 0 || !q.sessions.contains(&session) {
            let _ = tx.send(Err("连接已失效".into()));
            return rx;
        }
        if let Err(error) = command.validate() {
            let _ = tx.send(Err(error));
            return rx;
        }
        if matches!(command, Command::Release {}) {
            Self::clear_session(&mut q, session);
            q.jobs
                .push_front(Job::new(Work::Release(session), vec![tx]));
            self.inner.wake.notify_one();
            return rx;
        }
        if let Command::Move { dx, dy } = command {
            if let Some(Job {
                work: Work::Command(owner, Command::Move { dx: x, dy: y }, queued_device),
                replies: JobReplies::Json(replies),
                ..
            }) = q.jobs.back_mut()
            {
                if *owner == session && *queued_device == elevated_device && replies.len() < 32 {
                    *x = (*x + dx).clamp(-500, 500);
                    *y = (*y + dy).clamp(-500, 500);
                    replies.push(tx);
                    return rx;
                }
            }
        }
        if let Command::Motion { dx, dy, scroll } = command {
            if let Some(Job {
                work:
                    Work::Command(
                        owner,
                        Command::Motion {
                            dx: x,
                            dy: y,
                            scroll: s,
                        },
                        queued_device,
                    ),
                replies: JobReplies::Json(replies),
                deadline: queued_deadline,
                ..
            }) = q.jobs.back_mut()
            {
                if *owner == session
                    && *queued_device == elevated_device
                    && replies.len() < 32
                    && queued_deadline.is_some_and(|until| enqueued < until)
                {
                    *x = (*x + dx).clamp(-500, 500);
                    *y = (*y + dy).clamp(-500, 500);
                    *s = (*s + scroll).clamp(-600, 600);
                    *queued_deadline = (*queued_deadline).min(deadline);
                    replies.push(tx);
                    return rx;
                }
            }
        }
        if q.jobs.len() >= 16 {
            let _ = tx.send(Err("控制繁忙，指令已丢弃".into()));
            return rx;
        }
        q.jobs.push_back(Job {
            work: Work::Command(session, command, elevated_device),
            replies: JobReplies::Json(vec![tx]),
            enqueued,
            deadline,
        });
        self.inner.wake.notify_one();
        rx
    }
    fn queue_diagnostic(&self, work: Work) -> Result<oneshot::Receiver<Result<serde_json::Value, String>>, String> {
        let (tx, rx) = oneshot::channel();
        {
            let mut q = self.inner.queue.lock().unwrap();
            if q.closing || q.suspended || q.reset_pending || q.jobs.len() >= 16 {
                return Err("控制不可用".into());
            }
            q.jobs.push_back(Job::new(work, vec![tx]));
            self.inner.wake.notify_one();
        }
        Ok(rx)
    }
    async fn diagnostic(&self, work: Work) -> Result<serde_json::Value, String> {
        let rx = self.queue_diagnostic(work)?;
        tokio::time::timeout(Duration::from_secs(4), rx)
            .await
            .map_err(|_| "诊断超时")?
            .map_err(|_| "控制线程已退出")?
    }
    pub async fn probe(&self) -> Result<serde_json::Value, String> {
        self.diagnostic(Work::Probe).await
    }
    pub async fn restore_volume(&self, value: f32) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(4);
        let (started, mut start) = oneshot::channel();
        let result = self.queue_diagnostic(Work::Restore { value, deadline, started })?;
        if tokio::time::timeout_at(deadline.into(), &mut start).await.is_err() {
            start.close();
            if start.try_recv().is_err() { return Err("音量恢复超时".into()); }
        }
        result.await.map_err(|_| "控制线程已退出")?.map(|_| ())
    }
    pub async fn probe_windows(&self) -> Result<serde_json::Value, String> {
        if self.inner.queue.lock().unwrap().closing { return Err("控制不可用".into()); }
        if let Some(read) = &self.windows {
            let permit = self.window_slots.clone().try_acquire_owned()
                .map_err(|_| "正在读取窗口列表，请稍后刷新")?;
            let read = read.clone();
            let task = tokio::task::spawn_blocking(move || { let _permit = permit; read() });
            let result = tokio::time::timeout(Duration::from_secs(4), task).await
                .map_err(|_| "窗口列表超时")?.map_err(|_| "无法读取窗口列表")?;
            if self.inner.queue.lock().unwrap().closing { return Err("控制不可用".into()); }
            return result;
        }
        self.diagnostic(Work::ProbeWindows)
            .await
            .map_err(|error| if error == "诊断超时" { "窗口列表超时".to_string() } else { error })
    }
    pub async fn switch_window(
        &self, hwnd: u64, authorize: impl FnOnce() -> Result<(), String> + Send + 'static,
    ) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(4);
        let (started, mut start) = oneshot::channel();
        let result = self.queue_diagnostic(Work::SwitchWindow { hwnd, deadline, started, authorize: Box::new(authorize) })?;
        if tokio::time::timeout_at(deadline.into(), &mut start).await.is_err() {
            start.close();
            if start.try_recv().is_err() {
                return Err("窗口切换超时".into());
            }
        }
        // Once execution starts, report its actual result instead of a misleading timeout.
        result.await.map_err(|_| "控制线程已退出")?.map(|_| ())
    }
    pub fn shutdown(&self) {
        if let Some(queue) = &self.launch_queue { queue.close(); }
        self.audio.stop();
        {
            let mut q = self.inner.queue.lock().unwrap();
            Self::clear(&mut q);
            q.closing = true;
            self.inner.wake.notify_one();
        }
        if let Some(worker) = self.worker.lock().unwrap().take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod clock_tests {
    use super::*;
    #[test]
    fn execution_gap_uses_both_clocks_and_resets_its_baseline() {
        let mut clock = ExecutionClock::new();
        assert!(!clock.interrupted());
        clock.monotonic -= Duration::from_secs(6);
        assert!(clock.interrupted());
        assert!(!clock.interrupted());
        clock.wall -= Duration::from_secs(6);
        assert!(clock.interrupted());
        clock.wall += Duration::from_secs(60);
        assert!(!clock.interrupted());
    }
}
