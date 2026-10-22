//! OMP domain types: the shared contracts (handoff C1–C8) between the
//! engine's OMP runtime owner and every client surface (desktop, mobile,
//! remote). These types describe OMP-owned state; Zeron renders and
//! controls it, never re-derives it.
//!
//! New OMP-facing code uses these types. The legacy multi-harness types in
//! [`crate::agent`] remain only until the W14 cutover removes their
//! runtimes; do not add OMP behavior to them, and never synthesize a
//! legacy harness id for an OMP session.

use serde::{Deserialize, Serialize};

/// Provider-qualified model identity (C6): a model is selected by
/// `provider` + `model_id` together. Serialized as `provider/model-id`
/// (the model id itself may contain further `/` segments; the provider is
/// always the first). Unknown/future models must round-trip through this
/// type rather than disappearing into a static enum (C1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpModelRef {
    pub provider: String,
    pub model_id: String,
}

impl OmpModelRef {
    pub fn new(provider: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model_id: model_id.into(),
        }
    }

    /// Parse `provider/model-id`. A bare model id without a provider
    /// segment is rejected: guessing a provider would silently select a
    /// different model than the user picked.
    pub fn parse(text: &str) -> Option<Self> {
        let (provider, model_id) = text.split_once('/')?;
        if provider.is_empty() || model_id.is_empty() {
            return None;
        }
        Some(Self::new(provider, model_id))
    }
}

impl std::fmt::Display for OmpModelRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.provider, self.model_id)
    }
}

/// Runtime capability snapshot (C1), client-facing slice: who the runtime
/// is and what transport was negotiated. Discovered commands/models/tools
/// ride their own catalog payloads; a capability counts as supported when
/// runtime discovery or the pinned docs advertise it — never merely
/// because a request could be serialized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpRuntimeInfo {
    /// Probed `omp --version` (e.g. `18.12.5`), when the probe succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// True when `version` is the release this build is qualified against.
    pub qualified: bool,
    /// Negotiated RPC protocol version (framing version, not a feature set).
    pub protocol_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_frame_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_reassembled_frame_bytes: Option<i64>,
}

/// Binding between an app chat and one OMP native session (C2). Persisted
/// by the owning engine; `session_file` is a host-local path and must not
/// be exposed to remote clients as a file handle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpSessionBinding {
    /// App-side chat identifier that displays this session.
    pub chat_id: String,
    /// OMP named profile the session belongs to; `None` = default profile.
    /// A binding never moves between profiles implicitly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// OMP native session id (from `get_state`).
    pub session_id: String,
    /// Native session file on the owning host, once OMP persists it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    /// Active tree leaf entry id, when known. Branch/fork/switch update
    /// the binding only after the runtime accepts the transition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leaf_id: Option<String>,
    /// Process generation that currently owns the session. Events stamped
    /// with an older generation are stale and must be ignored (C3).
    pub generation: u64,
}

/// Process lifecycle (C3). Agent activity, maintenance (retry/compaction),
/// pending interactions, and settlement are separate dimensions carried by
/// their own fields/events — never collapsed into this state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OmpProcessState {
    /// No runtime and none starting (not installed, or not yet launched).
    Unavailable,
    Starting,
    Ready,
    Closing,
    Failed,
}

/// Delivery state of one user prompt (C3). Transitions require runtime
/// evidence (admission ack, `prompt_result`, `session_settled`), never
/// timers or UI inference. `Uncertain` is the post-crash state: admission
/// may have happened, so the prompt must NOT be replayed blindly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OmpDeliveryState {
    /// Durable in the owning engine's queue, not yet sent.
    Queued,
    /// Sent to the runtime, admission not yet confirmed.
    Dispatching,
    /// Runtime confirmed admission (queued or running natively).
    Admitted,
    /// Runtime consumed/yielded the prompt (`prompt_result` observed).
    Consumed,
    /// Runtime refused the prompt.
    Rejected,
    /// Outcome unknown after a crash/disconnect; reconcile against
    /// native history before any resubmission.
    Uncertain,
}

