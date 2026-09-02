//! Thin wrapper over `gh-axi` (mutations) and `gh` (structured reads) — the
//! single owner of every GitHub invocation in this crate.
//!
//! Mutations go through `gh-axi` (idempotent, TOON). Reads and body parsing go
//! through raw `gh --json` (gh-axi's `issue list/view` have no `--json`).

use crate::error::{QueueError, Result};
use serde::Deserialize;
use std::collections::HashSet;
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

    pub fn has_hold(&self) -> bool {
        self.labels
            .iter()
            .any(|l| l.name == "hold" || l.name == "hold:captain")
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

    /// Parse `blocked-by: #<n>` edges from the body.
    pub fn blocked_by(&self) -> Vec<u64> {
        let mut out = Vec::new();
        for line in self.body.lines() {
            let t = line.trim();
            if let Some(rest) = t.strip_prefix("blocked-by:") {
                if let Some(num) = rest.trim().strip_prefix('#') {
                    if let Ok(n) = num.split_whitespace().next().unwrap_or("").parse::<u64>() {
                        out.push(n);
                    }
                }
            }
        }
        out
    }

    /// The primary queue state (priority: done > hold > blocked > in-flight > queued).
    pub fn state_of(&self, open_numbers: &HashSet<u64>) -> &'static str {
        if !self.is_open() {
            return "done";
        }
        if self.has_hold() {
            return "hold";
        }
        if self.blocked_by().iter().any(|n| open_numbers.contains(n)) {
            return "blocked";
        }
        if self.assigned() {
            return "in-flight";
        }
        "queued"
    }
}

pub fn gh_list_issues(state: &str, repo: Option<&str>) -> Result<Vec<Issue>> {
    let mut args: Vec<&str> = vec![
        "issue",
        "list",
        "--state",
        state,
        "--limit",
        "100",
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

    fn open_set(nums: &[u64]) -> HashSet<u64> {
        nums.iter().copied().collect()
    }

    #[test]
    fn parses_blocked_by_edges() {
        let i = issue("OPEN", &[], &[], "some text\nblocked-by: #7\nblocked-by: #9 - needs refactor\nmore");
        assert_eq!(i.blocked_by(), vec![7, 9]);
    }

    #[test]
    fn classifies_queue_states() {
        let s = open_set(&[7]);
        assert_eq!(issue("OPEN", &[], &[], "").state_of(&s), "queued");
        assert_eq!(issue("OPEN", &["me"], &[], "").state_of(&s), "in-flight");
        assert_eq!(issue("CLOSED", &[], &[], "").state_of(&s), "done");
        assert_eq!(issue("OPEN", &[], &["hold"], "").state_of(&s), "hold");
        assert_eq!(issue("OPEN", &[], &["hold:captain"], "").state_of(&s), "hold");
        assert_eq!(issue("OPEN", &[], &[], "blocked-by: #7").state_of(&s), "blocked");
    }

    #[test]
    fn closed_blocker_is_not_blocking() {
        // blocker #7 is NOT in the open set → not blocking → queued
        let s = open_set(&[]);
        assert_eq!(issue("OPEN", &[], &[], "blocked-by: #7").state_of(&s), "queued");
    }

    #[test]
    fn hold_beats_in_flight() {
        let s = open_set(&[]);
        assert_eq!(issue("OPEN", &["me"], &["hold"], "").state_of(&s), "hold");
    }
}
