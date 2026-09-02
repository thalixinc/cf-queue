//! Version + self-update for a `cargo install --git`-distributed binary.

use crate::error::{QueueError, Result};
use crate::toon;
use std::cmp::Ordering;
use std::process::Command;

pub const REPO: &str = "https://github.com/thalixinc/cf-queue";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn cmd_version() {
    println!("cf-queue {VERSION}");
}

fn semver_cmp(a: &str, b: &str) -> Ordering {
    let pa: Vec<u64> = a.split('.').map(|s| s.parse().unwrap_or(0)).collect();
    let pb: Vec<u64> = b.split('.').map(|s| s.parse().unwrap_or(0)).collect();
    for i in 0..pa.len().max(pb.len()) {
        match pa
            .get(i)
            .copied()
            .unwrap_or(0)
            .cmp(&pb.get(i).copied().unwrap_or(0))
        {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    Ordering::Equal
}

fn fetch_latest_version() -> Result<String> {
    let url = "https://api.github.com/repos/thalixinc/cf-queue/contents/Cargo.toml";
    let out = Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "10",
            "-H",
            "Accept: application/vnd.github.raw+json",
            url,
        ])
        .output()
        .map_err(|e| {
            QueueError::operational(format!("`curl` not available: {e}"), "UPDATE_CHECK")
        })?;
    if !out.status.success() {
        return Err(QueueError::operational(
            "could not reach the version source (network or curl failure)",
            "UPDATE_CHECK",
        )
        .with_suggestions(vec![
            "Check network access and that `curl` is installed.".into()
        ]));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .find_map(|l| {
            let l = l.trim();
            l.strip_prefix("version = \"")
                .and_then(|r| r.strip_suffix('"'))
                .map(String::from)
        })
        .ok_or_else(|| {
            QueueError::operational("version not found in remote Cargo.toml", "UPDATE_CHECK")
        })
}

pub fn cmd_update(check: bool, json: bool) -> Result<()> {
    let latest = fetch_latest_version()?;
    let available = semver_cmp(&latest, VERSION) == Ordering::Greater;

    if check {
        if json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "package": "cf-queue", "current": VERSION, "latest": latest, "available": available,
                }))
                .unwrap()
            );
        } else {
            println!(
                "{}",
                toon::join(&[
                    format!(
                        "update:\n  package: cf-queue\n  current: {VERSION}\n  latest: {latest}\n  available: {available}"
                    ),
                    if available {
                        toon::help(&["Run `cf-queue update` to upgrade".to_string()])
                    } else {
                        toon::help(&["Already up to date".to_string()])
                    },
                ])
            );
        }
        return Ok(());
    }

    if !available {
        if json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "ok": true, "action": "update", "current": VERSION, "latest": latest, "available": false,
                }))
                .unwrap()
            );
        } else {
            println!("ok: cf-queue already at latest ({VERSION})");
        }
        return Ok(());
    }

    let status = Command::new("cargo")
        .args(["install", "--git", REPO, "--force"])
        .status()
        .map_err(|e| QueueError::operational(format!("`cargo` not available: {e}"), "UPDATE"))?;
    if !status.success() {
        return Err(
            QueueError::operational("cargo install failed", "UPDATE").with_suggestions(vec![
                format!("Run `cargo install --git {REPO} --force` manually."),
            ]),
        );
    }
    if json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "ok": true, "action": "update", "current": VERSION, "latest": latest, "available": true,
            }))
            .unwrap()
        );
    } else {
        println!("update: cf-queue upgraded {VERSION} -> {latest}");
    }
    Ok(())
}
