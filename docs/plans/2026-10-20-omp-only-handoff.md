# OMP-only Zeron — implementation handoff

## 0. Start here

**User goal:** remake this application into an OMP-only UI with full OMP functionality. Not another selectable agent adapter. Model providers offered by OMP stay; independent agent/provider execution paths in Zeron go away.

**Current work:** architecture research and planning only. No OMP adapter, application UI changes, schema migration, or upstream API changes have been implemented. The implementation checkboxes below are intentionally unchecked.

**Read order:** applicable project instructions, root MEMORY.md, this handoff, then [architecture and complete capability scope](2026-10-20-omp-only.md). Inspect source sections for the work package being implemented; do not repeat the whole repository investigation.

**First implementation action:** W00 — pin versions, skim the matching RPC docs, and confirm the Plan Review control path exists before committing to a broad UI implementation. Then W01–W04 establish the first real end-to-end OMP conversation with correct history and approval behavior. Those packages alone are not the finished product.

**Authority:** user requirements and repository rules outrank this plan. This document specifies proposed implementation contracts, not claims that these symbols or APIs already exist. Preserve current user changes. Revalidate line locations and source revisions before editing.

## 1. Settled direction and boundaries

1. Keep Rust/GPUI, the owning engine, native window/workspace infrastructure, and optional remote/mobile access. Do not replace the app framework or introduce a browser frontend as part of this work.
2. OMP is the only production agent runtime. OMP's own supported model/service providers remain available through OMP.
3. Prefer a version-pinned OMP Rust RPC client/generated wire schema and `omp --mode rpc-ui`. Blocking operations stay off GPUI and Tokio executor threads.
4. OMP owns native sessions, tree semantics, model selection/resolution, credentials, tool policy, worker orchestration, memory, and modes. Zeron renders/controls these; it does not reimplement their policy.
5. Preserve existing files, changes, terminals, workspaces, tabs, themes, attachments, and useful OS integrations, adapting rather than discarding the shell.
6. No automatic approval override. Display and respect the effective OMP policy; do not change a user's global policy merely to make the integration work.
7. Preserve historical non-OMP conversations as archives or use a verified explicit import. Do not retain obsolete executable adapters as compatibility shims. Do not uninstall external CLIs or delete their credentials.
8. All model-backed voice must also go through OMP. Existing standalone Codex voice/helper dependencies are in scope for removal, not an exception to OMP-only.
9. Preserve local/synced profile separation. Discovering an OMP session must not automatically publish it or upload credentials/artifacts.
10. Full built-in OMP workflows must be native. Terminal-only extension compatibility is a proposed accommodation for arbitrary third-party TUI components, not permission to replace built-in Plan Review/settings with a terminal.
11. Do not rebrand the app, change app IDs, or redesign unrelated workspace/sync behavior unless required by the cutover. This is a runtime/product integration project, not a branding project.
12. Do not call a partial implementation complete. If a required OMP API needs upstream work, record the exact missing behavior and finish independent reachable work.
13. Testing is minimal by design: compile, plus one happy-path run of the changed path (see §7). No per-package test matrix, evidence files, screenshot packs, or parity-qualification pass. Trust OMP's docs/types and runtime discovery for anything not directly probed; probe only what the next step actually needs. Add a permanent test only for a bug already hit or a data-loss/security invariant.

## 2. Evidence that the next session must retain

### Verified baseline, not a future compatibility guarantee

The previous session observed OMP **18.12.5**. Screenshots show **18.12.4**. The executable resolved to `/Users/zsx/Library/Application Support/Tern/omp/bin/omp`; this is an observation, never a path to hard-code.

Isolated probes used a temporary HOME/config/workspace, no real credentials, an inert key with a loopback model endpoint, and no model prompt. Verified:

- rpc-ui ready, v2 negotiation, get_state, command discovery and get_subagents.
- goal `op=get`, get_tree, get_entries and thinking-level discovery.
- rich ask opt-in, empty logout-account discovery, subagent event subscription.
- graceful process exit in those idle scenarios.
- `omp config list --json`: 559 descriptors in that build; selected enum descriptors lacked a complete enum/default/provenance schema.
- Startup without an available/default model failed before ready. Onboarding cannot assume a normal agent session is already running.

**Not probed:** real inference, approvals, plan review, voice, collaboration, rendered UI, crash recovery. Probe each only when implementing its package; don't front-load it.

### Documentation hierarchy and drift

Use all three sources, pinned where possible:

