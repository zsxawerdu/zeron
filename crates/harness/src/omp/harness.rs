//! `OmpHarness`: the engine-facing driver over [`super::OmpRuntime`].
//!
//! One `run()` = one `omp --mode rpc-ui` process attached to one native
//! session (resumed by native session ID when the engine passes one). The
//! blocking client lives on a `spawn_blocking` thread; wire events are
//! normalized into the engine's display projection ([`super::normalize`])
//! while OMP native history remains the authoritative record.
//!
//! Settlement is runtime evidence only (handoff C3): the run ends `Done`
//! when the prompt's `prompt_result` reports `sessionSettled`, or a later
//! `session_settled` arrives — never on a bare `agent_end`, because
//! background work can still wake the session.
//!
//! Interim limits (owned by later packages, not hidden):
//! - Extension UI rides the legacy `request_input` bridge (W04 replaces
//!   it with full-fidelity interactions). A server `cancel` cannot retract
//!   an already-open legacy question; its late answer is ignored by OMP.
//! - Presentation UI requests (notify/status/widget/title/editor-text,
//!   open_url) are logged, not rendered (W04/W05).
//! - The process ends with the run; live reconfiguration across runs is a
//!   respawn+resume, so `reconfigures_in_place` stays false.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::{StreamExt, stream::BoxStream};
use omp_rpc::wire::{
    self, ExtensionUiRequest, PromptStatus, RpcNotification,
    ThinkingLevel as WireThinkingLevel,
};
use omp_rpc::{Event, RpcInbound};
use tokio::sync::{mpsc, oneshot};
use zeron_proto::omp::{OmpModelRef, OmpSlashCommand};
use zeron_proto::{
    AgentEvent, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest, SteeringMode,
    UserInputAnswer, UserInputQuestion,
};

use super::normalize::{extension_ui_request, Normalizer};
use super::{OmpExecutable, OmpLaunch, OmpRuntime, OmpSession};
use crate::{Harness, HarnessError, RunControls, SteerMessage};

type Input =
    dyn Fn(Vec<UserInputQuestion>) -> oneshot::Receiver<Vec<UserInputAnswer>> + Send + Sync;

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

fn next_generation() -> u64 {
    NEXT_GENERATION.fetch_add(1, Ordering::Relaxed)
}

#[derive(Default)]
pub struct OmpHarness {
    executable: Option<PathBuf>,
    profile: Option<String>,
    approval_mode: Option<super::OmpApprovalMode>,
}

impl OmpHarness {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_executable(mut self, path: impl Into<PathBuf>) -> Self {
        self.executable = Some(path.into());
        self
    }
    pub fn with_profile(mut self, profile: impl Into<String>) -> Self {
        self.profile = Some(profile.into());
        self
    }
    /// Session-scoped approval-mode override for every run of this
    /// driver instance. Test/probe use (e.g. tightening to
    /// `AlwaysAsk`); the registered product driver never sets it.
    pub fn with_approval_mode(mut self, mode: super::OmpApprovalMode) -> Self {
        self.approval_mode = Some(mode);
        self
    }
    fn executable(&self) -> Result<OmpExecutable, HarnessError> {
        OmpExecutable::resolve(self.executable.as_deref())
    }
    fn launch(&self, cwd: &Path, session: OmpSession) -> OmpLaunch {
        let mut launch = OmpLaunch::new(cwd);
        launch.profile = self.profile.clone();
        launch.approval_mode = self.approval_mode;
        launch.session = session;
        launch
    }
    /// Short-lived runtime for catalog discovery (ephemeral) or a
    /// read-only native-history query (resumed), on a blocking thread.
    /// Discovery fails (with the child's stderr) when no model is
    /// selectable — onboarding owns that state (W05).
    async fn probe<T, F>(&self, cwd: PathBuf, query: F) -> Result<T, HarnessError>
    where
        T: Send + 'static,
        F: FnOnce(&OmpRuntime) -> Result<T, HarnessError> + Send + 'static,
    {
        self.probe_session(cwd, OmpSession::Ephemeral, query).await
    }

    async fn probe_session<T, F>(
        &self,
        cwd: PathBuf,
        session: OmpSession,
        query: F,
    ) -> Result<T, HarnessError>
    where
        T: Send + 'static,
        F: FnOnce(&OmpRuntime) -> Result<T, HarnessError> + Send + 'static,
    {
        let executable = self.executable()?;
        let launch = self.launch(&cwd, session);
        tokio::task::spawn_blocking(move || {
            let runtime = OmpRuntime::spawn(executable, &launch, next_generation())?;
            let result = query(&runtime);
            let closed = runtime.close();
            result.and_then(|value| closed.map(|()| value))
        })
        .await
        .map_err(|error| HarnessError::Protocol(format!("OMP discovery task failed: {error}")))?
    }

