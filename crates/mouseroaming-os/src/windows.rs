use crate::clipboard::SystemClipboard;
use crate::{
    CaptureControl, CaptureGuard, ClipboardProvider, InputEvent, InputInjector, OsBackend, OsError,
    PermissionStatus, MOUSEROAMING_MAGIC_EXTRA,
};
use mouseroaming_proto::{KeyCode, MouseButton, OsKind, ScreenRect};
use std::mem::size_of;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, Once};
use tokio::sync::mpsc;
use tracing::{debug, warn};
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE,
    KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, MOUSEEVENTF_ABSOLUTE,
    MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
    MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateCursor, DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics,
    PostThreadMessageW, SetCursorPos, SetSystemCursor, SetWindowsHookExW, SystemParametersInfoW,
    TranslateMessage, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED,
    LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, OCR_APPSTARTING, OCR_CROSS, OCR_HAND, OCR_IBEAM, OCR_NO,
    OCR_NORMAL, OCR_SIZEALL, OCR_SIZENESW, OCR_SIZENS, OCR_SIZENWSE, OCR_SIZEWE, OCR_UP, OCR_WAIT,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SPI_SETCURSORS,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
    XBUTTON1, XBUTTON2,
};

static DPI_INIT: Once = Once::new();

fn ensure_dpi_aware() {
    DPI_INIT.call_once(|| unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    });
}

const OCR_SYSTEM_CURSORS: [u32; 13] = [
    OCR_NORMAL,
    OCR_IBEAM,
    OCR_WAIT,
    OCR_CROSS,
    OCR_UP,
    OCR_SIZENWSE,
    OCR_SIZENESW,
    OCR_SIZEWE,
    OCR_SIZENS,
    OCR_SIZEALL,
    OCR_NO,
    OCR_HAND,
    OCR_APPSTARTING,
];

static CURSOR_HIDDEN: AtomicBool = AtomicBool::new(false);
static CURSOR_LOCK: Mutex<()> = Mutex::new(());

/// Replaces standard system cursors with a 1x1 100% transparent cursor so the cursor vanishes.
pub fn set_windows_cursor_visible(visible: bool) {
    let _lock = CURSOR_LOCK.lock().unwrap();
    let is_hidden = CURSOR_HIDDEN.load(Ordering::SeqCst);

    if !visible && !is_hidden {
        unsafe {
            let h_module = GetModuleHandleW(std::ptr::null());
            // Create a 1x1 fully transparent blank cursor
            // AND mask = 0xFF (transparent), XOR mask = 0x00 (no color)
            let and_plane = [0xFFu8];
            let xor_plane = [0x00u8];
            for &id in &OCR_SYSTEM_CURSORS {
                let blank_cursor = CreateCursor(
                    h_module,
                    0,
                    0,
                    1,
                    1,
                    and_plane.as_ptr() as *const std::ffi::c_void,
                    xor_plane.as_ptr() as *const std::ffi::c_void,
                );
                if !blank_cursor.is_null() {
                    // SetSystemCursor takes ownership of blank_cursor and destroys it when replaced
                    let _ = SetSystemCursor(blank_cursor, id);
                }
            }
        }
        CURSOR_HIDDEN.store(true, Ordering::SeqCst);
        debug!("Windows system cursor hidden");
    } else if visible && is_hidden {
        unsafe {
            // Restores all standard system cursors from system default scheme
            let _ = SystemParametersInfoW(SPI_SETCURSORS, 0, null_mut(), 0);
        }
        CURSOR_HIDDEN.store(false, Ordering::SeqCst);
        debug!("Windows system cursor restored");
    }
}

struct HookRuntimeState {
    control: Arc<CaptureControl>,
    tx: mpsc::UnboundedSender<InputEvent>,
    last_x: AtomicI32,
    last_y: AtomicI32,
    has_last: AtomicBool,
    was_grabbed: AtomicBool,
}

static HOOK_STATE: Mutex<Option<Arc<HookRuntimeState>>> = Mutex::new(None);

