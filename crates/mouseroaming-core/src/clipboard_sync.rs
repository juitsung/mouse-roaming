use mouseroaming_os::{ClipboardProvider, OsError};
use mouseroaming_proto::{Message, MAX_CLIPBOARD_BYTES};
use sha2::{Digest, Sha256};

pub struct ClipboardSync {
    last_hash: Option<[u8; 32]>,
    seq: u64,
}

impl ClipboardSync {
    pub fn new() -> Self {
        Self {
            last_hash: None,
            seq: 0,
        }
    }

    fn hash_text(text: &str) -> [u8; 32] {
        let digest = Sha256::digest(text.as_bytes());
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&digest);
        arr
    }

    /// Seed the watcher with current clipboard content so startup does not broadcast stale text.
    pub fn seed(&mut self, provider: &dyn ClipboardProvider) {
        if let Ok(Some(text)) = provider.get_text() {
            if !text.is_empty() && text.len() <= MAX_CLIPBOARD_BYTES {
                self.last_hash = Some(Self::hash_text(&text));
            }
        }
    }

    /// Poll local clipboard; returns `Some(Message::ClipboardText)` only when content changed.
    pub fn poll_local_change(&mut self, provider: &dyn ClipboardProvider) -> Option<Message> {
        let text = provider.get_text().ok()??;
        if text.is_empty() || text.len() > MAX_CLIPBOARD_BYTES {
            return None;
        }
        let hash = Self::hash_text(&text);
        if self.last_hash == Some(hash) {
            return None;
        }
        self.last_hash = Some(hash);
        self.seq = self.seq.wrapping_add(1);
        Some(Message::ClipboardText {
            seq: self.seq,
            text,
        })
    }

    /// Apply remote clipboard text locally without triggering a feedback echo on the next poll.
    pub fn apply_remote(
        &mut self,
        provider: &dyn ClipboardProvider,
        text: &str,
    ) -> Result<bool, OsError> {
        if text.is_empty() || text.len() > MAX_CLIPBOARD_BYTES {
            return Ok(false);
        }
        let hash = Self::hash_text(text);
        if self.last_hash == Some(hash) {
            return Ok(false);
        }
        self.last_hash = Some(hash);
        provider.set_text(text)?;
        Ok(true)
    }
}

impl Default for ClipboardSync {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mouseroaming_os::mock::MockClipboard;

    #[test]
    fn clipboard_sync_deduplicates_and_prevents_echo() {
        let cb = MockClipboard::new();
        let mut sync = ClipboardSync::new();

        cb.set_text("hello world").unwrap();
        let msg = sync.poll_local_change(&cb);
        assert!(matches!(msg, Some(Message::ClipboardText { text, .. }) if text == "hello world"));

        // Polling again with same content produces nothing
        assert!(sync.poll_local_change(&cb).is_none());

        // Applying remote clipboard must NOT re-emit on next local poll
        assert!(sync.apply_remote(&cb, "from macOS").unwrap());
        assert_eq!(cb.get_text().unwrap().as_deref(), Some("from macOS"));
        assert!(sync.poll_local_change(&cb).is_none());
    }
}
