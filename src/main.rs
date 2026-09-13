//! cf-queue — AXI-compliant GitHub-issue work queue for AI agents.

mod error;
mod gh;
mod ops;
mod setup;
mod toon;
mod version;

use error::{QueueError, Result};
use std::collections::HashMap;

const BIN: &str = "cf-queue";

struct Parsed {
    positionals: Vec<String>,
    flags: HashMap<String, Vec<String>>,
}

impl Parsed {
    fn parse(args: &[String]) -> Result<Parsed> {
        let mut positionals = Vec::new();
        let mut flags: HashMap<String, Vec<String>> = HashMap::new();
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            if let Some(rest) = a.strip_prefix("--") {
                if let Some((name, value)) = rest.split_once('=') {
                    flags
                        .entry(name.to_string())
                        .or_default()
                        .push(value.to_string());
                } else if takes_value(rest) {
                    // A value flag with nothing after it must not silently become "absent".
                    let v = args
                        .get(i + 1)
                        .ok_or_else(|| QueueError::usage(format!("--{rest} needs a value")))?;
                    flags.entry(rest.to_string()).or_default().push(v.clone());
                    i += 1;
                } else {
                    flags.entry(rest.to_string()).or_default();
                }
            } else if a == "-v" || a == "-V" || a == "-h" {
                flags.entry(a.clone()).or_default();
            } else {
                positionals.push(a.clone());
            }
            i += 1;
        }
        Ok(Parsed { positionals, flags })
    }

    fn flag(&self, name: &str) -> bool {
        self.flags.contains_key(name)
    }

    fn value(&self, name: &str) -> Option<String> {
        self.flags.get(name).and_then(|v| v.first().cloned())
    }

    fn values(&self, name: &str) -> Vec<String> {
        self.flags.get(name).cloned().unwrap_or_default()
    }
}

fn takes_value(name: &str) -> bool {
    matches!(
        name,
        "body"
            | "body-file"
            | "label"
            | "assignee"
            | "repo"
            | "state"
            | "pr"
            | "kind"
            | "by"
            | "epic"
            | "initiative"
            | "requested-by"
            | "project"
            | "board"
            | "org"
            | "issue"
    )
}

fn help() -> String {
    format!(
        "usage: {BIN} [command] [args] [flags]\n\
         commands[15]:\n\
         \x20 (none)=dashboard(list), add, list, show, start, done, reopen, hold, unhold, block, unblock, ready, ship, reconcile, version, update, setup\n\
         state model:\n\
         \x20 queued = open + unassigned · in-flight = open + assigned · done = closed · hold = `hold`/`hold:founder` label · blocked = `blocked-by: #<n>` or `owner/repo#<m>` in body (still open)\n\
         body lines:\n\
         \x20 Parent epic: #<n> (add --epic) · Parent initiative: <n> (add --initiative) · Requested-by: owner/repo#<m> (add --requested-by) · Artifacts: <path> · shown by `show`\n\
         ship (merge → close linked issues → board sync, one step):\n\
         \x20 {BIN} ship <pr> [--issue <n,...>] [--project <name>] [--repo <owner/name>]\n\
         done (close the ticket; INVARIANT — a `--pr` must be MERGED first):\n\
         \x20 {BIN} done <n> --pr https://github.com/o/r/pull/<n>   refuses unless the PR state is MERGED ('PR <url> not merged (state=X); merge first'), leaving the ticket open\n\
         \x20 {BIN} done <n>                                       no PR: closes as today (non-PR work)\n\
         reconcile (board back-fill: find/remove foreign board items so done == board Done):\n\
         \x20 {BIN} reconcile --repo <owner/name> --board <n> [--yes] [--org <org>]\n\
         version & updates (installs the latest release via `cargo install --git <repo> --force`):\n\
         \x20 {BIN} version                prints the version; a newer release prompts [y/N] (non-tty: reported, never blocks)\n\
         \x20 {BIN} version --yes          prints the version and auto-updates when a newer release exists\n\
         \x20 {BIN} update [--check] [--json]   update now; --check reports available; --json for machine-read\n\
         flags: --repo <owner/name>, --json, --help, -v/-V/--version\n\
         examples:\n\
         \x20 {BIN} add \"Fix login\" --label bug\n\
         \x20 {BIN} add "Status field" --epic 12 --requested-by owner/repo#45 --label task\n\
         \x20 {BIN} add "Shipping epic" --initiative 7 --label epic\n\
         \x20 {BIN} list --state blocked\n\
         \x20 {BIN} ready\n\
         \x20 {BIN} show 42\n\
         \x20 {BIN} start 42\n\
         \x20 {BIN} done 42 --pr https://github.com/o/r/pull/7\n\
         \x20 {BIN} ship 42 --project codefactory\n\
         \x20 {BIN} reconcile --repo thalixinc/codefactory --board 11\n\
         \x20 {BIN} hold 42 --kind founder\n\
         \x20 {BIN} block 42 --by 39 | --by owner/repo#3\n\
         \x20 {BIN} setup skill | setup hooks\n\
         \x20 {BIN} version [--yes] | update [--check] [--json]"
    )
}

fn required<'a>(args: &'a [String], idx: usize, what: &str) -> Result<&'a str> {
    args.get(idx)
        .map(String::as_str)
        .ok_or_else(|| QueueError::usage(format!("missing {what}")))
}

