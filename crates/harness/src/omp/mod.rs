//! OMP runtime driver: one long-lived `omp --mode rpc-ui` child per active
//! native session, spoken to through the vendored official Rust client
//! (crates/omp-rpc; pin recorded in
//! docs/plans/2026-10-20-omp-only-handoff.md, W00).
//!
//! The client is a blocking transport with its own reader/writer threads.
//! Drive [`OmpRuntime`] from dedicated threads only — never GPUI or Tokio
//! executor threads. This module is the transport/lifecycle layer; engine
//! session binding, event projection, and UI interactions live above it
//! (handoff W02–W04).
//!
//! Policy invariants:
//! - Spawned WITHOUT `--no-ui` (tool approval prompts fail closed with it)
//!   and WITHOUT `--auto-approve`/`--approval-mode`: the user's effective
//!   OMP approval policy is never overridden here.
//! - The executable is resolved from explicit configuration or GUI-safe
//!   discovery; a missing OMP is an error, never a fallback to another
//!   agent runtime.
//! - One runtime owns one native session; the process is kept across
//!   prompts and closed only by [`OmpRuntime::close`] or drop (stdin EOF,
//!   then process-group SIGTERM/SIGKILL via the client — Unix; off Unix
//!   only the leader is killed, see crates/omp-rpc README).

mod harness;
mod discovery;
mod interactions;
pub mod model_admin;
mod normalize;
pub mod settings;

pub use discovery::{scan_sessions, sessions_root, OmpDiscoveredSession};
pub use harness::{OmpHarness, OmpInputBridge};

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Receiver;

use omp_rpc::{Client, ClientOptions, Event};
pub use omp_rpc::{wire, Command as WireCommand, RpcInbound};

use crate::HarnessError;

/// The OMP release this integration is qualified against. Newer runtimes
/// run unqualified (discovery still governs advertised capabilities);
/// older ones may lack wire commands this client sends.
pub const PINNED_OMP_VERSION: semver::Version = semver::Version::new(18, 12, 5);

/// Resolved `omp` executable and its probed version.
#[derive(Clone, Debug)]
pub struct OmpExecutable {
    pub path: PathBuf,
    pub version: Option<semver::Version>,
}

impl OmpExecutable {
    /// Resolution order: explicit `override_path`, `$OMP_EXECUTABLE`, then
    /// PATH plus the usual per-user install locations (GUI launches do not
    /// inherit a login-shell PATH). The Tern app-support path observed
    /// during planning is deliberately NOT a candidate: it is a machine
    /// detail, not an install contract.
    pub fn resolve(override_path: Option<&Path>) -> Result<Self, HarnessError> {
        if let Some(path) = override_path
            .map(Path::to_path_buf)
            .or_else(|| std::env::var_os("OMP_EXECUTABLE").map(PathBuf::from))
        {
            let path = crate::executable::validate_native_override(&path)?;
            return Ok(Self {
                version: crate::executable::binary_version(&path),
                path,
            });
        }
        let home = crate::executable::home_or_current_dir();
        let path = crate::executable::find_on_paths(
            "omp",
            vec![
                home.join(".local/bin/omp"),
                home.join(".npm-global/bin/omp"),
                PathBuf::from("/opt/homebrew/bin/omp"),
                PathBuf::from("/usr/local/bin/omp"),
            ],
        )
        .ok_or_else(|| {
            HarnessError::NotInstalled(
                "OMP is not installed: install omp or set OMP_EXECUTABLE".into(),
            )
        })?;
        Ok(Self {
            version: crate::executable::binary_version(&path),
            path,
        })
    }

    /// True when the probed version is the pinned, qualified release.
    pub fn qualified(&self) -> bool {
        self.version.as_ref() == Some(&PINNED_OMP_VERSION)
    }

    fn require_supported(&self) -> Result<(), HarnessError> {
        if let Some(version) = &self.version {
            if *version < PINNED_OMP_VERSION {
                return Err(HarnessError::Protocol(format!(
                    "omp {version} is unsupported; this build requires omp {PINNED_OMP_VERSION} or newer"
                )));
            }
        }
        Ok(())
    }
}