/// How a user message is delivered to a live session (C3): a fresh prompt,
/// a steering message into the running turn, or a follow-up after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OmpPromptKind {
    #[default]
    Prompt,
    Steer,
    FollowUp,
}

/// OMP thinking level as an opaque token (`inherit`, `off`, `minimal`,
/// `low`, `medium`, `high`, `xhigh`, `max`, …). The available set is
/// runtime-discovered per model (`get_available_thinking_levels`), so this
/// is a string newtype, not a closed enum: a level this build has never
/// heard of must survive round-trips instead of being dropped (C1).
/// Requested vs effective level stays distinguishable at call sites (C6);
/// display controls (hide/expand reasoning) are presentation, never this.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OmpThinkingLevel(pub String);

impl OmpThinkingLevel {
    pub fn new(level: impl Into<String>) -> Self {
        Self(level.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for OmpThinkingLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Session-scoped execution selections (C6). These apply to one native
/// session only; persistent role assignments/defaults live in OMP config
/// and are managed through OMP (W05/W06), never copied into chat state.
/// `None` means "no session override" — OMP resolves the effective value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpSessionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<OmpModelRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<OmpThinkingLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fast_mode: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slow_mode: Option<bool>,
}

impl OmpSessionConfig {
    pub fn is_empty(&self) -> bool {
        self.model.is_none()
            && self.thinking_level.is_none()
            && self.fast_mode.is_none()
            && self.slow_mode.is_none()
    }

    /// Derive the OMP-native session config from the legacy chat-config
    /// spellings (`model` string + closed [`crate::ReasoningLevel`]
    /// ladder). This is the replacement path for the limited shapes:
    /// new producers populate `RunRequest::omp` through here (or set the
    /// fields directly) instead of relying on the legacy fields, which
    /// remain only for non-OMP runtimes until W14.
    ///
    /// Lossy edges are explicit, never silent substitutions: a model
    /// string that is not provider-qualified (including the picker
    /// sentinel `"default"`) yields no model override, and legacy ladder
    /// rungs above OMP's top (`Ultra`/`Ultracode`/`Ultrathink`) clamp to
    /// `max`, matching the harness's legacy wire mapping.
    pub fn from_legacy(model: Option<&str>, reasoning: Option<crate::ReasoningLevel>) -> Option<Self> {
        let config = Self {
            model: model
                .filter(|model| *model != "default")
                .and_then(OmpModelRef::parse),
            thinking_level: reasoning.map(|level| {
                OmpThinkingLevel::new(match level {
                    crate::ReasoningLevel::Minimal => "minimal",
                    crate::ReasoningLevel::Low => "low",
                    crate::ReasoningLevel::Medium => "medium",
                    crate::ReasoningLevel::High => "high",
                    crate::ReasoningLevel::XHigh => "xhigh",
                    crate::ReasoningLevel::Max
                    | crate::ReasoningLevel::Ultra
                    | crate::ReasoningLevel::Ultracode
                    | crate::ReasoningLevel::Ultrathink => "max",
                })
            }),
            fast_mode: None,
            slow_mode: None,
        };
        (!config.is_empty()).then_some(config)
    }
}

/// Full OMP model descriptor (C1/C6): provider-qualified identity,
/// runtime-discovered thinking levels as opaque tokens, modalities and
/// limits, plus the raw native record so unknown/future metadata
/// survives without being forced into presentation fields. The legacy
/// [`crate::Model`] shape derives from this lossily (unknown thinking
/// tokens drop there only; they stay here).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpModelDescriptor {
    pub model: OmpModelRef,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub reasoning: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thinking_levels: Vec<OmpThinkingLevel>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_modalities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<i64>,
    /// Raw native model record, preserved as-is (C4-style: unknown
    /// fields are data, not errors).
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub native: serde_json::Value,
}

impl OmpModelDescriptor {
    /// Lossy legacy projection for existing pickers: known thinking
    /// tokens map onto the legacy ladder, unknown ones drop.
    pub fn to_legacy(&self) -> crate::Model {
        crate::Model {
            id: self.model.to_string(),
            label: self.label.clone(),
            description: self.description.clone(),
            reasoning_levels: self
                .thinking_levels
                .iter()
                .filter_map(|level| match level.as_str() {
                    "minimal" => Some(crate::ReasoningLevel::Minimal),
                    "low" => Some(crate::ReasoningLevel::Low),
                    "medium" => Some(crate::ReasoningLevel::Medium),
                    "high" => Some(crate::ReasoningLevel::High),
                    "xhigh" => Some(crate::ReasoningLevel::XHigh),
                    "max" => Some(crate::ReasoningLevel::Max),
                    _ => None,
                })
                .collect(),
            options: Vec::new(),
        }
    }
}

/// Owner-side full-detail fetch for one tool call (C4). The transcript
/// projection carries summaries plus this key; full input/output/diff
/// stays in OMP native history on the owning host and is fetched lazily,
/// authorized by the owner — never replicated wholesale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpToolDetailRequest {
    pub chat_id: String,
    pub tool_call_id: String,
    /// Native entry id when the projection stamped one; otherwise the
    /// owner resolves the entry by tool call id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
    /// Binding generation the caller's projection was rendered from
    /// (revision check, C2/C4). When present and different from the
    /// owner's current generation, the projection is stale: the owner
    /// answers `Expired` without fetching, and the caller re-reads the
    /// session instead of rendering detail against a moved session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
}

