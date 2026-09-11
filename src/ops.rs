//! Verb orchestration — the 11 queue verbs. Mutations delegate to `gh-axi`;
//! reads (list/show/ready) and body ops (block/unblock) use raw `gh --json`
//! and render cf-queue's own TOON.

use crate::error::{QueueError, Result};
use crate::gh::{self, Edge};
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

fn already(json: bool, note: &str) {
    if json {
        println!(
            "{}",
            serde_json::to_string(&json!({"ok": true, "already": true})).unwrap()
        );
    } else {
        println!("already: true ({note})");
    }
}

/// Write `text` to a private temp file and hand it to `f` as a path; the file is removed afterwards.
/// `create_new` refuses a pre-planted path (symlink included) instead of following it.
fn with_temp_body<T>(tag: &str, text: &str, f: impl FnOnce(&str) -> Result<T>) -> Result<T> {
    use std::io::Write;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("cf-queue-{tag}-{}-{nanos}.md", std::process::id()));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let written = opts
        .open(&tmp)
        .and_then(|mut fh| fh.write_all(text.as_bytes()))
        .map_err(|e| QueueError::operational(format!("cannot write temp body {}: {e}", tmp.display()), "BODY"));
    let res = written.and_then(|_| f(&tmp.to_string_lossy()));
    let _ = std::fs::remove_file(&tmp);
    res
}

// ---------------------------------------------------------------------------
// add
// ---------------------------------------------------------------------------

/// Machine lines first (`Parent epic:`, `Requested-by:`), then the author's body.
fn assemble_body(epic: Option<u64>, requested_by: Option<&Edge>, user: &str) -> String {
    let mut out = String::new();
    if let Some(n) = epic {
        out.push_str(&format!("Parent epic: #{n}\n"));
    }
    if let Some(e) = requested_by {
        out.push_str(&format!("Requested-by: {e}\n"));
    }
    let user = user.trim();
    if !user.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(user);
        out.push('\n');
    }
    out
}

/// The new issue's number and URL, from gh-axi's `issue create` output.
fn created(out: &str) -> Option<(u64, String)> {
    // gh-axi quotes the url (`url: "https://…"`), so strip quotes before matching.
    let url = out
        .split_whitespace()
        .map(|w| w.trim_matches('"'))
        .find(|w| w.starts_with("https://github.com/") && w.contains("/issues/"))?;
    let n = url.rsplit("/issues/").next()?.trim_end_matches(|c: char| !c.is_ascii_digit());
    Some((n.parse().ok()?, url.to_string()))
}