fn get_hook_state() -> Option<Arc<HookRuntimeState>> {
    HOOK_STATE.lock().ok().and_then(|g| g.clone())
}

fn send_single_input(input: INPUT) -> Result<(), OsError> {
    let sent = unsafe { SendInput(1, &input, size_of::<INPUT>() as i32) };
    if sent != 1 {
        return Err(OsError::Inject("Win32 SendInput failed".into()));
    }
    Ok(())
}

pub struct WindowsInjector;

impl WindowsInjector {
    pub fn new() -> Self {
        ensure_dpi_aware();
        Self
    }

    fn move_abs_send_input(&self, x: i32, y: i32) -> Result<(), OsError> {
        let screen = self.virtual_screen()?;
        let norm_x = if screen.width > 1 {
            (((x - screen.x) as i64 * 65535) / (screen.width as i64 - 1)).clamp(0, 65535) as i32
        } else {
            0
        };
        let norm_y = if screen.height > 1 {
            (((y - screen.y) as i64 * 65535) / (screen.height as i64 - 1)).clamp(0, 65535) as i32
        } else {
            0
        };
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: norm_x,
                    dy: norm_y,
                    mouseData: 0,
                    dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                    time: 0,
                    dwExtraInfo: MOUSEROAMING_MAGIC_EXTRA,
                },
            },
        };
        send_single_input(input)
    }
}

impl InputInjector for WindowsInjector {
    fn move_cursor_abs(&self, x: i32, y: i32) -> Result<(), OsError> {
        self.move_abs_send_input(x, y)?;
        let _ = unsafe { SetCursorPos(x, y) };
        Ok(())
    }

    fn move_cursor_rel(&self, dx: i32, dy: i32) -> Result<(), OsError> {
        let (cx, cy) = self.cursor_position()?;
        let screen = self.virtual_screen()?;
        let nx = (cx + dx).clamp(screen.x, screen.right());
        let ny = (cy + dy).clamp(screen.y, screen.bottom());
        self.move_abs_send_input(nx, ny)
    }

    fn mouse_button(&self, button: MouseButton, pressed: bool) -> Result<(), OsError> {
        let (flags, data) = match (button, pressed) {
            (MouseButton::Left, true) => (MOUSEEVENTF_LEFTDOWN, 0),
            (MouseButton::Left, false) => (MOUSEEVENTF_LEFTUP, 0),
            (MouseButton::Right, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
            (MouseButton::Right, false) => (MOUSEEVENTF_RIGHTUP, 0),
            (MouseButton::Middle, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
            (MouseButton::Middle, false) => (MOUSEEVENTF_MIDDLEUP, 0),
            (MouseButton::Back, true) => (MOUSEEVENTF_XDOWN, XBUTTON1 as u32),
            (MouseButton::Back, false) => (MOUSEEVENTF_XUP, XBUTTON1 as u32),
            (MouseButton::Forward, true) => (MOUSEEVENTF_XDOWN, XBUTTON2 as u32),
            (MouseButton::Forward, false) => (MOUSEEVENTF_XUP, XBUTTON2 as u32),
        };
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: data,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: MOUSEROAMING_MAGIC_EXTRA,
                },
            },
        };
        send_single_input(input)
    }

    fn mouse_scroll(&self, dx: f32, dy: f32) -> Result<(), OsError> {
        if dy.abs() > f32::EPSILON {
            let wheel_delta = (dy * 120.0).round() as i32;
            let input = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: 0,
                        dy: 0,
                        mouseData: wheel_delta as u32,
                        dwFlags: MOUSEEVENTF_WHEEL,
                        time: 0,
                        dwExtraInfo: MOUSEROAMING_MAGIC_EXTRA,
                    },
                },
            };
            send_single_input(input)?;
        }
        if dx.abs() > f32::EPSILON {
            let wheel_delta = (dx * 120.0).round() as i32;
            let input = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: 0,
                        dy: 0,
                        mouseData: wheel_delta as u32,
                        dwFlags: MOUSEEVENTF_HWHEEL,
                        time: 0,
                        dwExtraInfo: MOUSEROAMING_MAGIC_EXTRA,
                    },
                },
            };
            send_single_input(input)?;
        }
        Ok(())
    }

    fn key_event(&self, key: KeyCode, pressed: bool) -> Result<(), OsError> {
        let Some((vk, extended)) = key.to_windows_vk() else {
            return Ok(());
        };
        let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) as u16 };
        let mut flags = 0u32;
        if extended {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        if !pressed {
            flags |= KEYEVENTF_KEYUP;
        }
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: MOUSEROAMING_MAGIC_EXTRA,
                },
            },
        };
        send_single_input(input)
    }

    fn cursor_position(&self) -> Result<(i32, i32), OsError> {
        let mut pt = POINT { x: 0, y: 0 };
        let ok = unsafe { GetCursorPos(&mut pt) };
        if ok == 0 {
            return Err(OsError::Inject("GetCursorPos failed".into()));
        }
        Ok((pt.x, pt.y))
    }

    fn virtual_screen(&self) -> Result<ScreenRect, OsError> {
        unsafe {
            let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let w = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1) as u32;
            let h = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1) as u32;
            Ok(ScreenRect::new(x, y, w, h))
        }
    }
}