/// Which native session a launch attaches to. Product sessions persist:
/// `Ephemeral` (`--no-session`) is for probes and discovery only.
#[derive(Clone, Debug, Default)]
pub enum OmpSession {
    #[default]
    New,
    /// Native session to resume (`--resume <id-or-path>`); callers pass
    /// the session ID, never a host path, across trust boundaries.
    Resume(PathBuf),
    Ephemeral,
}

/// OMP `tools.approvalMode` values. A launch may override the mode for
/// its own session only (`--approval-mode`); the product default is
/// `None` — the user's effective OMP policy untouched. Settings UI
/// (W06) owns any persistent choice; tests may tighten to `AlwaysAsk`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OmpApprovalMode {
    AlwaysAsk,
    AlwaysAskCached,
    Write,
    Yolo,
}

impl OmpApprovalMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AlwaysAsk => "always-ask",
            Self::AlwaysAskCached => "always-ask-cached",
            Self::Write => "write",
            Self::Yolo => "yolo",
        }
    }
}

/// Launch parameters for one runtime process.
#[derive(Clone, Debug)]
pub struct OmpLaunch {
    /// Working directory; also scopes project settings/context discovery.
    pub cwd: PathBuf,
    /// OMP named profile (`--profile`); `None` keeps the user's default.
    pub profile: Option<String>,
    pub session: OmpSession,
    /// Extra environment for the child (isolation in tests/probes). Never
    /// used to smuggle credentials: OMP owns credential storage.
    pub env: Vec<(String, String)>,
    /// Where child stderr is written; a temp file when unset. Stderr is a
    /// diagnostics channel, kept separate from the RPC stdout stream.
    pub stderr_log: Option<PathBuf>,
    /// Session-scoped approval-mode override; `None` (the product
    /// default) never puts `--approval-mode` on the command line.
    pub approval_mode: Option<OmpApprovalMode>,
}

impl OmpLaunch {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            profile: None,
            session: OmpSession::default(),
            env: Vec::new(),
            stderr_log: None,
            approval_mode: None,
        }
    }

    fn args(&self) -> Vec<std::ffi::OsString> {
        let mut args: Vec<std::ffi::OsString> = vec!["--mode".into(), "rpc-ui".into()];
        if let Some(profile) = &self.profile {
            args.push("--profile".into());
            args.push(profile.into());
        }
        if let Some(mode) = self.approval_mode {
            args.push(format!("--approval-mode={}", mode.as_str()).into());
        }
        match &self.session {
            OmpSession::New => {}
            OmpSession::Resume(path) => {
                args.push("--resume".into());
                args.push(path.as_os_str().to_owned());
            }
            OmpSession::Ephemeral => args.push("--no-session".into()),
        }
        args
    }
}

/// Runtime capability snapshot (handoff C1, transport slice): executable
/// identity plus the negotiated transport. Discovered commands/models are
/// queried through the wire and formalized with the W02 domain types.
#[derive(Clone, Debug)]
pub struct OmpCapabilities {
    pub executable: OmpExecutable,
    pub protocol_version: u32,
    pub max_frame_bytes: Option<i64>,
    pub max_reassembled_frame_bytes: Option<i64>,
}

/// A live OMP process plus its typed wire client. `generation` identifies
/// this process instance; callers stamp it on every event so frames from
/// a replaced process are recognizable as stale (handoff C3).
pub struct OmpRuntime {
    client: Option<Client>,
    events: Option<Receiver<Event>>,
    executable: OmpExecutable,
    generation: u64,
    stderr_log: PathBuf,
}

