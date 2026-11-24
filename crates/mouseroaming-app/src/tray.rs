use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

pub enum TrayAction {
    ToggleWindow,
    TogglePause,
    ReturnLocal,
    Quit,
}

pub struct AppTray {
    _tray: TrayIcon,
    id_toggle: MenuId,
    id_pause: MenuId,
    id_return: MenuId,
    id_quit: MenuId,
}

impl AppTray {
    pub fn try_new() -> anyhow::Result<Self> {
        let menu = Menu::new();
        let item_toggle = MenuItem::new("顯示 / 隱藏主視窗", true, None);
        let item_pause = MenuItem::new("暫停 / 恢復跨機切換", true, None);
        let item_return = MenuItem::new("游標立即返回本機", true, None);
        let item_quit = MenuItem::new("結束 MouseRoaming", true, None);

        let id_toggle = item_toggle.id().clone();
        let id_pause = item_pause.id().clone();
        let id_return = item_return.id().clone();
        let id_quit = item_quit.id().clone();

        menu.append(&item_toggle)?;
        menu.append(&item_pause)?;
        menu.append(&item_return)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&item_quit)?;

        let icon = generate_tray_icon()?;
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("MouseRoaming — 跨 Windows / macOS 滑鼠共享")
            .with_icon(icon)
            .build()?;

        Ok(Self {
            _tray: tray,
            id_toggle,
            id_pause,
            id_return,
            id_quit,
        })
    }

    pub fn poll_actions(&self) -> Vec<TrayAction> {
        let mut actions = Vec::new();
        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            if ev.id == self.id_toggle {
                actions.push(TrayAction::ToggleWindow);
            } else if ev.id == self.id_pause {
                actions.push(TrayAction::TogglePause);
            } else if ev.id == self.id_return {
                actions.push(TrayAction::ReturnLocal);
            } else if ev.id == self.id_quit {
                actions.push(TrayAction::Quit);
            }
        }
        while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
            if matches!(
                ev,
                TrayIconEvent::DoubleClick { .. }
            ) {
                actions.push(TrayAction::ToggleWindow);
            }
        }
        actions
    }
}

/// Generates a clean 32x32 RGBA tray icon (rounded slate badge with cyan roaming cursor).
fn generate_tray_icon() -> anyhow::Result<Icon> {
    let w: u32 = 32;
    let h: u32 = 32;
    let mut rgba = vec![0u8; (w * h * 4) as usize];

    for y in 0..h {
        for x in 0..w {
            let idx = ((y * w + x) * 4) as usize;
            let dx = x as f32 - 15.5;
            let dy = y as f32 - 15.5;
            let in_rounded = dx.abs() <= 14.0 && dy.abs() <= 14.0 && (dx.abs().max(0.0) - 10.0).max(0.0).hypot((dy.abs().max(0.0) - 10.0).max(0.0)) <= 4.0;
            if in_rounded {
                // Deep indigo-blue background
                rgba[idx] = 24;
                rgba[idx + 1] = 44;
                rgba[idx + 2] = 78;
                rgba[idx + 3] = 255;

                // Subtle dual-screen divider line
                if x == 16 && (5..27).contains(&y) {
                    rgba[idx] = 56;
                    rgba[idx + 1] = 189;
                    rgba[idx + 2] = 248;
                }

                // Crisp white pointer arrow crossing the middle
                let px = x as i32 - 9;
                let py = y as i32 - 7;
                if px >= 0 && py >= 0 && py <= 16 && px <= py && (py <= 12 || px <= 4) {
                    rgba[idx] = 240;
                    rgba[idx + 1] = 249;
                    rgba[idx + 2] = 255;
                }
            }
        }
    }

    Ok(Icon::from_rgba(rgba, w, h)?)
}