    /// Owner-side full tool detail from OMP native history (handoff C4):
    /// attaches briefly to the (idle) native session, reads its entries,
    /// and returns the original call + result payloads. Absence is
    /// `Expired`; any runtime failure is `Unavailable` — detail is never
    /// reconstructed from the lossy projection.
    pub async fn tool_detail(
        &self,
        resume: &str,
        cwd: &Path,
        tool_call_id: &str,
    ) -> zeron_proto::omp::OmpToolDetail {
        use zeron_proto::omp::OmpToolDetail;
        let tool_call_id = tool_call_id.to_owned();
        let fetched = self
            .probe_session(
                cwd.to_path_buf(),
                OmpSession::Resume(PathBuf::from(resume)),
                move |runtime| {
                    let entries = runtime.call(&wire::GetEntriesCommand { since: None })?;
                    Ok(extract_tool_detail(&entries.entries, &tool_call_id))
                },
            )
            .await;
        match fetched {
            Ok(Some(native)) => OmpToolDetail::Available { native, revision: 0 },
            Ok(None) => OmpToolDetail::Expired,
            Err(error) => OmpToolDetail::Unavailable {
                reason: error.to_string(),
            },
        }
    }

    /// Full-fidelity model catalog (W02/C1/C6): provider-qualified
    /// descriptors with per-model thinking levels as opaque tokens and
    /// the raw native record preserved. [`Harness::models`] derives
    /// the lossy legacy picker shape from this.
    pub async fn omp_models(
        &self,
    ) -> Result<Vec<zeron_proto::omp::OmpModelDescriptor>, HarnessError> {
        let cwd = std::env::current_dir()?;
        self.probe(cwd, |runtime| {
            let discovered = runtime.call(&wire::GetAvailableModelsCommand {})?;
            let mut models = Vec::with_capacity(discovered.len());
            for model in &discovered {
                let levels = if model.reasoning {
                    runtime
                        .call(&wire::SetModelCommand {
                            provider: model.provider.clone(),
                            model_id: model.id.clone(),
                        })
                        .and_then(|_| {
                            runtime.call(&wire::GetAvailableThinkingLevelsCommand {})
                        })
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                models.push(zeron_proto::omp::OmpModelDescriptor {
                    model: OmpModelRef::new(model.provider.clone(), model.id.clone()),
                    label: model.name.clone(),
                    description: Some(model.provider.clone()),
                    reasoning: model.reasoning,
                    thinking_levels: levels
                        .iter()
                        .map(|level| {
                            zeron_proto::omp::OmpThinkingLevel::new(level.as_str())
                        })
                        .collect(),
                    input_modalities: model.input.clone(),
                    context_window: model.context_window,
                    max_output_tokens: model.max_tokens,
                    native: serde_json::to_value(model)
                        .unwrap_or(serde_json::Value::Null),
                });
            }
            Ok(models)
        })
        .await
    }

    /// Full-fidelity slash commands for `cwd` (W02): aliases,
    /// subcommands and source preserved. [`Harness::commands_for`]
    /// derives the lossy legacy projection from this.
    pub async fn omp_commands(
        &self,
        cwd: &Path,
    ) -> Result<Vec<zeron_proto::omp::OmpSlashCommand>, HarnessError> {
        self.probe(cwd.to_path_buf(), |runtime| {
            Ok(runtime
                .call(&wire::GetAvailableCommandsCommand {})?
                .iter()
                .map(super::normalize::omp_slash_command)
                .collect())
        })
        .await
    }

    /// Providers OMP can log in to and their auth state, straight from
    /// OMP (`get_login_providers`). Discovery only — no login is started.
    pub async fn login_providers(&self) -> Result<Vec<wire::LoginProvider>, HarnessError> {
        let cwd = std::env::current_dir()?;
        self.probe(cwd, |runtime| runtime.call(&wire::GetLoginProvidersCommand {}))
            .await
    }

    /// Stored credentials `logout` can remove for one provider, as OMP
    /// reports them. Labels/details only — never credential material,
    /// and Zeron never reads OMP's credential store directly.
    pub async fn logout_accounts(
        &self,
        provider_id: &str,
    ) -> Result<Vec<wire::LogoutAccount>, HarnessError> {
        let cwd = std::env::current_dir()?;
        let provider_id = provider_id.to_owned();
        self.probe(cwd, move |runtime| {
            runtime.call(&wire::GetLogoutAccountsCommand { provider_id })
        })
        .await
    }

    /// Fork a native session onto a new session file/id (OMP `fork`),
    /// optionally cutting at `entry_id`. Returns the NEW native session
    /// ID, or `None` when OMP reports the fork was cancelled (e.g. an
    /// extension vetoed it) — the original binding then stays valid.
    pub async fn fork_session(
        &self,
        resume: &str,
        cwd: &Path,
        entry_id: Option<&str>,
    ) -> Result<Option<String>, HarnessError> {
        let entry_id = entry_id.map(str::to_owned);
        self.probe_session(
            cwd.to_path_buf(),
            OmpSession::Resume(PathBuf::from(resume)),
            move |runtime| {
                let result = runtime.fork(entry_id.as_deref())?;
                if result.cancelled {
                    return Ok(None);
                }
                Ok(Some(runtime.state()?.session_id))
            },
        )
        .await
    }

    /// Discover existing native sessions for this driver's profile
    /// (read-only header scan; nothing is imported or attached).
    pub async fn discover_sessions(&self) -> Vec<super::OmpDiscoveredSession> {
        let profile = self.profile.clone();
        tokio::task::spawn_blocking(move || {
            let home = crate::executable::home_or_current_dir();
            let root = super::sessions_root(profile.as_deref(), &home);
            super::scan_sessions(&root, profile.as_deref())
        })
        .await
        .unwrap_or_default()
    }
}

/// Map one discovered model. `levels` are the runtime-reported thinking
/// levels for THIS model (queried after selecting it, like Pi);
/// `inherit`/`off` are not ladder rungs and are dropped here (the
/// thinking on/off option arrives with the W05 model hub).
fn wire_model(model: &wire::ModelInfo, levels: &[WireThinkingLevel]) -> Model {
    Model {
        id: format!("{}/{}", model.provider, model.id),
        label: model.name.clone(),
        description: Some(model.provider.clone()),
        reasoning_levels: levels
            .iter()
            .filter_map(|level| match level {
                WireThinkingLevel::Minimal => Some(ReasoningLevel::Minimal),
                WireThinkingLevel::Low => Some(ReasoningLevel::Low),
                WireThinkingLevel::Medium => Some(ReasoningLevel::Medium),
                WireThinkingLevel::High => Some(ReasoningLevel::High),
                WireThinkingLevel::Xhigh => Some(ReasoningLevel::XHigh),
                WireThinkingLevel::Max => Some(ReasoningLevel::Max),
                WireThinkingLevel::Inherit | WireThinkingLevel::Off => None,
            })
            .collect(),
        options: Vec::new(),
    }
}

/// Text of the last user-authored message in native session entries
/// (`get_entries` raw maps), skipping synthetic (OMP-injected) user
/// messages. Content may be a plain string or text blocks.
fn last_user_text(entries: &[serde_json::Map<String, serde_json::Value>]) -> Option<String> {
    entries.iter().rev().find_map(|entry| {
        if entry.get("type").and_then(serde_json::Value::as_str) != Some("message") {
            return None;
        }
        let message = entry.get("message")?;
        if message.get("role").and_then(serde_json::Value::as_str) != Some("user") {
            return None;
        }
        if message.get("synthetic").and_then(serde_json::Value::as_bool) == Some(true) {
            return None;
        }
        let content = message.get("content")?;
        if let Some(text) = content.as_str() {
            return Some(text.to_owned());
        }
        let blocks: Vec<&str> = content
            .as_array()?
            .iter()
            .filter_map(|block| block.get("text").and_then(serde_json::Value::as_str))
            .collect();
        (!blocks.is_empty()).then(|| blocks.join("\n"))
    })
}

/// Full native detail for one tool call from session entries: the
/// assistant `toolCall` block (name + original arguments) and the
/// `toolResult` message (content + details), preserved as-is (C4).
/// `None` when neither side exists in native history any more.
fn extract_tool_detail(
    entries: &[serde_json::Map<String, serde_json::Value>],
    tool_call_id: &str,
) -> Option<serde_json::Value> {
    let mut call = None;
    let mut result = None;
    for entry in entries {
        if entry.get("type").and_then(serde_json::Value::as_str) != Some("message") {
            continue;
        }
        let Some(message) = entry.get("message") else { continue };
        match message.get("role").and_then(serde_json::Value::as_str) {
            Some("assistant") => {
                if let Some(blocks) = message.get("content").and_then(serde_json::Value::as_array) {
                    for block in blocks {
                        if block.get("type").and_then(serde_json::Value::as_str) == Some("toolCall")
                            && block.get("id").and_then(serde_json::Value::as_str) == Some(tool_call_id)
                        {
                            call = Some(block.clone());
                        }
                    }
                }
            }
            Some("toolResult") => {
                if message.get("toolCallId").and_then(serde_json::Value::as_str) == Some(tool_call_id) {
                    result = Some(message.clone());
                }
            }
            _ => {}
        }
    }
    if call.is_none() && result.is_none() {
        return None;
    }
    let mut native = serde_json::Map::new();
    if let Some(call) = call {
        native.insert("call".into(), call);
    }
    if let Some(result) = result {
        native.insert("result".into(), result);
    }
    Some(serde_json::Value::Object(native))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entries(values: Vec<serde_json::Value>) -> Vec<serde_json::Map<String, serde_json::Value>> {
        values
            .into_iter()
            .map(|value| value.as_object().unwrap().clone())
            .collect()
    }

    #[test]
    fn last_user_text_skips_synthetic_and_reads_blocks() {
        let entries = entries(vec![
            json!({"type": "message", "message": {"role": "user", "content": "first"}}),
            json!({"type": "message", "message": {"role": "assistant", "content": []}}),
            json!({"type": "message", "message": {"role": "user", "synthetic": true, "content": "injected"}}),
            json!({"type": "message", "message": {"role": "user", "content": [{"type": "text", "text": "second"}]}}),
        ]);
        assert_eq!(last_user_text(&entries).as_deref(), Some("second"));
        assert_eq!(last_user_text(&[]), None);
    }

    #[test]
    fn thinking_token_maps_known_and_rejects_unknown() {
        assert_eq!(
            wire_thinking_token("xhigh"),
            Some(WireThinkingLevel::Xhigh)
        );
        assert_eq!(wire_thinking_token("inherit"), Some(WireThinkingLevel::Inherit));
        // A future token must not be silently substituted.
        assert_eq!(wire_thinking_token("ludicrous"), None);
    }

    #[test]
    fn extract_tool_detail_pairs_call_and_result() {
        let entries = entries(vec![
            json!({"type": "message", "message": {"role": "assistant", "content": [
                {"type": "text", "text": "reading"},
                {"type": "toolCall", "id": "tc1", "name": "read", "arguments": {"path": "seed.txt", "futureArg": 1}},
            ]}}),
            json!({"type": "message", "message": {"role": "toolResult", "toolCallId": "tc1", "toolName": "read", "content": [{"type": "text", "text": "contents"}], "details": {"lines": 3}}}),
            json!({"type": "message", "message": {"role": "toolResult", "toolCallId": "other", "content": []}}),
        ]);
        let detail = extract_tool_detail(&entries, "tc1").expect("detail");
        assert_eq!(detail["call"]["arguments"]["futureArg"], json!(1));
        assert_eq!(detail["result"]["details"]["lines"], json!(3));
        assert!(extract_tool_detail(&entries, "missing").is_none());
    }
}

/// Map an opaque thinking token onto the wire ladder. Unknown/future
/// tokens return `None`: requested ≠ effective, and Zeron must not
/// silently substitute a different level (C6).
fn wire_thinking_token(token: &str) -> Option<WireThinkingLevel> {
    Some(match token {
        "inherit" => WireThinkingLevel::Inherit,
        "off" => WireThinkingLevel::Off,
        "minimal" => WireThinkingLevel::Minimal,
        "low" => WireThinkingLevel::Low,
        "medium" => WireThinkingLevel::Medium,
        "high" => WireThinkingLevel::High,
        "xhigh" => WireThinkingLevel::Xhigh,
        "max" => WireThinkingLevel::Max,
        _ => return None,
    })
}

fn wire_thinking(level: ReasoningLevel) -> WireThinkingLevel {
    match level {
        ReasoningLevel::Minimal => WireThinkingLevel::Minimal,
        ReasoningLevel::Low => WireThinkingLevel::Low,
        ReasoningLevel::Medium => WireThinkingLevel::Medium,
        ReasoningLevel::High => WireThinkingLevel::High,
        ReasoningLevel::XHigh => WireThinkingLevel::Xhigh,
        // Legacy ladder rungs above OMP's ladder clamp to its top.
        ReasoningLevel::Max
        | ReasoningLevel::Ultra
        | ReasoningLevel::Ultracode
        | ReasoningLevel::Ultrathink => WireThinkingLevel::Max,
    }
}

#[async_trait]
impl Harness for OmpHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Omp
    }
    fn display_name(&self) -> &str {
        "OMP"
    }
    fn supports_steering(&self) -> bool {
        true
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::StepBoundary
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[]
    }
    fn installed(&self) -> bool {
        self.executable().is_ok()
    }
    fn executable_path(&self) -> Option<PathBuf> {
        self.executable().ok().map(|exe| exe.path)
    }
    fn deterministic_turn_end(&self) -> bool {
        true
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        let cwd = std::env::current_dir()?;
        self.probe(cwd, |runtime| {
            let discovered = runtime.call(&wire::GetAvailableModelsCommand {})?;
            let mut models = Vec::with_capacity(discovered.len());
            for model in &discovered {
                // Thinking levels are per-model: select, then ask (the
                // Pi driver's approach). A model that cannot be selected
                // (e.g. no credentials) reports no ladder rather than
                // failing the whole catalog.
                let levels = if model.reasoning {
                    runtime
                        .call(&wire::SetModelCommand {
                            provider: model.provider.clone(),
                            model_id: model.id.clone(),
                        })
                        .and_then(|_| {
                            runtime.call(&wire::GetAvailableThinkingLevelsCommand {})
                        })
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                models.push(wire_model(model, &levels));
            }
            Ok(models)
        })
        .await
    }
    fn model_context(&self) -> Result<Option<crate::ModelContext>, HarnessError> {
        crate::model_context::context(
            HarnessId::Omp,
            &self.executable()?.path,
            &[],
        )
        .map(Some)
    }
    async fn commands(&self) -> Result<Vec<zeron_proto::SlashCommand>, HarnessError> {
        self.commands_for(&std::env::current_dir()?).await
    }
    async fn commands_for(
        &self,
        cwd: &Path,
    ) -> Result<Vec<zeron_proto::SlashCommand>, HarnessError> {
        Ok(self
            .omp_commands(cwd)
            .await?
            .iter()
            .map(OmpSlashCommand::to_legacy)
            .collect())
    }
    async fn skills(
        &self,
        _cwd: &Path,
    ) -> Result<Option<Vec<zeron_proto::invocation::Skill>>, HarnessError> {
        // OMP advertises skills through runtime discovery (commands), not a
        // filesystem convention this driver may guess at.
        Ok(None)
    }
    async fn safe_to_redispatch(&self, resume: &str, cwd: &Path, prompt: &str) -> bool {
        // Verify against OMP native history: safe only when the last
        // user-authored native message provably is not this prompt. Any
        // failure to verify (spawn, protocol, shape) is unsafe.
        let prompt = prompt.trim().to_owned();
        let checked = self
            .probe_session(
                cwd.to_path_buf(),
                OmpSession::Resume(PathBuf::from(resume)),
                move |runtime| {
                    let entries = runtime.call(&wire::GetEntriesCommand { since: None })?;
                    Ok(match last_user_text(&entries.entries) {
                        // No user-authored message in native history: the
                        // prompt provably never landed.
                        None => true,
                        Some(last) => {
                            let last = last.trim();
                            last != prompt && !last.contains(&prompt)
                        }
                    })
                },
            )
            .await;
        match checked {
            Ok(safe) => safe,
            Err(error) => {
                tracing::warn!("OMP redispatch check failed: {error}");
                false
            }
        }
    }
    async fn run(
        &self,
        request: RunRequest,
        controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        let executable = self.executable()?;
        let cwd = PathBuf::from(&request.cwd);
        // The resume token is the OMP native session ID (`--resume` also
        // accepts a path, but a host filesystem path must never ride the
        // synced registry as another device's file handle — handoff C2).
        let resume = request
            .resume
            .clone()
            .filter(|token| !token.trim().is_empty());
        let (session, retry_fresh) = match &resume {
            Some(token) => (
                OmpSession::Resume(PathBuf::from(token)),
                Some(self.launch(&cwd, OmpSession::New)),
            ),
            None => (OmpSession::New, None),
        };
        let launch = self.launch(&cwd, session);
        let (tx, rx) = mpsc::channel(256);
        let RunControls {
            realtime: _,
            execution_lease: lease,
            request_input,
            mut steering,
            interrupt,
            turn,
        } = controls;
        let input: Arc<Input> = Arc::from(request_input);
        let (steer_tx, steer_rx) = std::sync::mpsc::channel::<SteerMessage>();
        tokio::spawn(async move {
            while let Some(message) = steering.recv().await {
                if steer_tx.send(message).is_err() {
                    break;
                }
            }
        });
        tokio::task::spawn_blocking(move || {
            let _lease = lease;
            run_blocking(
                executable,
                launch,
                retry_fresh,
                request,
                tx,
                steer_rx,
                interrupt,
                input,
                turn,
            );
        });
        Ok(
            futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|v| (v, rx)) })
                .boxed(),
        )
    }
}

