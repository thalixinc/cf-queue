---
name: cf-queue
description: GitHub-issue work queue for AI agents — high-level verbs (add/list/start/done/hold/block/ready) over gh-axi/gh, with GitHub issues as the source of truth. Use when adding, listing, starting, completing, holding, or unblocking tracked work.
user-invocable: false
---

# cf-queue

AXI-compliant wrapper over `gh-axi`/`gh` that turns GitHub issues into a durable work queue. GitHub issues are the single source of truth.

Run the CLI for the always-current surface — do not trust memorized flags:

```sh
cf-queue            # dashboard: queue summary
cf-queue --help     # full command reference
```

Core verbs:

- `cf-queue add <title> [--body …] [--label …] [--assignee …]` — file work (creates an issue).
- `cf-queue list [--state queued|in-flight|done|hold|blocked]` — the queue.
- `cf-queue ready` — work that is dispatchable now (open, unassigned, un-held, un-blocked).
- `cf-queue start <n>` — claim it (assign yourself → in-flight).
- `cf-queue done <n> [--pr <url>]` — close it.
- `cf-queue hold <n> [--kind captain]` / `unhold <n>` — pause / resume.
- `cf-queue block <n> --by <m>` / `unblock <n> --by <m>` — dependency edges.

State is derived from the issue: `queued` = open + unassigned · `in-flight` = open + assigned · `done` = closed · `hold` = `hold` label · `blocked` = `blocked-by: #<n>` line in the body.
