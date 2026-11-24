use crate::clipboard::SystemClipboard;
use crate::{
    CaptureControl, CaptureGuard, ClipboardProvider, InputEvent, InputInjector, OsBackend, OsError,
    PermissionStatus, MOUSEROAMING_MAGIC_EXTRA,
};
use core_foundation::base::{CFRelease, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::mach_port::CFMachPortRef;
use core_foundation::runloop::{
    kCFRunLoopCommonModes, CFRunLoopAddSource, CFRunLoopGetCurrent, CFRunLoopRef, CFRunLoopRun,
    CFRunLoopStop,
};
use core_foundation::string::CFString;
use core_graphics::display::{CGDisplay, CGPoint};
use mouseroaming_proto::{KeyCode, MouseButton, OsKind, ScreenRect};
use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::{debug, warn};

type CGEventRef = *mut c_void;
type CGEventTapProxy = *mut c_void;
type CGEventSourceRef = *mut c_void;
type CGEventMask = u64;
type CGEventFlags = u64;
type CGEventType = u32;
type CGMouseButton = u32;

const K_CG_HID_EVENT_TAP: u32 = 0;
const K_CG_SESSION_EVENT_TAP: u32 = 1;
const K_CG_HEAD_INSERT_EVENT_TAP: u32 = 0;
const K_CG_EVENT_TAP_OPTION_DEFAULT: u32 = 0;

const K_CG_EVENT_LEFT_MOUSE_DOWN: CGEventType = 1;
const K_CG_EVENT_LEFT_MOUSE_UP: CGEventType = 2;
const K_CG_EVENT_RIGHT_MOUSE_DOWN: CGEventType = 3;
const K_CG_EVENT_RIGHT_MOUSE_UP: CGEventType = 4;
const K_CG_EVENT_MOUSE_MOVED: CGEventType = 5;
const K_CG_EVENT_LEFT_MOUSE_DRAGGED: CGEventType = 6;
const K_CG_EVENT_RIGHT_MOUSE_DRAGGED: CGEventType = 7;
const K_CG_EVENT_KEY_DOWN: CGEventType = 10;
const K_CG_EVENT_KEY_UP: CGEventType = 11;
const K_CG_EVENT_FLAGS_CHANGED: CGEventType = 12;
const K_CG_EVENT_SCROLL_WHEEL: CGEventType = 22;
const K_CG_EVENT_OTHER_MOUSE_DOWN: CGEventType = 25;
const K_CG_EVENT_OTHER_MOUSE_UP: CGEventType = 26;
const K_CG_EVENT_OTHER_MOUSE_DRAGGED: CGEventType = 27;
const K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT: CGEventType = 0xFFFFFFFE;

const K_CG_MOUSE_EVENT_BUTTON_NUMBER: u32 = 3;
const K_CG_MOUSE_EVENT_DELTA_X: u32 = 4;
const K_CG_MOUSE_EVENT_DELTA_Y: u32 = 5;
const K_CG_KEYBOARD_EVENT_KEYCODE: u32 = 9;
const K_CG_SCROLL_WHEEL_EVENT_DELTA_AXIS_1: u32 = 11;
const K_CG_SCROLL_WHEEL_EVENT_DELTA_AXIS_2: u32 = 12;
const K_CG_EVENT_SOURCE_USER_DATA: u32 = 42;

const K_CG_EVENT_FLAG_MASK_ALPHA_SHIFT: CGEventFlags = 0x00010000;
const K_CG_EVENT_FLAG_MASK_SHIFT: CGEventFlags = 0x00020000;
const K_CG_EVENT_FLAG_MASK_CONTROL: CGEventFlags = 0x00040000;
const K_CG_EVENT_FLAG_MASK_ALTERNATE: CGEventFlags = 0x00080000;
const K_CG_EVENT_FLAG_MASK_COMMAND: CGEventFlags = 0x00100000;

const K_CG_SCROLL_EVENT_UNIT_LINE: u32 = 1;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: CGEventMask,
        callback: extern "C" fn(CGEventTapProxy, CGEventType, CGEventRef, *mut c_void) -> CGEventRef,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CFMachPortCreateRunLoopSource(
        allocator: *const c_void,
        port: CFMachPortRef,
        order: isize,
    ) -> core_foundation::runloop::CFRunLoopSourceRef;

    fn CGEventCreate(source: CGEventSourceRef) -> CGEventRef;
    fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
    fn CGEventGetFlags(event: CGEventRef) -> CGEventFlags;
    fn CGEventSetFlags(event: CGEventRef, flags: CGEventFlags);
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    fn CGEventSetIntegerValueField(event: CGEventRef, field: u32, value: i64);

    fn CGEventCreateMouseEvent(
        source: CGEventSourceRef,
        mouse_type: CGEventType,
        mouse_cursor_position: CGPoint,
        mouse_button: CGMouseButton,
    ) -> CGEventRef;
    fn CGEventCreateScrollWheelEvent(
        source: CGEventSourceRef,
        units: u32,
        wheel_count: u32,
        wheel1: i32,
        wheel2: i32,
    ) -> CGEventRef;
    fn CGEventCreateKeyboardEvent(
        source: CGEventSourceRef,
        virtual_key: u16,
        key_down: bool,
    ) -> CGEventRef;
    fn CGEventPost(tap: u32, event: CGEventRef);

    fn CGWarpMouseCursorPosition(new_cursor_position: CGPoint) -> i32;
    fn CGAssociateMouseAndMouseCursorPosition(connected: bool) -> i32;
    fn CGDisplayHideCursor(display: u32) -> i32;
    fn CGDisplayShowCursor(display: u32) -> i32;
}

