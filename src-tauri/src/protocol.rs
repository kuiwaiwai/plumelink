use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Command {
    Move {
        dx: i32,
        dy: i32,
    },
    Motion {
        dx: i32,
        dy: i32,
        scroll: i32,
    },
    Click {
        button: Button,
        count: u8,
    },
    Buttonstate {
        button: Button,
        down: bool,
    },
    Scroll {
        #[serde(default)]
        dx: i32,
        dy: i32,
    },
    Key {
        key: Key,
    },
    Combo {
        modifiers: Vec<Modifier>,
        key: ComboKey,
    },
    Volume {
        action: Volume,
    },
    Switch {
        action: Switch,
    },
    Desktop {},
    Taskview {},
    Appswitch {
        direction: Direction,
    },
    Virtualdesktop {
        direction: Direction,
    },
    Notifications {},
    Zoom {
        direction: ZoomDirection,
    },
    Text {
        text: String,
    },
    Magnifier {
        enabled: bool,
    },
    Shutdown {
        confirmed: bool,
    },
    Release {},
    ListWindows {},
    SwitchToWindow {
        hwnd: u64,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowEntry {
    pub hwnd: u64,
    pub title: String,
    pub app: String,
    pub icon: Option<String>,
    pub minimized: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Button {
    Left,
    Right,
    Middle,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Key {
    Space,
    Left,
    Right,
    Up,
    Down,
    Enter,
    Escape,
    Backspace,
    Tab,
    Delete,
    Home,
    End,
    Pageup,
    Pagedown,
    F5,
    F11,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Modifier {
    Ctrl,
    Alt,
    Shift,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComboKey {
    Space,
    Left,
    Right,
    Up,
    Down,
    Enter,
    Escape,
    Backspace,
    Tab,
    Home,
    End,
    Pageup,
    Pagedown,
    F5,
    F11,
    A,
    C,
    F,
    V,
    W,
    X,
    Y,
    Z,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Volume {
    Up,
    Down,
    Mute,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Switch {
    Next,
    Previous,
    Confirm,
    Cancel,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Left,
    Right,
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ZoomDirection {
    In,
    Out,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum ClientMessage {
    Auth {
        token: String,
    },
    Ping {},
    #[serde(rename = "qftOpen")]
    QftOpen {},
    #[serde(rename = "qftStage")]
    QftStage {
        id: i32,
        command: Command,
    },
}
impl Command {
    pub fn validate(&self) -> Result<(), String> {
        let valid = match self {
            Self::Move { dx, dy } => dx.abs_diff(0) <= 500 && dy.abs_diff(0) <= 500,
            Self::Motion { dx, dy, scroll } => {
                dx.abs_diff(0) <= 500 && dy.abs_diff(0) <= 500 && scroll.abs_diff(0) <= 600
            }
            Self::Scroll { dx, dy } => dx.abs_diff(0) <= 600 && dy.abs_diff(0) <= 600,
            Self::Combo { modifiers, .. } => {
                (1..=3).contains(&modifiers.len())
                    && (0..modifiers.len()).all(|i| !modifiers[i + 1..].contains(&modifiers[i]))
            }
            Self::Click { count, .. } => *count == 1 || *count == 2,
            Self::Text { text } => {
                !text.is_empty()
                    && text.encode_utf16().count() <= 1000
                    && !text
                        .chars()
                        .any(|c| (c < ' ' && !['\r', '\n', '\t'].contains(&c)) || c == '\u{7f}')
            }
            Self::Shutdown { confirmed } => *confirmed,
            _ => true,
        };
        if valid {
            Ok(())
        } else {
            Err("指令参数超出允许范围".into())
        }
    }
}
impl ClientMessage {
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > 8192 {
            return Err("消息过大".into());
        }
        let message: Self = serde_json::from_str(text).map_err(|_| "消息结构无效")?;
        match &message {
            Self::QftStage { id, command } => {
                if *id <= 0 {
                    return Err("动作序号无效".into());
                }
                command.validate()?;
            }
            Self::Auth { token } if !valid_token_shape(token) => return Err("令牌格式无效".into()),
            _ => {}
        }
        Ok(message)
    }
}
pub fn valid_token_shape(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reject_unsafe_commands() {
        for value in [
            serde_json::json!({"type":"shell","command":"calc"}),
            serde_json::json!({"type":"release","extra":true}),
            serde_json::json!({"type":"key","key":"ctrl"}),
        ] {
            assert!(serde_json::from_value::<Command>(value).is_err());
        }
        assert!(Command::Move {
            dx: i32::MIN,
            dy: 0
        }
        .validate()
        .is_err());
        assert!(Command::Click {
            button: Button::Left,
            count: 3
        }
        .validate()
        .is_err());
        assert!(Command::Text {
            text: "😀".repeat(501)
        }
        .validate()
        .is_err());
        assert!(Command::Text {
            text: "羽连 中文\nEnglish 😀".into()
        }
        .validate()
        .is_ok());
    }
}
