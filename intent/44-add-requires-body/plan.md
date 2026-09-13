# Plan: #44 — `cf-queue add` omits `--body` (GHAXI_FAILED on every add)

Issue: #44. Requested-by: thalixinc/codefactory#442. Author: Planner. Status: draft.

## Goal

`gh-axi issue create` requires BOTH `--title` AND `--body` when non-interactive. `cf-queue add`
builds the argv with `--title` and flags, but omits `--body` entirely when the user passes neither
`--body` nor `--body-file` (and no `--epic`/`--initiative`/`--requested-by`, which take the
`--body-file` path). Result: `cf-queue add "any title" --label bug` fails with `GHAXI_FAILED` on
every invocation.

## Fix

Always pass a body flag to `gh-axi issue create`: `--body <b>` when a body is given, `--body-file
<f>` when a file is given, and an empty `--body ""` when neither — so the argv never omits it.

## Files that change

- `src/ops.rs` — extract a `body_args(body, body_file) -> Vec<&str>` helper (single owner of the
  body-flag decision) and use it in the `add` `else` branch; add a regression test.

## Validation

- `cargo test` — new `add_always_passes_a_body_flag` unit test asserts the three cases, including
  the empty `--body ""` fallback (fails pre-fix).
- `cargo build` / `cargo clippy` clean.
- Smoke: a real `cf-queue add "…" --label bug` against a disposable repo creates the issue (no
  `GHAXI_FAILED`).

## Out of scope

- The separate `gh-axi` `version` verb gap (no `gh-axi version`) — a gh-axi repo item, flagged
  separately.
