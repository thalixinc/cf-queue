//! `setup skill` / `setup hooks` — teach the agent about cf-queue.

use crate::error::{QueueError, Result};
use std::fs;
use std::path::PathBuf;

const SKILL: &str = include_str!("../skills/cf-queue/SKILL.md");

fn home_dir() -> Result<PathBuf> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| QueueError::operational("HOME is not set", "SETUP"))
}

fn cwd() -> Result<PathBuf> {
    std::env::current_dir().map_err(|e| QueueError::operational(e.to_string(), "CWD"))
}

fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            QueueError::operational(format!("cannot create {}: {e}", parent.display()), "SETUP")
        })?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).map_err(|e| {
        QueueError::operational(format!("cannot write {}: {e}", tmp.display()), "SETUP")
    })?;
    fs::rename(&tmp, path).map_err(|e| {
        QueueError::operational(format!("cannot rename to {}: {e}", path.display()), "SETUP")
    })?;
    Ok(())
}

fn skill_target(global: bool) -> Result<PathBuf> {
    let base = if global { home_dir()? } else { cwd()? };
    Ok(base
        .join(".claude")
        .join("skills")
        .join("cf-queue")
        .join("SKILL.md"))
}

pub fn cmd_setup_skill(global: bool, json: bool) -> Result<()> {
    let target = skill_target(global)?;
    let existed = target.exists();
    atomic_write(&target, SKILL.as_bytes())?;
    if json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "ok": true, "action": "setup-skill", "path": target.display().to_string(), "already": existed,
            }))
            .unwrap()
        );
    } else {
        let tag = if existed { " (updated)" } else { "" };
        println!("ok: setup skill -> {}{}", target.display(), tag);
    }
    Ok(())
}

fn settings_target(global: bool) -> Result<PathBuf> {
    let base = if global { home_dir()? } else { cwd()? };
    Ok(base.join(".claude").join("settings.json"))
}

fn hook_already_present(session_start: &serde_json::Value) -> bool {
    session_start
        .as_array()
        .map(|entries| {
            entries.iter().any(|e| {
                e["hooks"]
                    .as_array()
                    .map(|hs| {
                        hs.iter().any(|h| {
                            h["command"]
                                .as_str()
                                .map(|c| c.contains("cf-queue"))
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

pub fn cmd_setup_hooks(global: bool, json: bool) -> Result<()> {
    let target = settings_target(global)?;

    let mut root: serde_json::Value = if target.exists() {
        let text = fs::read_to_string(&target).map_err(|e| {
            QueueError::operational(format!("cannot read {}: {e}", target.display()), "SETUP")
        })?;
        serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    if hook_already_present(&root["hooks"]["SessionStart"]) {
        if json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({"ok": true, "already": true})).unwrap()
            );
        } else {
            println!("already: true (cf-queue SessionStart hook present)");
        }
        return Ok(());
    }

    let entry = serde_json::json!({
        "matcher": "",
        "hooks": [{ "type": "command", "command": "cf-queue", "timeout": 10 }]
    });
    if root["hooks"].as_object().is_none() {
        root["hooks"] = serde_json::json!({});
    }
    let hooks = root["hooks"].as_object_mut().unwrap();
    let session_start = hooks
        .entry("SessionStart")
        .or_insert_with(|| serde_json::json!([]));
    if let Some(arr) = session_start.as_array_mut() {
        arr.push(entry);
    }

    atomic_write(
        &target,
        serde_json::to_string_pretty(&root).unwrap().as_bytes(),
    )?;
    if json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "ok": true, "action": "setup-hooks", "path": target.display().to_string(),
            }))
            .unwrap()
        );
    } else {
        println!("ok: setup hooks -> {}", target.display());
        println!(
            "{}",
            crate::toon::help(&[
                "Restart your agent session for the hook to take effect.".to_string()
            ])
        );
    }
    Ok(())
}
