//! Claude Code の全体の設定に、状態をサイドバーへ送るフックを足す
use std::fs;

use serde_json::{json, Value};

use crate::{home, Result};

/// フックを付けるイベント、絞り込み、送る状態
const HOOKS: [(&str, Option<&str>, &str); 6] = [
    ("SessionStart", None, "idle"),
    ("UserPromptSubmit", None, "working"),
    ("PostToolUse", None, "working"),
    ("Notification", Some("permission_prompt"), "blocked"),
    ("Stop", None, "stop"),
    ("SessionEnd", None, "end"),
];

pub fn run() -> Result {
    let path = home().join(".claude/settings.json");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => "{}".to_owned(),
    };
    let mut settings: Value =
        serde_json::from_str(&text).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if !add_hooks(&mut settings)? {
        println!("already set up: {}", path.display());
        return Ok(());
    }
    if path.exists() {
        let backup = path.with_extension("json.bak");
        fs::copy(&path, &backup).map_err(|e| e.to_string())?;
        println!("saved the original settings to {}", backup.display());
    }
    let text = serde_json::to_string_pretty(&settings).unwrap() + "\n";
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| e.to_string())?;
    println!("added the hooks: {}", path.display());
    Ok(())
}

/// まだないイベントにだけ足す。足したら true
fn add_hooks(settings: &mut Value) -> Result<bool> {
    let hooks = settings
        .as_object_mut()
        .ok_or("settings is not an object")?
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("hooks is not an object")?;
    let mut added = false;
    for (event, matcher, state) in HOOKS {
        let groups = hooks
            .entry(event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| format!("hooks.{event} is not an array"))?;
        let exists = groups.iter().any(|group| {
            group["hooks"].as_array().into_iter().flatten().any(|hook| {
                hook["command"]
                    .as_str()
                    .is_some_and(|c| c.starts_with("flock hook "))
            })
        });
        if exists {
            continue;
        }
        let mut group = json!({
            "hooks": [{ "type": "command", "command": format!("flock hook {state}"), "timeout": 5 }]
        });
        if let Some(matcher) = matcher {
            group["matcher"] = json!(matcher);
        }
        groups.push(group);
        added = true;
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 既存のフックは残して足し_2回目は何もしない() {
        let mut settings = json!({
            "model": "opus",
            "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "other" }] }] }
        });
        assert!(add_hooks(&mut settings).unwrap());
        let stop = settings["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
        assert_eq!(stop[1]["hooks"][0]["command"], "flock hook stop");
        assert_eq!(settings["hooks"]["Notification"][0]["matcher"], "permission_prompt");
        assert!(!add_hooks(&mut settings).unwrap());
    }
}