/// Outcome of a detail fetch. `Unavailable`/`Expired`/`Forbidden` are
/// explicit terminal states the UI renders as-is; detail is never
/// reconstructed or invented when the native record is gone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum OmpToolDetail {
    #[serde(rename_all = "camelCase")]
    Available {
        /// The native tool-call/result payload, preserved as-is.
        /// Presentation layers pick renderable fields; unknown fields
        /// survive for generic inspection (C4).
        native: serde_json::Value,
        /// Owner binding generation that served this detail (revision
        /// echo): a caller whose projection has since moved past it
        /// treats the detail as stale. 0 from the harness layer; the
        /// owning engine stamps its binding generation.
        #[serde(default)]
        revision: u64,
    },
    /// The native session or entry no longer exists (deleted/compacted
    /// beyond reach).
    Expired,
    /// The owning host is offline or the fetch failed; retry may succeed.
    #[serde(rename_all = "camelCase")]
    Unavailable { reason: String },
    /// The requester is not authorized for this chat's details.
    Forbidden,
}

/// One slash-command subcommand as OMP advertises it (W02/W07).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpSlashSubcommand {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Argument usage hint, when OMP gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
}

/// A slash command with full OMP fidelity (W02): aliases, subcommands
/// and source survive this projection. The legacy
/// [`crate::SlashCommand`] shape carries only name/description/hint;
/// converting via [`OmpSlashCommand::to_legacy`] is the documented
/// lossy boundary until W07 renders this type directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpSlashCommand {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subcommands: Vec<OmpSlashSubcommand>,
    /// Wire source token (`builtin`/`skill`/`extension`/`custom`/
    /// `mcp_prompt`/`file`); opaque so future sources round-trip (C1).
    pub source: String,
}

impl OmpSlashCommand {
    /// Lossy legacy projection: aliases/subcommands/source are dropped.
    pub fn to_legacy(&self) -> crate::SlashCommand {
        crate::SlashCommand {
            name: self.name.clone(),
            description: self.description.clone(),
            input_hint: self.input_hint.clone(),
        }
    }
}

