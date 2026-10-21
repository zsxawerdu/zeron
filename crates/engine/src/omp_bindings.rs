//! Host-local OMP session bindings (OMP-only handoff C2): which native
//! OMP session each app chat is bound to — profile, session id/file,
//! leaf, and owning process generation.
//!
//! This state is deliberately NOT synced: `session_file` is a path on
//! this host and must never become another device's file handle. The
//! synced workspace row keeps only the native session id
//! (`chats.harness_session_id`, the resume token); this store is the
//! host's richer record, and every stamp is also journaled as an
//! `AgentEvent::OmpSessionBound`, so the store is a rebuildable cache —
//! a missing/corrupt file starts empty rather than failing boot.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use zeron_proto::omp::OmpSessionBinding;

pub struct OmpBindingStore {
    path: PathBuf,
    bindings: Mutex<HashMap<String, OmpSessionBinding>>,
}

impl OmpBindingStore {
    /// Open (or start) the store at `path`. Host-local; one JSON object
    /// keyed by chat id.
    pub fn open(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let bindings = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_else(|| {
                if path.exists() {
                    tracing::warn!(path = %path.display(), "OMP binding store unreadable; starting empty (journals can rebuild it)");
                }
                HashMap::new()
            });
        Self {
            path,
            bindings: Mutex::new(bindings),
        }
    }

    /// Record the latest binding for `chat_id` (newest stamp wins) and
    /// persist atomically: write a sibling temp file, then rename over
    /// the store so a crash never leaves a torn JSON document.
    pub fn stamp(&self, chat_id: &str, mut binding: OmpSessionBinding) {
        binding.chat_id = chat_id.to_owned();
        let snapshot = {
            let mut bindings = self.lock();
            if bindings.get(chat_id) == Some(&binding) {
                return;
            }
            bindings.insert(chat_id.to_owned(), binding);
            bindings.clone()
        };
        if let Err(error) = self.persist(&snapshot) {
            tracing::warn!(chat = %chat_id, error = %error, "OMP binding store write failed");
        }
    }

    pub fn get(&self, chat_id: &str) -> Option<OmpSessionBinding> {
        self.lock().get(chat_id).cloned()
    }

    /// Drop a chat's binding (chat deleted/archived away from OMP).
    pub fn forget(&self, chat_id: &str) {
        let snapshot = {
            let mut bindings = self.lock();
            if bindings.remove(chat_id).is_none() {
                return;
            }
            bindings.clone()
        };
        if let Err(error) = self.persist(&snapshot) {
            tracing::warn!(chat = %chat_id, error = %error, "OMP binding store write failed");
        }
    }

    fn persist(&self, bindings: &HashMap<String, OmpSessionBinding>) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string(bindings)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        let temp = self.path.with_extension("json.tmp");
        std::fs::write(&temp, raw)?;
        std::fs::rename(&temp, &self.path)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, OmpSessionBinding>> {
        self.bindings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(session: &str) -> OmpSessionBinding {
        OmpSessionBinding {
            chat_id: String::new(),
            profile: None,
            session_id: session.into(),
            session_file: Some(format!("/host/sessions/{session}.jsonl")),
            leaf_id: None,
            generation: 7,
        }
    }

    #[test]
    fn stamps_persist_atomically_and_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("omp-sessions.json");
        let store = OmpBindingStore::open(&path);
        store.stamp("chat-1", binding("native-1"));
        // The engine owns the chat id; the driver-emitted copy has none.
        assert_eq!(store.get("chat-1").unwrap().chat_id, "chat-1");
        assert!(!dir.path().join("omp-sessions.json.tmp").exists());

        let reopened = OmpBindingStore::open(&path);
        let got = reopened.get("chat-1").unwrap();
        assert_eq!(got.session_id, "native-1");
        assert_eq!(got.generation, 7);

        reopened.forget("chat-1");
        assert!(OmpBindingStore::open(&path).get("chat-1").is_none());
    }

    #[test]
    fn corrupt_store_starts_empty_instead_of_failing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("omp-sessions.json");
        std::fs::write(&path, "{ torn").unwrap();
        let store = OmpBindingStore::open(&path);
        assert!(store.get("chat-1").is_none());
        store.stamp("chat-1", binding("native-9"));
        assert_eq!(
            OmpBindingStore::open(&path).get("chat-1").unwrap().session_id,
            "native-9"
        );
    }
}