unsafe extern "system" fn mouse_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(null_mut(), code, wparam, lparam);
    }

    let info = &*(lparam as *const MSLLHOOKSTRUCT);
    if (info.flags & LLMHF_INJECTED) != 0 || info.dwExtraInfo == MOUSEROAMING_MAGIC_EXTRA {
        return CallNextHookEx(null_mut(), code, wparam, lparam);
    }

    let Some(state) = get_hook_state() else {
        return CallNextHookEx(null_mut(), code, wparam, lparam);
    };
    if !state.control.is_running() {
        return CallNextHookEx(null_mut(), code, wparam, lparam);
    }

    let grabbed = state.control.is_grabbed();
    let msg = wparam as u32;

    if msg == WM_MOUSEMOVE {
        let x = info.pt.x;
        let y = info.pt.y;
        if grabbed {
            let (ax, ay) = state.control.anchor();
            let was = state.was_grabbed.swap(true, Ordering::SeqCst);
            if !was {
                // First event after entering grab: anchor cursor without injecting jump delta.
                let _ = SetCursorPos(ax, ay);
                state.last_x.store(ax, Ordering::Relaxed);
                state.last_y.store(ay, Ordering::Relaxed);
                return 1;
            }
            let dx = x - ax;
            let dy = y - ay;
            if dx != 0 || dy != 0 {
                let _ = state.tx.send(InputEvent::MouseMove { x, y, dx, dy });
                let _ = SetCursorPos(ax, ay);
            }
            return 1;
        } else {
            state.was_grabbed.store(false, Ordering::SeqCst);
            let (dx, dy) = if state.has_last.swap(true, Ordering::Relaxed) {
                let lx = state.last_x.swap(x, Ordering::Relaxed);
                let ly = state.last_y.swap(y, Ordering::Relaxed);
                (x - lx, y - ly)
            } else {
                state.last_x.store(x, Ordering::Relaxed);
                state.last_y.store(y, Ordering::Relaxed);
                (0, 0)
            };
            let _ = state.tx.send(InputEvent::MouseMove { x, y, dx, dy });
            return CallNextHookEx(null_mut(), code, wparam, lparam);
        }
    }

    // Buttons & Scroll: only intercept when grabbed
    if !grabbed {
        return CallNextHookEx(null_mut(), code, wparam, lparam);
    }

    let event = match msg {
        WM_LBUTTONDOWN => Some(InputEvent::MouseButton {
            button: MouseButton::Left,
            pressed: true,
        }),
        WM_LBUTTONUP => Some(InputEvent::MouseButton {
            button: MouseButton::Left,
            pressed: false,
        }),
        WM_RBUTTONDOWN => Some(InputEvent::MouseButton {
            button: MouseButton::Right,
            pressed: true,
        }),
        WM_RBUTTONUP => Some(InputEvent::MouseButton {
            button: MouseButton::Right,
            pressed: false,
        }),
        WM_MBUTTONDOWN => Some(InputEvent::MouseButton {
            button: MouseButton::Middle,
            pressed: true,
        }),
        WM_MBUTTONUP => Some(InputEvent::MouseButton {
            button: MouseButton::Middle,
            pressed: false,
        }),
        WM_XBUTTONDOWN | WM_XBUTTONUP => {
            let xbtn = ((info.mouseData >> 16) & 0xFFFF) as u16;
            let button = if xbtn == XBUTTON1 {
                MouseButton::Back
            } else {
                MouseButton::Forward
            };
            Some(InputEvent::MouseButton {
                button,
                pressed: msg == WM_XBUTTONDOWN,
            })
        }
        WM_MOUSEWHEEL => {
            let delta = ((info.mouseData >> 16) as i16) as f32 / 120.0;
            Some(InputEvent::MouseScroll { dx: 0.0, dy: delta })
        }
        WM_MOUSEHWHEEL => {
            let delta = ((info.mouseData >> 16) as i16) as f32 / 120.0;
            Some(InputEvent::MouseScroll { dx: delta, dy: 0.0 })
        }
        _ => None,
    };

    if let Some(ev) = event {
        let _ = state.tx.send(ev);
        return 1;
    }

    CallNextHookEx(null_mut(), code, wparam, lparam)
}

