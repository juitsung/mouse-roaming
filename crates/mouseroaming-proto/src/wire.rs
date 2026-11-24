use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROTOCOL_VERSION: u16 = 1;
pub const DEFAULT_PORT: u16 = 24850;
pub const MAX_FRAME_SIZE: usize = 4 * 1024 * 1024;
pub const MAX_CLIPBOARD_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum ProtoError {
    #[error("frame size {0} exceeds maximum allowed {1}")]
    FrameTooLarge(usize, usize),
    #[error("incomplete frame: expected {expected} bytes, got {actual}")]
    IncompleteFrame { expected: usize, actual: usize },
    #[error("serialization error: {0}")]
    Bincode(#[from] bincode::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OsKind {
    Windows,
    MacOS,
    Linux,
    Unknown,
}

impl OsKind {
    pub fn current() -> Self {
        #[cfg(target_os = "windows")]
        {
            Self::Windows
        }
        #[cfg(target_os = "macos")]
        {
            Self::MacOS
        }
        #[cfg(target_os = "linux")]
        {
            Self::Linux
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        {
            Self::Unknown
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::MacOS => "macOS",
            Self::Linux => "Linux",
            Self::Unknown => "Unknown",
        }
    }

    pub fn from_tag(tag: &str) -> Self {
        match tag.to_ascii_lowercase().as_str() {
            "windows" | "win" => Self::Windows,
            "macos" | "darwin" | "mac" => Self::MacOS,
            "linux" => Self::Linux,
            _ => Self::Unknown,
        }
    }

    pub fn as_tag(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::MacOS => "macos",
            Self::Linux => "linux",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    pub fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
        }
    }

    pub fn all() -> [Self; 4] {
        [Self::Left, Self::Right, Self::Top, Self::Bottom]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Top => "Top",
            Self::Bottom => "Bottom",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl ScreenRect {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width: width.max(1),
            height: height.max(1),
        }
    }

    pub fn right(&self) -> i32 {
        self.x + self.width as i32 - 1
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height as i32 - 1
    }

    pub fn center(&self) -> (i32, i32) {
        (
            self.x + (self.width as i32) / 2,
            self.y + (self.height as i32) / 2,
        )
    }

    pub fn entry_point(&self, edge: Edge, ratio: f32) -> (i32, i32) {
        let r = ratio.clamp(0.0, 1.0);
        let inset = 2i32;
        match edge {
            Edge::Left => (
                self.x + inset,
                self.y + (r * (self.height.saturating_sub(1)) as f32).round() as i32,
            ),
            Edge::Right => (
                self.right() - inset,
                self.y + (r * (self.height.saturating_sub(1)) as f32).round() as i32,
            ),
            Edge::Top => (
                self.x + (r * (self.width.saturating_sub(1)) as f32).round() as i32,
                self.y + inset,
            ),
            Edge::Bottom => (
                self.x + (r * (self.width.saturating_sub(1)) as f32).round() as i32,
                self.bottom() - inset,
            ),
        }
    }

    pub fn edge_ratio(&self, edge: Edge, x: i32, y: i32) -> f32 {
        match edge {
            Edge::Left | Edge::Right => {
                let denom = self.height.saturating_sub(1).max(1) as f32;
                ((y - self.y) as f32 / denom).clamp(0.0, 1.0)
            }
            Edge::Top | Edge::Bottom => {
                let denom = self.width.saturating_sub(1).max(1) as f32;
                ((x - self.x) as f32 / denom).clamp(0.0, 1.0)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Message {
    Hello {
        protocol_version: u16,
        peer_id: String,
        hostname: String,
        os: OsKind,
        screen: ScreenRect,
        pairing_token: Option<String>,
    },
    Welcome {
        protocol_version: u16,
        peer_id: String,
        hostname: String,
        os: OsKind,
        screen: ScreenRect,
        accepted: bool,
        reason: Option<String>,
    },
    Ping {
        seq: u64,
    },
    Pong {
        seq: u64,
    },
    /// Server -> Client: cursor enters the client's screen on `edge` at normalized `ratio` [0.0, 1.0].
    Enter {
        edge: Edge,
        ratio: f32,
    },
    /// Client -> Server (or Server -> Client): control leaves the remote screen via `edge` at `ratio`.
    Leave {
        edge: Edge,
        ratio: f32,
    },
    /// Force release all currently depressed keys and mouse buttons on the receiving side.
    ReleaseAll,
    /// Relative mouse movement delta in logical pixels.
    MouseMoveRel {
        dx: i32,
        dy: i32,
    },
    /// Absolute mouse movement (normalized 0.0..=1.0).
    MouseMoveAbs {
        x_ratio: f32,
        y_ratio: f32,
    },
    MouseButton {
        button: MouseButton,
        pressed: bool,
    },
    /// Mouse scroll delta in standard wheel ticks/lines (positive dy = scroll up).
    MouseScroll {
        dx: f32,
        dy: f32,
    },
    KeyEvent {
        key: crate::KeyCode,
        pressed: bool,
    },
    ClipboardText {
        seq: u64,
        text: String,
    },
    ScreenChanged {
        screen: ScreenRect,
    },
}

pub fn encode_frame(msg: &Message) -> Result<Vec<u8>, ProtoError> {
    let payload = bincode::serialize(msg)?;
    if payload.len() > MAX_FRAME_SIZE {
        return Err(ProtoError::FrameTooLarge(payload.len(), MAX_FRAME_SIZE));
    }
    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(&payload);
    Ok(buf)
}

pub fn decode_frame(frame_bytes: &[u8]) -> Result<Message, ProtoError> {
    if frame_bytes.len() < 4 {
        return Err(ProtoError::IncompleteFrame {
            expected: 4,
            actual: frame_bytes.len(),
        });
    }
    let len = u32::from_be_bytes([
        frame_bytes[0],
        frame_bytes[1],
        frame_bytes[2],
        frame_bytes[3],
    ]) as usize;
    if len > MAX_FRAME_SIZE {
        return Err(ProtoError::FrameTooLarge(len, MAX_FRAME_SIZE));
    }
    if frame_bytes.len() < 4 + len {
        return Err(ProtoError::IncompleteFrame {
            expected: 4 + len,
            actual: frame_bytes.len(),
        });
    }
    let msg = bincode::deserialize(&frame_bytes[4..4 + len])?;
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KeyCode;

    #[test]
    fn round_trip_all_messages() {
        let samples = vec![
            Message::Hello {
                protocol_version: PROTOCOL_VERSION,
                peer_id: "peer-1".into(),
                hostname: "win-desktop".into(),
                os: OsKind::Windows,
                screen: ScreenRect::new(0, 0, 2560, 1440),
                pairing_token: Some("123456".into()),
            },
            Message::Welcome {
                protocol_version: PROTOCOL_VERSION,
                peer_id: "peer-2".into(),
                hostname: "macbook-pro".into(),
                os: OsKind::MacOS,
                screen: ScreenRect::new(0, 0, 1728, 1117),
                accepted: true,
                reason: None,
            },
            Message::Enter {
                edge: Edge::Left,
                ratio: 0.42,
            },
            Message::Leave {
                edge: Edge::Right,
                ratio: 0.85,
            },
            Message::MouseMoveRel { dx: -12, dy: 34 },
            Message::MouseButton {
                button: MouseButton::Right,
                pressed: true,
            },
            Message::MouseScroll { dx: 0.0, dy: -3.0 },
            Message::KeyEvent {
                key: KeyCode::LeftCtrl,
                pressed: true,
            },
            Message::ClipboardText {
                seq: 7,
                text: "Hello 跨平台剪貼簿!".into(),
            },
            Message::ReleaseAll,
        ];

        for msg in samples {
            let encoded = encode_frame(&msg).expect("encode");
            let decoded = decode_frame(&encoded).expect("decode");
            assert_eq!(msg, decoded);
        }
    }

    #[test]
    fn screen_rect_ratio_and_entry() {
        let rect = ScreenRect::new(0, 0, 1920, 1080);
        let ratio = rect.edge_ratio(Edge::Right, 1919, 540);
        assert!((ratio - 0.5).abs() < 0.01);

        let mac_rect = ScreenRect::new(0, 0, 1440, 900);
        let (x, y) = mac_rect.entry_point(Edge::Left, ratio);
        assert_eq!(x, 2);
        assert!((y - 450).abs() <= 2);
    }
}
