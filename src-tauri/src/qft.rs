//! QFT2 adapter. Receivers and synchronous effects live on the native worker.
use crate::{control::Native, protocol::Command};
use qfast_transmission::{Action, Body, Config, Executor, Frame, Receiver, Vector};

pub const ACTION_COMMAND: u16 = 1;
#[derive(Clone)]
pub struct Failure {
    pub lease: u32,
    pub action_id: u32,
    pub stage: i32,
    pub message: &'static str,
}
impl Failure {
    pub fn matches(&self, frame: Frame) -> bool {
        frame.lease == self.lease && matches!(frame.body, Body::Ack {
            action_id, status: qfast_transmission::Status::ExecutionUncertain, ..
        } if action_id == self.action_id)
    }
}
fn failure_message(message: &str) -> Option<&'static str> {
    [
        "此应用需要管理员权限，请在电脑端启动",
        "应用文件已移动或删除，请在电脑端重新添加",
        "应用入口已移除，请刷新列表",
        "快捷入口已移除，请刷新列表",
        "文件夹已移动或删除，请在电脑端重新添加",
        "无法打开文件夹，请在电脑端检查访问权限",
        "无法启动应用，请在电脑端检查该应用是否可用",
        "请更新手机端以使用快捷启动",
        "启动请求无效",
        "启动请求过多，请稍后再试",
        "Windows 拒绝输入：请检查锁屏、UAC 或管理员窗口",
        "管理员控制未启用",
        "管理员控制未启用或已到期",
    ].into_iter().find(|known| *known == message)
}
pub fn config() -> Config {
    Config {
        lease_ms: 15_000,
        motion_ttl_ms: 400,
        action_ttl_ms: 800,
        base_interval_us: 16_000,
        max_catchup: 16_384,
        ..Config::default()
    }
}
pub fn capability(session: u64) -> serde_json::Value {
    let config = config();
    serde_json::json!({"version":2,"frameBytes":64,"session":session.to_string(),
        "leaseMs":config.lease_ms,"motionTtlMs":config.motion_ttl_ms,"actionTtlMs":config.action_ttl_ms,
        "maxCatchup":config.max_catchup,"adapter":"logical-pointer-v1"})
}
pub struct Session {
    pub device: Option<String>,
    pub receiver: Receiver,
    config: Config,
    staged: Option<(i32, Command)>,
    staged_high: i32,
    pub failure: Option<Failure>,
}
impl Session {
    pub fn new(
        session: u64,
        now: u64,
        native: &mut dyn Native,
        config: Config,
    ) -> Result<Self, String> {
        native.qft_begin(session)?;
        Ok(Self {
            device: None,
            receiver: Receiver::new(session, 1, now, config).map_err(|e| format!("{e:?}"))?,
            config,
            staged: None,
            staged_high: 0,
            failure: None,
        })
    }
    pub fn stage(&mut self, id: i32, command: Command) -> Result<(), String> {
        command.validate()?;
        if id <= self.staged_high
            || self.receiver.is_closed()
            || matches!(
                command,
                Command::Move { .. } | Command::Motion { .. } | Command::Scroll { dx: 0, .. }
            )
        {
            return Err("QFT 动作阶段无效".into());
        }
        // The sender allows only one pending action. Once a stage has been
        // consumed, its immutable identity and outcome remain in the core cache;
        // stage IDs cannot be reused with a different payload on this session.
        if self.staged.is_some() {
            return Err("QFT 动作尚未完成".into());
        }
        self.staged_high = id;
        self.staged = Some((id, command));
        Ok(())
    }
    pub fn receive(
        &mut self,
        session: u64,
        frame: Frame,
        now: u64,
        queue: u32,
        native: &mut dyn Native,
    ) -> Result<Frame, String> {
        if matches!(frame.body,Body::Action {action:Action {code,..},..}|Body::CancelAction {action:Action {code,..},..} if code!=ACTION_COMMAND)
        {
            return Err("未知 QFT 动作".into());
        }
        let mut failure = None;
        let mut executor = Adapter {
            device: self.device.as_deref(),
            native,
            session,
            staged: &mut self.staged,
            failure: &mut failure,
        };
        // queue_us is measured by our worker. Close the whole lease on a stale
        // effect so a later cumulative prefix cannot resurrect discarded input.
        // Sender capture TTL is separate: the wire does not carry peer clocks.
        let queue_limit = match frame.body {
            Body::Checkpoint { .. } => Some(u32::from(self.config.motion_ttl_ms) * 1000),
            Body::Action { .. } => Some(u32::from(self.config.action_ttl_ms) * 1000),
            _ => None,
        };
        if frame.lease == self.receiver.grant().lease
            && queue_limit.is_some_and(|limit| queue >= limit)
        {
            self.receiver
                .receive(
                    Frame {
                        body: Body::Cancel,
                        ..frame
                    },
                    now,
                    queue,
                    &mut executor,
                )
                .map_err(|e| format!("QFT queue expiry: {e:?}"))?;
        }
        let result = self.receiver
            .receive(frame, now, queue, &mut executor)
            .map_err(|e| format!("QFT: {e:?}"));
        if let (Some((stage, message)), Body::Action { id, .. }) = (failure, frame.body) {
            self.failure = Some(Failure { lease: frame.lease, action_id: id, stage, message });
        }
        result
    }
    pub fn poll(&mut self, session: u64, now: u64, native: &mut dyn Native) {
        let mut failure = None;
        let mut executor = Adapter {
            device: self.device.as_deref(),
            native,
            session,
            staged: &mut self.staged,
            failure: &mut failure,
        };
        let _ = self.receiver.poll(now, &mut executor);
    }
}
struct Adapter<'a> {
    device: Option<&'a str>,
    native: &'a mut dyn Native,
    session: u64,
    staged: &'a mut Option<(i32, Command)>,
    failure: &'a mut Option<(i32, &'static str)>,
}
impl Executor for Adapter<'_> {
    type Error = String;
    fn apply(&mut self, delta: Vector) -> Result<(), String> {
        self.native.qft_apply_for(self.session, self.device, delta)
    }
    fn action(&mut self, action: Action) -> Result<(), String> {
        let Some((id, command)) = self.staged.take() else {
            return Err("缺少动作载荷".into());
        };
        if action.code != ACTION_COMMAND || id != action.argument {
            return Err("动作载荷不匹配".into());
        }
        let result = (|| {
            if matches!(command, Command::Release {}) {
                return self.native.qft_release(self.session);
            }
            if matches!(
                command,
                Command::Move { .. }
                    | Command::Motion { .. }
                    | Command::Click { .. }
                    | Command::Buttonstate { down: true, .. }
                    | Command::Scroll { .. }
                    | Command::Zoom { .. }
            ) {
                self.native.qft_validate(self.session)?;
            }
            self.native.qft_action_for(self.session, self.device, &command)
        })();
        if let Err(message) = &result {
            if let Some(message) = failure_message(message) {
                *self.failure = Some((id, message));
            }
        }
        result
    }
    fn release(&mut self) -> Result<(), String> {
        self.staged.take();
        self.native.release_for(self.session)
    }
}