unsafe extern "system" fn keyboard_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(null_mut(), code, wparam, lparam);
    }

    let info = &*(lparam as *const KBDLLHOOKSTRUCT);
    if (info.flags & LLKHF_INJECTED) != 0 || info.dwExtraInfo == MOUSEROAMING_MAGIC_EXTRA {
        return CallNextHookEx(null_mut(), code, wparam, lparam);
    }

    let Some(state) = get_hook_state() else {
        return CallNextHookEx(null_mut(), code, wparam, lparam);
    };
    if !state.control.is_running() {
        return CallNextHookEx(null_mut(), code, wparam, lparam);
    }

    let msg = wparam as u32;
    let pressed = matches!(msg, WM_KEYDOWN | WM_SYSKEYDOWN);
    let released = matches!(msg, WM_KEYUP | WM_SYSKEYUP);
    if !pressed && !released {
        return CallNextHookEx(null_mut(), code, wparam, lparam);
    }

    let vk = info.vkCode as u16;
    let extended = (info.flags & LLKHF_EXTENDED) != 0;

    // Hardware-level emergency escape: Ctrl + Alt + Esc ALWAYS releases grab immediately
    if pressed && vk == 0x1B {
        let ctrl_down = (GetAsyncKeyState(0x11) as u16 & 0x8000) != 0;
        let alt_down = (GetAsyncKeyState(0x12) as u16 & 0x8000) != 0;
        if ctrl_down && alt_down {
            state.control.set_grabbed(false);
            set_windows_cursor_visible(true);
            let _ = state.tx.send(InputEvent::EmergencyRelease);
            return 1;
        }
    }

    let Some(key) = KeyCode::from_windows_vk(vk, extended) else {
        return CallNextHookEx(null_mut(), code, wparam, lparam);
    };

    let grabbed = state.control.is_grabbed();
    if grabbed {
        let _ = state.tx.send(InputEvent::Key { key, pressed });
        return 1;
    }

    // Even when not grabbed, forward Ctrl+Alt+Arrow hotkeys for manual screen switching
    if pressed && matches!(key, KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::ArrowUp | KeyCode::ArrowDown) {
        let ctrl_down = (GetAsyncKeyState(0x11) as u16 & 0x8000) != 0;
        let alt_down = (GetAsyncKeyState(0x12) as u16 & 0x8000) != 0;
        if ctrl_down && alt_down {
            let _ = state.tx.send(InputEvent::Key { key: KeyCode::LeftCtrl, pressed: true });
            let _ = state.tx.send(InputEvent::Key { key: KeyCode::LeftAlt, pressed: true });
            let _ = state.tx.send(InputEvent::Key { key, pressed: true });
            return CallNextHookEx(null_mut(), code, wparam, lparam);
        }
    }

    CallNextHookEx(null_mut(), code, wparam, lparam)
}