/// Snapshot of OMP's native steering/follow-up queue (`get_state`'s
/// queued-messages view, surfaced by `queue_update` events). Texts only:
/// the runtime gives queued items no IDs, so reconciliation against
/// local rows is by in-order text pairing within each list (C3) —
/// identical duplicate texts are fundamentally ambiguous there and must
/// be reported as such, never silently paired (handoff W03).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpQueueSnapshot {
    #[serde(default)]
    pub steering: Vec<String>,
    #[serde(default)]
    pub follow_up: Vec<String>,
    /// Leading `steering` entries already live-steered into the running
    /// response; native queue removal cannot reach them.
    #[serde(default)]
    pub live_steered: u32,
}

/// One selectable option of an interaction, with its OMP-supplied
/// description/preview intact (C5 — never flattened to a bare label).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpInteractionOption {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

/// One question of a rich `ask` interaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpInteractionQuestion {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    pub question: String,
    #[serde(default)]
    pub options: Vec<OmpInteractionOption>,
    #[serde(default)]
    pub multi: bool,
    /// Index into `options` of the recommended choice, when OMP gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended: Option<u32>,
}

/// What kind of user interaction OMP is waiting on (C5). `Cancel` closes
/// the interaction named by `target_id`; a late answer to it is invalid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum OmpInteractionKind {
    #[serde(rename_all = "camelCase")]
    Select { options: Vec<OmpInteractionOption> },
    Confirm,
    #[serde(rename_all = "camelCase")]
    Input {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Editor {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prefill: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Ask { questions: Vec<OmpInteractionQuestion> },
    #[serde(rename_all = "camelCase")]
    Cancel { target_id: String },
}

/// A pending OMP interaction (approval, question, editor) (C5). The owner
/// resolves it exactly once: competing-view answers, stale generations,
/// cancellation, deadline expiry, and host shutdown all resolve
/// deterministically, and an unresolved approval is never an approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpInteraction {
    /// Runtime request id; answers must echo it.
    pub request_id: String,
    /// Process generation that issued it; answers from another
    /// generation are stale and rejected.
    pub generation: u64,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub kind: OmpInteractionKind,
    /// Runtime deadline in milliseconds, when OMP set one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

/// One answer inside an `ask` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpInteractionAnswer {
    pub question_id: String,
    #[serde(default)]
    pub selected_options: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_input: Option<String>,
}

/// The single accepted response to an [`OmpInteraction`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "response", rename_all = "kebab-case")]
pub enum OmpInteractionResponse {
    #[serde(rename_all = "camelCase")]
    Value { value: String },
    #[serde(rename_all = "camelCase")]
    Confirmed { confirmed: bool },
    #[serde(rename_all = "camelCase")]
    Answers { answers: Vec<OmpInteractionAnswer> },
    #[serde(rename_all = "camelCase")]
    Cancelled {
        #[serde(default)]
        timed_out: bool,
    },
}

/// Severity of an extension notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OmpNotifyKind {
    Info,
    Warning,
    Error,
}

/// Fire-and-forget extension presentation (C5-adjacent): notifications,
/// status/widget text, title, composer text, and external URLs. These
/// never block the agent and never enter the synced transcript; the
/// owning engine surfaces them transiently. `OpenUrl` URLs are
/// scheme-validated at conversion time (http/https only) — an extension
/// must not be able to launch arbitrary local schemes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "presentation", rename_all = "kebab-case")]
pub enum OmpPresentation {
    #[serde(rename_all = "camelCase")]
    Notify {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<OmpNotifyKind>,
    },
    #[serde(rename_all = "camelCase")]
    Status {
        key: String,
        /// `None` clears this status key.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Widget {
        key: String,
        /// `None` clears this widget.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lines: Option<Vec<String>>,
    },
    #[serde(rename_all = "camelCase")]
    Title { title: String },
    #[serde(rename_all = "camelCase")]
    EditorText { text: String },
    #[serde(rename_all = "camelCase")]
    OpenUrl {
        url: String,
        /// Short loopback redirect to `url` (login flows); preferred
        /// copy/open target when present.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        launch_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        instructions: Option<String>,
    },
}

