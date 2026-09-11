//! Thin wrapper over `gh-axi` (mutations) and `gh` (structured reads) — the
//! single owner of every GitHub invocation in this crate.
//!
//! Mutations go through `gh-axi` (idempotent, TOON). Reads and body parsing go
//! through raw `gh --json` (gh-axi's `issue list/view` have no `--json`).

use crate::error::{QueueError, Result};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::process::Command;

const LIST_FIELDS: &str = "number,title,state,assignees,labels,body,url";

/// Run `gh-axi <args...>` (a mutation), returning trimmed stdout.
pub fn run_ghaxi(args: &[&str]) -> Result<String> {
    let out = Command::new("gh-axi")
        .args(args)
        .output()
        .map_err(|e| QueueError::operational(format!("gh-axi not runnable: {e}"), "GHAXI_EXEC"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let first = stderr.lines().next().unwrap_or("unknown gh-axi error");
        return Err(QueueError::operational(
            format!("gh-axi {} failed: {first}", args.join(" ")),
            "GHAXI_FAILED",
        ));
    }
    Ok(stdout)
}

/// Run `gh <args...>`, returning trimmed stdout (JSON expected by callers).
fn run_gh(args: &[&str]) -> Result<String> {
    let out = Command::new("gh")
        .args(args)
        .output()
        .map_err(|e| QueueError::operational(format!("gh not runnable: {e}"), "GH_EXEC"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let first = stderr.lines().next().unwrap_or("unknown gh error");
        return Err(QueueError::operational(
            format!("gh {} failed: {first}", args.join(" ")),
            "GH_FAILED",
        )
        .with_suggestions(vec![
            "Run `gh auth status` to confirm authentication.".into()
        ]));
    }
    Ok(stdout)
}

// ---------------------------------------------------------------------------
// Issue references / blocked-by edges
// ---------------------------------------------------------------------------

/// An issue reference: same-repo (`#n`) or cross-repo (`owner/repo#n`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Edge {
    pub repo: Option<String>,
    pub number: u64,
}

impl Edge {
    /// Parse `39`, `#39`, or `owner/repo#39`.
    pub fn parse(s: &str) -> Result<Edge> {
        let s = s.trim();
        let bad = || {
            QueueError::usage(format!(
                "invalid issue reference {s:?} — use <n>, #<n>, or owner/repo#<n>"
            ))
        };
        let (repo, num) = match s.rsplit_once('#') {
            Some((r, n)) => (r, n),
            None => ("", s),
        };
        let number = num.parse::<u64>().map_err(|_| bad())?;
        if repo.is_empty() {
            return Ok(Edge { repo: None, number });
        }
        let well_formed = repo
            .split_once('/')
            .is_some_and(|(o, r)| !o.is_empty() && !r.is_empty() && !r.contains('/'));
        if !well_formed {
            return Err(bad());
        }
        Ok(Edge { repo: Some(repo.to_string()), number })
    }

    pub fn is_cross(&self) -> bool {
        self.repo.is_some()
    }
}

impl fmt::Display for Edge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.repo {
            Some(r) => write!(f, "{r}#{}", self.number),
            None => write!(f, "#{}", self.number),
        }
    }
}

/// Resolved blocker state for one `list`/`ready` pass. Same-repo edges resolve
/// against the open set already fetched; cross-repo edges are looked up once each.
/// An edge that could not be resolved (unknown repo, no permission) blocks and is
/// reported through `warnings()` — never silently ignored.
#[derive(Default, Debug)]
pub struct Blockers {
    pub open_same: HashSet<u64>,
    pub cross: HashMap<Edge, std::result::Result<bool, String>>,
}

impl Blockers {
    pub fn blocks(&self, e: &Edge) -> bool {
        match e.repo {
            None => self.open_same.contains(&e.number),
            Some(_) => match self.cross.get(e) {
                Some(Ok(open)) => *open,
                _ => true,
            },
        }
    }

    pub fn warnings(&self) -> Vec<String> {
        let mut w: Vec<String> = self
            .cross
            .iter()
            .filter_map(|(e, r)| r.as_ref().err().map(|m| format!("{e}: {m} (treated as blocking)")))
            .collect();
        w.sort();
        w
    }
}

/// Is the issue open? `repo` None means gh's default repo detection.
pub fn issue_open(repo: Option<&str>, number: u64) -> Result<bool> {
    let n = number.to_string();
    let mut args: Vec<&str> = vec!["issue", "view", &n, "--json", "state"];
    args.extend(repo_args(repo));
    let text = run_gh(&args)?;
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| QueueError::operational(format!("cannot parse state: {e}"), "GH_PARSE"))?;
    Ok(v["state"].as_str() == Some("OPEN"))
}