fn check_accessibility_trusted(prompt: bool) -> bool {
    unsafe {
        if !prompt {
            return AXIsProcessTrustedWithOptions(std::ptr::null());
        }
        let key = CFString::new("AXTrustedCheckOptionPrompt");
        let val = CFBoolean::true_value();
        let dict = CFDictionary::from_CFType_pairs(&[(key, val)]);
        AXIsProcessTrustedWithOptions(dict.as_concrete_TypeRef() as *const c_void)
    }
}

pub struct MacOsInjector {
    active_flags: AtomicU64,
    pressed_buttons: Mutex<u8>,
}

impl MacOsInjector {
    pub fn new() -> Self {
        Self {
            active_flags: AtomicU64::new(0),
            pressed_buttons: Mutex::new(0),
        }
    }

    fn tag_and_post(&self, event: CGEventRef) -> Result<(), OsError> {
        if event.is_null() {
            return Err(OsError::Inject("CGEvent creation returned null".into()));
        }
        unsafe {
            CGEventSetIntegerValueField(
                event,
                K_CG_EVENT_SOURCE_USER_DATA,
                MOUSEROAMING_MAGIC_EXTRA as i64,
            );
            CGEventPost(K_CG_HID_EVENT_TAP, event);
            CFRelease(event as *const c_void);
        }
        Ok(())
    }

    fn update_modifier_flag(&self, key: KeyCode, pressed: bool) {
        let mask = match key {
            KeyCode::LeftShift | KeyCode::RightShift => K_CG_EVENT_FLAG_MASK_SHIFT,
            KeyCode::LeftCtrl | KeyCode::RightCtrl => K_CG_EVENT_FLAG_MASK_CONTROL,
            KeyCode::LeftAlt | KeyCode::RightAlt => K_CG_EVENT_FLAG_MASK_ALTERNATE,
            KeyCode::LeftMeta | KeyCode::RightMeta => K_CG_EVENT_FLAG_MASK_COMMAND,
            KeyCode::CapsLock => K_CG_EVENT_FLAG_MASK_ALPHA_SHIFT,
            _ => return,
        };
        if pressed {
            self.active_flags.fetch_or(mask, Ordering::SeqCst);
        } else {
            self.active_flags.fetch_and(!mask, Ordering::SeqCst);
        }
    }
}

