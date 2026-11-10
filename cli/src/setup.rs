//! Claude Code と Antigravity CLI の全体の設定に、状態をサイドバーへ送るフックを足す
use std::{fs, path::Path};

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
    // Linux には util-linux の flock があるので、PATH を頼らずこのバイナリを直接呼ぶ
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.display().to_string();
    update(&home().join(".claude/settings.json"), |s| add_hooks(s, &exe))?;
    if home().join(".gemini").is_dir() {
        update(&home().join(".gemini/config/hooks.json"), |h| add_antigravity_hooks(h, &exe))?;
    }
    Ok(())
}

/// JSON の設定を読んで直し、変わったら元を .bak に残して書き戻す
fn update(path: &Path, change: impl FnOnce(&mut Value) -> Result<bool>) -> Result {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => "{}".to_owned(),
    };
    let mut settings: Value =
        serde_json::from_str(&text).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if !change(&mut settings)? {
        println!("already set up: {}", path.display());
        return Ok(());
    }
    if path.exists() {
        let backup = path.with_extension("json.bak");
        fs::copy(path, &backup).map_err(|e| e.to_string())?;
        println!("saved the original settings to {}", backup.display());
    }
    let text = serde_json::to_string_pretty(&settings).unwrap() + "\n";
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| e.to_string())?;
    println!("added the hooks: {}", path.display());
    Ok(())
}

/// Antigravity CLI の hooks.json に flock という名前で足す。あれば何もしない。足したら true。
/// PreToolUse は {} を返すとツールが拒否されるので使わない
fn add_antigravity_hooks(hooks: &mut Value, exe: &str) -> Result<bool> {
    let hooks = hooks.as_object_mut().ok_or("hooks.json is not an object")?;
    if hooks.contains_key("flock") {
        return Ok(false);
    }
    let hook = |state: &str| json!({ "command": format!("{exe} hook {state}"), "timeout": 5 });
    hooks.insert(
        "flock".into(),
        json!({
            "SessionStart": [hook("idle")],
            "PreInvocation": [hook("working")],
            "PostToolUse": [{ "matcher": "*", "hooks": [hook("working")] }],
            "Stop": [hook("stop")],
        }),
    );
    Ok(true)
}

/// まだないイベントにだけ足す。足したら true
fn add_hooks(settings: &mut Value, exe: &str) -> Result<bool> {
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
                    .is_some_and(|c| c.contains("flock hook "))
            })
        });
        if exists {
            continue;
        }
        let mut group = json!({
            "hooks": [{ "type": "command", "command": format!("{exe} hook {state}"), "timeout": 5 }]
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
        assert!(add_hooks(&mut settings, "/bin/flock").unwrap());
        let stop = settings["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
        assert_eq!(stop[1]["hooks"][0]["command"], "/bin/flock hook stop");
        assert_eq!(settings["hooks"]["Notification"][0]["matcher"], "permission_prompt");
        assert!(!add_hooks(&mut settings, "/bin/flock").unwrap());
    }

    #[test]
    fn antigravity_には名前付きで1度だけ足す() {
        let mut hooks = json!({ "other": {} });
        assert!(add_antigravity_hooks(&mut hooks, "/bin/flock").unwrap());
        assert_eq!(hooks["flock"]["Stop"][0]["command"], "/bin/flock hook stop");
        assert_eq!(hooks["flock"]["PostToolUse"][0]["matcher"], "*");
        assert!(hooks["flock"].get("PreToolUse").is_none());
        assert!(hooks.get("other").is_some());
        assert!(!add_antigravity_hooks(&mut hooks, "/bin/flock").unwrap());
    }
}
