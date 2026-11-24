pub mod clipboard;
pub mod mock;

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "macos")]
pub mod macos;

use mouseroaming_proto::{KeyCode, MouseButton, OsKind, ScreenRect};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::mpsc;

pub const MOUSEROAMING_MAGIC_EXTRA: usize = 0x4D52_4F41; // "MROA"

#[derive(Debug, Error)]
pub enum OsError {
    #[error("OS permission denied: {0}")]
    PermissionDenied(String),
    #[error("OS hook error: {0}")]
    Hook(String),
    #[error("input injection error: {0}")]
    Inject(String),
    #[error("clipboard error: {0}")]
    Clipboard(String),
    #[error("unsupported platform")]
    Unsupported,
}

#[derive(Debug, Clone)]
pub struct PermissionStatus {
    pub granted: bool,
    pub message: String,
    pub advisory: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    MouseMove {
        x: i32,
        y: i32,
        dx: i32,
        dy: i32,
    },
    MouseButton {
        button: MouseButton,
        pressed: bool,
    },
    MouseScroll {
        dx: f32,
        dy: f32,
    },
    Key {
        key: KeyCode,
        pressed: bool,
    },
    /// Fired when the hardware-level emergency escape combo (Ctrl+Alt+Esc) is pressed.
    EmergencyRelease,
}

#[derive(Debug)]
pub struct CaptureControl {
    grabbed: AtomicBool,
    anchor_x: AtomicI32,
    anchor_y: AtomicI32,
    running: AtomicBool,
}

impl CaptureControl {
    pub fn new(anchor_x: i32, anchor_y: i32) -> Self {
        Self {
            grabbed: AtomicBool::new(false),
            anchor_x: AtomicI32::new(anchor_x),
            anchor_y: AtomicI32::new(anchor_y),
            running: AtomicBool::new(true),
        }
    }

    pub fn is_grabbed(&self) -> bool {
        self.grabbed.load(Ordering::SeqCst)
    }

    pub fn set_grabbed(&self, value: bool) {
        self.grabbed.store(value, Ordering::SeqCst);
    }

    pub fn anchor(&self) -> (i32, i32) {
        (
            self.anchor_x.load(Ordering::Relaxed),
            self.anchor_y.load(Ordering::Relaxed),
        )
    }

    pub fn set_anchor(&self, x: i32, y: i32) {
        self.anchor_x.store(x, Ordering::Relaxed);
        self.anchor_y.store(y, Ordering::Relaxed);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn stop(&self) {
        self.grabbed.store(false, Ordering::SeqCst);
        self.running.store(false, Ordering::SeqCst);
    }
}

pub struct CaptureGuard {
    control: Arc<CaptureControl>,
    on_drop: Option<Box<dyn FnOnce() + Send>>,
}

impl CaptureGuard {
    pub fn new(control: Arc<CaptureControl>, on_drop: Box<dyn FnOnce() + Send>) -> Self {
        Self {
            control,
            on_drop: Some(on_drop),
        }
    }

    pub fn noop(control: Arc<CaptureControl>) -> Self {
        Self {
            control,
            on_drop: None,
        }
    }
}

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        self.control.stop();
        if let Some(f) = self.on_drop.take() {
            f();
        }
    }
}

pub trait InputInjector: Send + Sync {
    fn move_cursor_abs(&self, x: i32, y: i32) -> Result<(), OsError>;
    fn move_cursor_rel(&self, dx: i32, dy: i32) -> Result<(), OsError>;
    fn mouse_button(&self, button: MouseButton, pressed: bool) -> Result<(), OsError>;
    fn mouse_scroll(&self, dx: f32, dy: f32) -> Result<(), OsError>;
    fn key_event(&self, key: KeyCode, pressed: bool) -> Result<(), OsError>;
    fn cursor_position(&self) -> Result<(i32, i32), OsError>;
    fn virtual_screen(&self) -> Result<ScreenRect, OsError>;
}

pub trait ClipboardProvider: Send + Sync {
    fn get_text(&self) -> Result<Option<String>, OsError>;
    fn set_text(&self, text: &str) -> Result<(), OsError>;
}

pub trait OsBackend: Send + Sync {
    fn os_kind(&self) -> OsKind;
    fn check_permissions(&self) -> PermissionStatus;
    fn injector(&self) -> Arc<dyn InputInjector>;
    fn clipboard(&self) -> Arc<dyn ClipboardProvider>;
    fn set_cursor_visible(&self, _visible: bool) {}
    fn start_capture(
        &self,
        control: Arc<CaptureControl>,
    ) -> Result<(mpsc::UnboundedReceiver<InputEvent>, CaptureGuard), OsError>;
}

pub fn create_native_backend() -> Result<Arc<dyn OsBackend>, OsError> {
    #[cfg(target_os = "windows")]
    {
        Ok(Arc::new(windows::WindowsBackend::new()))
    }
    #[cfg(target_os = "macos")]
    {
        Ok(Arc::new(macos::MacOsBackend::new()))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Err(OsError::Unsupported)
    }
}