pub fn encoded(frame: Frame) -> [u8; qfast_transmission::WIRE_BYTES] {
    qfast_transmission::encode(frame).expect("canonical core response")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Button;

    #[test]
    fn displaced_pointer_blocks_press_but_never_blocks_lift() {
        #[derive(Default)]
        struct DisplacedPointer(Vec<bool>);
        impl Native for DisplacedPointer {
            fn qft_validate(&mut self, _: u64) -> Result<(), String> {
                Err("pointer moved externally".into())
            }
            fn execute(&mut self, command: &Command) -> Result<(), String> {
                if let Command::Buttonstate { down, .. } = command {
                    self.0.push(*down);
                }
                Ok(())
            }
            fn release(&mut self) -> Result<(), String> {
                Ok(())
            }
        }
        let mut native = DisplacedPointer::default();
        for down in [true, false] {
            let mut staged = Some((
                1,
                Command::Buttonstate {
                    button: Button::Left,
                    down,
                },
            ));
            let mut adapter = Adapter {
                device: None,
                native: &mut native,
                session: 1,
                staged: &mut staged,
                failure: &mut None,
            };
            let result = adapter.action(Action {
                code: ACTION_COMMAND,
                argument: 1,
            });
            assert_eq!(result.is_err(), down);
        }
        assert_eq!(native.0, [false]);
    }
}
