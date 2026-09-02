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
    fn parse(args: &[String]) -> Parsed {
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
                } else if i + 1 < args.len() && takes_value(rest) {
                    flags
                        .entry(rest.to_string())
                        .or_default()
                        .push(args[i + 1].clone());
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
        Parsed { positionals, flags }
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
        "body" | "body-file" | "label" | "assignee" | "repo" | "state" | "pr" | "kind" | "by"
    )
}

fn help() -> String {
    format!(
        "usage: {BIN} [command] [args] [flags]\n\
         commands[13]:\n\
         \x20 (none)=dashboard(list), add, list, show, start, done, reopen, hold, unhold, block, unblock, ready, version, update, setup\n\
         state model:\n\
         \x20 queued = open + unassigned · in-flight = open + assigned · done = closed · hold = `hold` label · blocked = `blocked-by: #<n>` in body\n\
         flags: --repo <owner/name>, --json, --help, -v/-V/--version\n\
         examples:\n\
         \x20 {BIN} add \"Fix login\" --label bug\n\
         \x20 {BIN} list --state ready\n\
         \x20 {BIN} ready\n\
         \x20 {BIN} show 42\n\
         \x20 {BIN} start 42\n\
         \x20 {BIN} done 42 --pr https://github.com/o/r/pull/7\n\
         \x20 {BIN} hold 42 --kind captain\n\
         \x20 {BIN} block 42 --by 39\n\
         \x20 {BIN} setup skill | setup hooks\n\
         \x20 {BIN} version | update [--check]"
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

    let parsed = Parsed::parse(args);
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
            ops::add(
                title,
                body.as_deref(),
                body_file.as_deref(),
                &labels,
                &assignees,
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
        "block" => {
            let n = number(rest, 0, "issue")?;
            let by = parsed
                .value("by")
                .ok_or_else(|| QueueError::usage("missing --by <n>"))?
                .parse::<u64>()
                .map_err(|_| QueueError::usage("invalid --by number"))?;
            ops::block(n, by, repo.as_deref(), json)
        }
        "unblock" => {
            let n = number(rest, 0, "issue")?;
            let by = parsed
                .value("by")
                .ok_or_else(|| QueueError::usage("missing --by <n>"))?
                .parse::<u64>()
                .map_err(|_| QueueError::usage("invalid --by number"))?;
            ops::unblock(n, by, repo.as_deref(), json)
        }
        "version" => {
            version::cmd_version();
            Ok(())
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