/// One provider OMP can authenticate against (W05), as reported by
/// `get_login_providers`. Discovery only: no credential material ever
/// rides this type, and Zeron never reads OMP's credential store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpLoginProvider {
    pub id: String,
    pub name: String,
    /// The provider can be used at all (configured/reachable upstream).
    pub available: bool,
    /// A stored credential currently authenticates this provider.
    pub authenticated: bool,
}

/// One stored credential `logout` can remove (W05), as reported by
/// `get_logout_accounts`. Labels/details only — never secrets.
/// `kind` is the native token (`api_key`/`oauth`); unknown future
/// kinds survive as-is rather than being dropped (C1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpLogoutAccount {
    pub credential_id: i64,
    pub provider: String,
    pub label: String,
    pub detail: String,
    pub kind: String,
    /// The credential the current session may be using.
    pub active: bool,
}

/// Outcome of an OMP `logout` (W05). `remaining_source` names auth
/// that still applies after removal (e.g. an env-var key), when OMP
/// reports one; `None` means nothing remains for that provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpLogoutOutcome {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_source: Option<String>,
}

/// Onboarding readiness (W05): can this device run OMP, and is any
/// provider authenticated so a first session can select a model?
/// Computed by the owning engine from executable probing plus OMP's
/// own provider discovery — never from a fabricated static list.
/// When discovery fails, `providers` is empty and `discovery_error`
/// carries the actionable cause (missing executable, no selectable
/// model, …) for the onboarding surface to show verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpReadiness {
    /// Probed `omp --version`, when the executable resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// True when `version` is this build's qualified release.
    pub qualified: bool,
    pub providers: Vec<OmpLoginProvider>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery_error: Option<String>,
}

impl OmpReadiness {
    /// A first turn is possible: the runtime is qualified and at
    /// least one provider is authenticated.
    pub fn ready_for_first_turn(&self) -> bool {
        self.qualified && self.providers.iter().any(|p| p.authenticated)
    }
}

/// Persistent model-role configuration (W05), read from OMP's
/// effective config (`modelRoles`, `cycleOrder`,
/// `retry.fallbackChains`, `defaultThinkingLevel`,
/// `modelRoleStorage`). This is the persistent layer: assigning a
/// role here never changes a live session's model — session-only
/// switching is the composer picker's `OmpSessionConfig` path, and
/// the two must stay visibly separate. Selectors are kept verbatim
/// (`provider/model-id`, or `@role` for automatic resolution through
/// another role); unknown/custom roles survive round-trips (C1).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpModelRoles {
    /// Every role the hub offers: OMP's built-ins first, then custom
    /// roles discovered in assignments/cycle order, de-duplicated.
    pub roles: Vec<String>,
    /// Role → selector, only for roles with an explicit assignment.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub assignments: std::collections::BTreeMap<String, String>,
    /// Ctrl+P-style cycling order over role names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cycle_order: Vec<String>,
    /// Fallback chains keyed by role, `provider/model-id`, or
    /// `provider/*`, each an ordered selector list.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub fallback_chains: std::collections::BTreeMap<String, Vec<String>>,
    /// Persistent default thinking level token, when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_thinking_level: Option<String>,
    /// Where OMP saves role assignments (`global`/`project`), as reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<String>,
}

/// Model catalog snapshot for the hub (W05): runtime-discovered
/// descriptors, or the last-known version/profile-scoped cache when
/// live discovery fails. `stale` + `error` are explicit UI states —
/// a stale catalog is labeled, never silently presented as live,
/// and an unavailable one explains itself instead of falling back
/// to a fabricated static list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpCatalogSnapshot {
    pub models: Vec<OmpModelDescriptor>,
    pub stale: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Provider usage-limit reports (W05) from `omp usage --json`.
/// Report internals are provider-shaped and evolve upstream, so
/// they ride as raw native records (C4-style); Zeron renders the
/// summary fields it knows and never invents quota numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpProviderLimits {
    pub generated_at_ms: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reports: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accounts_without_usage: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_credentials: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub capacity: serde_json::Value,
}

