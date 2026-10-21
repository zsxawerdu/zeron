//! Legacy archive decoding (W02/W14 boundary): pre-OMP sessions open
//! as read-only archives, fully separated from active OMP request
//! types.
//!
//! Separation rules enforced by this module's shape:
//! - It decodes only the persisted doc projection ([`SessionDoc`]
//!   entries). It never constructs a `RunRequest`, never resumes a
//!   native session, and never synthesizes a legacy harness id for a
//!   chat — an archived chat has no executable runtime identity here.
//! - Tool parts written before OMP carried no native entry id. Their
//!   [`ArchivedSession::tool_detail_key`] is `None`: callers must
//!   render the explicit expired/unavailable detail state (handoff
//!   C4), never invent a key or fetch against a guessed session.
//! - OMP-era parts keep their `omp_entry_id`, so a replayed session
//!   can still fetch full native detail owner-side via that key.

use crate::parts::MessagePart;
use crate::schema::{DocError, SessionDoc, SessionMessageEntry};

/// A decoded legacy/OMP-era session opened for reading only.
#[derive(Debug, Clone, PartialEq)]
pub struct ArchivedSession {
    pub chat_id: String,
    pub entries: Vec<SessionMessageEntry>,
}

impl ArchivedSession {
    /// Open a persisted session doc as an archive. Decoding is lenient
    /// in the same way as the live projection: a malformed part costs
    /// that part, not the whole history.
    pub fn open(doc: &SessionDoc) -> Result<Self, DocError> {
        let chat_id = doc
            .chat_id()
            .ok_or_else(|| DocError::Schema("archive doc has no chatId".into()))?;
        Ok(Self {
            chat_id,
            entries: doc.read_entries()?,
        })
    }

    /// The owner-side detail-fetch key for one tool part, when the
    /// part carries one. Legacy (pre-OMP) parts return `None` — there
    /// is no native record to fetch, and none may be synthesized.
    pub fn tool_detail_key<'a>(&self, entry: &'a SessionMessageEntry, part_id: &str) -> Option<&'a str> {
        entry.parts.iter().find_map(|part| match part {
            MessagePart::Tool {
                id,
                omp_entry_id,
                ..
            } if id == part_id => omp_entry_id.as_deref(),
            _ => None,
        })
    }

    /// True when every tool part predates OMP native detail (no part
    /// carries an entry id): a pure legacy archive.
    pub fn is_legacy_only(&self) -> bool {
        self.entries.iter().all(|entry| {
            entry.parts.iter().all(|part| match part {
                MessagePart::Tool { omp_entry_id, .. } => omp_entry_id.is_none(),
                _ => true,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::MessageRole;
    use crate::{MessageStatus, ToolDiffStat};
    use zeron_proto::{ToolCall, ToolDiff};

    fn entry(id: &str, omp_entry_id: Option<&str>) -> SessionMessageEntry {
        SessionMessageEntry {
            id: id.into(),
            role: MessageRole::Assistant,
            parts: vec![
                MessagePart::Text {
                    id: format!("{id}-t"),
                    text: "ran it".into(),
                },
                MessagePart::Tool {
                    id: format!("{id}-tool"),
                    call: ToolCall::Exec {
                        command: "cargo test".into(),
                    },
                    is_error: false,
                    resolved: true,
                    output: Some("ok".into()),
                    diff: Some(ToolDiff {
                        path: "/w/a.rs".into(),
                        old_text: Some("a\n".into()),
                        new_text: "b\n".into(),
                    }),
                    output_ref: None,
                    output_bytes: None,
                    diff_ref: None,
                    diff_stats: Some(vec![ToolDiffStat {
                        path: "/w/a.rs".into(),
                        additions: 1,
                        deletions: 1,
                    }]),
                    subagent_ref: None,
                    subagent_status: None,
                    subagent_tail: None,
                    omp_entry_id: omp_entry_id.map(str::to_owned),
                },
            ],
            created_at: 7,
            device_id: "dev-a".into(),
            status: Some(MessageStatus::Complete),
            continuation_of: None,
            duration_ms: None,
        }
    }

    /// W02 Check, doc level: an OMP-era session replays with its tool
    /// detail key intact, and a pre-OMP archived session still opens.
    #[test]
    fn replay_keeps_tool_detail_key_and_legacy_archive_still_opens() {
        // OMP-era replay: native entry id survives the doc round-trip.
        let omp_doc = SessionDoc::init("chat-omp").unwrap();
        omp_doc.push_message(&entry("m1", Some("native-entry-1"))).unwrap();
        let omp = ArchivedSession::open(&omp_doc).unwrap();
        assert!(!omp.is_legacy_only());
        assert_eq!(
            omp.tool_detail_key(&omp.entries[0], "m1-tool"),
            Some("native-entry-1")
        );
        match &omp.entries[0].parts[1] {
            MessagePart::Tool { call, diff, .. } => {
                assert_eq!(
                    call,
                    &ToolCall::Exec {
                        command: "cargo test".into()
                    }
                );
                assert_eq!(diff.as_ref().unwrap().new_text, "b\n");
            }
            other => panic!("unexpected {other:?}"),
        }

        // Legacy archive: opens read-only, no detail key is synthesized.
        let legacy_doc = SessionDoc::init("chat-legacy").unwrap();
        legacy_doc.push_message(&entry("m0", None)).unwrap();
        let legacy = ArchivedSession::open(&legacy_doc).unwrap();
        assert!(legacy.is_legacy_only());
        assert_eq!(legacy.entries.len(), 1);
        assert_eq!(legacy.tool_detail_key(&legacy.entries[0], "m0-tool"), None);
    }
}