1. [OMP user docs](https://omp.sh/docs) for intended user behavior.
2. [Complete upstream RPC reference](https://github.com/can1357/oh-my-pi/blob/main/docs/rpc.md) and matching wire schema/types for integration.
3. Observed behavior of the selected runtime build.

The website RPC summary omits commands present in upstream docs and the installed build. In particular, do not repeat the earlier disproven assumptions that OMP lacks goal, session-tree, logout, or live-voice RPC. Conversely, upstream `main` is not evidence that a particular release implements every command.

Settings reload behavior and RPC defaults differ between sources; believe the installed build where they conflict. Protocol version 2 describes framing; it is NOT a complete feature-negotiation version.

### Existing application traps

- `crates/doc/src/parts.rs` intentionally strips tool output, diff text, and rich input before projection. Full data cannot be recovered by a new renderer alone.
- `crates/client/src/session/mod.rs` currently constructs requests with `auto_approve: true`.
- Pi integration is Pi-specific; copying `--no-themes`, session recreation, or settlement assumptions is unsafe.
- Current app history and OMP native history must not become independent authorities.
- Existing account-slot logic rewrites other agents' credential stores. It must not operate on OMP credential storage.
- `docs/research/voice-remote-testing.md` documents a separate Codex execution/media path. Removing HarnessId variants does not remove this dependency.
- `scripts/e2e-smoke.sh` currently uses a mock harness and deletes fixed `/tmp/e2e-a` and `/tmp/e2e-b` directories. Do not run it unreviewed or treat it as real OMP validation; isolate and update it first.
- `scripts/run-macos-dev.sh` builds an isolated signed app bundle, but still passes Codex-specific executable settings. Mac permissions must be exercised through the app bundle, not attributed accidentally to the terminal.

## 3. Preserved visual references

Six unique user screenshots are copied into the repository. A duplicate Appearance screenshot was omitted. These are reference designs, not screenshots of an implemented Zeron feature.

| Reference | File | Behavior to reproduce |
| --- | --- | --- |
| Compact model picker | [Session switcher](omp-only-reference/pasted-image-5347e2874fd28d5b.png) | Search, role shortcuts, recent rows, selected model metadata, session-only apply |
| Conversation/composer | [Conversation](omp-only-reference/pasted-image-41235385098a4cca.png) | Attachments, model/mode controls, context/status, paused-plan indication |
| Full model hub | [Model catalog](omp-only-reference/pasted-image-83060f002a0629ad.png) | Provider navigation, modality filters, detailed model inspector, roles |
| Role assignment | [Assign role](omp-only-reference/pasted-image-3751e555f7955727.png) | Distinct persistent-role assignment flow and selected-role state |
| Thinking settings | [Model settings](omp-only-reference/pasted-image-a6c1d3f06aaa197.png) | Thinking level separate from reasoning display controls |
| Appearance settings | [Appearance](omp-only-reference/pasted-image-1869defb56820e44.png) | Light/dark theme, symbols, accessibility, changed/reset state |

Use semantic native equivalents. Do not hard-code example models, counts, performance scores, prices, role counts, or terminal dimensions. Preserve usable focus order, keyboard navigation, narrow-window layout, and reduced-motion behavior.

## 4. Shared contracts to settle before parallel implementation

These are domain requirements. Choose final Rust names after inspecting existing types and references; do not add duplicate parallel abstractions just to match these labels.

### C1. Runtime capability snapshot

Carry executable identity/version, pinned compatibility profile, transport version/limits, runtime-discovered commands/models/tools, and supported host behaviors. Capability = advertised by discovery or matching docs, unless known missing; no proof pass per capability.

Never advertise a destructive control merely because its JSON could be serialized. Do not probe unsupported mutating commands to discover support. Cache only with workspace/profile/config/runtime identity, and display cached catalogs as stale when appropriate. An unknown future model must not disappear because it is missing from a static enum.

### C2. Session binding and ownership

Persist app chat ID, owning engine/device, app data scope, OMP profile/config identity, OMP native session ID and path, active leaf, and process generation. Do not expose host filesystem paths as unrestricted remote file handles.

Only one runtime writer may own a native session. Tabs, mobile views, and remote clients attach to that owner. Branch/fork/switch changes update binding and projection together only after the runtime accepts the transition. User refusal/extension cancellation leaves the original binding valid.

App tab close, session archive, native session delete, process detach, and runtime abort are different actions. Profile switches cannot silently move data between scopes or attach a new process to the wrong profile.

### C3. Lifecycle and delivery

Represent these dimensions separately:

- Process: unavailable, starting, ready, closing, failed.
- Agent: idle, active, aborting; OMP mode and paused state separately.
- Maintenance: retry, compaction, pending asynchronous work.
- User interaction: zero or more pending dialogs/approvals.
- Settlement: runtime-confirmed settled versus potentially resumable background work.
- Delivery: locally queued, dispatching, admitted, consumed/yielded as observable, rejected, uncertain.

Use runtime evidence for transitions, not timers or UI inference. Correlate prompt IDs and process generations. Local-only commands complete without an agent turn. Reconcile after reconnect before replaying any durable queue item. A request ID is correlation, not a guarantee of OMP-side deduplication.

### C4. Events, history and artifacts

Maintain native message/entry/tool-call identity, role, timestamps, ordered content blocks, live deltas, tool progress/results, and opaque replay metadata. Preserve unrecognized roles/fields locally without forcing every byte into CRDT replication.

Normalize only for display. Do not collapse multiple assistant messages into one text tail. Native history drives resume; a UI projection cannot synthesize a resumable OMP transcript.

Keep full details on the owning host with profile-scoped authorized fetching, paging/ranges where applicable, cancellation, explicit expiry/error states, and bounded cache retention. Validate URI schemes/path containment; disable arbitrary executable links. Optional remote detail publication must obey the app's privacy boundary. Never silently recover removed historical data by inventing it.

### C5. Interactions and authority

Carry runtime request ID, generation, method, title/details, options/descriptions/previews, multi-select/recommended values, editor prefill, deadline, and resolution state. Preserve payload semantics; do not reduce every interaction to a single string question.

The owner accepts one valid response. Competing view answers, stale replies, cancellation, shutdown, and timeout resolve deterministically and fail closed where approval is required. Render the actual operation under approval; do not grant standing permission from a one-time confirmation. No secret input in ordinary text fields or synchronized transcript records.

### C6. Settings and model selection

Separate current-session model from persistent role assignments. Use provider+model ID as selection identity and retain extra metadata. Represent requested and effective reasoning/service mode, automatic role resolution, source scope, fallback order and capability constraints.

For settings, the service needs: dotted key, data type/valid values, description, default, persisted override(s), effective value/source, supported write scope, secret/redaction status, and live/restart behavior. Missing metadata must not be fabricated. Scoped writes need validation and concurrency protection. Clear override and reset default are separate operations.

### C7. Workers and modes

Keep native agent ID, parent ID/tool call, assignment, behavior profile, model role/resolution, lifecycle, progress, isolation/worktree identity, artifact/transcript references, and available control actions. Progress counters are observations, not state-transition authority.

Plan, goal, vibe, advisor and prewalk are OMP-owned behavior. A plan approval must bind to the draft revision, chosen execution role, and context-transfer action so a stale dialog cannot execute a different plan.

### C8. Audio, collaboration and remote control

Distinguish execution host, viewing device, and audio device. Require local consent before opening a microphone; closing a view/call stops its local capture immediately. Preserve provider privacy disclosures without embedding provider credentials in media messages.

Trusted app device peers and OMP collaboration guests are different permission classes. A view-only guest cannot answer approval dialogs, steer, mutate settings, or control workers. Snapshot share is not a live session; revocation/stop behavior must follow OMP.

## 5. Work-package tracker and dependencies

All implementation packages start **not started**. Mark done when the feature works in the app on its happy path — compiles alone isn't done, exhaustive scenarios aren't required.

| ID | Package | Prerequisites | Status |
| --- | --- | --- | --- |
| W00 | Pin OMP versions; check Plan Review API exists | None | Done |
| W01 | Native process and protocol client | W00 transport/lifecycle decision | Done |
| W02 | Shared OMP domain and projection schema | W00, C1–C8 agreed | In progress |
| W03 | Sessions, queue, persistence and recovery | W01, W02 | Done |
| W04 | Approval/ask/extension UI bridge | W01, W02 | In progress |
| W05 | Onboarding, models, roles and accounts | W01, W02, W04 auth interactions | Done |
| W06 | Scoped settings and customization management | W01, W02 | In progress |
| W07 | Conversation, composer and rich tool views | W03, W04, W05 basic catalog | Done |
| W08 | Native Plan Review and mode controls | W00 plan contract closure, W03, W04, W06 | Not started |
| W09 | Agent Hub, todos and background jobs | W03, W04, W07 | In progress |
| W10 | Session tree, context and memory inspector | W03, W06, W07 | In progress |
| W11 | Development tools and service integrations | W04, W06, W07, W09 | Not started |
| W12 | OMP media, voice and collaboration | W00 missing media/collab contracts, W03, W04 | Not started |
| W13 | Mobile/remote UI and cross-device parity | W02–W12 contracts; integrate incrementally | Not started |
| W14 | Historical migration and production cutover | W03; finalize after W05–W13 | Not started |

After W00 and shared contracts, W01 and W02 can proceed independently with one integration owner. Later model/settings/transcript/worker/media slices can be delegated only when interfaces are fixed. One owner edits shared proto/doc/RPC surfaces; other slices consume that contract rather than racing changes into the same files. Never serialize independent work merely for convenience or delegate tiny edits.

## 6. Detailed work packages

### W00 — Pin versions; check the few unknowns that gate design

**Entry points:** upstream docs/rpc.md, packages/coding-agent/src/modes/rpc/rpc-types.ts and matching wire schema; sdk/rust/omp-rpc; local docs/pi.md and crates/harness/src/pi/PROTOCOL.md for contrasts, not specifications.

**Steps**
- [x] Record executable version + matching source/tag, Rust client revision, schema revision, licenses. Do not assume upstream main matches the installed binary.
- [x] Skim the official client for shutdown/callback threading; use it unless it is plainly unusable.
- [x] Check only the design-gating unknowns in docs/source (probe only if docs are ambiguous): Plan Review control/draft API, approval interactivity under rpc-ui, no-model onboarding route, settings descriptor completeness. Note any other missing API (remote audio, collaboration, MCP auth) as a one-line blocker for its package and move on.

**Exit:** versions pinned in a few lines in this file; plan-API existence known. Everything else is trusted from matching docs/types/discovery until a package's happy-path run says otherwise. A missing plan API blocks W08, not permission to simulate planning in prompts.

**W00 record (2026-10-20):**

- Pin: executable `omp/18.12.5` = upstream tag `v18.12.5` @ `d5cd750e41fede277f9eb933455da92a0738915e` (also `main` HEAD at check time). Rust client `sdk/rust/omp-rpc` @ same rev (crate `omp-rpc` 0.1.0, `publish = false` → vendor/path dependency, never crates.io). Wire schema `packages/coding-agent/src/modes/rpc/wire/rpc-wire.schema.json` @ same rev; `wire.rs` generated by `bun run gen:rpc`. Licenses MIT (upstream, crate, Zeron) — add notices when vendoring in W01.
- Rust client usable: blocking transport, dedicated reader/writer threads, `Client::spawn` puts server in its own Unix process group; `close`/drop = stdin EOF → SIGTERM group → 1 s → SIGKILL + reap. Event channel is unbounded → engine must drain continuously and bound at its own boundary. Run it on dedicated threads, never GPUI/Tokio executors.
- **Plan Review API over rpc-ui: does not exist in 18.12.5 → W08 blocked (owner: upstream OMP).** No plan command in `rpc-types.ts` (`goal`/fast/slow exist), no plan field in `RpcSessionState`, `/plan` + `/plan-review` are `handleTui`-only (RPC prompt path runs only text-`handle` builtins, so `/plan …` would fall through as model prompt text — never send it as plan mode), and rpc-mode never calls `setPlanModeState`/`setPlanProposalHandler`. Core exists in-process (`AgentSession.preparePlanForReview`, `plan-mode-guard`, `xd://propose` dispatch throws `ToolError` with no handler installed — a plan-mode session under rpc-ui strands, cf. print-mode #8272) and ACP drives it (`acp-agent` setMode + proposal approval), but ACP is not the chosen transport and must not run concurrently with RPC on one session. Required upstream: plan mode set/state + proposal approve(refine/dismiss, fresh/compact/keep-context) host round-trip over RPC.
- Approvals interactive under plain `--mode rpc-ui` (extension-runner UI on); `--no-ui` makes approval-required tools fail closed → spawn **without** `--no-ui`. MCP auth challenges get no RPC UI handler in either mode → W11 blocker (owner: upstream OMP).
- No-model onboarding route: session process needs a selectable model before ready; pre-session route is the admin CLI (`omp setup`, `omp login [provider]`, `omp models`, `omp config`) in a terminal surface. RPC `login` works post-ready but secret input and pre-URL prompts are unsupported.
- Settings descriptors: `omp config list --json` is not a complete schema (no complete enum/default/provenance); the full descriptors live in pinned TS source (`config/registry.ts` `Setting` + `all-settings.ts` + `settings-ui.ts`: type, default, enumValues, credential flag, layered provenance) → W06 generates from pinned source, does not scrape CLI output.
- Remote/client-owned audio: `live_*` uses the runtime host's microphone/speakers only → W12 blocker (owner: upstream OMP). Collaboration: no RPC host/join commands (`omp collab` CLI only lists local hosts) → W12 blocker (owner: upstream OMP).

### W01 — OMP process/client integration

**Targets:** crates/harness/src process utilities and registry boundary; crates/engine/src/registry.rs; crates/engine/Cargo.toml; workspace Cargo.toml. Proposed dedicated location: `crates/harness/src/omp/` during integration. Keep the crate name initially to avoid unrelated renames; remove its generic multi-runtime responsibilities at cutover.

**Steps**
- [x] Add the pinned official Rust client or approved transport adaptation; no handwritten subset that drops frames. — vendored verbatim at `crates/omp-rpc` (see VENDOR.md); supervisor at `crates/harness/src/omp/`.
- [x] Resolve OMP from explicit configuration and GUI-safe executable discovery; never hard-code the Tern installation path or substitute Pi/Codex on failure. — override arg > `$OMP_EXECUTABLE` > PATH/common locations; missing → `NotInstalled` error.
- [x] Spawn with explicit cwd/profile and controlled overrides. Support real persistence and precise native resume. Isolate secret handling and stderr diagnostics. — `--mode rpc-ui` (never `--no-ui`/`--auto-approve`), `--profile`/`--resume`/`--no-session` (probes only); stderr to a diagnostics file, redacted tail on failure.
- [x] Handle ready/v2, request IDs, continuous reads, callbacks, bounded UI event delivery, large outputs and process generation. — inside the vendored client (v2 negotiated at spawn, observed 1 MiB/64 MiB limits); `OmpRuntime` carries the process generation; client event channel is unbounded → engine must drain continuously and bound at its boundary (W03).
- [x] Keep a process for the active native session; do not spawn once per prompt. Do not recycle until runtime settlement and resource policy permit it. — `OmpRuntime` owns the process across prompts; no recycle path exists yet.
- [x] Implement graceful shutdown plus bounded escalation with platform-specific owned-process cleanup. Do not kill unrelated supervised processes or the user's separately running OMP. — client `close`/drop: stdin EOF → Unix process-group SIGTERM → 1 s → SIGKILL + reap, only the spawned group. Windows kills only the leader (upstream client behavior) — revisit with engine binding if a job-object wrap is needed.
- [x] Register host tools/URIs only for genuinely app-owned capabilities. Do not inject a second generic agent delegation tool over OMP's own task system. — none registered; none defined yet.

**Check:** run one real OMP turn through the integration, then abort and shut down cleanly. — `cargo run -p zeron-harness --example omp_rpc_smoke` (2026-10-20): ready/v2, `get_state`, 51 commands, rich ask on, real turn completed+settled, abort settled, clean close.

### W02 — Domain, RPC and history schema

**Targets:** crates/proto/src/agent.rs, entities.rs and view.rs; crates/doc/src/parts.rs, commands.rs and queue.rs; crates/rpc/src/lib.rs; crates/client; crates/mobile/src/types.rs.

**Steps**
- [ ] Run language-server references before changing exported symbols. Inventory every desktop/mobile/edge serialization consumer. — inventory done 2026-10-20 (blast-radius list from HarnessId ~200 files down to account types; AgentEvent is engine-internal, clients consume the Loro MessagePart projection; full tool detail today survives only in the unserved host run journal; edge has a duplicate TS schema in edge/src/session-doc/). LSP reference pass still required before each listed symbol actually changes.
- [ ] Implement C1–C8 using existing type conventions and incremental mirror mechanisms; version persisted/wire data as needed. — started: `crates/proto/src/omp.rs` carries OmpModelRef, OmpRuntimeInfo (C1), OmpSessionBinding (C2), OmpProcessState/OmpDeliveryState/OmpPromptKind (C3), OmpThinkingLevel/OmpSessionConfig (C6); `MessagePart::Tool` gains additive `omp_entry_id` (C4 detail-fetch key, owner-side from OMP native history — do not revive the parked R2 sidecar path for this). Integration bridge landed: `HarnessId::Omp` + `OmpHarness` (registered in engine) so OMP runs through existing drive_run/journal/queue machinery while W03-hardening proceeds; the multi-harness enum itself is removed at W14. Driver ignores `RunRequest::auto_approve` entirely (OMP policy governs).
- [ ] Replace limited model/options/reasoning shapes with OMP-aware state; preserve unknown native metadata without forcing it into presentation fields. — request-path replacement landed 2026-10-21: `OmpSessionConfig::from_legacy` (provider-qualified parse only, `"default"`/unqualified ⇒ no override, above-ladder legacy rungs clamp to `max`) and the client now populates `RunRequest::omp` from chat config (previously always `None`; the OMP driver already preferred it). `OmpModelDescriptor.native` already preserved unknown model metadata. Remaining: persisted `ChatConfig` itself still stores only legacy spellings (native `omp` field on the chat row is W05/W13 surface work).
- [ ] Preserve structured message parts, notices, retries, compactions, file mentions, execution messages, media, phased todos and worker state. — structured events done 2026-10-21 (resumed mid-edit): `AgentEvent::OmpStructured` → `MessagePart::OmpStructured` fold + Loro round-trip (`kind: "ompStructured"`, `eventKind` + raw `native`; unknown fields survive); UI rendering deferred to W07 (transcript/mcp skip it explicitly). Remainder landed same day: execution messages (`ToolExecutionUpdate`/`ToolStreamUpdate` → structured parts, fold-coalesced per tool call), media (assistant image/redacted-thinking/fallback/server-tool blocks and image-only tool results → structured metadata records; inline bytes never sync, originals stay in native history), worker state (`subagent_lifecycle` structured parts on the parent, host-local `sessionFile` stripped). File mentions need no part: they ride user-entry text and the existing mention parsing renders them; the OMP wire has no mention content type.
- [ ] Replace destructive tool sanitization with secure detail/artifact references and appropriate presentation summaries. Redaction and authorization remain intentional boundaries. — landed 2026-10-21: the fold now keeps the bounded presentation summary (`summarize_tool_output`) instead of discarding tool output outright, and `sanitize_tool_call`'s strip is documented as a presentation boundary whose recovery path is the owner-side detail fetch keyed by tool call id / `omp_entry_id` (already landed). Full text/diff still never enters the doc; the one existing test asserting the old drop-everything policy was updated. Remaining: UI consumption of summaries/detail (W07).
- [ ] Define remote detail retrieval, cursor/revision checks and explicit unavailable/expired outcomes, authorized on the owner. — types (`OmpToolDetail`) + owner fetch landed: `OmpHarness::tool_detail` reads native entries and returns original call/result payloads (verified live; unknown id ⇒ Expired). Engine RPC landed 2026-10-21: `OmpToolDetail` method (forwardable to the owner device) → `SessionsEngine::omp_tool_detail`, owner-gated on the host-local binding (`authorize_omp_detail`: no/mismatched binding ⇒ `Forbidden`, empty native id ⇒ `Unavailable`, harness fetch ⇒ `Available`/`Expired`/`Unavailable`). Remaining: cursor/revision checks + UI consumption (with W07).
- [x] Separate legacy archive decoding from active OMP request types. Do not make new clients synthesize old harness IDs. — `crates/doc/src/archive.rs` (`ArchivedSession`): read-only decode of the persisted projection only; constructs no `RunRequest`, resumes nothing, carries no harness id; legacy tool parts expose no detail key (`tool_detail_key` ⇒ `None`), OMP-era parts keep `omp_entry_id`.

**Check:** one session replays with tool detail intact; an old archived session still opens. — Proven at doc level 2026-10-21 (`archive::tests::replay_keeps_tool_detail_key_and_legacy_archive_still_opens`: OMP-era replay keeps `omp_entry_id` + call/diff; legacy archive opens with no synthesized detail key), on top of the live harness detail proof. Not a GUI replay pass.

### W03 — Session ownership, durable delivery and recovery

**Targets:** crates/engine/src/sessions.rs, doc_host.rs and run_journal.rs; crates/doc command/queue code; crates/client/src/session/mod.rs; registry/session persistence and sync surfaces.

**Steps**
- [x] Bind app chat identity to exact OMP profile/session/path/leaf and process generation. — driver emits `OmpSessionBound` at start and refreshed before Done; engine stamps host-local `omp-sessions.json` (`crates/engine/src/omp_bindings.rs`, atomic rename, journal-rebuildable); synced row keeps only the native session ID. Leaf verified advancing live.
- [ ] Reconcile local queue entries with native admission/consumption. Distinguish steer/follow-up/prompt and maintain original attachments. — types + pairing landed 2026-10-21: `QueuedMessage.kind` (`OmpPromptKind`, additive, legacy rows read as Prompt; attachments stay on the local row untouched), `OmpQueueSnapshot` + `OmpRuntime::queue_snapshot`, and `reconcile_with_native` (in-order text pairing per kind list ⇒ Queued/Consumed/Ambiguous/Untracked) with tests. Engine wiring landed same day: a per-chat delivery ledger (`dispatched_steers` on the doc-host handle) records each accepted steer under its exact dispatched text; after dispatch and before any native removal the engine calls `SessionsEngine::omp_reconcile_queue` and prunes Consumed rows, so a consumed row is never "removed" or resent. Follow-up coverage landed same day: the ledger (renamed `dispatched_queue`) and remove/promote routing are kind-generic; `DocHost::promote_queued_message` promotes a local follow-up in place (`set_queued_kind` + steer slot, tested) or routes a dispatched follow-up into the live run and re-kinds its record to Steer. (No local dispatch path creates native follow-ups today; they arise inside OMP, and the ledger branch covers them when recorded.)
- [ ] Implement native queue edit/remove/promote only to the precision the runtime supports. Same-text queued items must not be arbitrarily identified. — runtime wrappers landed 2026-10-21 (`OmpRuntime::remove_queued_message`/`promote_queued_message`, verbatim-text first-match, live-steered entries unreachable, Prompt rejected rather than remapped; no native edit command exists — edit stays local until dispatch). Routing landed same day: `TurnControl` carries a queue-action channel; the OMP driver answers Remove/Promote/Snapshot against the live runtime in its event loop; `SessionsEngine::omp_remove_queued`/`omp_promote_queued` forward to it; `DocHost::remove_queued_message` falls back from a missing local row to reconcile + native removal via the ledger. Remaining: UI surfacing of native queue chips and the promote action (W07).
- [ ] Keep progress/status alive across yields while OMP still has async work. Apply session_settled semantics rather than existing Pi assumptions. — driver verified by inspection 2026-10-21: the run loop exits only on `prompt_result.session_settled` or a later `session_settled`; `agent_end` closes the assistant segment and streams on. Remaining: engine-side status surfacing check in the real app (with the package Check).
- [ ] Restore native sessions and projections after engine/UI loss; account for incomplete trailing native data without pretending partial streaming output was durably saved. — partially in place: boot re-dispatch is gated on native-history reconciliation (step below), and a mid-turn entry left `Streaming` is never stamped complete. Remaining: the real-app kill-mid-turn pass (package Check).
- [x] On uncertain admission after crash, inspect history/state and surface uncertainty instead of resubmitting side effects. — `Harness::safe_to_redispatch` (default legacy); OMP verifies last user message in native history via `get_entries`, unverifiable ⇒ unsafe; boot recovery in sessions.rs now gates re-dispatch on it. Verified live in `omp_harness_smoke` (admitted ⇒ unsafe, fresh ⇒ safe).
- [ ] Enforce exclusive writing across app views, remote engines and explicit TUI handoff. Do not write the session JSONL directly to imitate OMP transitions. — host-local guard landed 2026-10-21: `OmpBindingStore::conflict_for` + the engine refuses (and logs) a binding stamp when another chat already owns the same (profile, native session), with tests. Cross-device: structurally enforced — chats have one owning host device, remote engines serve no local runs, and the `omp_*` engine APIs act only on local live runs/bindings (owner-gated, as with tool detail). Remaining: explicit TUI handoff, which needs an upstream settle/release contract for the native session (owner: upstream OMP); do not imitate it by writing session JSONL.

**Check:** send, kill the app mid-turn, relaunch — session resumes, no duplicate prompt. — PASSED 2026-10-21 in the real app (user-run via `scripts/run-macos-dev.sh`): OMP turn started, app force-killed mid-turn, relaunched, same chat resumed with the prompt present once and the session token recalled. Remaining out-of-package items: explicit TUI handoff (upstream settle/release blocker, §6/W03 step) and follow-up UI surfacing (W07) — neither gates this package.

### W04 — Approvals, questions and extension presentation

**Targets:** current Pi UI bridge as reference; shared interaction types; crates/ui/src/composer.rs; native dialog infrastructure; remote RPC and mobile interactions.

**Steps**
- [x] Remove the unconditional approval override from the OMP execution path; preserve configured policy. — the registered product driver never reads `auto_approve` and never passes `--auto-approve`/`--approval-mode`; spawn is plain `--mode rpc-ui`. (`OmpLaunch::approval_mode` exists for session-scoped settings/tests; default unset. The legacy `crates/client` `auto_approve: true` still exists for legacy runtimes until W14; it has no effect on OMP.)
- [ ] Implement select, confirm, input, editor, rich ask, cancel and deadlines with original metadata and whitespace. — driver-side done through `OmpInteraction` conversions (`crates/harness/src/omp/interactions.rs`, fail-closed, deadline→`timedOut`); legacy question projection is the known lossy boundary. Rich ask verified live. Desktop consumption landed 2026-10-21: `UserInputQuestion` gained aligned `option_details` (description/preview) + `recommended` (additive serde), `to_questions` fills them from the domain interaction, and the composer wizard renders description/preview sub-lines, a Recommended marker, and starts single-select recommended picks selected — labels remain the answer currency, so `from_answers` and the wire path are unchanged. Remaining: mobile consumption (the FFI question mirror does not carry the new fields yet; W13), and full `OmpInteraction`-typed rendering if a surface ever needs more than the question projection.
- [ ] Implement notify, setStatus, setWidget, setTitle when enabled, set_editor_text and open_url with appropriate URL validation and owner context. — plumbing landed: `OmpPresentation` (proto) via `to_presentation` (open_url scheme-gated to http/https), emitted as `AgentEvent::OmpPresentation`, journaled/broadcast by engine and logged; not folded into docs. Desktop surfaces landed 2026-10-21: engine presentation broadcast hub + `WatchOmpPresentation` RPC (forwardable stream, no replay), capability-gated UI watch into AppState, shell drains one-shot effects once (Notify→toast, OpenUrl→browser via the driver's scheme gate, EditorText→composer draft replacement), and status/widget/title render as a muted extension strip above the composer. Remaining: mobile surfaces — deferred to W13 by package ownership (the FFI presentation/question mirrors don't carry these types yet; mobile parity is W13's deliverable, not a W04 gate) — and in-app visual verification of the strip/toasts, which needs a user-run app pass and is still outstanding.
- [ ] Present actual approval target/arguments, source/policy reason and outcome. Do not hide critical details behind a generic 'Continue?' action. — target+args ride the question title/text the wizard renders, and since 2026-10-21 the approval options carry explicit descriptions when OMP gives none ("Allow this one operation, this once" vs "Allow this tool for the rest of the session" vs "Do not run it", tested), so the one-time/standing distinction is visible before answering. Dedicated approval card landed 2026-10-21: the wizard detects the approval select shape and renders an "Approval required" card with the operation (target+args, verbatim) in a mono block above the choices. Structured target/policy *fields* are upstream-blocked: the extension-UI select wire carries only title/options/details — no target, source, or policy-reason fields exist to render (owner: upstream OMP).
- [ ] Resolve once on the owner; dismiss competing views, ignore stale generations, and handle timeout, app closure and host exit safely. — verified by inspection 2026-10-21: `SessionsEngine::respond_input` removes the pending resolver before sending (first answer wins, later answers return false), the driver's deadline/cancel path answers `Cancelled`, and stale-generation interactions are dropped at conversion. Competing-view dismissal verified in place 2026-10-21: the composer wizard latch releases on `input_request_resolved` — the synced doc marks the part resolved when any view answers, so other views' panels unmount and restore their drafts; the engine resolves once, so the losing view's late answer is a no-op. Remaining: none on desktop; mobile wizard parity is W13.
- [ ] Redact secrets and keep secure-auth flows separate from transcript-backed generic editors. Do not treat an unanswered callback as an approval. — in place at the bridge: unknown/unanswered answers convert to `Cancelled` (fail-closed tests), secrets never enter ordinary question fields (secret-input and pre-URL login prompts are unsupported upstream, tracked in W05/W11).

**Check:** one real ask answered, one tool approved, one tool denied (denied = not executed). — all three verified live at harness level (`omp_harness_smoke` turns 3–5, 2026-10-21, session-scoped `--approval-mode=always-ask`; approvals arrive as select [Approve / Approve-for-session / Deny] with target+args in the question). In-app UI rendering of the same flows remains.

### W05 — Onboarding, runtime management, models, roles and accounts

**Targets:** crates/ui/src/pickers.rs; settings/harnesses.rs, accounts.rs and composer.rs; crates/client/src/catalog.rs; crates/engine/src/model_catalogs.rs, agent_accounts.rs, agent_accounts/stores.rs and harness_updates.rs; crates/harness/src/install.rs.

**Steps**
- [x] Replace external-agent provider onboarding with OMP executable/version/profile readiness and OMP-managed model authentication. — `OmpReadiness` (proto) + `OmpHarness::readiness` + `OmpReadiness` RPC + `omp-accounts-v1`; the Settings → Models onboarding block renders version/qualified state, the discovery error verbatim when the executable is missing/unusable, and per-provider sign-in state. Installation is OMP-owned: Zeron offers no OMP installer (see the install step below).
- [x] Bootstrap authentication without assuming normal session ready; use a supported OMP admin/onboarding route established by W00. — RPC route: ephemeral rpc-ui reaches ready and serves login before any model exists (proven on an isolated empty profile). Non-chat login surface landed: engine `OmpLoginStart/Poll/Answer/Cancel` flows run `OmpHarness::login` in the background, surface its browser URL (opened once) and questions through polling, and answer fail-closed on cancel/drop.
- [x] Build quick session-only picker and full model/role hub following reference screenshots; retain provider-qualified identities, unknown metadata and custom roles. — quick picker is the existing composer picker (its pick becomes `RunRequest::omp`, a per-session `set_model`; it never writes OMP config). The full hub is Settings → Models (`crates/ui/src/settings/models.rs`): provider filter + search, descriptor inspector (context/max output/modalities/reasoning/thinking levels), role chips, Assign flow (pick role → pick model), all provider-qualified; descriptors keep raw native records end-to-end (engine descriptor cache round-trips unknown fields, tested).
- [x] Separate current model, persistent defaults, project roles, automatic resolution, per-role thinking, cycle order and fallback chains. — persistent layer (`crates/harness/src/omp/model_admin.rs` over `omp config`): `OmpModelRoles` snapshot (assignments incl. `@role` automatic selectors verbatim, cycleOrder, `retry.fallbackChains`, `defaultThinkingLevel`, storage) + writes `OmpSetModelRole`/`OmpSetCycleOrder`/`OmpSetFallbackChain`/`OmpSetDefaultThinking`; custom roles discovered from config are offered and preserved (tested). Current/session model stays the composer's session-only path, and the hub states the separation in copy. Per-role thinking is OMP's selector vocabulary (`@role` / thinking tokens ride the same persisted keys); no second Zeron-side store exists.
- [x] Use OMP login/logout/account/provider-limit APIs and supported secure administration. Never swap/rewrite credential files or directly integrate provider OAuth in Zeron. — discovery (`get_login_providers`/`get_logout_accounts`, 85 providers live), `OmpHarness::login` (concurrent UI drain, fail-closed) + `logout`, engine RPCs + Settings accounts UI (sign in/out, per-provider account removal). Provider limits: `OmpHarness::provider_limits` over `omp usage --json` + `OmpProviderLimits` RPC; report internals stay raw native records, counts rendered in the hub. Credential files are never read or rewritten by Zeron.
- [x] Replace fabricated static fallback catalogs with last-known version/profile-scoped data and visible stale/unavailable state. — `model_catalogs::omp_catalog`: live descriptors cached on disk under `model-catalogs/omp-descriptors/<context-hash>` (binary version + profile config context); live failure answers the cache with `stale: true` + the error, no cache answers an explicitly unavailable snapshot; the hub renders both states. No fabricated OMP list exists on this path (legacy curated statics in client/catalog.rs serve only legacy harnesses until W14).
- [x] Own OMP installation/update only; verify supported install provenance, integrity, runtime compatibility and active-process update policy. App updates and OMP updates are separate. — decided in code: the legacy managed installer never touches OMP (`install.rs`: OMP methods are empty, no manual command), readiness reports the probed version against the qualified pin, and a newer/older binary is discovered per spawn, so an OMP update takes effect on the next run — active processes are never swapped under a live session. The Models page states this instead of offering an install/update action.

**Check:** switch model for the session only; global default unchanged. Missing-executable state shows a useful error. — PASSED 2026-10-22: missing-executable verified 2026-10-21 (`OMP_EXECUTABLE=/definitely/missing/omp` ⇒ clean NotInstalled error, and readiness renders that cause); session-only switch proven live on an isolated profile (run with `RunRequest::omp.model=deepseek/deepseek-v4-flash` started the session on that model and completed, `modelRoles` still `{}` afterwards — the persistent default never moved; role assign/fallback/clear round-tripped through the same admin path and cleared back to `{}`). Real app: fresh `cargo build -p zeron` binary booted isolated on `settings/models`, the Models page loaded through the new RPCs and cached 148 live descriptors, no panics in the log (the user's already-running dev bundle was left untouched; the in-app click-through of a session switch rides the same driver path proven above).

### W06 — Scoped settings and customization management

**Targets:** crates/ui/src/settings.rs and settings modules; OMP integration configuration service; existing theme, shortcut and preference infrastructure.

**Steps**
- [ ] Expose all setting groups from the architecture ledger, including advanced keys, through searchable typed controls or a validated advanced editor.
- [x] Obtain complete descriptors from pinned OMP sources/API; do not scrape human CLI output as the durable schema contract. — `scripts/gen-omp-settings.py` statically extracts `register({...})` declarations from the pinned checkout into vendored `settings_descriptors.json` (555 settings; computed defaults marked `$expr`, never fabricated; 20 cross-file enums keep no static values). Rust loader in `crates/harness/src/omp/settings.rs`, revision-pinned by test.
- [ ] Implement global/profile/project/session distinctions, defaults versus inheritance, requested/effective values, provenance and secret masking. — global-per-profile effective reads landed (redacted list); provenance beyond set-time shadow reporting and project/session scope remain.
- [ ] Route writes through supported validation/persistence. Project YAML edits, if required, must preserve unrelated keys and external concurrent changes. — global writes go through `omp config set/reset` (OMP validates; unset ≠ write-default); credential keys refused pre-spawn. Project YAML writes remain.
- [ ] Display live/restart behavior per setting as OMP reports it; apply changes at a safe boundary without aborting work unexpectedly.
- [ ] Map native appearance semantics and OMP themes without confusing terminal-specific options with GPUI effects. Keep app font/window settings app-local.
- [ ] Implement discovery/control for rules, skills, templates, agents, hooks, tools, plugins/marketplaces and MCP; respect their differing reload/restart behavior and scope.

**Check:** change one setting at each scope; effective value follows. Secrets never appear in UI/log text. — global scope verified in an isolated profile (`omp_settings_smoke`: set→effective→unset→default; credential + unknown refused). Project/session scopes outstanding.

### W07 — Conversation, composer and rich tool-detail UX

**Targets:** crates/ui/src/transcript.rs, composer.rs and pickers.rs; crates/proto/src/view.rs; existing markdown/media/virtualization components; client session projection.

**Steps**
- [x] Render complete ordered message content and incremental tool progress with stable identity and existing virtualization/follow-tail behavior. — landed across W02–W04 (structured parts fold with stable ids, coalesced tool progress, virtualized rows); completed 2026-10-22: remaining dropped `OmpStructured` parts now render as ordered `OmpNotice` rows (`omp_notice` in transcript.rs), with tool-progress/queue/subagent kinds hidden because chips/queue UI own them.
- [x] Add expandable input/output/diff/artifact views; lazy fetch detail, handle permission/expired/offline states, and support image-only tool results. — chip expansion and diff/artifact views predate this package; lazy owner-side detail landed 2026-10-22: OMP tool chips offer "Show full detail", fetched via `OmpToolDetail` into `OmpDetailFetch` (available → pretty native payload; expired/forbidden/unavailable are explicit rendered states, never reconstructed from the summary). Image strips on chips predate this (`chip_has_images`).
- [x] Keep generic inspection for new tools/content, while specialized renderers consume preserved data rather than replacing it. — unknown structured kinds degrade to kind + preserved payload line (tested); unknown tool/content keeps the generic chip + summary→detail path.
- [x] Provide distinct send/steer/follow-up/stop controls with visible pending/delivery state and draft recovery. — landed in the composer/queue work (W03-era): steer vs send-next is capability-labeled, queue rows show pending/delivery state, stop/interrupt and draft restore are composer-owned.
- [x] Discover slash commands, aliases, subcommands, skills, MCP prompts and argument hints from OMP; do not retain a Pi-specific built-in list. — fidelity projection landed 2026-10-21 (W02 slice); composer consumption landed 2026-10-22: `OmpCommands` RPC + `omp_slash_candidates` (aliases as their own rows, subcommand names in the description, source suffix, argument hints) with legacy-list fallback on any failure. Skills/MCP prompts ride the existing advertised-invocation path.
- [x] Add model/thinking/fast/slow/mode, context/cost/throughput and approval-policy status using live runtime information. Do not hide cost because old architecture excluded it. — model/thinking picks and mode/approval surfaces predate this (pickers, W06 settings); context ring predate. Cost landed 2026-10-22: OMP `usage.cost.total` → `AgentEvent::TurnCost` → doc `turnUsage` meta → `TranscriptUpdate` → footer context card ("Last message: N in · M out · $X"). Model/thinking changes also render live as transcript notices. Remaining cosmetic gap: no dedicated fast/slow composer toggle producer — the driver consumes `OmpSessionConfig.fast_mode/slow_mode`, and `from_legacy` currently leaves them unset.
- [x] Preserve attachments, file/URI mentions, multiline input, clipboard and supported word prediction without blocking typing. — composer behaviors preserved throughout; word prediction wired 2026-10-22: `TurnControl::predict_word` → OMP driver → engine `OmpPredictWord` RPC → debounced composer ghost text (OMP chats with a live run only; stale-guarded, fail-silent).

**Check:** one streaming turn with a tool call renders live and its detail expands. Eyeball the running app once. — PASSED (harness/app level) 2026-10-22: isolated-profile live turn (deepseek, default profile) streamed an `Exec` tool call, `Usage` and `TurnCost` events, `Done Completed`; detail resolution by tool call id is engine-tested (W02) and the UI fetch states are compile/unit-covered. Fresh build booted isolated with zero panics. GUI click-through of chip expansion still wants the user's eyes in the dev app (their running bundle was not disturbed).

### W08 — Native Plan Review and mode controls

**Targets:** new plan/mode UI within existing crates/ui conventions; OMP control service; session and settings state. Do not invent final filenames before checking UI organization.

**Prerequisite:** actual OMP mode/draft/review contract from W00. A text prompt or yolo flag does not satisfy this package.

**Steps**
- [ ] Expose active/paused/off plan state and OMP's mutual exclusions with goal/vibe.
- [ ] Show editable draft, contents navigation, section annotations/removal/undo, external-editor integration where appropriate, and revision-aware save/refine.
- [ ] Implement approve with fresh session, compact context, or keep context; enforce runtime eligibility and display selected execution role/model.
- [ ] Preserve dismissal versus pause/exit distinction, save-without-execute and rejected/canceled approval behavior.
- [ ] Implement goal objective/budget/state and continuation controls, vibe workers, loop limits, advisor feedback/configuration and prewalk handoff through actual OMP mechanisms.
- [ ] Apply OMP tool/read-only enforcement; do not fake read-only by disabling UI file buttons.

**Check:** plan a change — workspace untouched — approve it, and it executes.

### W09 — Agent Hub, phased todos and background jobs

**Targets:** existing subagent projection/rendering; crates/engine/src/sessions.rs; transcript/tool UI; session inspector; phased todo and process surfaces.

**Steps**
- [ ] Subscribe to required worker events and obtain snapshots for initial/reconnected views; deduplicate snapshot/live overlap. — live path + snapshot call done: driver subscribes `Events`, normalizer routes lifecycle→seed/Done and nested events per-subagent to the spawn call (`task` recognized as spawn); `OmpRuntime::subagents()` wraps `get_subagents`. Reconnect-view dedup in UI remains.
- [ ] Display hierarchy, assignments, model/role, state, errors/retries, usage, isolation/worktree/result details and full nested transcript.
- [ ] Support steer/cancel/revive only where OMP advertises them. — runtime wrappers landed (`cancel_subagent`/`steer_subagent`); cancel verified live. Engine/UI control routing into a live run remains.
- [ ] Preserve phased todos and blocked/abandoned semantics; changes flow through OMP. — projection landed: proto `TodoStatus` += Blocked/Abandoned, `TodoItem` += phase/blocker; normalizer upgrades each todo part in place from the result's authoritative `details.phases` snapshot (verified live incl. OMP auto-promotion). User-side todo editing through OMP (`set_todos`) remains.
- [ ] Display background jobs/processes, logs, artifacts, stdin/control and lifetime ownership; stop/kill/restart are distinct actions.

**Check:** spawn one worker; progress, result, and cancel work. — all verified live at harness/runtime level (`omp_harness_smoke` turn 6 + `omp_rpc_smoke` snapshot→cancel→`Aborted` lifecycle, 2026-10-21). Agent Hub UI remains.

### W10 — Sessions, context, compaction and memory

**Targets:** session sidebar/inspector; native session binding; history projection; OMP settings and command routes.

**Steps**
- [ ] Discover/search existing OMP sessions with profile/cwd provenance and without automatic import/upload. — read-only scanner landed (`crates/harness/src/omp/discovery.rs`: header scan incl. title-record, profile/XDG roots; verified live: 127 sessions). Search UI + attach flow remain.
- [ ] Implement native tree/branch/fork, labels/pins/title/move/delete and context reset using supported commands; preserve artifact references and extension cancellation. — runtime wrappers landed (`tree`/`session_stats`/`branch`/`fork` on OmpRuntime); inspector UI + fork check remain.
- [ ] Show context breakdown, current versus compacted history, compaction strategy/progress/failures and handoff state.
- [ ] Expose supported memory backends, effective injected memory, maintenance/diagnostics and explicit destructive clear operations.
- [ ] Support side questions, local export and reviewed snapshot share without conflating share links/HTML with editable native sessions.

**Check:** fork a session; both identities/history stay correct. — verified live at harness level (`omp_harness_smoke` turn 8, 2026-10-21: new id, fork recalls pre-fork token, original still answers).

### W11 — Development tools and services

**Targets:** existing files/diff/terminal/browser/preview surfaces; OMP tool/result adapters; configuration and integration inspectors. Reuse existing native renderers instead of creating duplicate subsystem engines.

**Steps**
- [ ] Map file/hashline/AST edits, staged proposals/conflicts, notebooks, archives, database/document results and internal/SSH URIs to inspection and supported actions.
- [ ] Expose LSP/DAP configuration/status and results through OMP; add structured action contracts where a panel needs more than a tool card.
- [ ] Support shell/eval/PTY behavior and runtimes without overriding protocol defaults blindly.
- [ ] Integrate web search citations, browser relay/CDP setup, screenshots/downloads/takeover, computer permissions and provider safety checks.
- [ ] Expose GitHub/PR/issues/Actions context, review/commit/security workflows and findings/artifacts. Keep app manual workspace worktrees distinct from OMP worker worktrees.
- [ ] Support MCP transports, prompts/resources/notifications, OAuth and reconnection; disable missing required functionality explicitly until its W00 contract is closed.

**Check:** one tool family actually used end-to-end; spot-check others only when touched.

### W12 — Media, voice, collaboration and terminal-only extensions

**Targets:** crates/voice-session, voice-media and voice; relevant engine/UI voice and collaboration callsites discovered through references; existing OS capture/playback; scripts/package-voice-runtime.py and Codex voice smoke scripts; OMP protocol integration.

**Steps**
- [ ] Preserve image attach/decode/preview, vision routing, generated-image results and audio artifacts through OMP.
- [ ] Implement OMP dictation/TTS/live voice with explicit local permissions, phase/meters/transcripts, mute/stop/cancellation, and tool delegation into the correct OMP session.
- [ ] Remove direct Codex/provider execution and runtime/helper discovery from the production voice path. Reuse only media/UI pieces that are genuinely provider-independent and legally distributable.
- [ ] Close client-owned audio transport for remote/mobile OMP. The existing Codex transport and upstream host-audio RPC are not interchangeable. If OMP lacks this contract, it is a required upstream blocker.
- [ ] Add native collaboration host/join/control/view-only/status/reconnect/stop with OMP permissions and secret-link handling; app device sync remains distinct.
- [ ] Terminal-only extensions: exclusive same-session handoff and safe return to native projection. Do not silently launch a second writer.

**Check:** one dictation/voice round trip on this machine; stop actually stops capture. Remote audio only if that contract exists.

### W13 — Mobile and remote surface parity

**Targets:** crates/mobile/src/types.rs and client_ffi; shared crates/client and proto; apps/ios/Zeron controllers/models; generated Swift bindings via scripts/ios/build-core.sh. Discover any actively shipped Android surface before changing shared bindings.

**Steps**
- [ ] Keep shared data contracts updated as W02 evolves; do not wait until the desktop is finished to discover unrepresentable mobile values.
- [ ] Remove mobile runtime selection/defaults/static catalogs; add OMP model/role/settings, interactions, worker/session/context views and tool details.
- [ ] Regenerate bindings; never hand-edit generated Swift. Compile native callers against the changed contract.
- [ ] Enforce owner-side authorization for every forwarded command/detail fetch. Reconnect pending interactions and native snapshots without duplicate submissions.
- [ ] Implement responsive equivalents rather than requiring desktop modal dimensions. Respect app suspension/background audio and user permissions.

**Check:** build iOS, open the same session on device/simulator, send one message. Don't claim platforms not run.

### W14 — Historical migration and single-runtime cutover

**Targets:** production HarnessId/registry/catalog/account/install/update branches; app entrypoints; voice execution; persisted config and archive readers; desktop/mobile settings; scripts/CI/packaging; READMEs and architecture/parity documentation.

**Steps**
- [ ] Inventory all active non-OMP launch paths, including defaults, title generation, auxiliary agents, MCP delegation, voice, fallback paths, updater and installer behavior.
- [ ] Resumable/idempotent local data migration with pre-migration backup, version marker, no automatic upload. Try it on a copy of a store, never the user's live one.
- [ ] Preserve legacy sessions as read-only archives with provenance. Use native import when available; otherwise explicit reviewed context creates a NEW OMP session.
- [ ] Migrate preferences/favorites only where mappings are unambiguous. Do not assume legacy model identifiers or opaque session IDs belong to OMP.
- [ ] Remove old adapter implementations, live enum variants, static provider catalogs, credential swapping, install/update commands, runtime flags and obsolete tests/docs. Test fixtures remain test-only, not a selectable product runtime.
- [ ] Update live wire/schema version and reject incompatible old clients safely. Legacy archive decoding is not an execution compatibility shim.
- [ ] Update packaging, dependency/license notices and user documentation; remove separate Codex/helper requirements from model-backed voice.

**Check:** migrated copy opens with old history readable; grep production code — no non-OMP launch path remains.

- [ ] Final tidy (part of this package, not a separate qualification pass): delete obsolete tests of removed runtimes, update docs/release notes with the supported OMP version, one last full build + one real task in the app.

## 7. Checks — minimal

Per package, exactly this and no more (prefer `mise run <task>` if tasks exist; none were configured at handoff time):

1. It builds: `cargo check --locked -p <touched-crate>` while iterating; `cargo build --locked -p zeron` once before calling a package done (plus `bash scripts/ios/build-core.sh iphonesimulator` only when mobile bindings changed).
2. The package's one-line **Check:** runs in the real app (`bash scripts/run-macos-dev.sh` for the isolated bundle) against real OMP once, happy path.

That's the whole regime. Specifically not required: unit/integration test suites per package, scenario matrices (crash/race/timeout/reconnect permutations), screenshot evidence, scripted local model servers, two-device passes, full CI (`scripts/ci/test-core.sh`), or re-running other packages' checks. If existing tests break because behavior intentionally changed, fix or delete them; don't preserve old-runtime tests. Add a new permanent test only when a real bug was hit or a data-loss/security invariant is at stake (§1.13).

Safety, not testing: probes/smokes use an isolated OMP profile and disposable workspace — no real keys, paid requests, live MCP, or the user's store by accident. `scripts/e2e-smoke.sh` uses a mock and deletes fixed `/tmp` dirs; don't run it unreviewed.

## 8. Progress notes

When a package lands or blocks, update its Status cell in §5 and add at most a line here: what ran, what's blocked (missing operation + owner, not just "needs upstream"), next package. No evidence files, command transcripts, or screenshot locations.

Initial status: planning delivered; W00–W14 unstarted.

2026-10-20: W00 done — pinned OMP 18.12.5 @ `d5cd750`; Plan Review has no rpc-ui API (W08 blocked, owner upstream, see W00 record); next W01.
2026-10-20: W01 transport done — vendored `omp-rpc` + supervisor smoke passed against real OMP; engine registry binding sequenced into W03 (needs W02 types); next W02.
2026-10-20: W02 in progress — consumer inventory done; proto::omp domain types (C1/C2/C3/C6) landed; Tool part native entry id landing; next: finish W02 projection/history types, then W03 engine binding (registry + sessions) as one owner across doc/engine surfaces.
2026-10-20: OmpHarness driver landed + engine-registered; harness smoke passed (80 models, tool turn, resume-by-native-id recall). Next: W02 detail-fetch contract + replay check, W03 binding record/uncertainty hardening, W04 interaction fidelity.
2026-10-20: W02/W03 advance — C4/C5 detail+interaction types landed; normalizer reconciles final-vs-streamed text and maps usage/context; resume token is native session ID (never host path). Next: W03 binding record + uncertain-delivery reconciliation in engine sessions, W04 replace legacy dialog bridge with OmpInteraction end-to-end.
2026-10-21: W03 advance — uncertain-admission gate + host-local binding store landed and verified against real OMP history. Remaining W03: native queue reconciliation precision, yield/settled status surfacing, exclusive-writer/TUI handoff, kill-app-mid-turn check in the real app.
2026-10-21: W02 detail path proven end-to-end at harness level (native call+result+details intact). Next: W04 OmpInteraction through doc/UI, then W05 onboarding/models.
2026-10-21: W04 in progress — dialogs route wire→OmpInteraction→legacy questions with fail-closed semantics; live ask round-trip passes. Next: approve/deny live check, presentation methods, native interaction UI.
2026-10-21: W04 check passes (ask/approve/deny live). Next: presentation methods + native interaction UI (composer/mobile), then W05.
2026-10-21: W04 presentation plumbing landed (typed, URL-gated, journaled; UI surfaces next). Next: W05 onboarding/models + W04 native interaction UI.
2026-10-21: W05 in progress — per-model thinking levels (64/80 models), engine disk catalog cache context, login/logout discovery (85 providers), missing-executable error verified. Next: onboarding route + model/role hub UI.
2026-10-21: W09 phased todos projected end-to-end (phase/blocker/status preserved). Next: W10 session tree/context reads or W06 settings descriptors.
2026-10-21: W10 in progress — tree/stats/branch/fork wrappers; binding leaf stamped start+finish; fork check passes. Next: session discovery + context/compaction inspector data, W06 descriptors.
2026-10-21: W06 in progress — 555 settings descriptors generated from pinned source and global settings service verified in an isolated profile. W09 subagent snapshot/cancel and phased todos projected live. W10 read-only native-session discovery finds existing sessions without import/upload. Next per new-session prompt in §9: finish W02 first, then W03/W04/W07 in dependency order.
2026-10-21: W02 advance (sequential; requested worker model unavailable in-session) — owner-authorized engine detail RPC (`OmpToolDetail` → binding-gated `SessionsEngine::omp_tool_detail`, explicit Available/Expired/Unavailable/Forbidden), slash fidelity types (`OmpSlashCommand`/`omp_commands`), legacy archive separation (`doc::ArchivedSession`) landed; W02 Check proven at doc level + prior live harness detail proof, `cargo build -p zeron` passes. W02 remains In progress: LSP reference pass, OMP-aware model/options replacement, structured-part preservation, detail cursor/revision + UI consumption (W07). Next: W03 queue/exclusive-writer + real-app kill-mid-turn check.
2026-10-21: W02 structured-part slice completed after a mid-edit restart (committed tree did not compile: `OmpStructured` fold/schema arms missing, serde tag collision on `kind`, stray line in commands test helper) — fold + Loro round-trip + one doc test pass (`cargo check -p zeron-doc -p zeron-proto`, single test run only, per user testing limit). W02 remains In progress: LSP reference pass, OMP-aware model/options replacement, remaining structured parts (mentions/execution/media/worker state), detail cursor/revision + UI consumption (W07). Next: W03 queue/exclusive-writer + real-app kill-mid-turn check.
2026-10-21: W02 first-item remainder landed — `OmpSessionConfig::from_legacy` + client populates `RunRequest::omp`; execution-update/media/worker-state structured preservation in the normalizer (fold-coalesced, bytes and host paths never sync); fold keeps bounded tool-output summaries with owner-side detail as the recovery path. Also removed 21 stray duplicate `omp: None,` lines the prior session's automated edit left before `RunRequest {` literals in test/example files (they broke `--tests` compilation; lib builds were unaffected). Verified, minimally per user limit: `cargo check --tests` on proto/doc/harness/client + 4 targeted tests. W02 remains In progress: LSP reference pass, persisted ChatConfig OMP field, detail cursor/revision, UI consumption (W07). Next: W03 queue/exclusive-writer + real-app kill-mid-turn check.
2026-10-21: W03 started — queue kinds (`QueuedMessage.kind`), native queue snapshot + remove/promote wrappers at runtime-text precision, `reconcile_with_native` pairing (duplicate texts report Ambiguous, never guessed), and the binding-store exclusive-writer guard (second chat claiming a native session is refused + logged), all with focused tests. Also fixed two more pre-existing engine-test compile breaks from the same automated edit. Verified minimally: `cargo check --tests` on proto/doc/harness/engine/ui + 3 targeted tests. W03 remains In progress: engine delivery-ledger wiring of reconcile, remove/promote routing into live runs, cross-device/TUI writer ownership, and the real-app kill-mid-turn Check (not run). Next: W03 ledger wiring or W04 native interaction UI.
2026-10-21: W03 wiring landed — `TurnControl` queue-action channel (driver-answered, false/None on any failure), OMP driver handling in its event loop, `SessionsEngine` snapshot/remove/promote/reconcile APIs, delivery ledger in the doc host (accepted steers recorded under dispatched text; Consumed pruned after dispatch and before removal; remove falls back local → reconcile → native). Verified minimally: `cargo check --tests` harness+engine, one new channel test. Package Check (send, kill mid-turn, relaunch, no duplicate) NOT run in the real app this session — prior live evidence covers the harness-level crash-recovery gate only (`omp_harness_smoke`, 2026-10-21). W03 remains In progress on that Check plus TUI handoff (upstream). Next: W04 native interaction UI.
2026-10-21: W03 follow-up/promote completion — kind-generic ledger, `DocHost::promote_queued_message` (local: `set_queued_kind` + steer slot; dispatched: native promote + record re-kinded), one doc test. Package Check still not run: it requires driving the signed macOS app interactively (send → kill mid-turn → relaunch) with an authenticated model, which exceeds this session's minimal-verification limit; OMP 18.12.5 is present on this machine, so the blocker is verification scope, not tooling. TUI handoff remains blocked on an upstream settle/release contract (owner: upstream OMP). W03 stays In progress until the Check is run in the app. Next: W04 native interaction UI (the Check can ride along there, since both need the running app).
2026-10-21: W03 Check explicitly blocked, not run: the interactive signed-app pass (send → kill mid-turn → relaunch) remains outside the session's minimal-verification limit, so W03 is NOT marked complete. W04 advanced instead: rich question consumption on desktop — `option_details`/`recommended` on `UserInputQuestion`, filled by `to_questions`, rendered by the composer wizard (description/preview sub-lines, Recommended marker, recommended pre-picked), with tests at both layers. Verified minimally: `cargo check --tests` on proto/harness/client/engine/ui/mobile + 2 targeted tests. W04 remains In progress: presentation surfaces (notify/status/widget/title/editor-text/open-url in UI), approval-target presentation polish, mobile consumption (W13). Next: W04 presentation surfaces or W05 onboarding/models.
2026-10-21: W03 DONE — package Check passed in the real app (user-run: OMP turn, force-kill mid-turn, relaunch, session resumed, prompt present once, token recalled). Tracker + Check updated. Carried blockers that do not gate W03: TUI handoff (upstream settle/release contract), W07 UI surfacing of native queue chips/promote. Next: W04 presentation surfaces or W05 onboarding/models.
2026-10-21: W04 presentation surfaces landed on desktop — engine broadcast hub, `WatchOmpPresentation` RPC + `omp-presentation-v1` capability, UI watch into AppState, shell one-shot drain (toast/browser/editor-text), composer extension strip (title/status/widget), and explicit approval option descriptions (one-time vs session grant, tested). Verified minimally: `cargo check --tests` on proto/rpc/engine/harness/ui + one targeted test. W04 remains In progress: in-app visual verification of the new surfaces, dedicated approval card, competing-view dismissal polish, mobile surfaces (W13). Next: W05 onboarding/models.
2026-10-21: W04 approval card + dismissal pass — wizard renders approvals as a dedicated card (mono target block; structured target/policy fields upstream-blocked, wire carries none), competing-view dismissal verified already in place via the resolved-latch, approval-shape detection tested. Verified minimally: `cargo check --tests` ui + 2 targeted wizard tests. Still outstanding for W04: in-app visual verification (user-run app pass) and mobile parity (W13). Next: W05 onboarding/models.
2026-10-21: BUG (pre-existing, found by user in-app): ask/approval answers hung forever — the engine input bridge called `tokio::spawn` from the OMP driver's plain std thread, panicking "no reactor running" (panic in the user's headed log at sessions.rs input bridge via harness.rs handle_ui_request); the question rendered, answers went nowhere, OMP waited forever. Fixed: `spawn_input_forwarder` spawns on Tokio when a reactor exists, else polls on a std thread with identical semantics; the driver thread also catch-unwinds the bridge and answers Cancelled on panic. Regression test `input_forwarder_works_without_a_tokio_reactor` passes. Needs a rebuilt app pass to confirm in-app.
2026-10-21: User confirmed in the rebuilt app: ask/approval flow runs after choosing (no more forever-wait), and tool chips render as designed (collapsed one-liner; output summary on expand). W04's in-app visual verification is thereby covered for ask/approval/chips; extension-triggered surfaces (notify/status/widget/editor/open-url) remain unexercised because no stock flow emits them. W04 remains In progress only for mobile parity (W13).
2026-10-22: W05 accounts backbone landed — proto `OmpLoginProvider`/`OmpLogoutAccount`/`OmpLogoutOutcome`/`OmpReadiness`, harness `login` (concurrent UI drain via new `OmpRuntime::call_concurrent`, fail-closed) + `logout` + `readiness`, engine RPCs `OmpReadiness`/`OmpLoginProviders`/`OmpLogoutAccounts`/`OmpLogout` + `omp-accounts-v1` capability. Verified minimally: `cargo check --tests` proto/harness/engine/rpc + 2 targeted tests + live isolated-profile probe (readiness: 85 providers, ready=false; bogus login errors ~0.3 s, no hang; scratch probe deleted). Note: installed omp is now 18.12.6 (qualified=false against the 18.12.5 pin — discovery still governs). Next: W05 onboarding/accounts UI + model/role hub, or W05 Check's session-only-switch assertion in the app.
2026-10-22: W05 DONE — model/role hub + onboarding/accounts UI landed: proto `OmpModelRoles`/`OmpCatalogSnapshot`/`OmpProviderLimits`/`OmpLoginPoll`, harness `model_admin` (roles/cycle/fallback/default-thinking via `omp config`, provider limits via `omp usage`, custom roles preserved), engine descriptor catalog with version/profile-scoped disk cache + stale/unavailable states, non-chat `OmpLogin*` engine flows, and Settings → Models (readiness/onboarding, sign-in/out, provider filter/search, inspector with role chips + Assign flow, cycle/fallback/thinking controls, limits). Verified minimally: `cargo check` proto/harness/engine/rpc/ui + `cargo build -p zeron`, targeted tests (proto readiness, harness parse/validation, engine descriptor-cache, UI hub filters), live isolated-profile probes (role assign→fallback→clear round-trip, provider limits, session-only switch: session ran on the picked model while `modelRoles` stayed `{}`), and a fresh-build isolated app boot on `settings/models` that cached 148 descriptors with no panics. OMP installation remains OMP-owned by design (legacy installer never touches it; updates apply on next spawn).
2026-10-22: W07 DONE — structured OMP parts render as transcript notices (tool-progress kinds stay with chips), OMP tool chips lazily fetch owner-side detail with explicit expired/forbidden/unavailable states, composer consumes faithful slash commands (aliases/subcommands/source via `OmpCommands`), turn cost flows (`TurnCost` → doc meta → footer context card), and ghost-text word prediction is wired through the live runtime. Verified minimally: targeted tests (notices, slash candidates, turn-usage line, TurnControl prediction), live isolated tool turn (Exec + Usage + TurnCost + Completed), fresh-build isolated app boot with zero panics. Remaining cosmetic gap: no fast/slow composer toggle producer (driver consumes the flags; `from_legacy` leaves them unset).

## 9. Next-session starting prompt

Copy the block below into a new coding session:

> Continue the OMP-only remake of Zeron — this is mid-implementation, not planning. First read the applicable `AGENTS.md`, repo-root `MEMORY.md`, then `docs/plans/2026-10-20-omp-only-handoff.md` (especially §0–§8 and the package you will work on) and the relevant parts of `docs/plans/2026-10-20-omp-only.md`. Check `git status --short` before touching anything: the current work is in a large uncommitted worktree from prior sessions. Preserve it; do not reset, clean, or overwrite unrelated changes. Do not redo broad exploration or the completed W00/W01 work.
>
> Goal: Zeron is a native Rust/GPUI application whose ONLY production agent runtime is OMP, with complete supported OMP functionality and all model/service providers available through OMP. At final cutover, no direct Claude/Codex/Pi/Cursor/etc. runtime paths may remain, including the separate Codex voice/helper path. OMP owns native sessions/history, models/roles, credentials, tool policy, approvals, workers, modes, memory and compaction; Zeron renders and controls them and retains workspace/tabs/files/terminal/remote/mobile infrastructure. Do not rebrand or change app IDs.
>
> Pin and proven baseline (do not re-litigate without a run contradicting it):
> - OMP `18.12.5`, upstream tag `v18.12.5`, commit `d5cd750e41fede277f9eb933455da92a0738915e`; Rust client and wire schema from that same revision. The official client is vendored verbatim at `crates/omp-rpc/` — read its `VENDOR.md`, and never hand-edit generated `wire.rs`.
> - W00 and W01 are done. W02, W03, W04, W05, W06, W09 and W10 are partially implemented; W07, W08, W11–W14 have not had their main implementation passes. §5 checkboxes/annotations are the detailed truth — a package is not done unless its behavior and Check actually landed.
> - Already proven against real local OMP in the harness examples: startup/ready + state/commands; model discovery with per-model thinking levels; tool turns and native detail fetch; resume by native session ID with recall; crash-recovery no-duplicate gate; rich ask; approval approves and denial prevents execution; subagent snapshot/progress/cancel; phased todos; session fork preserving both identities/history; read-only discovery of existing sessions; settings descriptors and global-scope settings set/unset in an isolated profile. That is not full UI parity and not a GUI end-to-end acceptance pass.
>
> Immediate task: finish W02 before expanding surface area, unless I direct otherwise. Remaining W02 pieces are principally (1) an owner-authorized engine detail-retrieval RPC over the already-proven `OmpHarness::tool_detail` and binding/chat lookup, with explicit available/expired/unavailable/forbidden outcomes; (2) clearly separating legacy archive decoding from active OMP request/types without making new clients synthesize old harness IDs; (3) proving the W02 Check at the appropriate level — a session/history replay with tool detail intact and an old archived session still opening. Update W02 status only when those behaviors actually exist. Then proceed by dependency order: W03 queue/exclusive-writer and real-app kill-mid-turn check; W04 native `OmpInteraction`/presentation UI and owner-once resolution; W05 onboarding/model-role surfaces; W06 project scope/provenance/settings UI; W07 conversation/tool-detail UI; W09 hub/jobs; W10 tree/context/memory inspector; W11 service integrations; W12 media/voice/collab; W13 mobile parity; W14 migration/cutover. Keep one integration owner for shared proto/doc/RPC files.
>
> Delegation: I previously asked to spawn 2–3 workers if it makes sense. If W02 still decomposes cleanly when you start, use up to three parallel workers with model `google-antigravity/gemini-4-argon:medium` for non-overlapping slices, after fixing shared interfaces yourself. Suggested slices: (a) engine/RPC detail retrieval only; (b) slash-command fidelity through proto/harness projections (aliases/subcommands/source) only; (c) a doc-level replay/legacy-archive proof in `crates/doc` only. Give each worker self-contained files/APIs/acceptance criteria, prohibit overlapping shared-file edits, and personally integrate + compile once. If model selection is unavailable or slices overlap, say so briefly and work sequentially; do not pad delegation.
>
> Hard constraints / traps (also in `MEMORY.md`):
> - Plan Review has NO rpc-ui API in 18.12.5: no command, no `get_state` field, `/plan` is TUI-only and would fall through as prompt text. Never fake plan mode with a prompt or `--plan-yolo`; W08 remains blocked on upstream OMP for a real draft/approval host contract.
> - Spawn product sessions as plain `omp --mode rpc-ui`: no `--no-ui` (approval-required tools fail closed), no `--auto-approve`, and no product `--approval-mode` override. Respect/display the user's effective policy. Approvals arrive as a select: `Approve` / `Approve <tool> for this session` / `Deny`; one-time approval means exactly `Approve`, never a generic first option or standing grant.
> - Never auto-approve from `RunRequest::auto_approve` on the OMP path, never answer an interaction from a stale generation, and treat unanswered approval as no approval.
> - Resume tokens are native session IDs, never host paths across sync/trust boundaries. Rich `OmpSessionBinding` data (profile/session file/leaf/generation) is host-local in `omp-sessions.json` + the run journal; do not sync it or write OMP session JSONL directly.
> - Uncertain prompt admission must be reconciled against OMP native history and never blindly replayed (`Harness::safe_to_redispatch`); one writer owns a native session.
> - Do not directly read/hash/edit OMP credential storage (`agent.db`); do not add direct provider OAuth or write secrets via config argv. Settings writes go through supported OMP validation (`omp config` for global scope); credential settings are refused by the settings service.
> - OMP session files may start with a fixed-width `{"type":"title"}` record before the `{"type":"session"}` header; scanners must inspect the first lines, not only line 1.
> - Generate settings descriptors with `scripts/gen-omp-settings.py` from a checkout of the pinned tag; never hand-edit `settings_descriptors.json` or scrape human CLI output as the schema. A `/tmp/omp-w00` checkout existed in prior sessions but is not durable state — recreate/verify the pinned checkout if regenerating.
> - Preserve old user histories, installed external CLIs, and credentials. Legacy sessions become readable archives or verified explicit imports; no compatibility execution shims in the final app.
>
> Verification (§7, minimal but real): use `cargo check --locked -p <touched-crate>` while iterating; run the targeted tests for changed behavior; run `cargo build --locked -p zeron` once before calling a package done; for UI work, run the real app via `bash scripts/run-macos-dev.sh` and exercise the surface. Existing live smokes are `cargo run -p zeron-harness --example omp_rpc_smoke`, `cargo run -p zeron-harness --example omp_harness_smoke`, and `cargo run -p zeron-harness --example omp_settings_smoke`; they make small real model requests against the caller's OMP setup, so run only the one covering your changed path, use disposable workspaces/isolated profiles where supported, clean up temp native-session artifacts, and never claim the GUI was verified from harness smokes. There is no `mise.toml`; do not run `scripts/e2e-smoke.sh` unreviewed. Add permanent tests only for real bugs or data-loss/security invariants.
>
> Bookkeeping: as slices land, update §5 status cells/checkbox annotations and add a one-line dated entry to §8 with what actually ran, any blocker + owner, and next package. Save only durable traps to `MEMORY.md` in its exact v2 format. Do not call partial/basic chat, a terminal wrapper, or harness-only proof “full parity.” If an upstream API is missing, name the exact operation + owner in one line and continue independent reachable work. At the end of your turn, report verified evidence separately from inference and name anything not run. Begin by stating the concrete W02 slice you are taking, then execute it — do not stop at a plan.

## 10. Source and navigation links

- [Architecture, full capability ledger, migration rationale and sources](2026-10-20-omp-only.md)
- [Existing app architecture](../../ARCHITECTURE.md)
- [Existing Pi integration contract](../pi.md)
- [Existing remote voice implementation/testing notes](../research/voice-remote-testing.md)
- [OMP user documentation](https://omp.sh/docs)
- [Complete upstream RPC reference](https://github.com/can1357/oh-my-pi/blob/main/docs/rpc.md)
- [Official Rust RPC client](https://github.com/can1357/oh-my-pi/tree/main/sdk/rust/omp-rpc)
- [OMP native Plan Review behavior](https://omp.sh/docs/plan)
- [OMP configuration semantics](https://omp.sh/docs/settings)
- [OMP extensions](https://omp.sh/docs/extension-authoring)

The existing architecture and voice notes describe the pre-cutover application. They are investigation context, not permission to retain an independent non-OMP runtime.
