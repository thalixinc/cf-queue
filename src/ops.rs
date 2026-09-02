//! Verb orchestration — the 11 queue verbs. Mutations delegate to `gh-axi`;
//! reads (list/show/ready) and body ops (block/unblock) use raw `gh --json`
//! and render cf-queue's own TOON.

use crate::error::{QueueError, Result};
use crate::gh;
use crate::toon;
use serde_json::json;
use std::collections::HashSet;

const BIN: &str = "cf-queue";
const DESCRIPTION: &str = "GitHub-issue work queue";

fn ghaxi_repo(repo: Option<&str>) -> Vec<&str> {
    match repo {
        Some(r) => vec!["-R", r],
        None => vec![],
    }
}

/// Run a gh-axi mutation and print a terse `ok:` confirmation.
fn mutation(args: &[&str], ok: &str, json: bool) -> Result<()> {
    gh::run_ghaxi(args)?;
    if json {
        println!(
            "{}",
            serde_json::to_string(&json!({"ok": true, "action": ok})).unwrap()
        );
    } else {
        println!("ok: {ok}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// add
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub fn add(
    title: &str,
    body: Option<&str>,
    body_file: Option<&str>,
    labels: &[String],
    assignees: &[String],
    repo: Option<&str>,
    json: bool,
) -> Result<()> {
    let mut args: Vec<&str> = vec!["issue", "create"];
    args.extend(ghaxi_repo(repo));
    args.push("--title");
    args.push(title);
    if let Some(b) = body {
        args.push("--body");
        args.push(b);
    }
    if let Some(f) = body_file {
        args.push("--body-file");
        args.push(f);
    }
    for l in labels {
        args.push("--label");
        args.push(l);
    }
    for a in assignees {
        args.push("--assignee");
        args.push(a);
    }
    let out = gh::run_ghaxi(&args)?;
    if json {
        println!(
            "{}",
            serde_json::to_string(&json!({"ok": true, "action": "add", "title": title})).unwrap()
        );
    } else {
        println!("ok: add");
        println!("{out}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// list / ready
// ---------------------------------------------------------------------------

/// Compute counts per primary state.
fn summarize(issues: &[gh::Issue], open_set: &HashSet<u64>) -> Vec<(String, usize)> {
    let mut counts: Vec<(&'static str, usize)> = vec![
        ("queued", 0),
        ("in-flight", 0),
        ("done", 0),
        ("hold", 0),
        ("blocked", 0),
    ];
    for i in issues {
        let s = i.state_of(open_set);
        if let Some(c) = counts.iter_mut().find(|(name, _)| *name == s) {
            c.1 += 1;
        }
    }
    counts
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
}

pub fn list(state: Option<&str>, repo: Option<&str>, json: bool) -> Result<()> {
    // "done" filters closed issues; everything else works from the open set.
    let (issues, open_set): (Vec<gh::Issue>, HashSet<u64>) = if state == Some("done") {
        let closed = gh::gh_list_issues("closed", repo)?;
        (closed, HashSet::new())
    } else {
        let open = gh::gh_list_issues("open", repo)?;
        let set: HashSet<u64> = open.iter().map(|i| i.number).collect();
        (open, set)
    };

    let filtered: Vec<&gh::Issue> = issues
        .iter()
        .filter(|i| match state {
            Some(s) => i.state_of(&open_set) == s,
            None => true,
        })
        .collect();

    if json {
        let arr: Vec<_> = filtered
            .iter()
            .map(|i| {
                json!({
                    "number": i.number, "state": i.state_of(&open_set), "title": i.title,
                    "assignees": i.assignee_logins(), "labels": i.label_names(), "url": i.url,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"ok": true, "queue": arr})).unwrap()
        );
        return Ok(());
    }

    let rows: Vec<Vec<String>> = filtered
        .iter()
        .map(|i| {
            vec![
                i.number.to_string(),
                i.state_of(&open_set).to_string(),
                i.title.clone(),
            ]
        })
        .collect();
    let summary = summarize(&issues, &open_set)
        .into_iter()
        .map(|(k, v)| format!("  {k}: {v}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut blocks = vec![
        toon::header(BIN, DESCRIPTION),
        toon::list("queue", &["number", "state", "title"], &rows),
        format!("summary:\n{summary}"),
    ];
    let help = vec![format!("Run `cf-queue show <n>` for details")];
    blocks.push(toon::help(&help));
    println!("{}", toon::join(&blocks));
    Ok(())
}

pub fn ready(repo: Option<&str>, json: bool) -> Result<()> {
    list(Some("queued"), repo, json)
}

// ---------------------------------------------------------------------------
// show
// ---------------------------------------------------------------------------

pub fn show(number: u64, repo: Option<&str>, json: bool) -> Result<()> {
    let i = gh::gh_view_issue(number, repo)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "number": i.number, "state": i.state, "title": i.title,
                "assignees": i.assignee_logins(), "labels": i.label_names(),
                "body": i.body, "url": i.url,
            }))
            .unwrap()
        );
        return Ok(());
    }
    println!(
        "{}",
        toon::join(&[
            toon::header(BIN, DESCRIPTION),
            format!(
                "#{} {}\nstate: {}\nassignees: {}\nlabels: {}",
                i.number,
                i.title,
                i.state.to_lowercase(),
                if i.assignee_logins().is_empty() {
                    "—".to_string()
                } else {
                    i.assignee_logins()
                },
                if i.label_names().is_empty() {
                    "—".to_string()
                } else {
                    i.label_names()
                },
            ),
            i.body.clone(),
        ])
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// start / done / reopen / hold / unhold
// ---------------------------------------------------------------------------

pub fn start(number: u64, repo: Option<&str>, json: bool) -> Result<()> {
    let n = number.to_string();
    let mut args: Vec<&str> = vec!["issue", "edit", &n, "--add-assignee", "@me"];
    args.extend(ghaxi_repo(repo));
    mutation(&args, &format!("start {number} -> in-flight"), json)
}

pub fn done(number: u64, pr: Option<&str>, repo: Option<&str>, json: bool) -> Result<()> {
    let n = number.to_string();
    let mut args: Vec<&str> = vec!["issue", "close", &n, "--reason", "completed"];
    let comment = pr.map(|u| format!("PR: {u}"));
    if let Some(c) = &comment {
        args.push("--comment");
        args.push(c);
    }
    args.extend(ghaxi_repo(repo));
    mutation(&args, &format!("done {number}"), json)
}

pub fn reopen(number: u64, repo: Option<&str>, json: bool) -> Result<()> {
    let n = number.to_string();
    let mut args: Vec<&str> = vec!["issue", "reopen", &n];
    args.extend(ghaxi_repo(repo));
    mutation(&args, &format!("reopen {number}"), json)
}

pub fn hold(number: u64, kind: Option<&str>, repo: Option<&str>, json: bool) -> Result<()> {
    let n = number.to_string();
    let label = if kind == Some("captain") {
        "hold:captain"
    } else {
        "hold"
    };
    let mut args: Vec<&str> = vec!["issue", "edit", &n, "--add-label", label];
    args.extend(ghaxi_repo(repo));
    mutation(&args, &format!("hold {number} ({label})"), json)
}

pub fn unhold(number: u64, repo: Option<&str>, json: bool) -> Result<()> {
    let n = number.to_string();
    let mut args: Vec<&str> = vec![
        "issue",
        "edit",
        &n,
        "--remove-label",
        "hold",
        "--remove-label",
        "hold:captain",
    ];
    args.extend(ghaxi_repo(repo));
    mutation(&args, &format!("unhold {number}"), json)
}

// ---------------------------------------------------------------------------
// block / unblock (body manipulation)
// ---------------------------------------------------------------------------

fn edit_body(number: u64, repo: Option<&str>, new_body: &str) -> Result<()> {
    let n = number.to_string();
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("cf-queue-{number}-{}.md", std::process::id()));
    std::fs::write(&tmp, new_body)
        .map_err(|e| QueueError::operational(format!("cannot write temp body: {e}"), "BODY"))?;
    let tmp_s = tmp.to_string_lossy().to_string();
    let mut args: Vec<&str> = vec!["issue", "edit", &n, "--body-file", &tmp_s];
    args.extend(ghaxi_repo(repo));
    let res = gh::run_ghaxi(&args);
    let _ = std::fs::remove_file(&tmp);
    res.map(|_| ())
}

pub fn block(number: u64, by: u64, repo: Option<&str>, json: bool) -> Result<()> {
    let body = gh::gh_view_body(number, repo)?;
    let edge = format!("blocked-by: #{by}");
    if body.lines().any(|l| l.trim() == edge) {
        if json {
            println!(
                "{}",
                serde_json::to_string(&json!({"ok": true, "already": true})).unwrap()
            );
        } else {
            println!("already: true ({edge})");
        }
        return Ok(());
    }
    let mut new_body = body.trim_end().to_string();
    if !new_body.is_empty() {
        new_body.push('\n');
    }
    new_body.push_str(&format!("{edge}\n"));
    edit_body(number, repo, &new_body)?;
    if json {
        println!(
            "{}",
            serde_json::to_string(
                &json!({"ok": true, "action": "block", "number": number, "by": by})
            )
            .unwrap()
        );
    } else {
        println!("ok: block {number} by {edge}");
    }
    Ok(())
}

pub fn unblock(number: u64, by: u64, repo: Option<&str>, json: bool) -> Result<()> {
    let body = gh::gh_view_body(number, repo)?;
    let edge = format!("blocked-by: #{by}");
    let kept: Vec<&str> = body.lines().filter(|l| l.trim() != edge).collect();
    if kept.len() == body.lines().count() {
        if json {
            println!(
                "{}",
                serde_json::to_string(&json!({"ok": true, "already": true})).unwrap()
            );
        } else {
            println!("already: true (no {edge})");
        }
        return Ok(());
    }
    let new_body = kept.join("\n");
    edit_body(number, repo, &new_body)?;
    if json {
        println!(
            "{}",
            serde_json::to_string(
                &json!({"ok": true, "action": "unblock", "number": number, "by": by})
            )
            .unwrap()
        );
    } else {
        println!("ok: unblock {number} (removed {edge})");
    }
    Ok(())
}