impl OmpRuntime {
    /// Spawn the child and wait for its `ready` frame (v2 negotiated when
    /// advertised). Blocks on client threads — call from a dedicated
    /// thread. Startup without a selectable model exits before ready; the
    /// error carries the child's stderr tail so onboarding (W05) can show
    /// the actionable cause instead of a bare "server closed".
    pub fn spawn(
        executable: OmpExecutable,
        launch: &OmpLaunch,
        generation: u64,
    ) -> Result<Self, HarnessError> {
        executable.require_supported()?;
        let stderr_log = match &launch.stderr_log {
            Some(path) => path.clone(),
            None => {
                let mut path = std::env::temp_dir();
                path.push(format!("zeron-omp-{}-{generation}.stderr.log", std::process::id()));
                path
            }
        };
        let stderr = std::fs::File::create(&stderr_log)?;
        let mut process = Command::new(&executable.path);
        process
            .args(launch.args())
            .current_dir(&launch.cwd)
            .stderr(Stdio::from(stderr));
        crate::compose_std_child_path(&mut process, &executable.path);
        for (key, value) in &launch.env {
            process.env(key, value);
        }
        let (client, events) =
            Client::spawn(process, ClientOptions::default()).map_err(|error| {
                let tail = read_stderr_tail(&stderr_log, 8);
                if tail.is_empty() {
                    map_rpc_error(error)
                } else {
                    HarnessError::Protocol(format!(
                        "omp failed to start ({error}): {}",
                        crate::redact_secrets(&tail)
                    ))
                }
            })?;
        Ok(Self {
            client: Some(client),
            events: Some(events),
            executable,
            generation,
            stderr_log,
        })
    }

