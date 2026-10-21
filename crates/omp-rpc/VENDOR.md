# Vendored: omp-rpc (official OMP Rust RPC client)

- Source: https://github.com/can1357/oh-my-pi/tree/main/sdk/rust/omp-rpc
- Snapshot: tag `v18.12.5`, commit `d5cd750e41fede277f9eb933455da92a0738915e`
- Runtime pin: `omp/18.12.5` (see docs/plans/2026-10-20-omp-only-handoff.md, W00 record)
- License: MIT (see THIRD_PARTY_NOTICES.md)

Copied verbatim: `src/` (client.rs, frame.rs, host_tool.rs, host_uri.rs, lib.rs,
turn.rs, wire.rs). Excluded: upstream `tests/` and `Cargo.lock`.
Local changes: `Cargo.toml` only — dropped the standalone `[workspace]` table so
the crate joins this workspace. No source edits.

`src/wire.rs` is generated upstream by `bun run gen:rpc` from
`packages/coding-agent/src/modes/rpc/wire/rpc-wire.schema.json`. Never edit it
here. To update: check out the new upstream tag, re-copy `src/`, bump the pin
above and in the W00 record, then re-run the W01 smoke
(`cargo run -p zeron-harness --example omp_rpc_smoke`).