pub struct WindowsBackend {
    injector: Arc<WindowsInjector>,
    clipboard: Arc<SystemClipboard>,
}

impl WindowsBackend {
    pub fn new() -> Self {
        ensure_dpi_aware();
        Self {
            injector: Arc::new(WindowsInjector::new()),
            clipboard: Arc::new(SystemClipboard::new()),
        }
    }
}

impl Default for WindowsBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl OsBackend for WindowsBackend {
    fn os_kind(&self) -> OsKind {
        OsKind::Windows
    }

    fn check_permissions(&self) -> PermissionStatus {
        PermissionStatus {
            granted: true,
            message: "Windows 低階輸入掛鉤已就緒".into(),
            advisory: Some(
                "提示：若需控制「以系統管理員身分執行」的視窗（如工作管理員），請以系統管理員身分執行 MouseRoaming（受 Windows UIPI 安全機制限制）。".into(),
            ),
        }
    }

    fn injector(&self) -> Arc<dyn InputInjector> {
        self.injector.clone()
    }

    fn clipboard(&self) -> Arc<dyn ClipboardProvider> {
        self.clipboard.clone()
    }

    fn set_cursor_visible(&self, visible: bool) {
        set_windows_cursor_visible(visible);
    }

    fn start_capture(
        &self,
        control: Arc<CaptureControl>,
    ) -> Result<(mpsc::UnboundedReceiver<InputEvent>, CaptureGuard), OsError> {
        let (tx, rx) = mpsc::unbounded_channel();
        let state = Arc::new(HookRuntimeState {
            control: control.clone(),
            tx,
            last_x: AtomicI32::new(0),
            last_y: AtomicI32::new(0),
            has_last: AtomicBool::new(false),
            was_grabbed: AtomicBool::new(false),
        });

        {
            let mut guard = HOOK_STATE
                .lock()
                .map_err(|_| OsError::Hook("hook state poisoned".into()))?;
            *guard = Some(state);
        }

        let thread_id = Arc::new(AtomicU32::new(0));
        let thread_id_clone = thread_id.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();

        let handle = std::thread::Builder::new()
            .name("mouseroaming-win32-hook".into())
            .spawn(move || unsafe {
                ensure_dpi_aware();
                let tid = GetCurrentThreadId();
                thread_id_clone.store(tid, Ordering::SeqCst);

                let hmodule = GetModuleHandleW(std::ptr::null());
                let mouse_hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), hmodule, 0);
                let kb_hook =
                    SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook_proc), hmodule, 0);

                if mouse_hook.is_null() || kb_hook.is_null() {
                    if !mouse_hook.is_null() {
                        UnhookWindowsHookEx(mouse_hook);
                    }
                    if !kb_hook.is_null() {
                        UnhookWindowsHookEx(kb_hook);
                    }
                    let _ = ready_tx.send(Err("SetWindowsHookExW failed".into()));
                    return;
                }

                debug!("Win32 low-level mouse & keyboard hooks installed on thread {tid}");
                let _ = ready_tx.send(Ok(()));

                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }

                UnhookWindowsHookEx(mouse_hook);
                UnhookWindowsHookEx(kb_hook);
                set_windows_cursor_visible(true);
                if let Ok(mut guard) = HOOK_STATE.lock() {
                    *guard = None;
                }
                debug!("Win32 low-level hooks uninstalled cleanly");
            })
            .map_err(|e| OsError::Hook(e.to_string()))?;

        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(OsError::Hook(e)),
            Err(_) => return Err(OsError::Hook("hook thread exited unexpectedly".into())),
        }

        let guard = CaptureGuard::new(
            control,
            Box::new(move || {
                set_windows_cursor_visible(true);
                let tid = thread_id.load(Ordering::SeqCst);
                if tid != 0 {
                    unsafe {
                        let _ = PostThreadMessageW(tid, WM_QUIT, 0, 0);
                    }
                }
                if let Err(e) = handle.join() {
                    warn!("Win32 hook thread join warning: {:?}", e);
                }
            }),
        );

        Ok((rx, guard))
    }
}
