# mem v2
# audience: models only. not docs. not changelog. loaded every session, so keep it small.
# load: after AGENTS.md, before non-trivial work.
# keep: traps, gotchas, schema, environment quirks, "don't do X" + why. one fact per entry.
# never: what code or docs already say, feature descriptions, test counts, status, todos.
# user rules ("never edit X") go in AGENTS.md, not here.
# write: upsert by id. never clobber other ids.
# delete: wrong, stale or promoted → remove the entry outright. vcs keeps history.
# tombstone: only to stop a false fact being re-learned: `k: !drop <why>`. remove after 30 days.
# conflict: AGENTS.md > this. this > unaided recall.
# budget: ≤40 entries, ~10 KB. k ≤300 chars. over budget → promote or delete before adding.
# when: date last verified. relying on one >30 days old → re-check; holds → bump, fails → delete.
# shape, exactly (blank line between entries):
#
# id: kebab-slug
# k: the fact, one line
# src: path/or/api
# when: 2026-10-20

id: omp-rpc-reference-drift
k: omp.sh/docs/rpc is incomplete versus upstream docs/rpc.md and observed OMP 18.12.5. Do not infer missing APIs from the website summary; pin source/runtime and probe. Upstream main may also exceed the installed build.
src: https://omp.sh/docs/rpc ; https://github.com/can1357/oh-my-pi/blob/main/docs/rpc.md
when: 2026-10-20

id: omp-plan-no-rpc-api
k: OMP 18.12.5 has NO Plan Review/plan-mode API over rpc-ui: no RPC command, no get_state field, /plan is handleTui-only (falls through as prompt text in RPC), rpc-mode never installs the xd://propose handler so plan-mode sessions strand. Don't simulate with prompts or --plan-yolo.
src: docs/plans/2026-10-20-omp-only-handoff.md (W00 record)
when: 2026-10-20

id: omp-rpc-ui-approval-flag
k: Spawn omp as plain `--mode rpc-ui`; adding `--no-ui` makes approval-required tools fail closed instead of prompting. MCP auth challenges have no RPC UI handler in any RPC mode.
src: https://github.com/can1357/oh-my-pi/blob/main/docs/rpc.md (Extension UI Sub-Protocol)
when: 2026-10-20

id: omp-version-format
k: `omp --version` prints `omp/18.12.5`; version parsers must accept name/semver. Shared binary_version probe also caches failures per process.
src: crates/harness/src/executable.rs
when: 2026-10-20

id: omp-approval-select-shape
k: OMP tool approvals arrive as extension-UI select with options [Approve, Approve <tool> for this session, Deny] and target/args in the question text — never answer first-option generically; one-time "Approve" only, session-wide grant is a distinct explicit choice.
src: crates/harness/examples/omp_harness_smoke.rs (live 2026-10-21)
when: 2026-10-21

id: omp-session-file-title-record
k: OMP session .jsonl files open with a fixed-width {"type":"title"} record (in-place retitle slot); the {"type":"session"} header is the second line. Scanners must scan first lines, not just line 1.
src: crates/harness/src/omp/discovery.rs
when: 2026-10-21