#[allow(clippy::too_many_arguments)]
pub fn add(
    title: &str,
    body: Option<&str>,
    body_file: Option<&str>,
    labels: &[String],
    assignees: &[String],
    epic: Option<u64>,
    requested_by: Option<&str>,
    repo: Option<&str>,
    json: bool,
) -> Result<()> {
    let requested_by = match requested_by {
        Some(s) => {
            let e = Edge::parse(s)?;
            if !e.is_cross() {
                return Err(QueueError::usage(format!(
                    "--requested-by needs owner/repo#<m> (got {s:?})"
                )));
            }
            Some(e)
        }
        None => None,
    };
    if body.is_some() && body_file.is_some() {
        return Err(QueueError::usage("use only one of --body / --body-file"));
    }
    if let Some(n) = epic {
        // Fail before creating anything if the parent does not exist.
        gh::gh_view_issue(n, repo)
            .map_err(|e| QueueError::usage(format!("--epic {n}: {}", e.message)))?;
    }

    let mut args: Vec<&str> = vec!["issue", "create"];
    args.extend(ghaxi_repo(repo));
    args.push("--title");
    args.push(title);
    for l in labels {
        args.push("--label");
        args.push(l);
    }
    for a in assignees {
        args.push("--assignee");
        args.push(a);
    }

    let out = if epic.is_some() || requested_by.is_some() {
        let user = match (body, body_file) {
            (Some(b), _) => b.to_string(),
            (None, Some(f)) => std::fs::read_to_string(f).map_err(|e| {
                QueueError::operational(format!("cannot read {f}: {e}"), "BODY")
            })?,
            (None, None) => String::new(),
        };
        let text = assemble_body(epic, requested_by.as_ref(), &user);
        with_temp_body("add", &text, |path| {
            let mut a = args.clone();
            a.push("--body-file");
            a.push(path);
            gh::run_ghaxi(&a)
        })?
    } else {
        if let Some(b) = body {
            args.push("--body");
            args.push(b);
        }
        if let Some(f) = body_file {
            args.push("--body-file");
            args.push(f);
        }
        gh::run_ghaxi(&args)?
    };

    let new = created(&out);
    match (epic, &new) {
        (Some(parent), Some((child, url))) => {
            let p = parent.to_string();
            let c = child.to_string();
            let mut a: Vec<&str> = vec!["issue", "subissue", "add", &p, &c];
            a.extend(ghaxi_repo(repo));
            // The issue exists either way: say which one, so nobody creates it twice.
            gh::run_ghaxi(&a).map_err(|e| {
                QueueError::operational(
                    format!("issue #{child} created ({url}), but linking it under epic #{parent} failed: {}", e.message),
                    "GHAXI_FAILED",
                )
                .with_suggestions(vec![format!("Run `gh-axi issue subissue add {parent} {child}` to finish the link.")])
            })?;
        }
        (Some(parent), None) => {
            return Err(QueueError::operational(
                "issue created but its number could not be read from gh-axi output; sub-issue link not made",
                "GHAXI_PARSE",
            )
            .with_suggestions(vec![format!("Run `gh-axi issue subissue add {parent} <new>` by hand.")]));
        }
        (None, _) => {}
    }

    if json {
        println!(
            "{}",
            serde_json::to_string(&json!({
                "ok": true, "action": "add", "title": title,
                "number": new.as_ref().map(|(n, _)| *n), "url": new.as_ref().map(|(_, u)| u.clone()),
                "epic": epic, "requested_by": requested_by.map(|e| e.to_string()),
            }))
            .unwrap()
        );
    } else {
        match &new {
            Some((n, _)) => println!("ok: add #{n}"),
            None => println!("ok: add"),
        }
        if let Some(n) = epic {
            println!("epic: {n} (sub-issue linked)");
        }
        println!("{out}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// list / ready
// ---------------------------------------------------------------------------

/// Compute counts per primary state.
fn summarize(issues: &[gh::Issue], blockers: &gh::Blockers) -> Vec<(String, usize)> {
    let mut counts: Vec<(&'static str, usize)> = vec![
        ("queued", 0),
        ("in-flight", 0),
        ("done", 0),
        ("hold", 0),
        ("blocked", 0),
    ];
    for i in issues {
        let s = i.state_of(blockers);
        if let Some(c) = counts.iter_mut().find(|(name, _)| *name == s) {
            c.1 += 1;
        }
    }
    counts
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
}

const STATES: [&str; 5] = ["queued", "in-flight", "done", "hold", "blocked"];

pub fn list(state: Option<&str>, repo: Option<&str>, json: bool) -> Result<()> {
    if let Some(s) = state {
        if !STATES.contains(&s) {
            return Err(QueueError::usage(format!(
                "unknown --state {s:?} (use {})",
                STATES.join(" | ")
            )));
        }
    }
    // "done" filters closed issues; a specific open-state filter works from the open set; the
    // UNFILTERED summary needs BOTH open and closed, so a closed issue counts toward `done`
    // (matching the board). A closed issue is `done` before any edge is consulted.
    let (issues, open_set): (Vec<gh::Issue>, HashSet<u64>) = match state {
        Some("done") => {
            let closed = gh::gh_list_issues("closed", repo)?;
            (closed, HashSet::new())
        }
        Some(_) => {
            let open = gh::gh_list_issues("open", repo)?;
            let set: HashSet<u64> = open.iter().map(|i| i.number).collect();
            (open, set)
        }
        None => {
            let open = gh::gh_list_issues("open", repo)?;
            let set: HashSet<u64> = open.iter().map(|i| i.number).collect();
            let mut all = open;
            all.extend(gh::gh_list_issues("closed", repo)?);
            (all, set)
        }
    };
    // Closed issues are `done` before any edge is consulted: no lookups for them.
    let blockers = if state == Some("done") {
        gh::Blockers::default()
    } else {
        gh::resolve_blockers(&issues, open_set)
    };
    let warnings = blockers.warnings();

    let filtered: Vec<&gh::Issue> = issues
        .iter()
        .filter(|i| match state {
            Some(s) => i.state_of(&blockers) == s,
            None => true,
        })
        .collect();

    if json {
        let arr: Vec<_> = filtered
            .iter()
            .map(|i| {
                json!({
                    "number": i.number, "state": i.state_of(&blockers), "title": i.title,
                    "assignees": i.assignee_logins(), "labels": i.label_names(), "url": i.url,
                    "blocked_by": i.blocked_by().iter().map(|e| e.to_string()).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"ok": true, "queue": arr, "warnings": warnings}))
                .unwrap()
        );
        return Ok(());
    }

    let rows: Vec<Vec<String>> = filtered
        .iter()
        .map(|i| {
            vec![
                i.number.to_string(),
                i.state_of(&blockers).to_string(),
                i.title.clone(),
            ]
        })
        .collect();
    let summary = summarize(&issues, &blockers)
        .into_iter()
        .map(|(k, v)| format!("  {k}: {v}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut blocks = vec![
        toon::header(BIN, DESCRIPTION),
        toon::list("queue", &["number", "state", "title"], &rows),
        format!("summary:\n{summary}"),
    ];
    if !warnings.is_empty() {
        blocks.push(format!(
            "warnings[{}]:\n{}",
            warnings.len(),
            warnings.iter().map(|w| format!("  - {w}")).collect::<Vec<_>>().join("\n")
        ));
    }
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

fn dash(s: Option<String>) -> String {
    s.filter(|s| !s.is_empty()).unwrap_or_else(|| "—".to_string())
}

pub fn show(number: u64, repo: Option<&str>, json: bool) -> Result<()> {
    let i = gh::gh_view_issue(number, repo)?;
    // Every edge resolved by the blocker's own state (same-repo ones too: show is one issue).
    let edges: Vec<(Edge, std::result::Result<bool, String>)> = i
        .blocked_by()
        .into_iter()
        .map(|e| {
            let r = gh::issue_open(e.repo.as_deref().or(repo), e.number).map_err(|err| err.message);
            (e, r)
        })
        .collect();
    let edge_text = |r: &std::result::Result<bool, String>| match r {
        Ok(true) => "open".to_string(),
        Ok(false) => "closed".to_string(),
        Err(m) => format!("unresolved: {m}"),
    };

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "number": i.number, "state": i.state, "title": i.title,
                "assignees": i.assignee_logins(), "labels": i.label_names(),
                "epic": i.epic(), "requested_by": i.requested_by(), "artifacts": i.artifacts(),
                "blocked_by": edges.iter().map(|(e, r)| json!({
                    "ref": e.to_string(), "open": r.as_ref().ok(), "error": r.as_ref().err(),
                })).collect::<Vec<_>>(),
                "body": i.body, "url": i.url,
            }))
            .unwrap()
        );
        return Ok(());
    }
    let blocked_by = if edges.is_empty() {
        "—".to_string()
    } else {
        edges
            .iter()
            .map(|(e, r)| format!("{e} ({})", edge_text(r)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    println!(
        "{}",
        toon::join(&[
            toon::header(BIN, DESCRIPTION),
            format!(
                "#{} {}\nstate: {}\nassignees: {}\nlabels: {}\nepic: {}\nrequested_by: {}\nartifacts: {}\nblocked_by: {}",
                i.number,
                i.title,
                i.state.to_lowercase(),
                dash(Some(i.assignee_logins())),
                dash(Some(i.label_names())),
                dash(i.epic().map(|n| n.to_string())),
                dash(i.requested_by()),
                dash(i.artifacts()),
                blocked_by,
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

/// The close-before-merge decision: `PR <url> not merged (state=X); merge first` when the PR
/// state is anything but `MERGED`. Pure (no shell-out) so the OPEN→refuse / MERGED→allow pair
/// is unit-testable against a fixture state string.
fn refuse_if_not_merged(url: &str, state: &str) -> Result<()> {
    if state == "MERGED" {
        Ok(())
    } else {
        Err(QueueError::usage(format!(
            "PR {url} not merged (state={state}); merge first"
        )))
    }
}

pub fn done(number: u64, pr: Option<&str>, repo: Option<&str>, json: bool) -> Result<()> {
    // Invariant (#321): when a PR is supplied, it MUST be MERGED before the ticket closes.
    // A seat's optimism ("I opened the PR, so done") is refused here, not a discipline nudge:
    // close-before-merge is a queue-tool invariant.
    if let Some(url) = pr {
        // Resolve the PR reference (URL or bare number) to its owner/repo + number.
        let (pr_repo, pr_number) = match gh::parse_pr_url(url) {
            Some(r) => r,
            None => {
                return Err(QueueError::usage(format!(
                    "PR reference {url:?} is not a GitHub pull request URL; close-before-merge cannot be verified"
                )))
            }
        };
        let repo_for_pr = if pr_repo.is_empty() { repo } else { Some(pr_repo.as_str()) };
        let state = gh::gh_pr_state(pr_number, repo_for_pr)?;
        refuse_if_not_merged(url, &state)?;
    }

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

/// `--kind` → label. `captain` is an alias for `founder` for one release (removed next).
fn hold_label(kind: Option<&str>) -> Result<&'static str> {
    Ok(match kind {
        None => "hold",
        Some("founder") => "hold:founder",
        Some("captain") => {
            eprintln!(
                "deprecated: --kind captain is an alias for --kind founder (label hold:founder) and will be removed in the next release"
            );
            "hold:founder"
        }
        Some(other) => {
            return Err(QueueError::usage(format!(
                "unknown --kind {other:?} (use founder, or omit for a plain hold)"
            )))
        }
    })
}

pub fn hold(number: u64, kind: Option<&str>, repo: Option<&str>, json: bool) -> Result<()> {
    let n = number.to_string();
    let label = hold_label(kind)?;
    let mut args: Vec<&str> = vec!["issue", "edit", &n, "--add-label", label];
    args.extend(ghaxi_repo(repo));
    mutation(&args, &format!("hold {number} ({label})"), json).map_err(|e| {
        let hint = format!("If the label is missing in the repo: gh label create '{label}' --force");
        e.with_suggestions(vec![hint])
    })
}

pub fn unhold(number: u64, repo: Option<&str>, json: bool) -> Result<()> {
    // Remove only the hold labels the issue carries: gh rejects labels the repo lacks.
    let i = gh::gh_view_issue(number, repo)?;
    let present: Vec<&str> = i
        .labels
        .iter()
        .map(|l| l.name.as_str())
        .filter(|l| gh::is_hold_label(l))
        .collect();
    if present.is_empty() {
        already(json, "no hold label");
        return Ok(());
    }
    let n = number.to_string();
    let mut args: Vec<&str> = vec!["issue", "edit", &n];
    for l in &present {
        args.push("--remove-label");
        args.push(l);
    }
    args.extend(ghaxi_repo(repo));
    mutation(&args, &format!("unhold {number} (removed {})", present.join(",")), json)
}

// ---------------------------------------------------------------------------
// ship (merge → close linked issues → board sync) + reconcile (board back-fill)
// ---------------------------------------------------------------------------

/// Merge a PR, then close every issue it closes and sync the board — one verb,
/// no manual reconciliation. `project` names the registered project for the
/// `cf board sync` tail; when absent the board sync is skipped with a note
/// (cf-queue is repo-scoped and cannot reverse-map a repo to a registry project).
pub fn ship(
    number: u64,
    linked: &[u64],
    project: Option<&str>,
    repo: Option<&str>,
    json: bool,
) -> Result<()> {
    let n = number.to_string();
    // 1. merge the PR (gh-axi is idempotent; an already-merged PR reports already).
    let mut merge_args: Vec<&str> = vec!["pr", "merge", &n];
    merge_args.extend(ghaxi_repo(repo));
    gh::run_ghaxi(&merge_args)?;

    // 2. close every linked issue still open (its queue state becomes `done` by derivation).
    let mut closed: Vec<u64> = Vec::new();
    for issue in linked {
        match gh::issue_open(repo, *issue) {
            Ok(true) => {
                let issue_n = issue.to_string();
                let mut c: Vec<&str> = vec!["issue", "close", &issue_n, "--reason", "completed"];
                c.extend(ghaxi_repo(repo));
                gh::run_ghaxi(&c)?;
                closed.push(*issue);
            }
            Ok(false) => {}
            Err(e) => return Err(e),
        }
    }

    // 3. sync the board (the queue derives `done` from the closed state; the board is
    //    the view, so it must be refreshed in the same step).
    let mut synced = false;
    if let Some(p) = project {
        if json {
            // Board sync JSON passes through only on the JSON plane (`cf board sync --json`);
            // the ship envelope is the single JSON document the caller parses.
            gh::run_cf_board_sync_json(p)?;
        } else {
            let out = gh::run_cf_board_sync(p)?;
            println!("  board synced ({p})");
            for line in out.lines().take(3) {
                println!("    {line}");
            }
        }
        synced = true;
    }

    if json {
        println!(
            "{}",
            serde_json::to_string(&json!({
                "ok": true, "action": "ship", "pr": number,
                "closed_issues": closed, "board_synced": synced,
                "project": project,
            }))
            .unwrap()
        );
    } else {
        println!("ok: ship pr #{number}");
        if closed.is_empty() {
            println!("  0 linked issue(s) closed");
        } else {
            for i in &closed {
                println!("  closed #{i}");
            }
        }
        if !synced {
            println!("  board sync skipped (pass --project <name> to sync the registered project's board)");
        }
    }
    Ok(())
}

/// A foreign (non-`repo`) board item that must leave the project's board so
/// `cf board status` Done agrees with `cf-queue list` done (the #285 back-fill).
pub struct ForeignItem {
    pub id: String,
    pub repository: String,
    pub number: Option<u64>,
    pub title: String,
}

/// Find board items whose content lives in a different repository than `repo`
/// (the "board includes a broader item set" delta). Report by default; remove
/// only with `apply`, so a reconciliation is inspected before it mutates.
pub fn reconcile(repo: &str, board: u64, org: &str, apply: bool, json: bool) -> Result<()> {
    let items = gh::gh_board_items(board, org)?;
    let foreign: Vec<ForeignItem> = items
        .into_iter()
        .filter(|i| !i.content.repository.is_empty() && i.content.repository != repo)
        .map(|i| ForeignItem {
            id: i.id,
            repository: i.content.repository,
            number: i.content.number,
            title: i.content.title,
        })
        .collect();

    // Remove first (only when asked), so the final envelope reports what actually happened.
    let mut removed = 0usize;
    if apply {
        for f in &foreign {
            gh::gh_board_item_delete(board, org, &f.id)?;
            removed += 1;
        }
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ok": true, "action": "reconcile", "repo": repo, "board": board,
                "foreign": foreign.iter().map(|f| json!({
                    "item_id": f.id, "repository": f.repository,
                    "number": f.number, "title": f.title,
                })).collect::<Vec<_>>(),
                "removed": removed,
            }))
            .unwrap()
        );
        return Ok(());
    }

    let rows: Vec<Vec<String>> = foreign
        .iter()
        .map(|f| {
            vec![
                f.number.map(|n| n.to_string()).unwrap_or_else(|| "—".into()),
                f.repository.clone(),
                f.title.clone(),
            ]
        })
        .collect();
    println!(
        "{}",
        toon::join(&[
            toon::header(BIN, DESCRIPTION),
            toon::list("foreign", &["number", "repository", "title"], &rows),
            if apply {
                format!("ok: reconcile {repo} (board #{board}) — removed {removed} foreign item(s)")
            } else if foreign.is_empty() {
                format!("ok: {repo} board #{board} has no foreign items — done == board Done")
            } else {
                format!(
                    "foreign items on board #{board} not from {repo}: {}",
                    foreign.len()
                )
            },
        ])
    );
    if !apply && !foreign.is_empty() {
        println!(
            "{}",
            toon::help(&[format!(
                "run `cf-queue reconcile {repo} --board {board} --yes` to remove the above"
            )])
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// block / unblock (body manipulation)
// ---------------------------------------------------------------------------

fn edit_body(number: u64, repo: Option<&str>, new_body: &str) -> Result<()> {
    let n = number.to_string();
    with_temp_body(&n, new_body, |path| {
        let mut args: Vec<&str> = vec!["issue", "edit", &n, "--body-file", path];
        args.extend(ghaxi_repo(repo));
        gh::run_ghaxi(&args).map(|_| ())
    })
}

pub fn block(number: u64, by: &str, repo: Option<&str>, json: bool) -> Result<()> {
    let edge = Edge::parse(by)?;
    if edge.repo.is_none() && edge.number == number {
        return Err(QueueError::usage("an issue cannot block itself"));
    }
    let body = gh::gh_view_body(number, repo)?;
    let line = format!("blocked-by: {edge}");
    if body.lines().any(|l| l.trim() == line) {
        already(json, &line);
        return Ok(());
    }
    let mut new_body = body.trim_end().to_string();
    if !new_body.is_empty() {
        new_body.push('\n');
    }
    new_body.push_str(&format!("{line}\n"));
    edit_body(number, repo, &new_body)?;
    if json {
        println!(
            "{}",
            serde_json::to_string(
                &json!({"ok": true, "action": "block", "number": number, "by": edge.to_string()})
            )
            .unwrap()
        );
    } else {
        println!("ok: block {number} by {line}");
    }
    Ok(())
}

pub fn unblock(number: u64, by: &str, repo: Option<&str>, json: bool) -> Result<()> {
    let edge = Edge::parse(by)?;
    let body = gh::gh_view_body(number, repo)?;
    let line = format!("blocked-by: {edge}");
    let kept: Vec<&str> = body.lines().filter(|l| l.trim() != line).collect();
    if kept.len() == body.lines().count() {
        already(json, &format!("no {line}"));
        return Ok(());
    }
    let new_body = format!("{}\n", kept.join("\n").trim_end());
    edit_body(number, repo, &new_body)?;
    if json {
        println!(
            "{}",
            serde_json::to_string(
                &json!({"ok": true, "action": "unblock", "number": number, "by": edge.to_string()})
            )
            .unwrap()
        );
    } else {
        println!("ok: unblock {number} (removed {line})");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_puts_machine_lines_first() {
        let rb = Edge::parse("thalixinc/a#45").unwrap();
        assert_eq!(
            assemble_body(Some(12), Some(&rb), "## Context\nwhy\n"),
            "Parent epic: #12\nRequested-by: thalixinc/a#45\n\n## Context\nwhy\n"
        );
        assert_eq!(assemble_body(Some(12), None, ""), "Parent epic: #12\n");
        assert_eq!(assemble_body(None, None, "x"), "x\n");
    }

    // REGRESSION (#285): a CLOSED issue must count toward `done` in the unfiltered summary.
    // `list` now fetches closed alongside open in the unfiltered path so `summarize` sees it;
    // this pins that a closed issue's `state_of` is `done` (not dropped from the count).
    #[test]
    fn unfiltered_summary_counts_closed_as_done() {
        let closed = gh::Issue {
            number: 232,
            title: "shipped".into(),
            state: "CLOSED".into(),
            assignees: vec![],
            labels: vec![],
            body: String::new(),
            url: String::new(),
        };
        let open = gh::Issue {
            number: 999,
            title: "open one".into(),
            state: "OPEN".into(),
            assignees: vec![],
            labels: vec![],
            body: String::new(),
            url: String::new(),
        };
        let blockers = gh::Blockers::default();
        let summary = summarize(&[closed, open], &blockers);
        let find = |s: &str| summary.iter().find(|(k, _)| k == s).map(|(_, v)| *v).unwrap_or(0);
        assert_eq!(find("done"), 1, "a closed issue must count as done");
        assert_eq!(find("queued"), 1, "an unassigned open issue is queued");
    }

    #[test]
    fn reads_created_number_from_ghaxi_output() {
        // Real gh-axi 0.1.x output: the url is quoted.
        let out = "issue:\n  number: 172\n  title: \"x: y\"\n  state: open\n  url: \"https://github.com/thalixinc/cf-queue/issues/172\"\nhelp[1]:\n  Run `gh-axi issue view 172`";
        assert_eq!(
            created(out),
            Some((172, "https://github.com/thalixinc/cf-queue/issues/172".into()))
        );
        assert_eq!(created("nothing here"), None);
    }

    #[test]
    fn hold_kinds() {
        assert_eq!(hold_label(None).unwrap(), "hold");
        assert_eq!(hold_label(Some("founder")).unwrap(), "hold:founder");
        assert_eq!(hold_label(Some("captain")).unwrap(), "hold:founder");
        assert!(hold_label(Some("boss")).is_err());
    }

    // INVARIANT (#321): close-before-merge — a MERGED PR allows the close, an OPEN PR refuses.
    #[test]
    fn close_requires_a_merged_pr() {
        let url = "https://github.com/thalixinc/codefactory/pull/314";
        // OPEN PR → refuse with the precise message.
        let err = refuse_if_not_merged(url, "OPEN").unwrap_err();
        assert!(err.message.contains("not merged (state=OPEN)"));
        assert!(err.message.contains("merge first"));
        // MERGED PR → allow.
        assert!(refuse_if_not_merged(url, "MERGED").is_ok());
    }
}