fn send(tx: &mpsc::Sender<Result<AgentEvent, HarnessError>>, event: AgentEvent) -> bool {
    tx.blocking_send(Ok(event)).is_ok()
}

fn done(
    tx: &mpsc::Sender<Result<AgentEvent, HarnessError>>,
    status: DoneStatus,
    error: Option<String>,
    session_id: Option<String>,
) {
    let _ = send(
        tx,
        AgentEvent::Done {
            status,
            result: None,
            error,
            session_id,
        },
    );
}

/// The binding as OMP currently reports it (state + tree leaf, both
/// best-effort: a failed read leaves the field unset rather than
/// failing the run). `chat_id` stays empty — the engine stamps it.
fn current_binding(
    launch: &OmpLaunch,
    runtime: &OmpRuntime,
    state: Option<&wire::SessionState>,
) -> zeron_proto::omp::OmpSessionBinding {
    zeron_proto::omp::OmpSessionBinding {
        chat_id: String::new(),
        profile: launch.profile.clone(),
        session_id: state
            .map(|state| state.session_id.clone())
            .unwrap_or_default(),
        session_file: state.and_then(|state| state.session_file.clone()),
        leaf_id: runtime.tree().ok().and_then(|tree| tree.leaf_id),
        generation: runtime.generation(),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_blocking(
    executable: OmpExecutable,
    launch: OmpLaunch,
    retry_fresh: Option<OmpLaunch>,
    request: RunRequest,
    tx: mpsc::Sender<Result<AgentEvent, HarnessError>>,
    steering: std::sync::mpsc::Receiver<SteerMessage>,
    interrupt: crate::CancellationToken,
    input: Arc<Input>,
    turn: crate::TurnControl,
) {
    let spawned = OmpRuntime::spawn(executable.clone(), &launch, next_generation());
    let mut runtime = match (spawned, &retry_fresh) {
        (Ok(runtime), _) => runtime,
        (Err(error), Some(fresh)) => {
            // A resume token OMP can no longer open starts fresh instead of
            // stranding the chat; a fresh-start failure surfaces normally.
            tracing::warn!("OMP resume failed ({error}); starting a new session");
            match OmpRuntime::spawn(executable, fresh, next_generation()) {
                Ok(runtime) => runtime,
                Err(error) => {
                    done(&tx, DoneStatus::Errored, Some(error.to_string()), None);
                    return;
                }
            }
        }
        (Err(error), None) => {
            done(&tx, DoneStatus::Errored, Some(error.to_string()), None);
            return;
        }
    };
    let Some(events) = runtime.take_events() else {
        done(&tx, DoneStatus::Errored, Some("OMP event stream missing".into()), None);
        return;
    };
    // Session-only configuration; failures are surfaced, not fatal: OMP
    // resolves the effective value and reports it in state.
    if let Some(model) = request.model.as_deref().filter(|model| *model != "default") {
        match OmpModelRef::parse(model) {
            Some(model) => {
                if let Err(error) = runtime.call(&wire::SetModelCommand {
                    provider: model.provider,
                    model_id: model.model_id,
                }) {
                    tracing::warn!("OMP set_model failed: {error}");
                }
            }
            None => tracing::warn!("OMP model {model:?} is not provider-qualified; keeping session default"),
        }
    }
    if let Some(level) = request.reasoning {
        if let Err(error) = runtime.call(&wire::SetThinkingLevelCommand {
            level: wire_thinking(level),
        }) {
            tracing::warn!("OMP set_thinking_level failed: {error}");
        }
    }
    // OMP-native session config (W02/C6) takes precedence over the
    // legacy spellings above: provider-qualified model, opaque thinking
    // token, fast/slow modes — all session-only, OMP resolves effective.
    if let Some(omp) = &request.omp {
        if let Some(model) = &omp.model {
            if let Err(error) = runtime.call(&wire::SetModelCommand {
                provider: model.provider.clone(),
                model_id: model.model_id.clone(),
            }) {
                tracing::warn!("OMP set_model (omp config) failed: {error}");
            }
        }
        if let Some(level) = &omp.thinking_level {
            match wire_thinking_token(level.as_str()) {
                Some(level) => {
                    if let Err(error) =
                        runtime.call(&wire::SetThinkingLevelCommand { level })
                    {
                        tracing::warn!("OMP set_thinking_level (omp config) failed: {error}");
                    }
                }
                // Unknown/future token: requested state stays in the
                // config record; OMP keeps its effective level rather
                // than Zeron guessing a mapping.
                None => tracing::warn!(
                    level = %level,
                    "OMP thinking level is not a known wire token; keeping session effective level"
                ),
            }
        }
        if let Some(enabled) = omp.fast_mode {
            let _ = runtime.call(&wire::SetFastModeCommand { enabled });
        }
        if let Some(enabled) = omp.slow_mode {
            let _ = runtime.call(&wire::SetSlowModeCommand { enabled });
        }
    }
    let _ = runtime.call(&wire::SetAskDialogCommand { enabled: true });
    // Subagent lifecycle + nested session events (W09 routing in the
    // normalizer); without this OMP sends no subagent frames at all.
    let _ = runtime.call(&wire::SetSubagentSubscriptionCommand {
        level: wire::SubagentSubscriptionLevel::Events,
    });
    let state = runtime.state().ok();
    // The engine-facing resume token is the native session ID, never the
    // host-local session file path.
    let mut session_token = state.as_ref().map(|state| state.session_id.clone());
    let mut assistant = uuid::Uuid::new_v4().to_string();
    if !send(
        &tx,
        AgentEvent::SessionStarted {
            harness: HarnessId::Omp,
            model: state
                .as_ref()
                .and_then(|state| state.model.as_ref())
                .map(|model| format!("{}/{}", model.provider, model.id))
                .unwrap_or_default(),
            tools: Vec::new(),
            cwd: request.cwd.clone(),
            session_id: session_token.clone().unwrap_or_default(),
            assistant_message_id: assistant.clone(),
        },
    ) {
        return finish(runtime, &tx, DoneStatus::Interrupted, None, session_token);
    }
    // Rich binding for the engine's host-local store (C2): profile,
    // native file/leaf, and this process generation. Journaled
    // host-locally; never folded into synced docs.
    if !send(
        &tx,
        AgentEvent::OmpSessionBound {
            binding: current_binding(&launch, &runtime, state.as_ref()),
        },
    ) {
        return finish(runtime, &tx, DoneStatus::Interrupted, None, session_token);
    }
    if let Err(error) = runtime.call(&wire::PromptCommand {
        message: request.prompt.clone(),
        images: None,
        streaming_behavior: None,
    }) {
        return finish(runtime, &tx, DoneStatus::Errored, Some(error.to_string()), session_token);
    }

    let (outbound_tx, outbound_rx) = std::sync::mpsc::channel::<RpcInbound>();
    let mut normalizer = Normalizer::default();
    normalizer.set_window(
        state
            .as_ref()
            .and_then(|state| state.model.as_ref())
            .and_then(|model| model.context_window),
    );
    let mut prompt_result: Option<PromptStatus> = None;
    let mut prompt_error: Option<String> = None;
    let mut interrupted = false;
    loop {
        while let Ok(frame) = outbound_rx.try_recv() {
            let _ = runtime.send(&frame);
        }
        while let Ok(message) = steering.try_recv() {
            if let Err(error) = runtime.call(&wire::SteerCommand {
                message: message.prompt,
                images: None,
            }) {
                tracing::warn!("OMP steer failed: {error}");
            }
        }
        // Host-routed native queue actions (W03): this driver is the
        // only writer of the runtime's queue, so remove/promote and
        // snapshot requests are answered here, against the live
        // runtime — the runtime's own result is the reply, and any
        // failure answers false/None rather than a local guess.
        while let Some(action) = turn.take_queue_action() {
            use crate::QueueAction;
            match action {
                QueueAction::Remove { text, kind, reply } => {
                    let _ = reply.send(runtime.remove_queued_message(&text, kind).unwrap_or(false));
                }
                QueueAction::Promote { text, reply } => {
                    let _ = reply.send(runtime.promote_queued_message(&text).unwrap_or(false));
                }
                QueueAction::Snapshot { reply } => {
                    let _ = reply.send(runtime.queue_snapshot().ok());
                }
            }
        }
        if !interrupted && interrupt.is_cancelled() {
            interrupted = true;
            let _ = runtime.abort();
        }
        let event = match events.recv_timeout(Duration::from_millis(50)) {
            Ok(event) => event,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if let Some(request) = extension_ui_request(&event) {
            if let Some(presentation) = super::interactions::to_presentation(request) {
                if !send(&tx, AgentEvent::OmpPresentation { presentation }) {
                    let _ = runtime.abort();
                    return finish(runtime, &tx, DoneStatus::Interrupted, None, session_token);
                }
                continue;
            }
            handle_ui_request(
                request.clone(),
                runtime.generation(),
                &input,
                &outbound_tx,
                &interrupt,
            );
            continue;
        }
        if let Event::Notification(notification) = &event {
            match notification {
                RpcNotification::PromptResult(result) => {
                    prompt_result = Some(result.status);
                    if let Some(error) = &result.error {
                        prompt_error = Some(error.message.clone());
                        let _ = send(
                            &tx,
                            AgentEvent::Error {
                                message: error.message.clone(),
                            },
                        );
                    }
                    if result.session_settled {
                        break;
                    }
                    continue;
                }
                RpcNotification::SessionSettled(_) if prompt_result.is_some() => break,
                RpcNotification::RpcAgentEvent(wire::RpcAgentEvent::AgentEnd(_)) => {
                    // A yield boundary: close this assistant message so any
                    // background-work wake streams into a fresh one.
                    let _ = send(
                        &tx,
                        AgentEvent::AssistantMessageCompleted {
                            assistant_message_id: assistant.clone(),
                        },
                    );
                    assistant = uuid::Uuid::new_v4().to_string();
                    continue;
                }
                _ => {}
            }
        }
        for projected in normalizer.map(&event) {
            if !send(&tx, projected) {
                let _ = runtime.abort();
                return finish(runtime, &tx, DoneStatus::Interrupted, None, session_token);
            }
        }
    }
    // Refresh the binding with the post-turn leaf/file before Done.
    let final_state = runtime.state().ok();
    if let Some(state) = &final_state {
        session_token = Some(state.session_id.clone());
        let _ = send(
            &tx,
            AgentEvent::OmpSessionBound {
                binding: current_binding(&launch, &runtime, Some(state)),
            },
        );
    }
    let status = if interrupted || prompt_result == Some(PromptStatus::Aborted) {
        DoneStatus::Interrupted
    } else if prompt_result == Some(PromptStatus::Error) || prompt_result.is_none() {
        DoneStatus::Errored
    } else {
        DoneStatus::Completed
    };
    let error = prompt_error.or_else(|| {
        (prompt_result.is_none()).then(|| {
            let tail = runtime.stderr_tail(8);
            if tail.is_empty() {
                "OMP exited before the prompt settled".to_string()
            } else {
                format!("OMP exited before the prompt settled: {tail}")
            }
        })
    });
    finish(runtime, &tx, status, error, session_token);
}

fn finish(
    runtime: OmpRuntime,
    tx: &mpsc::Sender<Result<AgentEvent, HarnessError>>,
    status: DoneStatus,
    error: Option<String>,
    session_token: Option<String>,
) {
    let session_id = session_token;
    if let Err(close_error) = runtime.close() {
        tracing::warn!("OMP runtime close failed: {close_error}");
    }
    done(tx, status, error, session_id);
}

/// Bridge one extension-UI request through the legacy input questions on
/// its own thread (the event loop must keep draining). Cancellation —
/// engine closed, deadline hit, run interrupted — always answers
/// `cancelled`, never a value: an unanswered approval must not approve.
fn handle_ui_request(
    request: ExtensionUiRequest,
    generation: u64,
    input: &Arc<Input>,
    outbound: &std::sync::mpsc::Sender<RpcInbound>,
    interrupt: &crate::CancellationToken,
) {
    use zeron_proto::omp::{OmpInteractionKind, OmpInteractionResponse};
    let Some(interaction) = super::interactions::to_interaction(&request, generation) else {
        tracing::debug!("OMP extension UI presentation request not yet rendered: {request:?}");
        return;
    };
    if matches!(interaction.kind, OmpInteractionKind::Cancel { .. }) {
        // Server withdrew a dialog; the legacy bridge cannot retract an
        // open question (W04). Late answers are ignored by OMP.
        return;
    }
    let questions = super::interactions::to_questions(&interaction);
    let timeout_ms = interaction.timeout_ms;
    let input = input.clone();
    let outbound = outbound.clone();
    let interrupt = interrupt.clone();
    std::thread::spawn(move || {
        let mut answers_rx = (input)(questions);
        let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
        let mut timed_out = false;
        let answers: Option<Vec<UserInputAnswer>> = loop {
            match answers_rx.try_recv() {
                Ok(answers) => break Some(answers),
                Err(oneshot::error::TryRecvError::Closed) => break None,
                Err(oneshot::error::TryRecvError::Empty) => {
                    if interrupt.is_cancelled() {
                        break None;
                    }
                    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                        timed_out = true;
                        break None;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
            }
        };
        let response = match &answers {
            Some(answers) => super::interactions::from_answers(&interaction, answers),
            None => OmpInteractionResponse::Cancelled { timed_out },
        };
        let _ = outbound.send(RpcInbound::ExtensionUiResponse(
            super::interactions::to_wire_response(&interaction.request_id, response),
        ));
    });
}
