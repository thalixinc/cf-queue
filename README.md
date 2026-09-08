# cf-queue

AXI-compliant Rust CLI that turns GitHub issues into a durable work queue for AI agents. A thin, single-owner wrapper over `gh-axi` (mutations) and `gh` (structured reads) — **GitHub issues are the single source of truth**; the tool only encodes the state-machine + conventions layer on top.

Working agreement: [`intent/README.md`](intent/README.md) (the chain and the five verbs) and [`REVIEW.md`](REVIEW.md) (the review policy).

## What it gives the agent

- **One verb per intent**, not a remembered `gh` recipe: `cf-queue start 42` assigns the issue to you; `cf-queue ready` lists dispatchable work.
- **Token-efficient TOON output** (default) with `--json` opt-in.
- **Idempotent mutations** (`already: true` on a no-op), 0/1/2 exit codes.
- **State derived from the issue itself** — no separate queue file to drift out of sync.

## State model

| State | GitHub mechanism |
|---|---|
| `queued` | issue open, unassigned (and not held/blocked) |
| `in-flight` | issue open, assigned |
| `done` | issue closed (reason completed) |
| `hold` | label `hold` or `hold:founder` (any `hold:*`; `hold:captain` is read for one more release) |
| `blocked` | `blocked-by: #<n>` or `blocked-by: owner/repo#<m>` line in body, with that issue still open |

## Install

Requires `gh` (authenticated) and `gh-axi` on `PATH`.

```sh
cargo install --git https://github.com/thalixinc/cf-queue
```

## Quick start

```sh
cf-queue add "Fix login" --label bug
cf-queue list                 # whole queue, state derived
cf-queue ready                # dispatchable now
cf-queue start 42             # claim it
cf-queue done 42 --pr https://github.com/o/r/pull/7
cf-queue block 42 --by 39     # 42 depends on 39
cf-queue ready                # 42 disappears until 39 closes
cf-queue block 42 --by thalixinc/other#3   # cross-repo edge, resolved by other#3's state
cf-queue add "Status field for A#45" --epic 12 --requested-by thalixinc/a#45 --label task
cf-queue hold 42 --kind founder            # waits on the founder (label hold:founder)
```

## Body lines cf-queue reads and writes

| Line | Written by | Read by |
|---|---|---|
| `Parent epic: #<n>` (first line) | `add --epic <n>` (also links the sub-issue via `gh-axi issue subissue add`) | `show` → `epic` |
| `Requested-by: owner/repo#<m>` | `add --requested-by` | `show` → `requested_by` |
| `Artifacts: <path>` | the SDLC tooling | `show` → `artifacts` |
| `blocked-by: #<n>` / `blocked-by: owner/repo#<m>` | `block` / `unblock` | `list`, `ready`, `show` |

`add` keeps any `--body` / `--body-file` content after the machine lines.

## Verbs

| Verb | gh call |
|---|---|
| `add <title> [--body/--body-file] [--label]… [--assignee]… [--epic <n>] [--requested-by owner/repo#<m>]` | `gh-axi issue create` (+ `issue subissue add <n> <new>` with `--epic`) |
| `list [--state queued\|in-flight\|done\|hold\|blocked]` | `gh issue list --json …` + local derivation |
| `show <n>` | `gh issue view --json …`; prints `epic`, `requested_by`, `artifacts`, `blocked_by` (each edge resolved: open / closed / unresolved) |
| `start <n>` | `gh-axi issue edit <n> --add-assignee @me` |
| `done <n> [--pr <url>]` | `gh-axi issue close <n> --reason completed [--comment "PR: <url>"]` |
| `ship <pr> [--issue <n,…>] [--project <name>]` | `gh-axi pr merge <pr>` + close each linked issue (GitHub's `Closes/Fixes #<n>`, or the explicit `--issue` list) + `cf board sync <project>` — merge → close + mark done + board sync in one step |
| `reconcile --repo <owner/repo> --board <n> [--yes] [--org <org>]` | `gh project item-list` + `gh project item-delete` — find (report) and, with `--yes`, remove board items from other repositories so `cf-queue list` done == `cf board status` Done |
| `reopen <n>` | `gh-axi issue reopen <n>` |
| `hold <n> [--kind founder]` | `gh-axi issue edit <n> --add-label hold` or `hold:founder` (`--kind captain` is a deprecated alias for `founder`, removed next release) |
| `unhold <n>` | `gh-axi issue edit <n> --remove-label …` for the hold labels the issue carries (`already: true` when none) |
| `block <n> --by <m>\|owner/repo#<m>` | append `blocked-by: #<m>` or `blocked-by: owner/repo#<m>` to body |
| `unblock <n> --by …` | remove that line from body |
| `ready` | open + unassigned + un-held + un-blocked (cross-repo blockers checked with `gh issue view -R owner/repo <m> --json state`) |

Global flags: `--repo <owner/name>` (default: gh's repo detection), `--json`, `--help`, `-v/--version`. Plus `version`, `update [--check]`, `setup skill|hooks [--project]` — same shape as the rest of the AXI family.

## Output contract

`list`/`ready` render a compact TOON `queue[N]{number,state,title}:` list plus a `summary:` count block. Mutations lead `ok:` and print the resulting state. `--json` is the machine-read opt-in. Exit codes: 0 success / 1 operational / 2 usage.

## Known limitations

- Same-repo `blocked-by:` edges resolve against the open-issue list (no extra calls); each distinct cross-repo edge costs one `gh issue view` per `list`/`ready`. An edge that cannot be resolved (unknown repo, no permission) **counts as blocking** and is listed under `warnings:` (`--json`: `warnings[]`), never silently ignored.
- `ready` fetches all open issues (bounded by `--limit 100`); fine for a codefactory-scale queue, not a 10k-issue monorepo.
- `hold` and `blocked` are presented as a single primary state (priority done > hold > blocked > in-flight > queued); a held in-flight issue shows as `hold`.

## License

MIT
