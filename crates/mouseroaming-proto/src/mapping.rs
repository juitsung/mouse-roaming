use crate::{KeyCode, OsKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyMappingConfig {
    /// When controlling between Windows and macOS, swap Ctrl <-> Cmd (Meta) so
    /// Ctrl+C/V/X/Z/A/S on Windows triggers Cmd+C/V/X/Z/A/S on macOS and vice versa.
    pub swap_ctrl_cmd_cross_os: bool,
    /// Invert vertical scroll direction when crossing between Windows and macOS
    /// (compensates for macOS Natural Scrolling).
    pub invert_scroll_cross_os: bool,
}

impl Default for KeyMappingConfig {
    fn default() -> Self {
        Self {
            swap_ctrl_cmd_cross_os: true,
            invert_scroll_cross_os: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModifierMapper {
    config: KeyMappingConfig,
}

impl ModifierMapper {
    pub fn new(config: KeyMappingConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &KeyMappingConfig {
        &self.config
    }

    pub fn is_win_mac_pair(source: OsKind, target: OsKind) -> bool {
        matches!(
            (source, target),
            (OsKind::Windows, OsKind::MacOS) | (OsKind::MacOS, OsKind::Windows)
        )
    }

    pub fn map_key(&self, key: KeyCode, source: OsKind, target: OsKind) -> KeyCode {
        if !self.config.swap_ctrl_cmd_cross_os || !Self::is_win_mac_pair(source, target) {
            return key;
        }
        match key {
            KeyCode::LeftCtrl => KeyCode::LeftMeta,
            KeyCode::RightCtrl => KeyCode::RightMeta,
            KeyCode::LeftMeta => KeyCode::LeftCtrl,
            KeyCode::RightMeta => KeyCode::RightCtrl,
            other => other,
        }
    }

    pub fn map_scroll(&self, dx: f32, dy: f32, source: OsKind, target: OsKind) -> (f32, f32) {
        if self.config.invert_scroll_cross_os && Self::is_win_mac_pair(source, target) {
            (-dx, -dy)
        } else {
            (dx, dy)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swaps_ctrl_and_cmd_between_windows_and_macos() {
        let mapper = ModifierMapper::new(KeyMappingConfig {
            swap_ctrl_cmd_cross_os: true,
            invert_scroll_cross_os: true,
        });

        assert_eq!(
            mapper.map_key(KeyCode::LeftCtrl, OsKind::Windows, OsKind::MacOS),
            KeyCode::LeftMeta
        );
        assert_eq!(
            mapper.map_key(KeyCode::LeftMeta, OsKind::MacOS, OsKind::Windows),
            KeyCode::LeftCtrl
        );
        assert_eq!(
            mapper.map_key(KeyCode::C, OsKind::Windows, OsKind::MacOS),
            KeyCode::C
        );
        assert_eq!(
            mapper.map_key(KeyCode::LeftCtrl, OsKind::Windows, OsKind::Windows),
            KeyCode::LeftCtrl
        );

        assert_eq!(
            mapper.map_scroll(1.0, -3.0, OsKind::Windows, OsKind::MacOS),
            (-1.0, 3.0)
        );
        assert_eq!(
            mapper.map_scroll(1.0, -3.0, OsKind::Windows, OsKind::Windows),
            (1.0, -3.0)
        );
    }
}