/// Build the blocker table for a set of issues: every cross-repo edge looked up once.
pub fn resolve_blockers(issues: &[Issue], open_same: HashSet<u64>) -> Blockers {
    let mut b = Blockers { open_same, cross: HashMap::new() };
    for e in issues.iter().flat_map(|i| i.blocked_by()).filter(Edge::is_cross) {
        if b.cross.contains_key(&e) {
            continue;
        }
        let r = issue_open(e.repo.as_deref(), e.number).map_err(|err| err.message);
        b.cross.insert(e, r);
    }
    b
}

// ---------------------------------------------------------------------------
// Issue model
// ---------------------------------------------------------------------------

#[derive(Deserialize, Debug, Clone)]
pub struct Issue {
    pub number: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub assignees: Vec<Assignee>,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub url: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Assignee {
    pub login: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Label {
    pub name: String,
}

impl Issue {
    pub fn is_open(&self) -> bool {
        self.state == "OPEN"
    }

    pub fn assigned(&self) -> bool {
        !self.assignees.is_empty()
    }

    /// `hold` or any `hold:<kind>` label.
    pub fn has_hold(&self) -> bool {
        self.labels.iter().any(|l| is_hold_label(&l.name))
    }

    pub fn assignee_logins(&self) -> String {
        self.assignees
            .iter()
            .map(|a| a.login.clone())
            .collect::<Vec<_>>()
            .join(",")
    }

    pub fn label_names(&self) -> String {
        self.labels
            .iter()
            .map(|l| l.name.clone())
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Parse `blocked-by: #<n>` and `blocked-by: owner/repo#<n>` edges from the body.
    /// The body grammar is strict (a `#` is required); bare numbers are CLI-only.
    pub fn blocked_by(&self) -> Vec<Edge> {
        self.body
            .lines()
            .filter_map(|l| l.trim().strip_prefix("blocked-by:"))
            .filter_map(|rest| rest.split_whitespace().next())
            .filter(|tok| tok.contains('#'))
            .filter_map(|tok| Edge::parse(tok).ok())
            .collect()
    }

    /// First non-empty `key …` body line, trimmed remainder.
    fn body_field(&self, key: &str) -> Option<String> {
        self.body
            .lines()
            .find_map(|l| l.trim().strip_prefix(key).map(str::trim).filter(|r| !r.is_empty()))
            .map(str::to_string)
    }

    /// `Parent epic: #<n>` → n.
    pub fn epic(&self) -> Option<u64> {
        self.body_field("Parent epic:")
            .and_then(|r| r.trim_start_matches('#').split_whitespace().next()?.parse().ok())
    }

    /// `Requested-by: owner/repo#<m>`.
    pub fn requested_by(&self) -> Option<String> {
        self.body_field("Requested-by:")
    }

    /// `Artifacts: <path>`.
    pub fn artifacts(&self) -> Option<String> {
        self.body_field("Artifacts:")
    }

    /// The primary queue state (priority: done > hold > blocked > in-flight > queued).
    pub fn state_of(&self, blockers: &Blockers) -> &'static str {
        if !self.is_open() {
            return "done";
        }
        if self.has_hold() {
            return "hold";
        }
        if self.blocked_by().iter().any(|e| blockers.blocks(e)) {
            return "blocked";
        }
        if self.assigned() {
            return "in-flight";
        }
        "queued"
    }
}

pub fn is_hold_label(name: &str) -> bool {
    name == "hold" || name.starts_with("hold:")
}

pub fn gh_list_issues(state: &str, repo: Option<&str>) -> Result<Vec<Issue>> {
    let mut args: Vec<&str> = vec![
        "issue",
        "list",
        "--state",
        state,
        "--limit",
        "1000",
        "--json",
        LIST_FIELDS,
    ];
    args.extend(repo_args(repo));
    let text = run_gh(&args)?;
    serde_json::from_str(&text).map_err(|e| {
        QueueError::operational(format!("cannot parse gh issue list: {e}"), "GH_PARSE")
    })
}
// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

fn repo_args(repo: Option<&str>) -> Vec<&str> {
    match repo {
        Some(r) => vec!["--repo", r],
        None => vec![],
    }
}

/// View a single issue.
pub fn gh_view_issue(number: u64, repo: Option<&str>) -> Result<Issue> {
    let n = number.to_string();
    let mut args: Vec<&str> = vec!["issue", "view", &n, "--json", LIST_FIELDS];
    args.extend(repo_args(repo));
    let text = run_gh(&args)?;
    serde_json::from_str(&text).map_err(|e| {
        QueueError::operational(format!("cannot parse gh issue view: {e}"), "GH_PARSE")
    })
}

/// Read just the body of an issue.
pub fn gh_view_body(number: u64, repo: Option<&str>) -> Result<String> {
    let n = number.to_string();
    let mut args: Vec<&str> = vec!["issue", "view", &n, "--json", "body"];
    args.extend(repo_args(repo));
    let text = run_gh(&args)?;
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| QueueError::operational(format!("cannot parse body: {e}"), "GH_PARSE"))?;
    Ok(v["body"].as_str().unwrap_or_default().to_string())
}

// ---------------------------------------------------------------------------
// PR merge + board reconciliation (the merge → close + board-sync flow)
// ---------------------------------------------------------------------------

/// Parse a GitHub PR URL into `(owner/repo, number)`. Accepts
/// `https://github.com/<owner>/<repo>/pull/<n>[#...]` and a bare `<n>`. Returns `None` for
/// anything that is not a PR reference (so a non-PR `--pr` can't silently pass the guard).
pub fn parse_pr_url(url: &str) -> Option<(String, u64)> {
    let s = url.trim().trim_end_matches(['/', '#']);
    // Bare number: `57` (repo resolved by gh's default detection via --repo).
    if let Ok(n) = s.parse::<u64>() {
        return Some((String::new(), n));
    }
    // Full URL: https://github.com/<owner>/<repo>/pull/<n>.
    let path = s.split("://").nth(1)?; // https://github.com/o/r/pull/7
    let mut segs = path.split('/').filter(|p| !p.is_empty()).collect::<Vec<_>>();
    // Drop the host (and any leading path if a non-github host is used).
    let host = segs.remove(0);
    if !host.contains("github.com") {
        return None;
    }
    let n_index = segs.iter().position(|p| *p == "pull")?;
    let owner = segs.get(n_index.saturating_sub(2))?.to_string();
    let repo = segs.get(n_index.saturating_sub(1))?.to_string();
    let number = segs.get(n_index + 1)?.parse::<u64>().ok()?;
    Some((format!("{owner}/{repo}"), number))
}

/// The state of a PR (`OPEN`, `MERGED`, `CLOSED`, `DRAFT`…) as `gh pr view --json state`.
/// `repo` names the PR's owner/repo (from `parse_pr_url`), else gh's default detection.
pub fn gh_pr_state(pr_number: u64, repo: Option<&str>) -> Result<String> {
    let n = pr_number.to_string();
    let mut args: Vec<&str> = vec!["pr", "view", &n, "--json", "state"];
    args.extend(repo_args(repo));
    let text = run_gh(&args)?;
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| QueueError::operational(format!("cannot parse pr state: {e}"), "GH_PARSE"))?;
    Ok(v["state"].as_str().unwrap_or("").to_string())
}

