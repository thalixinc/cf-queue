# cf-queue

AXI-compliant Rust CLI that turns GitHub issues into a durable work queue for AI agents. A thin, single-owner wrapper over `gh-axi` (mutations) and `gh` (structured reads) — **GitHub issues are the single source of truth**; the tool only encodes the state-machine + conventions layer on top.

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
| `hold` | label `hold` (or `hold:captain`) |
| `blocked` | `blocked-by: #<n>` line in body, with `#<n>` still open |

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
```

## Verbs

| Verb | gh call |
|---|---|
| `add <title> [--body/--body-file] [--label]… [--assignee]…` | `gh-axi issue create` |
| `list [--state queued\|in-flight\|done\|hold\|blocked]` | `gh issue list --json …` + local derivation |
| `show <n>` | `gh issue view --json …` |
| `start <n>` | `gh-axi issue edit <n> --add-assignee @me` |
| `done <n> [--pr <url>]` | `gh-axi issue close <n> --reason completed [--comment "PR: <url>"]` |
| `reopen <n>` | `gh-axi issue reopen <n>` |
| `hold <n> [--kind captain]` | `gh-axi issue edit <n> --add-label hold[:captain]` |
| `unhold <n>` | `gh-axi issue edit <n> --remove-label hold --remove-label hold:captain` |
| `block <n> --by <m>` | append `blocked-by: #<m>` to body |
| `unblock <n> --by <m>` | remove that line from body |
| `ready` | open + unassigned + un-held + un-blocked |

Global flags: `--repo <owner/name>` (default: gh's repo detection), `--json`, `--help`, `-v/--version`. Plus `version`, `update [--check]`, `setup skill|hooks [--project]` — same shape as the rest of the AXI family.

## Output contract

`list`/`ready` render a compact TOON `queue[N]{number,state,title}:` list plus a `summary:` count block. Mutations lead `ok:` and print the resulting state. `--json` is the machine-read opt-in. Exit codes: 0 success / 1 operational / 2 usage.

## Known limitations

- `blocked-by:` edges are **same-repo only** (`#<n>`), resolved against the open-issue list; cross-repo blockers are out of scope.
- `ready` fetches all open issues (bounded by `--limit 100`); fine for a codefactory-scale queue, not a 10k-issue monorepo.
- `hold` and `blocked` are presented as a single primary state (priority done > hold > blocked > in-flight > queued); a held in-flight issue shows as `hold`.

## License

MIT
