# Handoff — 2026-10-21 (OMP-only migration, mid-W04)

Read this first, then the governing docs:

- [Plan](docs/plans/2026-10-20-omp-only.md) — package definitions, contracts C1–C8, risks.
- [Progress tracker](docs/plans/2026-10-20-omp-only-handoff.md) — §5 status table, per-package steps, §8 dated progress log. **The §9 "next-session prompt" in that doc is stale** — it re-lists work that has since landed. Trust §5 + §8, not §9.

## Git state — read before touching anything

- Branch state: commit `888e0255` ("w03") contains all W02-first-item and W03 work.
- **The worktree has an uncommitted diff (21 files, ~+750 lines): all of W04 plus one bug fix.** Do not discard it. Either commit it as-is (suggested message: `w04`) or keep working on top of it.
- Nothing from this session was pushed.

## Where the migration stands

- **Done:** W00 (audit), W01 (OMP driver), W03 (sessions/queue/recovery — package Check passed in the real app, user-run).
- **In progress:** W02 (schema), W04 (native UI), W05, W06, W09, W10.
- **Not started:** W07, W11–W14. **W08 is upstream-blocked** (OMP 18.12.5 has no Plan Review rpc-ui API; do not rebuild stored-markdown approval).

## What landed this session

### W02, first item (committed)

- Structured parts: `AgentEvent::OmpStructured` → `MessagePart::OmpStructured`, Loro round-trip as `kind: "ompStructured"` with `eventKind` + raw `native`. Execution updates coalesce per tool call in the fold; media/redacted/fallback/worker-state preserved as metadata records — **inline bytes and host paths never sync** (`sessionFile` is stripped from lifecycle payloads).
- `OmpSessionConfig::from_legacy` (proto) + client populates `RunRequest::omp` from chat config — previously always `None`, so the driver's preferential path was dead. Unqualified/`"default"` model strings yield no override, never a fabricated provider; above-ladder reasoning clamps to `max`.
- Tool output: fold keeps the bounded summary (`summarize_tool_output`); full text/diff never enters the doc; recovery is the owner-side detail fetch (`OmpToolDetail`, keyed by tool call id / `omp_entry_id`).
- Cleanup: removed ~25 stray duplicate `omp: None,` lines a prior automated edit left in test files (they broke `--tests` builds).

### W03 (committed; package DONE)

- `QueuedMessage.kind` (prompt/steer/follow-up, additive), `OmpQueueSnapshot`, `reconcile_with_native` — in-order text pairing; **duplicate identical texts report `Ambiguous`, never guessed** (native queue is text-only, no IDs).
- `TurnControl` queue-action channel: host sends Remove/Promote/Snapshot; the OMP driver answers against the live runtime; failures answer `false`/`None`. Engine surfaces: `omp_queue_snapshot` / `omp_remove_queued` / `omp_promote_queued` / `omp_reconcile_queue`.
- Delivery ledger (`dispatched_queue` on the doc-host handle): accepted steers recorded under exact dispatched text; Consumed rows pruned after dispatch and before removal; `remove_queued_message` falls back local → reconcile → native. `promote_queued_message` handles local (re-kind + steer slot) and dispatched follow-ups.
- Exclusive writer: `OmpBindingStore::conflict_for` — a second chat claiming the same (profile, native session) is refused and logged. Cross-device is structural (one owning host per chat; `omp_*` APIs are local-only).
- **Check passed (user-run, real app):** send → force-kill mid-turn → relaunch → session resumed, prompt present once, token recalled.
- Carried, non-gating: TUI handoff (upstream settle/release contract doesn't exist — **never write session JSONL to imitate it**), native queue chip UI (W07).

### W04 (UNCOMMITTED — in the worktree diff)

- Rich questions reach the desktop wizard: `UserInputQuestion.option_details` (description/preview per option) + `recommended`, filled by `to_questions` in `crates/harness/src/omp/interactions.rs`. Wizard renders description/preview sub-lines, a "Recommended" marker, and pre-picks single-select recommended options. Labels remain the answer currency; `from_answers` unchanged, still fail-closed.
- Approval card: wizard detects the approval select shape and renders "Approval required" + the operation verbatim in a mono block. When OMP gives no descriptions, the projection adds them: "Allow this one operation, this once" / "Allow this tool for the rest of the session" / "Do not run it". **Structured target/policy fields are upstream-blocked** — the wire carries only title/options/details.
- Presentation surfaces: engine broadcast hub + `WatchOmpPresentation` RPC + `omp-presentation-v1` capability; UI watch into AppState; shell drains one-shot effects once (Notify→toast, OpenUrl→browser — scheme-gated at the driver, EditorText→composer draft); Title/Status/Widget render as a muted strip above the composer (`Composer::set_editor_text` is new).
- Competing-view dismissal: already in place (wizard latch releases on doc `resolved`; engine resolves once) — verified, not rebuilt.

## Bug found by the user in-app — FIXED, in the uncommitted diff

Ask/approval answers hung forever. Cause: the engine input bridge (`spawn_input_forwarder`, `crates/engine/src/sessions.rs`) called `tokio::spawn`, but the OMP driver asks from a plain std thread (`handle_ui_request`, `crates/harness/src/omp/harness.rs`) → panic "no reactor running" → question rendered, answer thread dead, OMP waited forever. Evidence: panic in `target/macos-dev/data/logs/zeron-headed.log` (2026-10-21 17:05:54).

Fix: spawn on Tokio when a reactor exists, otherwise poll on a std thread with identical semantics; the driver thread catch-unwinds the bridge and answers `Cancelled` on panic instead of hanging. Regression test: `input_forwarder_works_without_a_tokio_reactor` (engine lib). **User confirmed the fix in the rebuilt app** (ask/approval completes after choosing; tool chips render correctly).

## Verified / not verified

Verified (minimal, per user's standing "limit testing" instruction — compile + targeted tests only, no full suites, no live smokes unless noted):

- `cargo check --tests` green for proto, rpc, doc, harness, client, engine, ui, mobile at various points in this session.
- Targeted tests: `session_config_from_legacy_maps_only_qualified_models`, `omp_structured_part_round_trips_through_the_doc`, `tool_progress_structured_parts_coalesce_per_c
...[truncated 3072 chars]