impl InputInjector for MacOsInjector {
    fn move_cursor_abs(&self, x: i32, y: i32) -> Result<(), OsError> {
        let pt = CGPoint::new(x as f64, y as f64);
        let buttons = *self.pressed_buttons.lock().unwrap();
        let (ev_type, btn) = if (buttons & 1) != 0 {
            (K_CG_EVENT_LEFT_MOUSE_DRAGGED, 0)
        } else if (buttons & 2) != 0 {
            (K_CG_EVENT_RIGHT_MOUSE_DRAGGED, 1)
        } else if (buttons & 4) != 0 {
            (K_CG_EVENT_OTHER_MOUSE_DRAGGED, 2)
        } else {
            (K_CG_EVENT_MOUSE_MOVED, 0)
        };
        unsafe {
            let _ = CGWarpMouseCursorPosition(pt);
            let ev = CGEventCreateMouseEvent(null_mut(), ev_type, pt, btn);
            self.tag_and_post(ev)
        }
    }

    fn move_cursor_rel(&self, dx: i32, dy: i32) -> Result<(), OsError> {
        let (cx, cy) = self.cursor_position()?;
        let screen = self.virtual_screen()?;
        let nx = (cx + dx).clamp(screen.x, screen.right());
        let ny = (cy + dy).clamp(screen.y, screen.bottom());
        self.move_cursor_abs(nx, ny)
    }

    fn mouse_button(&self, button: MouseButton, pressed: bool) -> Result<(), OsError> {
        let (cx, cy) = self.cursor_position()?;
        let pt = CGPoint::new(cx as f64, cy as f64);
        let (ev_type, cg_btn, bit) = match (button, pressed) {
            (MouseButton::Left, true) => (K_CG_EVENT_LEFT_MOUSE_DOWN, 0, 1u8),
            (MouseButton::Left, false) => (K_CG_EVENT_LEFT_MOUSE_UP, 0, 1u8),
            (MouseButton::Right, true) => (K_CG_EVENT_RIGHT_MOUSE_DOWN, 1, 2u8),
            (MouseButton::Right, false) => (K_CG_EVENT_RIGHT_MOUSE_UP, 1, 2u8),
            (MouseButton::Middle, true) => (K_CG_EVENT_OTHER_MOUSE_DOWN, 2, 4u8),
            (MouseButton::Middle, false) => (K_CG_EVENT_OTHER_MOUSE_UP, 2, 4u8),
            (MouseButton::Back, true) => (K_CG_EVENT_OTHER_MOUSE_DOWN, 3, 8u8),
            (MouseButton::Back, false) => (K_CG_EVENT_OTHER_MOUSE_UP, 3, 8u8),
            (MouseButton::Forward, true) => (K_CG_EVENT_OTHER_MOUSE_DOWN, 4, 16u8),
            (MouseButton::Forward, false) => (K_CG_EVENT_OTHER_MOUSE_UP, 4, 16u8),
        };
        {
            let mut mask = self.pressed_buttons.lock().unwrap();
            if pressed {
                *mask |= bit;
            } else {
                *mask &= !bit;
            }
        }
        unsafe {
            let ev = CGEventCreateMouseEvent(null_mut(), ev_type, pt, cg_btn);
            if !ev.is_null() {
                CGEventSetFlags(ev, self.active_flags.load(Ordering::SeqCst));
            }
            self.tag_and_post(ev)
        }
    }

    fn mouse_scroll(&self, dx: f32, dy: f32) -> Result<(), OsError> {
        let v_lines = (dy * 3.0).round() as i32;
        let h_lines = (dx * 3.0).round() as i32;
        unsafe {
            let ev = CGEventCreateScrollWheelEvent(
                null_mut(),
                K_CG_SCROLL_EVENT_UNIT_LINE,
                2,
                v_lines,
                h_lines,
            );
            self.tag_and_post(ev)
        }
    }

