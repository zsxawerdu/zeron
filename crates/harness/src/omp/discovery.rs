//! Discovery of existing OMP native sessions (W10), read-only: scans the
//! session directories' file headers the same way OMP's own listing does
//! (`session-listing.ts`), without opening a runtime, importing, or
//! uploading anything. Results carry host-local paths — engine/view
//! layers must not sync them off this host (handoff C2).
//!
//! Directory resolution mirrors upstream `dirs.ts` for the common cases:
//! default profile `~/.omp/agent/sessions` (or `$XDG_DATA_HOME/omp/
//! sessions` on Linux when that root exists), named profiles under
//! `~/.omp/profiles/<name>/agent/sessions`. Agent-dir env overrides are
//! not honored here; launching always defers to OMP's own resolution.

use std::io::Read;
use std::path::{Path, PathBuf};

/// One discovered native session's header metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OmpDiscoveredSession {
    pub id: String,
    /// Host-local session file; never expose off-host.
    pub path: PathBuf,
    /// OMP profile this root belongs to (`None` = default profile).
    pub profile: Option<String>,
    /// Working directory the session was started in, when recorded.
    pub cwd: Option<String>,
    pub title: Option<String>,
    /// Parent session file when this session was forked.
    pub parent_session: Option<String>,
    /// Header timestamp as recorded by OMP (ISO string), unparsed.
    pub created: Option<String>,
    pub modified_ms: i64,
    pub size_bytes: u64,
}

/// The sessions root for `profile` under `home` (pure; existence not
/// required — discovery of a missing root simply yields nothing).
pub fn sessions_root(profile: Option<&str>, home: &Path) -> PathBuf {
    sessions_root_with_env(profile, home, &|key| std::env::var_os(key))
}

fn sessions_root_with_env(
    profile: Option<&str>,
    home: &Path,
    env: &impl Fn(&str) -> Option<std::ffi::OsString>,
) -> PathBuf {
    if let Some(profile) = profile.filter(|name| !name.is_empty()) {
        return home
            .join(".omp")
            .join("profiles")
            .join(profile)
            .join("agent")
            .join("sessions");
    }
    // Linux XDG flattening, only when OMP already uses it (the app root
    // exists) — mirroring upstream's existence check.
    if cfg!(target_os = "linux")
        && let Some(xdg) = env("XDG_DATA_HOME")
        && !xdg.is_empty()
    {
        let root = PathBuf::from(xdg).join("omp");
        if root.is_dir() {
            return root.join("sessions");
        }
    }
    home.join(".omp").join("agent").join("sessions")
}

/// Scan `root` (flat files + per-cwd subdirectories) for session headers,
/// newest first. Unreadable/unparseable files are skipped silently —
/// discovery must never fail the caller over one corrupt session.
pub fn scan_sessions(root: &Path, profile: Option<&str>) -> Vec<OmpDiscoveredSession> {
    let mut files = Vec::new();
    collect_jsonl(root, &mut files);
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                collect_jsonl(&entry.path(), &mut files);
            }
        }
    }
    let mut sessions: Vec<OmpDiscoveredSession> = files
        .iter()
        .filter_map(|path| scan_session_file(path, profile))
        .collect();
    sessions.sort_by_key(|session| std::cmp::Reverse(session.modified_ms));
    sessions
}

fn collect_jsonl(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "jsonl") {
                files.push(path);
            }
        }
    }
}

fn scan_session_file(path: &Path, profile: Option<&str>) -> Option<OmpDiscoveredSession> {
    let metadata = std::fs::metadata(path).ok()?;
    // Read a bounded prefix only. A fixed-width `title` record can
    // precede the session header (it exists so OMP can retitle in
    // place), so scan the first lines for the header, remembering any
    // title record.
    let mut prefix = Vec::with_capacity(8192);
    std::fs::File::open(path)
        .ok()?
        .take(64 * 1024)
        .read_to_end(&mut prefix)
        .ok()?;
    let mut header = None;
    let mut record_title = None;
    for line in prefix.split(|byte| *byte == b'\n').take(64) {
        let Ok(record) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        match record.get("type").and_then(serde_json::Value::as_str) {
            Some("session") => {
                header = Some(record);
                break;
            }
            Some("title") => {
                record_title = record
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
            }
            _ => {}
        }
    }
    let header = header?;
    let text = |key: &str| {
        header
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    let id = text("id")?;
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default();
    Some(OmpDiscoveredSession {
        id,
        path: path.to_path_buf(),
        profile: profile.map(str::to_owned),
        cwd: text("cwd"),
        title: text("title").or(record_title),
        parent_session: text("parentSession"),
        created: text("timestamp"),
        modified_ms,
        size_bytes: metadata.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_session(dir: &Path, name: &str, header: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), format!("{header}\n{{\"type\":\"message\"}}\n")).unwrap();
    }

    #[test]
    fn scans_headers_skips_junk_and_sorts_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sessions");
        write_session(
            &root.join("-tmp-proj"),
            "a.jsonl",
            r#"{"type":"session","id":"id-a","cwd":"/tmp/proj","title":"First","timestamp":"2026-10-01T00:00:00Z"}"#,
        );
        write_session(
            &root.join("-tmp-proj"),
            "b.jsonl",
            r#"{"type":"session","id":"id-b","parentSession":"a.jsonl"}"#,
        );
        write_session(&root, "junk.jsonl", r#"{"type":"message"}"#);
        std::fs::write(root.join("not-json.jsonl"), "nope\n").unwrap();
        std::fs::write(root.join("notes.txt"), "ignore\n").unwrap();
        // Real files open with a fixed-width title record before the
        // session header; the title comes from it.
        std::fs::write(
            root.join("-tmp-proj/titled.jsonl"),
            "{\"type\":\"title\",\"v\":1,\"title\":\"Recorded Title\"}\n{\"type\":\"session\",\"id\":\"id-t\",\"cwd\":\"/tmp/proj\"}\n",
        )
        .unwrap();

        let found = scan_sessions(&root, Some("work"));
        assert_eq!(found.len(), 3);
        let by_id: std::collections::HashMap<&str, &OmpDiscoveredSession> =
            found.iter().map(|s| (s.id.as_str(), s)).collect();
        assert_eq!(by_id["id-a"].cwd.as_deref(), Some("/tmp/proj"));
        assert_eq!(by_id["id-a"].title.as_deref(), Some("First"));
        assert_eq!(by_id["id-a"].profile.as_deref(), Some("work"));
        assert_eq!(by_id["id-b"].parent_session.as_deref(), Some("a.jsonl"));
        assert_eq!(by_id["id-t"].title.as_deref(), Some("Recorded Title"));
        assert!(found[0].modified_ms >= found[1].modified_ms);
    }

    #[test]
    fn roots_resolve_default_and_named_profiles() {
        let home = Path::new("/home/tester");
        let no_env = |_: &str| None;
        assert_eq!(
            sessions_root_with_env(None, home, &no_env),
            Path::new("/home/tester/.omp/agent/sessions")
        );
        assert_eq!(
            sessions_root_with_env(Some("work"), home, &no_env),
            Path::new("/home/tester/.omp/profiles/work/agent/sessions")
        );
    }
}
