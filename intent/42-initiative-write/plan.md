# Plan: #42 — `add --initiative` write (two-dev slice)

From spec.md (#42). Author: Planner. Status: draft.

## Files that change

- `src/ops.rs` (dev-1) — `assemble_body` + `add` + validation + JSON + unit tests.
- `src/main.rs` (dev-2) — `--initiative` flag parse + `takes_value` + `help()`.
- `README.md`, `skills/cf-queue/SKILL.md` (dev-2) — verb + body-lines docs.

## Contract (interface both devs hold to)

`ops::add` gains `initiative: Option<u64>` immediately AFTER `epic: Option<u64>` and BEFORE
`requested_by: Option<&str>`. `assemble_body` gains a leading `initiative: Option<u64>` and writes,
in order: `Parent initiative: {n}\n` (no `#`) FIRST, then `Parent epic: #{n}\n`, then
`Requested-by: {e}\n`, then the user body.

## Order of work

1. dev-1 (worktree `cf-queue-dev1`, branch `feat/42-initiative-ops`) — `src/ops.rs` only.
2. dev-2 (worktree `cf-queue-dev2`, branch `feat/42-initiative-main`) — `src/main.rs` + docs.

Disjoint files, so the two branches merge cleanly — the "same-file conflict rule" is respected by
assigning each file to exactly one dev.

## Proof

- `cargo build` + `cargo test` + `cargo clippy` green at integration (CLAUDE.md crew rule 5).
- Unit test: `assemble_body(Some(7), None, None, "")` == `"Parent initiative: 7\n"`.

## Risks

- The cross-file signature change means neither dev's worktree compiles in isolation; the full
  suite runs only after integration. Expected — the interface is the contract.
