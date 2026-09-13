# Spec: cf-queue `add --initiative` (write half of codefactory#426)

Issue: #42. Requested-by: thalixinc/codefactory#426. Author: Planner. Status: draft.

## Requirement

`cf-queue add --initiative <m>` writes `Parent initiative: <m>` as an EPIC's first body line,
mirroring the existing `Parent epic:` write for tasks. This is the WRITE half of codefactory#426
(Initiative tier); the codefactory read side (`cf board` `want_initiative` + `initiative_nesting`)
already merged and consumes this line — the read is dead until this write lands.

## Resolved decision — `--initiative` takes a NUMBER, not a slug

`--initiative <n>` takes an **issue number** (u64), exactly like `--epic <n>`. Not a slug.

Evidence (codefactory read side, already merged):

- `want_initiative` returns "the parent initiative number" `<N>` from the epic's first body line.
- `initiative_nesting` groups an epic under an initiative by `pinit == $inum`, where
  `$inum = (.number | tostring)` — i.e. the value written after `Parent initiative:` MUST be the
  parent initiative's issue number (as a string) to match.
- cf-queue references issues by number everywhere else (`--epic <n>`, `block --by <m>`).

A slug would never equal `$inum`, so the read side would silently orphan every initiative-parented
epic. Number is the only form the read side can consume.

## Written line

`Parent initiative: <n>` — NO `#` prefix, matching the ticket's `<m>`, the codefactory spec's
canonical `<N>`, and the read side's `$inum`. The read side also tolerates `#<n>` (it strips `#`),
but the canonical write is bare. This is an intentional asymmetry with `Parent epic: #<n>`,
inherited from the codefactory spec, which uses bare `<N>` for initiatives.

## Behaviour (mirrors `--epic` where it makes sense)

- `--initiative <n>` and `--epic <n>` are **mutually exclusive** — an epic belongs to an
  initiative, a task to an epic; the two parent lines are different tiers and never co-occur.
  Passing both is a usage error.
- `--initiative <n>` **validates the parent issue exists** before creating anything
  (`gh issue view`), exactly like `--epic`.
- **No sub-issue link** is created. The read side derives the initiative → epic relationship purely
  from labels + the `Parent initiative:` body line (`initiative_nesting` — "never a board field,
  never a guessed parent"), so a `gh-axi issue subissue add` link would be dead weight. This differs
  from `--epic`, where the sub-issue link IS the GitHub-native epic → task mechanism the read side
  relies on.

## Acceptance

- `cf-queue add "Shipping" --initiative 7 --label epic` creates an epic whose FIRST body line is
  `Parent initiative: 7`.
- `cf-queue add "x" --initiative 7 --epic 12` fails with a usage error.
- `cf-queue add "x" --initiative 999999` fails before creating (parent missing).
- `--json` output includes `"initiative": <n>`.
