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
    Media {
        action: Media,
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
    Launch {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request: Option<String>,
    },
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
#[repr(u16)]
pub enum Key {
    Space = 0x20,
    Left = 0x25,
    Right = 0x27,
    Up = 0x26,
    Down = 0x28,
    Enter = 0x0d,
    Escape = 0x1b,
    Backspace = 0x08,
    Tab = 0x09,
    Delete = 0x2e,
    Home = 0x24,
    End = 0x23,
    Pageup = 0x21,
    Pagedown = 0x22,
    F1 = 0x70,
    F2 = 0x71,
    F3 = 0x72,
    F4 = 0x73,
    F5 = 0x74,
    F6 = 0x75,
    F7 = 0x76,
    F8 = 0x77,
    F9 = 0x78,
    F10 = 0x79,
    F11 = 0x7a,
    F12 = 0x7b,
    A = 0x41,
    B = 0x42,
    C = 0x43,
    D = 0x44,
    E = 0x45,
    F = 0x46,
    G = 0x47,
    H = 0x48,
    I = 0x49,
    J = 0x4a,
    K = 0x4b,
    L = 0x4c,
    M = 0x4d,
    N = 0x4e,
    O = 0x4f,
    P = 0x50,
    Q = 0x51,
    R = 0x52,
    S = 0x53,
    T = 0x54,
    U = 0x55,
    V = 0x56,
    W = 0x57,
    X = 0x58,
    Y = 0x59,
    Z = 0x5a,
    #[serde(rename = "0")]
    Digit0 = 0x30,
    #[serde(rename = "1")]
    Digit1 = 0x31,
    #[serde(rename = "2")]
    Digit2 = 0x32,
    #[serde(rename = "3")]
    Digit3 = 0x33,
    #[serde(rename = "4")]
    Digit4 = 0x34,
    #[serde(rename = "5")]
    Digit5 = 0x35,
    #[serde(rename = "6")]
    Digit6 = 0x36,
    #[serde(rename = "7")]
    Digit7 = 0x37,
    #[serde(rename = "8")]
    Digit8 = 0x38,
    #[serde(rename = "9")]
    Digit9 = 0x39,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Modifier {
    Ctrl,
    Alt,
    Shift,
    Win,
}
pub type ComboKey = Key;
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Media {
    Playpause,
    Previous,
    Next,
    Stop,
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
    Standby {},
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
                (1..=4).contains(&modifiers.len())
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
            Self::Launch { id, request } => {
                id.len() == 32
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    && request.as_deref().is_none_or(crate::launcher::valid_id)
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