    fn key_event(&self, key: KeyCode, pressed: bool) -> Result<(), OsError> {
        self.update_modifier_flag(key, pressed);
        let Some(code) = key.to_macos_keycode() else {
            return Ok(());
        };
        unsafe {
            let ev = CGEventCreateKeyboardEvent(null_mut(), code, pressed);
            if !ev.is_null() {
                CGEventSetFlags(ev, self.active_flags.load(Ordering::SeqCst));
            }
            self.tag_and_post(ev)
        }
    }

    fn cursor_position(&self) -> Result<(i32, i32), OsError> {
        unsafe {
            let ev = CGEventCreate(null_mut());
            if ev.is_null() {
                return Err(OsError::Inject("CGEventCreate failed".into()));
            }
            let pt = CGEventGetLocation(ev);
            CFRelease(ev as *const c_void);
            Ok((pt.x.round() as i32, pt.y.round() as i32))
        }
    }

    fn virtual_screen(&self) -> Result<ScreenRect, OsError> {
        let displays = CGDisplay::active_displays()
            .map_err(|e| OsError::Inject(format!("CGDisplay::active_displays failed: {e}")))?;
        if displays.is_empty() {
            return Ok(ScreenRect::new(0, 0, 1440, 900));
        }
        let mut min_x = f64::MAX;
        let mut min_y = f64::MAX;
        let mut max_x = f64::MIN;
        let mut max_y = f64::MIN;
        for id in displays {
            let b = CGDisplay::new(id).bounds();
            min_x = min_x.min(b.origin.x);
            min_y = min_y.min(b.origin.y);
            max_x = max_x.max(b.origin.x + b.size.width);
            max_y = max_y.max(b.origin.y + b.size.height);
        }
        Ok(ScreenRect::new(
            min_x.floor() as i32,
            min_y.floor() as i32,
            (max_x - min_x).ceil().max(1.0) as u32,
            (max_y - min_y).ceil().max(1.0) as u32,
        ))
    }
}

struct TapContext {
    control: Arc<CaptureControl>,
    tx: mpsc::UnboundedSender<InputEvent>,
    tap_port: AtomicUsize,
    prev_flags: AtomicU64,
    was_grabbed: AtomicBool,
}

