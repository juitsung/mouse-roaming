use crate::{
    CaptureControl, CaptureGuard, ClipboardProvider, InputEvent, InputInjector, OsBackend, OsError,
    PermissionStatus,
};
use mouseroaming_proto::{KeyCode, MouseButton, OsKind, ScreenRect};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq)]
pub enum InjectedAction {
    MoveAbs(i32, i32),
    MoveRel(i32, i32),
    MouseButton { button: MouseButton, pressed: bool },
    MouseScroll { dx: f32, dy: f32 },
    Key { key: KeyCode, pressed: bool },
}

pub struct MockClipboard {
    text: Mutex<Option<String>>,
}

impl MockClipboard {
    pub fn new() -> Self {
        Self {
            text: Mutex::new(None),
        }
    }
}

impl Default for MockClipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipboardProvider for MockClipboard {
    fn get_text(&self) -> Result<Option<String>, OsError> {
        Ok(self.text.lock().unwrap().clone())
    }

    fn set_text(&self, text: &str) -> Result<(), OsError> {
        *self.text.lock().unwrap() = Some(text.to_string());
        Ok(())
    }
}

pub struct MockInjector {
    screen: Mutex<ScreenRect>,
    cursor: Mutex<(i32, i32)>,
    actions: Mutex<Vec<InjectedAction>>,
}

impl MockInjector {
    pub fn new(screen: ScreenRect) -> Self {
        let center = screen.center();
        Self {
            screen: Mutex::new(screen),
            cursor: Mutex::new(center),
            actions: Mutex::new(Vec::new()),
        }
    }

    pub fn recorded_actions(&self) -> Vec<InjectedAction> {
        self.actions.lock().unwrap().clone()
    }

    pub fn clear_actions(&self) {
        self.actions.lock().unwrap().clear();
    }
}

impl InputInjector for MockInjector {
    fn move_cursor_abs(&self, x: i32, y: i32) -> Result<(), OsError> {
        let screen = *self.screen.lock().unwrap();
        let cx = x.clamp(screen.x, screen.right());
        let cy = y.clamp(screen.y, screen.bottom());
        *self.cursor.lock().unwrap() = (cx, cy);
        self.actions
            .lock()
            .unwrap()
            .push(InjectedAction::MoveAbs(cx, cy));
        Ok(())
    }

    fn move_cursor_rel(&self, dx: i32, dy: i32) -> Result<(), OsError> {
        let screen = *self.screen.lock().unwrap();
        let mut cur = self.cursor.lock().unwrap();
        cur.0 = (cur.0 + dx).clamp(screen.x, screen.right());
        cur.1 = (cur.1 + dy).clamp(screen.y, screen.bottom());
        self.actions
            .lock()
            .unwrap()
            .push(InjectedAction::MoveRel(dx, dy));
        Ok(())
    }

    fn mouse_button(&self, button: MouseButton, pressed: bool) -> Result<(), OsError> {
        self.actions
            .lock()
            .unwrap()
            .push(InjectedAction::MouseButton { button, pressed });
        Ok(())
    }

    fn mouse_scroll(&self, dx: f32, dy: f32) -> Result<(), OsError> {
        self.actions
            .lock()
            .unwrap()
            .push(InjectedAction::MouseScroll { dx, dy });
        Ok(())
    }

    fn key_event(&self, key: KeyCode, pressed: bool) -> Result<(), OsError> {
        self.actions
            .lock()
            .unwrap()
            .push(InjectedAction::Key { key, pressed });
        Ok(())
    }

    fn cursor_position(&self) -> Result<(i32, i32), OsError> {
        Ok(*self.cursor.lock().unwrap())
    }

    fn virtual_screen(&self) -> Result<ScreenRect, OsError> {
        Ok(*self.screen.lock().unwrap())
    }
}

pub struct MockOsBackend {
    os: OsKind,
    injector: Arc<MockInjector>,
    clipboard: Arc<MockClipboard>,
    event_tx: Mutex<Option<mpsc::UnboundedSender<InputEvent>>>,
    cursor_visible: AtomicBool,
}

impl MockOsBackend {
    pub fn new(os: OsKind, screen: ScreenRect) -> Self {
        Self {
            os,
            injector: Arc::new(MockInjector::new(screen)),
            clipboard: Arc::new(MockClipboard::new()),
            event_tx: Mutex::new(None),
            cursor_visible: AtomicBool::new(true),
        }
    }

    pub fn is_cursor_visible(&self) -> bool {
        self.cursor_visible.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn mock_injector(&self) -> Arc<MockInjector> {
        self.injector.clone()
    }

    pub fn mock_clipboard(&self) -> Arc<MockClipboard> {
        self.clipboard.clone()
    }

    pub fn push_input(&self, event: InputEvent) -> bool {
        if let Some(tx) = self.event_tx.lock().unwrap().as_ref() {
            tx.send(event).is_ok()
        } else {
            false
        }
    }
}

impl OsBackend for MockOsBackend {
    fn os_kind(&self) -> OsKind {
        self.os
    }

    fn check_permissions(&self) -> PermissionStatus {
        PermissionStatus {
            granted: true,
            message: "Mock backend ready".into(),
            advisory: None,
        }
    }

    fn injector(&self) -> Arc<dyn InputInjector> {
        self.injector.clone()
    }

    fn clipboard(&self) -> Arc<dyn ClipboardProvider> {
        self.clipboard.clone()
    }

    fn set_cursor_visible(&self, visible: bool) {
        self.cursor_visible.store(visible, Ordering::SeqCst);
    }

    fn start_capture(
        &self,
        control: Arc<CaptureControl>,
    ) -> Result<(mpsc::UnboundedReceiver<InputEvent>, CaptureGuard), OsError> {
        let (tx, rx) = mpsc::unbounded_channel();
        *self.event_tx.lock().unwrap() = Some(tx);
        Ok((rx, CaptureGuard::noop(control)))
    }
}