    /// Submit a prompt and block until ITS `prompt_result` (correlated by
    /// request id inside the client; a stale `agent_end` cannot end the
    /// wait). Admission, yield, and `session_settled` stay distinguishable
    /// in the returned turn and on the event stream.
    pub fn prompt_and_wait(
        &self,
        command: &wire::PromptCommand,
        timeout: std::time::Duration,
    ) -> Result<omp_rpc::PromptTurn, HarnessError> {
        self.client
            .as_ref()
            .ok_or_else(|| HarnessError::Protocol("omp runtime is closed".into()))?
            .prompt_and_wait(command, timeout)
            .map_err(map_rpc_error)
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn pid(&self) -> Option<u32> {
        self.client.as_ref().and_then(Client::pid)
    }

    pub fn capabilities(&self) -> OmpCapabilities {
        let ready = self.client.as_ref().map(Client::ready);
        OmpCapabilities {
            executable: self.executable.clone(),
            protocol_version: self.client.as_ref().map_or(0, Client::protocol_version),
            max_frame_bytes: ready.and_then(|ready| ready.max_frame_bytes),
            max_reassembled_frame_bytes: ready.and_then(|ready| ready.max_reassembled_frame_bytes),
        }
    }

    /// The single event stream: every frame that is not a correlated
    /// response (agent events, extension UI requests, notifications,
    /// `session_settled`, …). Drain it continuously — the client's
    /// channel is unbounded, so the consumer is the backpressure.
    pub fn take_events(&mut self) -> Option<Receiver<Event>> {
        self.events.take()
    }

    /// Run a typed wire command and wait for its correlated response.
    pub fn call<C: WireCommand>(&self, command: &C) -> Result<C::Output, HarnessError> {
        self.client
            .as_ref()
            .ok_or_else(|| HarnessError::Protocol("omp runtime is closed".into()))?
            .call(command)
            .map_err(map_rpc_error)
    }

    /// Run a blocking wire command while draining the event stream on
    /// the calling thread: each event goes to `on_event`, which may
    /// queue inbound frames (extension UI responses) through the
    /// sender; queued frames are forwarded between events. For
    /// commands whose completion depends on those responses (login:
    /// `open_url`/`input` arrive mid-call), where draining only after
    /// the call would deadlock against the server's wait.
    pub fn call_concurrent<C, F>(
        &mut self,
        command: &C,
        mut on_event: F,
    ) -> Result<C::Output, HarnessError>
    where
        C: WireCommand + Sync,
        C::Output: Send,
        F: FnMut(&Event, &std::sync::mpsc::Sender<RpcInbound>),
    {
        let Some(client) = self.client.take() else {
            return Err(HarnessError::Protocol("omp runtime is closed".into()));
        };
        let Some(events) = self.events.take() else {
            self.client = Some(client);
            return Err(HarnessError::Protocol("OMP event stream missing".into()));
        };
        let (outbound_tx, outbound_rx) = std::sync::mpsc::channel::<RpcInbound>();
        let result = std::thread::scope(|scope| {
            let call_thread = scope.spawn(|| client.call(command).map_err(map_rpc_error));
            while !call_thread.is_finished() {
                while let Ok(frame) = outbound_rx.try_recv() {
                    let _ = client.send(&frame).map_err(map_rpc_error);
                }
                match events.recv_timeout(std::time::Duration::from_millis(50)) {
                    Ok(event) => on_event(&event, &outbound_tx),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            while let Ok(frame) = outbound_rx.try_recv() {
                let _ = client.send(&frame).map_err(map_rpc_error);
            }
            call_thread
                .join()
                .map_err(|_| HarnessError::Protocol("OMP call thread panicked".into()))?
        });
        self.client = Some(client);
        self.events = Some(events);
        result
    }

    /// Queue an inbound frame that expects no correlated response
    /// (extension UI responses, host tool/URI results).
    pub fn send(&self, frame: &RpcInbound) -> Result<(), HarnessError> {
        self.client
            .as_ref()
            .ok_or_else(|| HarnessError::Protocol("omp runtime is closed".into()))?
            .send(frame)
            .map_err(map_rpc_error)
    }

    /// Ghost-text suffix for the prose word ending at `cursor`
    /// (W07, `predict_word`); `None` when OMP has no suggestion.
    pub fn predict_word(&self, text: &str, cursor: i64) -> Result<Option<String>, HarnessError> {
        self.call(&wire::PredictWordCommand {
            text: text.to_owned(),
            cursor,
        })
    }

    /// Session state snapshot (`get_state`).
    pub fn state(&self) -> Result<wire::SessionState, HarnessError> {
        self.call(&wire::GetStateCommand {})
    }

    /// Abort the in-flight run (`abort`); the process and session stay up.
    pub fn abort(&self) -> Result<(), HarnessError> {
        self.call(&wire::AbortCommand {})
    }

    /// Snapshot of this session's subagents (`get_subagents`) — the
    /// initial/reconnected-view counterpart to the lifecycle stream.
    pub fn subagents(&self) -> Result<Vec<wire::SubagentSnapshot>, HarnessError> {
        self.call(&wire::GetSubagentsCommand {})
    }

    /// Cancel one subagent by native id (`cancel_subagent`); true when
    /// OMP accepted the cancellation.
    pub fn cancel_subagent(&self, subagent_id: &str) -> Result<bool, HarnessError> {
        self.call(&wire::CancelSubagentCommand {
            subagent_id: subagent_id.to_owned(),
        })
    }

    /// Steer one live subagent (`steer_subagent`).
    pub fn steer_subagent(&self, subagent_id: &str, message: &str) -> Result<(), HarnessError> {
        self.call(&wire::SteerSubagentCommand {
            subagent_id: subagent_id.to_owned(),
            message: message.to_owned(),
        })
    }

    /// Native steering/follow-up queue snapshot (`get_state`). Texts
    /// only — the runtime assigns queued items no IDs (see
    /// [`zeron_proto::omp::OmpQueueSnapshot`]).
    pub fn queue_snapshot(&self) -> Result<zeron_proto::omp::OmpQueueSnapshot, HarnessError> {
        let state = self.state()?;
        Ok(zeron_proto::omp::OmpQueueSnapshot {
            steering: state.queued_messages.steering,
            follow_up: state.queued_messages.follow_up,
            live_steered: state.queued_messages.live_steered.max(0) as u32,
        })
    }

    /// Remove one natively queued message by its verbatim chip text
    /// (`remove_queued_message`); true when OMP removed one. Precision
    /// is the runtime's: first text match within the named queue, and
    /// live-steered steering entries cannot be reached. Only
    /// steer/follow-up rows exist in the native queue — a `Prompt` row
    /// is a protocol error here, never silently remapped.
    pub fn remove_queued_message(
        &self,
        message: &str,
        queue: zeron_proto::omp::OmpPromptKind,
    ) -> Result<bool, HarnessError> {
        let queue = match queue {
            zeron_proto::omp::OmpPromptKind::Steer => wire::QueuedMessageQueue::Steering,
            zeron_proto::omp::OmpPromptKind::FollowUp => wire::QueuedMessageQueue::FollowUp,
            zeron_proto::omp::OmpPromptKind::Prompt => {
                return Err(HarnessError::Protocol(
                    "native queue removal supports steer/follow-up rows only".into(),
                ));
            }
        };
        Ok(self
            .call(&wire::RemoveQueuedMessageCommand {
                message: message.to_owned(),
                queue,
            })?
            .removed)
    }

    /// Move one queued follow-up into the steering queue by verbatim
    /// text (`promote_queued_message`); true when OMP promoted one.
    /// Same first-match text precision as removal.
    pub fn promote_queued_message(&self, message: &str) -> Result<bool, HarnessError> {
        Ok(self
            .call(&wire::PromoteQueuedMessageCommand {
                message: message.to_owned(),
            })?
            .promoted)
    }

    /// Native session tree + active leaf (`get_tree`). Tree nodes are
    /// OMP's raw entry maps, preserved as-is for the inspector (W10).
    pub fn tree(&self) -> Result<wire::SessionTree, HarnessError> {
        self.call(&wire::GetTreeCommand {})
    }

    /// Native session statistics (`get_session_stats`): message/tool
    /// counts and token usage as OMP accounts them.
    pub fn session_stats(&self) -> Result<wire::SessionStats, HarnessError> {
        self.call(&wire::GetSessionStatsCommand {})
    }

    /// Move the active leaf to `entry_id` within this session
    /// (`branch`). OMP owns the transition (incl. extension
    /// cancellation); callers update bindings only on success.
    pub fn branch(&self, entry_id: &str) -> Result<wire::BranchResult, HarnessError> {
        self.call(&wire::BranchCommand {
            entry_id: entry_id.to_owned(),
        })
    }

    /// Fork onto a new native session at `entry_id` (or the current
    /// leaf when `None`). Afterwards this process serves the NEW
    /// session — re-read `state()` for its id/file before stamping any
    /// binding.
    pub fn fork(&self, entry_id: Option<&str>) -> Result<wire::CancellationResult, HarnessError> {
        self.call(&wire::ForkCommand {
            entry_id: entry_id.map(str::to_owned),
        })
    }

    /// Last `lines` lines of child stderr, credentials redacted.
    pub fn stderr_tail(&self, lines: usize) -> String {
        crate::redact_secrets(&read_stderr_tail(&self.stderr_log, lines))
    }

    pub fn stderr_log(&self) -> &Path {
        &self.stderr_log
    }

    /// Graceful shutdown: the client closes stdin (server drains accepted
    /// commands, disposes the session, exits 0), then escalates against
    /// the process group only if the leader does not exit. Dropping the
    /// runtime without closing performs the same teardown but cannot
    /// report failure.
    pub fn close(mut self) -> Result<(), HarnessError> {
        match self.client.take() {
            Some(client) => client.close().map_err(map_rpc_error),
            None => Ok(()),
        }
    }
}

fn map_rpc_error(error: omp_rpc::Error) -> HarnessError {
    match error {
        omp_rpc::Error::Io(error) => HarnessError::Io(error),
        other => HarnessError::Protocol(other.to_string()),
    }
}

/// Bounded tail read: at most the last 64 KiB are scanned for `lines`
/// lines, so a huge diagnostics file never loads whole.
fn read_stderr_tail(path: &Path, lines: usize) -> String {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return String::new(),
    };
    let len = file.metadata().map_or(0, |meta| meta.len());
    let start = len.saturating_sub(64 * 1024);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return String::new();
    }
    let mut text = String::new();
    if file.read_to_string(&mut text).is_err() {
        return String::new();
    }
    let kept: Vec<&str> = text.lines().rev().take(lines).collect();
    kept.into_iter().rev().collect::<Vec<_>>().join("\n")
}
