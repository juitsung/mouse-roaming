use crate::{ClipboardProvider, OsError};
use std::sync::Mutex;

pub struct SystemClipboard {
    lock: Mutex<()>,
}

impl SystemClipboard {
    pub fn new() -> Self {
        Self {
            lock: Mutex::new(()),
        }
    }
}

impl Default for SystemClipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipboardProvider for SystemClipboard {
    fn get_text(&self) -> Result<Option<String>, OsError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| OsError::Clipboard("clipboard lock poisoned".into()))?;
        let mut cb =
            arboard::Clipboard::new().map_err(|e| OsError::Clipboard(e.to_string()))?;
        match cb.get_text() {
            Ok(text) => Ok(Some(text)),
            Err(arboard::Error::ContentNotAvailable) => Ok(None),
            Err(e) => Err(OsError::Clipboard(e.to_string())),
        }
    }

    fn set_text(&self, text: &str) -> Result<(), OsError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| OsError::Clipboard("clipboard lock poisoned".into()))?;
        let mut cb =
            arboard::Clipboard::new().map_err(|e| OsError::Clipboard(e.to_string()))?;
        cb.set_text(text.to_string())
            .map_err(|e| OsError::Clipboard(e.to_string()))?;
        Ok(())
    }
}