fn number(args: &[String], idx: usize, what: &str) -> Result<u64> {
    required(args, idx, what)?
        .parse::<u64>()
        .map_err(|_| QueueError::usage(format!("invalid {what} number")))
}

fn dispatch(args: &[String]) -> Result<()> {
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{}", help());
        return Ok(());
    }
    if args
        .iter()
        .any(|a| a == "-v" || a == "-V" || a == "--version")
    {
        println!("{BIN} {}", version::VERSION);
        return Ok(());
    }

    let parsed = Parsed::parse(args)?;
    let cmd = parsed.positionals.first().map(String::as_str).unwrap_or("");
    let rest = &parsed.positionals[1..];
    let json = parsed.flag("json");
    let repo = parsed.value("repo");

    match cmd {
        "add" => {
            let title = required(rest, 0, "title")?;
            let body = parsed.value("body");
            let body_file = parsed.value("body-file");
            let labels = parsed.values("label");
            let assignees = parsed.values("assignee");
            let epic = match parsed.value("epic") {
                Some(e) => Some(
                    e.trim_start_matches('#')
                        .parse::<u64>()
                        .map_err(|_| QueueError::usage("invalid --epic number"))?,
                ),
                None => None,
            };
            let initiative = match parsed.value("initiative") {
                Some(i) => Some(
                    i.trim_start_matches('#')
                        .parse::<u64>()
                        .map_err(|_| QueueError::usage("invalid --initiative number"))?,
                ),
                None => None,
            };
            let requested_by = parsed.value("requested-by");
            ops::add(
                title,
                body.as_deref(),
                body_file.as_deref(),
                &labels,
                &assignees,
                epic,
                initiative,
                requested_by.as_deref(),
                repo.as_deref(),
                json,
            )
        }
        "list" => {
            let state = parsed.value("state");
            ops::list(state.as_deref(), repo.as_deref(), json)
        }
        "ready" => ops::ready(repo.as_deref(), json),
        "show" => {
            let n = number(rest, 0, "issue")?;
            ops::show(n, repo.as_deref(), json)
        }
        "start" => {
            let n = number(rest, 0, "issue")?;
            ops::start(n, repo.as_deref(), json)
        }
        "done" => {
            let n = number(rest, 0, "issue")?;
            let pr = parsed.value("pr");
            ops::done(n, pr.as_deref(), repo.as_deref(), json)
        }
        "reopen" => {
            let n = number(rest, 0, "issue")?;
            ops::reopen(n, repo.as_deref(), json)
        }
        "hold" => {
            let n = number(rest, 0, "issue")?;
            let kind = parsed.value("kind");
            ops::hold(n, kind.as_deref(), repo.as_deref(), json)
        }
        "unhold" => {
            let n = number(rest, 0, "issue")?;
            ops::unhold(n, repo.as_deref(), json)
        }
        "ship" => {
            let n = number(rest, 0, "pr")?;
            let project = parsed.value("project");
            // Resolve the issues this PR closes (GitHub's `Closes/Fixes #<n>`), or an
            // explicit `--issue` list, so the merge closes + marks done in one step.
            let linked: Vec<u64> = if let Some(list) = parsed.value("issue") {
                list.split(',')
                    .map(|s| s.trim().trim_start_matches('#'))
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        s.parse::<u64>()
                            .map_err(|_| QueueError::usage("invalid --issue number"))
                    })
                    .collect::<Result<Vec<u64>>>()?
            } else {
                gh::gh_pr_closing_issues(n, repo.as_deref())?
            };
            ops::ship(n, &linked, project.as_deref(), repo.as_deref(), json)
        }
        "reconcile" => {
            let board = match parsed.value("board") {
                Some(b) => b
                    .trim_start_matches('#')
                    .parse::<u64>()
                    .map_err(|_| QueueError::usage("invalid --board number"))?,
                None => return Err(QueueError::usage("reconcile needs --board <n>")),
            };
            let repo_name = repo.clone().ok_or_else(|| {
                QueueError::usage("reconcile needs --repo <owner/repo> (the project's repository)")
            })?;
            let org = parsed
                .value("org")
                .unwrap_or_else(|| repo_name.split('/').next().unwrap_or("").to_string());
            let apply = parsed.flag("yes");
            ops::reconcile(&repo_name, board, &org, apply, json)
        }
        "block" | "unblock" => {
            let n = number(rest, 0, "issue")?;
            let by = parsed
                .value("by")
                .ok_or_else(|| QueueError::usage("missing --by <n> | owner/repo#<m>"))?;
            if cmd == "block" {
                ops::block(n, &by, repo.as_deref(), json)
            } else {
                ops::unblock(n, &by, repo.as_deref(), json)
            }
        }
        "version" => {
            version::cmd_version(parsed.flag("yes"))
        }
        "update" => version::cmd_update(parsed.flag("check"), json),
        "setup" => {
            let sub = rest.first().map(String::as_str).unwrap_or("");
            let global = !parsed.flag("project");
            match sub {
                "skill" => setup::cmd_setup_skill(global, json),
                "hooks" => setup::cmd_setup_hooks(global, json),
                _ => Err(QueueError::usage(
                    "unknown setup subcommand (skill | hooks)",
                )),
            }
        }
        other => Err(QueueError::usage(format!(
            "unknown command {other:?} — run `{BIN} --help`"
        ))),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = dispatch(&args) {
        eprintln!("{}", toon::error(&e.message, e.code, &e.suggestions));
        std::process::exit(e.exit_code());
    }
}