/// State of one non-chat OMP login flow (W05 onboarding): the engine
/// runs `OmpHarness::login` in the background and surfaces its
/// browser URL and questions through polling, because a login has
/// no chat to carry the question bridge. Terminal states are
/// `done`/`failed`/`cancelled`; an unanswered login is never an
/// approval of anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OmpLoginStatus {
    Running,
    AwaitingInput,
    Done,
    Failed,
    Cancelled,
}

/// Poll reply for one login flow. `questions` is present exactly in
/// `awaiting-input`; answers go back through `OmpLoginAnswer` with
/// the same question ids. `open_url` is the latest browser URL the
/// flow produced (http/https-gated before it reaches here).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpLoginPoll {
    pub login_id: String,
    pub provider_id: String,
    pub status: OmpLoginStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<crate::UserInputQuestion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_ref_round_trips_provider_qualified_ids() {
        let model = OmpModelRef::parse("openai/gpt-5.2").unwrap();
        assert_eq!(model.provider, "openai");
        assert_eq!(model.model_id, "gpt-5.2");
        assert_eq!(model.to_string(), "openai/gpt-5.2");
        // Model ids may themselves contain segments.
        let nested = OmpModelRef::parse("muse-code/muse/wonder").unwrap();
        assert_eq!(nested.provider, "muse-code");
        assert_eq!(nested.model_id, "muse/wonder");
        assert!(OmpModelRef::parse("bare-model").is_none());
        assert!(OmpModelRef::parse("/x").is_none());
        assert!(OmpModelRef::parse("x/").is_none());
    }

    #[test]
    fn session_config_from_legacy_maps_only_qualified_models() {
        assert_eq!(OmpSessionConfig::from_legacy(None, None), None);
        assert_eq!(OmpSessionConfig::from_legacy(Some("default"), None), None);
        // Unqualified model strings never fabricate a provider.
        assert_eq!(OmpSessionConfig::from_legacy(Some("bare"), None), None);
        let config = OmpSessionConfig::from_legacy(
            Some("openai/gpt-5.2"),
            Some(crate::ReasoningLevel::XHigh),
        )
        .unwrap();
        assert_eq!(config.model, OmpModelRef::parse("openai/gpt-5.2"));
        assert_eq!(config.thinking_level, Some(OmpThinkingLevel::new("xhigh")));
        // Above-ladder legacy rungs clamp to OMP's top, like the wire map.
        let clamped = OmpSessionConfig::from_legacy(None, Some(crate::ReasoningLevel::Ultra)).unwrap();
        assert_eq!(clamped.thinking_level, Some(OmpThinkingLevel::new("max")));
    }

    #[test]
    fn interaction_and_detail_serde_round_trip() {
        let interaction = OmpInteraction {
            request_id: "r1".into(),
            generation: 3,
            title: "Approve?".into(),
            message: Some("run bash".into()),
            kind: OmpInteractionKind::Ask {
                questions: vec![OmpInteractionQuestion {
                    id: "q1".into(),
                    header: None,
                    question: "Pick".into(),
                    options: vec![OmpInteractionOption {
                        label: "A".into(),
                        description: Some("first".into()),
                        preview: None,
                    }],
                    multi: false,
                    recommended: Some(0),
                }],
            },
            timeout_ms: Some(30_000),
        };
        let json = serde_json::to_value(&interaction).unwrap();
        assert_eq!(json["kind"]["kind"], "ask");
        assert_eq!(
            serde_json::from_value::<OmpInteraction>(json).unwrap(),
            interaction
        );
        let detail = OmpToolDetail::Available {
            native: serde_json::json!({"futureField": [1, 2]}),
            revision: 3,
        };
        let json = serde_json::to_value(&detail).unwrap();
        assert_eq!(json["outcome"], "available");
        assert_eq!(serde_json::from_value::<OmpToolDetail>(json).unwrap(), detail);
    }

    #[test]
    fn model_descriptor_preserves_unknown_metadata_and_levels() {
        let descriptor = OmpModelDescriptor {
            model: OmpModelRef::parse("prov/future-1").unwrap(),
            label: "Future".into(),
            description: None,
            reasoning: true,
            thinking_levels: vec![
                OmpThinkingLevel::new("high"),
                OmpThinkingLevel::new("ludicrous"), // unknown token survives
            ],
            input_modalities: vec!["text".into(), "image".into()],
            context_window: Some(200_000),
            max_output_tokens: None,
            native: serde_json::json!({"futureField": {"x": 1}}),
        };
        let json = serde_json::to_value(&descriptor).unwrap();
        assert_eq!(
            serde_json::from_value::<OmpModelDescriptor>(json).unwrap(),
            descriptor
        );
        // Legacy projection keeps only known ladder rungs.
        assert_eq!(
            descriptor.to_legacy().reasoning_levels,
            vec![crate::ReasoningLevel::High]
        );
        assert_eq!(descriptor.to_legacy().id, "prov/future-1");
    }

    #[test]
    fn slash_command_fidelity_round_trips_and_legacy_projection_is_lossy() {
        let cmd = OmpSlashCommand {
            name: "plan".into(),
            description: "Plan".into(),
            input_hint: Some("<goal>".into()),
            aliases: vec!["p".into()],
            subcommands: vec![OmpSlashSubcommand {
                name: "show".into(),
                description: None,
                usage: Some("[id]".into()),
            }],
            source: "builtin".into(),
        };
        let json = serde_json::to_value(&cmd).unwrap();
        assert_eq!(json["aliases"], serde_json::json!(["p"]));
        assert_eq!(json["source"], "builtin");
        assert_eq!(serde_json::from_value::<OmpSlashCommand>(json).unwrap(), cmd);
        let legacy = cmd.to_legacy();
        assert_eq!(legacy.name, "plan");
        assert_eq!(legacy.input_hint.as_deref(), Some("<goal>"));
    }

    #[test]
    fn readiness_requires_qualified_runtime_and_an_authenticated_provider() {
        let base = OmpReadiness {
            version: Some("18.12.5".into()),
            qualified: true,
            providers: vec![OmpLoginProvider {
                id: "anthropic".into(),
                name: "Anthropic".into(),
                available: true,
                authenticated: false,
            }],
            discovery_error: None,
        };
        assert!(!base.ready_for_first_turn());
        let mut ready = base.clone();
        ready.providers[0].authenticated = true;
        assert!(ready.ready_for_first_turn());
        // An unqualified runtime never counts, however many logins exist.
        let mut unqualified = ready.clone();
        unqualified.qualified = false;
        assert!(!unqualified.ready_for_first_turn());
        // Account types round-trip; unknown credential kinds survive.
        let account = OmpLogoutAccount {
            credential_id: 7,
            provider: "anthropic".into(),
            label: "work".into(),
            detail: "oauth".into(),
            kind: "future-kind".into(),
            active: true,
        };
        let json = serde_json::to_value(&account).unwrap();
        assert_eq!(json["credentialId"], 7);
        assert_eq!(serde_json::from_value::<OmpLogoutAccount>(json).unwrap(), account);
        let outcome = OmpLogoutOutcome { remaining_source: None };
        assert_eq!(
            serde_json::from_value::<OmpLogoutOutcome>(serde_json::to_value(&outcome).unwrap())
                .unwrap(),
            outcome
        );
    }

    #[test]
    fn tool_detail_outcomes_are_explicit_and_distinct() {
        for (value, tag) in [
            (OmpToolDetail::Expired, "expired"),
            (OmpToolDetail::Forbidden, "forbidden"),
            (OmpToolDetail::Unavailable { reason: "offline".into() }, "unavailable"),
        ] {
            let json = serde_json::to_value(&value).unwrap();
            assert_eq!(json["outcome"], tag);
            assert_eq!(serde_json::from_value::<OmpToolDetail>(json).unwrap(), value);
        }
    }
}