/// A linked issue a PR closes: `Closes #<n>` / `Fixes owner/repo#<n>` in the
/// PR body and commit messages, resolved by GitHub into `closingIssuesReferences`.
pub fn gh_pr_closing_issues(number: u64, repo: Option<&str>) -> Result<Vec<u64>> {
    let n = number.to_string();
    let mut args: Vec<&str> = vec!["pr", "view", &n, "--json", "closingIssuesReferences"];
    args.extend(repo_args(repo));
    let text = run_gh(&args)?;
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| QueueError::operational(format!("cannot parse pr closing issues: {e}"), "GH_PARSE"))?;
    let mut out: Vec<u64> = Vec::new();
    for r in v["closingIssuesReferences"].as_array().cloned().unwrap_or_default() {
        if let Some(num) = parse_closing_ref_number(&r) {
            out.push(num);
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

/// Extract the issue number from one `closingIssuesReferences` entry.
pub fn parse_closing_ref_number(v: &serde_json::Value) -> Option<u64> {
    v["number"].as_u64().or_else(|| {
        v["url"]
            .as_str()
            .and_then(|u| u.rsplit('/').next())
            .and_then(|s| s.parse::<u64>().ok())
    })
}

/// One board item's identity, as rendered by `gh project item-list`.
#[derive(Deserialize, Debug, Clone)]
pub struct BoardItem {
    pub id: String,
    #[serde(default)]
    pub content: BoardContent,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct BoardContent {
    #[serde(default)]
    pub repository: String,
    #[serde(default)]
    pub number: Option<u64>,
    #[serde(default)]
    pub title: String,
}

/// List a project board's items (`gh project item-list <n> --owner <org>`).
pub fn gh_board_items(board: u64, org: &str) -> Result<Vec<BoardItem>> {
    let board = board.to_string();
    let args: Vec<&str> = vec!["project", "item-list", &board, "--owner", org, "--format", "json", "--limit", "1000"];
    let text = run_gh(&args)?;
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| QueueError::operational(format!("cannot parse gh project item-list: {e}"), "GH_PARSE"))?;
    let items = v["items"].as_array().cloned().unwrap_or_default();
    serde_json::from_value(serde_json::Value::Array(items))
        .map_err(|e| QueueError::operational(format!("cannot parse board items: {e}"), "GH_PARSE"))
}

/// Delete one board item by its project-item id.
pub fn gh_board_item_delete(board: u64, org: &str, item_id: &str) -> Result<()> {
    let board = board.to_string();
    let args: Vec<&str> = vec!["project", "item-delete", &board, "--owner", org, "--id", item_id];
    run_gh(&args)?;
    Ok(())
}

/// Run `cf board sync <project>` (the board is the view over the issues; the queue
/// derives `done` from closed state, so the board must be refreshed in the same step).
/// `cf` may be absent outside a Chief-of-Staff home — then the sync is skipped with a
/// warning rather than failing the ship.
pub fn run_cf_board_sync(project: &str) -> Result<String> {
    let out = Command::new("cf")
        .args(["board", "sync", project])
        .output()
        .map_err(|e| {
            QueueError::operational(
                format!("cf not runnable (board sync skipped): {e}"),
                "CF_EXEC",
            )
        })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let first = stderr.lines().next().unwrap_or("unknown cf error");
        eprintln!("warning: cf board sync skipped: {first}");
        return Ok(String::new());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// JSON-plane board sync: runs `cf board sync <project> --json` and discards its
/// envelope (the caller emits the single JSON document for the whole ship).
pub fn run_cf_board_sync_json(project: &str) -> Result<()> {
    let out = Command::new("cf")
        .args(["board", "sync", project, "--json"])
        .output()
        .map_err(|e| {
            QueueError::operational(
                format!("cf not runnable (board sync skipped): {e}"),
                "CF_EXEC",
            )
        })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let first = stderr.lines().next().unwrap_or("unknown cf error");
        eprintln!("warning: cf board sync skipped: {first}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(state: &str, assignees: &[&str], labels: &[&str], body: &str) -> Issue {
        Issue {
            number: 1,
            title: "t".into(),
            state: state.into(),
            assignees: assignees.iter().map(|a| Assignee { login: (*a).into() }).collect(),
            labels: labels.iter().map(|l| Label { name: (*l).into() }).collect(),
            body: body.into(),
            url: String::new(),
        }
    }

    fn same(n: u64) -> Edge {
        Edge { repo: None, number: n }
    }

    fn cross(repo: &str, n: u64) -> Edge {
        Edge { repo: Some(repo.into()), number: n }
    }

    fn blockers(open: &[u64]) -> Blockers {
        Blockers { open_same: open.iter().copied().collect(), cross: HashMap::new() }
    }

    #[test]
    fn parses_edge_forms() {
        assert_eq!(Edge::parse("39").unwrap(), same(39));
        assert_eq!(Edge::parse("#39").unwrap(), same(39));
        assert_eq!(Edge::parse("thalixinc/other#3").unwrap(), cross("thalixinc/other", 3));
        for bad in ["", "x", "#", "other#3", "a/b/c#3", "thalixinc/#3", "/r#3", "a/b#x"] {
            assert!(Edge::parse(bad).is_err(), "{bad:?} should be rejected");
        }
        assert_eq!(same(7).to_string(), "#7");
        assert_eq!(cross("o/r", 7).to_string(), "o/r#7");
    }

    #[test]
    fn parses_blocked_by_edges() {
        let i = issue(
            "OPEN",
            &[],
            &[],
            "some text\nblocked-by: #7\nblocked-by: #9 - needs refactor\nblocked-by: o/r#3\nblocked-by: junk\nblocked-by: 2 things\nmore",
        );
        assert_eq!(i.blocked_by(), vec![same(7), same(9), cross("o/r", 3)]);
    }

    #[test]
    fn classifies_queue_states() {
        let s = blockers(&[7]);
        assert_eq!(issue("OPEN", &[], &[], "").state_of(&s), "queued");
        assert_eq!(issue("OPEN", &["me"], &[], "").state_of(&s), "in-flight");
        assert_eq!(issue("CLOSED", &[], &[], "").state_of(&s), "done");
        assert_eq!(issue("OPEN", &[], &["hold"], "").state_of(&s), "hold");
        assert_eq!(issue("OPEN", &[], &["hold:founder"], "").state_of(&s), "hold");
        assert_eq!(issue("OPEN", &[], &["hold:captain"], "").state_of(&s), "hold");
        assert_eq!(issue("OPEN", &[], &[], "blocked-by: #7").state_of(&s), "blocked");
    }

    #[test]
    fn closed_blocker_is_not_blocking() {
        // blocker #7 is NOT in the open set → not blocking → queued
        let s = blockers(&[]);
        assert_eq!(issue("OPEN", &[], &[], "blocked-by: #7").state_of(&s), "queued");
    }

    #[test]
    fn cross_repo_edges_resolve_by_state_and_unknown_blocks() {
        let i = issue("OPEN", &[], &[], "blocked-by: o/r#3");
        let mut b = blockers(&[]);
        b.cross.insert(cross("o/r", 3), Ok(true));
        assert_eq!(i.state_of(&b), "blocked");
        b.cross.insert(cross("o/r", 3), Ok(false));
        assert_eq!(i.state_of(&b), "queued");
        b.cross.insert(cross("o/r", 3), Err("gh failed: not found".into()));
        assert_eq!(i.state_of(&b), "blocked");
        assert_eq!(b.warnings(), vec!["o/r#3: gh failed: not found (treated as blocking)"]);
        // never looked up at all → still blocking
        assert_eq!(i.state_of(&blockers(&[])), "blocked");
    }

    #[test]
    fn hold_beats_in_flight() {
        let s = blockers(&[]);
        assert_eq!(issue("OPEN", &["me"], &["hold"], "").state_of(&s), "hold");
    }

    #[test]
    fn parses_body_fields() {
        let i = issue(
            "OPEN",
            &[],
            &[],
            "Parent epic: #12\nRequested-by: thalixinc/a#45\n\nArtifacts: intent/12-x/tickets/40-y/\nbody",
        );
        assert_eq!(i.epic(), Some(12));
        assert_eq!(i.requested_by().as_deref(), Some("thalixinc/a#45"));
        assert_eq!(i.artifacts().as_deref(), Some("intent/12-x/tickets/40-y/"));
        let none = issue("OPEN", &[], &[], "Parent epic:\nplain");
        assert_eq!(none.epic(), None);
        let later = issue("OPEN", &[], &[], "Parent epic:\nParent epic: #5");
        assert_eq!(later.epic(), Some(5));
        assert_eq!(none.requested_by(), None);
        assert_eq!(none.artifacts(), None);
    }

    #[test]
    fn parses_closing_ref_number_from_number_or_url() {
        let by_number = serde_json::json!({"number": 285, "url": "https://github.com/thalixinc/codefactory/issues/285"});
        assert_eq!(parse_closing_ref_number(&by_number), Some(285));
        // gh sometimes omits `number` on cross-repo references but keeps the url.
        let by_url = serde_json::json!({"url": "https://github.com/thalixinc/cmux-axi/issues/6"});
        assert_eq!(parse_closing_ref_number(&by_url), Some(6));
        assert_eq!(parse_closing_ref_number(&serde_json::json!({})), None);
        assert_eq!(parse_closing_ref_number(&serde_json::json!({"number": null})), None);
    }

    #[test]
    fn deserializes_board_items() {
        let text = r#"{"items":[{"id":"PVTI_1","status":"Done","content":{"repository":"thalixinc/codefactory","number":285,"title":"t"}},{"id":"PVTI_2","status":"Done","content":{"repository":"thalixinc/cmux-axi","number":6,"title":"layout"}}]}"#;
        let v: serde_json::Value = serde_json::from_str(text).unwrap();
        let items: Vec<BoardItem> = serde_json::from_value(serde_json::Value::Array(v["items"].as_array().unwrap().clone())).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].content.repository, "thalixinc/codefactory");
        assert_eq!(items[1].content.number, Some(6));
    }

    #[test]
    fn parses_pr_url_into_repo_and_number() {
        assert_eq!(
            parse_pr_url("https://github.com/thalixinc/codefactory/pull/314"),
            Some(("thalixinc/codefactory".to_string(), 314))
        );
        // trailing junk (fragment/diff anchors) is tolerated
        assert_eq!(
            parse_pr_url("https://github.com/thalixinc/codefactory/pull/314/files"),
            Some(("thalixinc/codefactory".to_string(), 314))
        );
        // a bare number parses (repo resolved by gh default detection)
        assert_eq!(parse_pr_url("57"), Some((String::new(), 57)));
        // non-PR references are refused so the guard can't be silently bypassed
        assert_eq!(parse_pr_url("https://github.com/thalixinc/codefactory/issues/314"), None);
        assert_eq!(parse_pr_url("not-a-pr"), None);
    }
}