extern "C" fn macos_event_tap_callback(
    _proxy: CGEventTapProxy,
    ev_type: CGEventType,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef {
    if user_info.is_null() {
        return event;
    }
    let ctx = unsafe { &*(user_info as *const TapContext) };

    if ev_type == K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT {
        let port = ctx.tap_port.load(Ordering::SeqCst) as CFMachPortRef;
        if !port.is_null() {
            unsafe { CGEventTapEnable(port, true) };
        }
        return event;
    }

    let extra = unsafe { CGEventGetIntegerValueField(event, K_CG_EVENT_SOURCE_USER_DATA) };
    if extra == MOUSEROAMING_MAGIC_EXTRA as i64 {
        return event;
    }

    if !ctx.control.is_running() {
        return event;
    }

    let grabbed = ctx.control.is_grabbed();
    let prev_grabbed = ctx.was_grabbed.swap(grabbed, Ordering::SeqCst);
    if grabbed && !prev_grabbed {
        unsafe {
            let _ = CGAssociateMouseAndMouseCursorPosition(false);
            let _ = CGDisplayHideCursor(CGDisplay::main().id);
        }
    } else if !grabbed && prev_grabbed {
        unsafe {
            let _ = CGAssociateMouseAndMouseCursorPosition(true);
            let _ = CGDisplayShowCursor(CGDisplay::main().id);
        }
    }

    match ev_type {
        K_CG_EVENT_MOUSE_MOVED
        | K_CG_EVENT_LEFT_MOUSE_DRAGGED
        | K_CG_EVENT_RIGHT_MOUSE_DRAGGED
        | K_CG_EVENT_OTHER_MOUSE_DRAGGED => {
            let loc = unsafe { CGEventGetLocation(event) };
            let dx = unsafe { CGEventGetIntegerValueField(event, K_CG_MOUSE_EVENT_DELTA_X) } as i32;
            let dy = unsafe { CGEventGetIntegerValueField(event, K_CG_MOUSE_EVENT_DELTA_Y) } as i32;
            let _ = ctx.tx.send(InputEvent::MouseMove {
                x: loc.x.round() as i32,
                y: loc.y.round() as i32,
                dx,
                dy,
            });
            return if grabbed { null_mut() } else { event };
        }
        K_CG_EVENT_LEFT_MOUSE_DOWN | K_CG_EVENT_LEFT_MOUSE_UP => {
            if grabbed {
                let _ = ctx.tx.send(InputEvent::MouseButton {
                    button: MouseButton::Left,
                    pressed: ev_type == K_CG_EVENT_LEFT_MOUSE_DOWN,
                });
                return null_mut();
            }
        }
        K_CG_EVENT_RIGHT_MOUSE_DOWN | K_CG_EVENT_RIGHT_MOUSE_UP => {
            if grabbed {
                let _ = ctx.tx.send(InputEvent::MouseButton {
                    button: MouseButton::Right,
                    pressed: ev_type == K_CG_EVENT_RIGHT_MOUSE_DOWN,
                });
                return null_mut();
            }
        }
        K_CG_EVENT_OTHER_MOUSE_DOWN | K_CG_EVENT_OTHER_MOUSE_UP => {
            if grabbed {
                let num = unsafe {
                    CGEventGetIntegerValueField(event, K_CG_MOUSE_EVENT_BUTTON_NUMBER)
                };
                let button = match num {
                    2 => MouseButton::Middle,
                    3 => MouseButton::Back,
                    _ => MouseButton::Forward,
                };
                let _ = ctx.tx.send(InputEvent::MouseButton {
                    button,
                    pressed: ev_type == K_CG_EVENT_OTHER_MOUSE_DOWN,
                });
                return null_mut();
            }
        }
        K_CG_EVENT_SCROLL_WHEEL => {
            if grabbed {
                let dy = unsafe {
                    CGEventGetIntegerValueField(event, K_CG_SCROLL_WHEEL_EVENT_DELTA_AXIS_1)
                } as f32
                    / 3.0;
                let dx = unsafe {
                    CGEventGetIntegerValueField(event, K_CG_SCROLL_WHEEL_EVENT_DELTA_AXIS_2)
                } as f32
                    / 3.0;
                let _ = ctx.tx.send(InputEvent::MouseScroll { dx, dy });
                return null_mut();
            }
        }
        K_CG_EVENT_KEY_DOWN | K_CG_EVENT_KEY_UP => {
            let code =
                unsafe { CGEventGetIntegerValueField(event, K_CG_KEYBOARD_EVENT_KEYCODE) } as u16;
            let flags = unsafe { CGEventGetFlags(event) };
            let pressed = ev_type == K_CG_EVENT_KEY_DOWN;

            // Emergency escape: Ctrl + Option + Esc
            if pressed
                && code == 0x35
                && (flags & K_CG_EVENT_FLAG_MASK_CONTROL) != 0
                && (flags & K_CG_EVENT_FLAG_MASK_ALTERNATE) != 0
            {
                ctx.control.set_grabbed(false);
                unsafe {
                    let _ = CGAssociateMouseAndMouseCursorPosition(true);
                    let _ = CGDisplayShowCursor(CGDisplay::main().id);
                }
                let _ = ctx.tx.send(InputEvent::EmergencyRelease);
                return null_mut();
            }

            if grabbed {
                if let Some(key) = KeyCode::from_macos_keycode(code) {
                    let _ = ctx.tx.send(InputEvent::Key { key, pressed });
                }
                return null_mut();
            }
        }
        K_CG_EVENT_FLAGS_CHANGED => {
            let new_flags = unsafe { CGEventGetFlags(event) };
            let old_flags = ctx.prev_flags.swap(new_flags, Ordering::SeqCst);
            if grabbed {
                let changed = new_flags ^ old_flags;
                let mappings = [
                    (K_CG_EVENT_FLAG_MASK_SHIFT, KeyCode::LeftShift),
                    (K_CG_EVENT_FLAG_MASK_CONTROL, KeyCode::LeftCtrl),
                    (K_CG_EVENT_FLAG_MASK_ALTERNATE, KeyCode::LeftAlt),
                    (K_CG_EVENT_FLAG_MASK_COMMAND, KeyCode::LeftMeta),
                    (K_CG_EVENT_FLAG_MASK_ALPHA_SHIFT, KeyCode::CapsLock),
                ];
                for (mask, key) in mappings {
                    if (changed & mask) != 0 {
                        let _ = ctx.tx.send(InputEvent::Key {
                            key,
                            pressed: (new_flags & mask) != 0,
                        });
                    }
                }
                return null_mut();
            }
        }
        _ => {}
    }

    event
}

pub struct MacOsBackend {
    injector: Arc<MacOsInjector>,
    clipboard: Arc<SystemClipboard>,
}

impl MacOsBackend {
    pub fn new() -> Self {
        Self {
            injector: Arc::new(MacOsInjector::new()),
            clipboard: Arc::new(SystemClipboard::new()),
        }
    }
}

impl Default for MacOsBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl OsBackend for MacOsBackend {
    fn os_kind(&self) -> OsKind {
        OsKind::MacOS
    }

    fn check_permissions(&self) -> PermissionStatus {
        let trusted = check_accessibility_trusted(false);
        if trusted {
            PermissionStatus {
                granted: true,
                message: "macOS 輔助使用權限已授權".into(),
                advisory: None,
            }
        } else {
            PermissionStatus {
                granted: false,
                message: "尚未授權 macOS 輔助使用（Accessibility）權限".into(),
                advisory: Some(
                    "請前往「系統設定 → 隱私權與安全性 → 輔助使用」與「輸入監控」，勾選 MouseRoaming 後重新啟動。".into(),
                ),
            }
        }
    }

    fn injector(&self) -> Arc<dyn InputInjector> {
        self.injector.clone()
    }

    fn clipboard(&self) -> Arc<dyn ClipboardProvider> {
        self.clipboard.clone()
    }

    fn set_cursor_visible(&self, visible: bool) {
        unsafe {
            if visible {
                let _ = CGAssociateMouseAndMouseCursorPosition(true);
                let _ = CGDisplayShowCursor(CGDisplay::main().id);
            } else {
                let _ = CGAssociateMouseAndMouseCursorPosition(false);
                let _ = CGDisplayHideCursor(CGDisplay::main().id);
            }
        }
    }

    fn notify(&self, title: &str, message: &str) {
        let t = title.replace('"', "\\\"");
        let m = message.replace('"', "\\\"");
        std::thread::spawn(move || {
            let _ = std::process::Command::new("afplay")
                .arg("/System/Library/Sounds/Ping.aiff")
                .spawn();
            let _ = std::process::Command::new("osascript")
                .arg("-e")
                .arg(format!("display notification \"{m}\" with title \"{t}\""))
                .spawn();
        });
    }

    fn start_capture(
        &self,
        control: Arc<CaptureControl>,
    ) -> Result<(mpsc::UnboundedReceiver<InputEvent>, CaptureGuard), OsError> {
        if !check_accessibility_trusted(true) {
            return Err(OsError::PermissionDenied(
                "請在 macOS 系統設定 → 隱私權與安全性 → 輔助使用中授權 MouseRoaming".into(),
            ));
        }

        let (tx, rx) = mpsc::unbounded_channel();
        let runloop_ptr = Arc::new(AtomicUsize::new(0));
        let runloop_ptr_clone = runloop_ptr.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let control_clone = control.clone();

        let handle = std::thread::Builder::new()
            .name("mouseroaming-macos-eventtap".into())
            .spawn(move || unsafe {
                let ctx = Box::new(TapContext {
                    control: control_clone,
                    tx,
                    tap_port: AtomicUsize::new(0),
                    prev_flags: AtomicU64::new(0),
                    was_grabbed: AtomicBool::new(false),
                });
                let ctx_ptr = Box::into_raw(ctx);

                let mask: CGEventMask = (1 << K_CG_EVENT_LEFT_MOUSE_DOWN)
                    | (1 << K_CG_EVENT_LEFT_MOUSE_UP)
                    | (1 << K_CG_EVENT_RIGHT_MOUSE_DOWN)
                    | (1 << K_CG_EVENT_RIGHT_MOUSE_UP)
                    | (1 << K_CG_EVENT_MOUSE_MOVED)
                    | (1 << K_CG_EVENT_LEFT_MOUSE_DRAGGED)
                    | (1 << K_CG_EVENT_RIGHT_MOUSE_DRAGGED)
                    | (1 << K_CG_EVENT_KEY_DOWN)
                    | (1 << K_CG_EVENT_KEY_UP)
                    | (1 << K_CG_EVENT_FLAGS_CHANGED)
                    | (1 << K_CG_EVENT_SCROLL_WHEEL)
                    | (1 << K_CG_EVENT_OTHER_MOUSE_DOWN)
                    | (1 << K_CG_EVENT_OTHER_MOUSE_UP)
                    | (1 << K_CG_EVENT_OTHER_MOUSE_DRAGGED);

                let tap = CGEventTapCreate(
                    K_CG_SESSION_EVENT_TAP,
                    K_CG_HEAD_INSERT_EVENT_TAP,
                    K_CG_EVENT_TAP_OPTION_DEFAULT,
                    mask,
                    macos_event_tap_callback,
                    ctx_ptr as *mut c_void,
                );
                if tap.is_null() {
                    let _ = Box::from_raw(ctx_ptr);
                    let _ = ready_tx.send(Err("CGEventTapCreate failed (check permissions)".into()));
                    return;
                }

                (*ctx_ptr).tap_port.store(tap as usize, Ordering::SeqCst);
                let source = CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0);
                let runloop = CFRunLoopGetCurrent();
                runloop_ptr_clone.store(runloop as usize, Ordering::SeqCst);
                CFRunLoopAddSource(runloop, source, kCFRunLoopCommonModes);
                CGEventTapEnable(tap, true);

                debug!("macOS CGEventTap installed cleanly");
                let _ = ready_tx.send(Ok(()));

                CFRunLoopRun();

                // Restore cursor association & visibility on exit
                let _ = CGAssociateMouseAndMouseCursorPosition(true);
                let _ = CGDisplayShowCursor(CGDisplay::main().id);
                CFRelease(source as *const c_void);
                CFRelease(tap as *const c_void);
                let _ = Box::from_raw(ctx_ptr);
                debug!("macOS CGEventTap stopped cleanly");
            })
            .map_err(|e| OsError::Hook(e.to_string()))?;

        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(OsError::Hook(e)),
            Err(_) => return Err(OsError::Hook("event tap thread exited unexpectedly".into())),
        }

        let guard = CaptureGuard::new(
            control,
            Box::new(move || {
                let rl = runloop_ptr.load(Ordering::SeqCst) as CFRunLoopRef;
                if !rl.is_null() {
                    unsafe { CFRunLoopStop(rl) };
                }
                if let Err(e) = handle.join() {
                    warn!("macOS event tap thread join warning: {:?}", e);
                }
            }),
        );

        Ok((rx, guard))
    }
}
