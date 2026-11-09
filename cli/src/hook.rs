//! Claude Code のフックから呼び、このペインの状態をサイドバーへ送る。
//! zellij の外では何もしない。フックを止めないよう、失敗はすべて無視する
use std::{
    env, fs,
    io::{self, Read},
    path::PathBuf,
    process,
    time::{Duration, SystemTime},
};

use serde_json::Value;

use crate::{pipe, state_dir};

/// 詳細のファイルを残す期間
const KEEP: Duration = Duration::from_secs(24 * 60 * 60);

pub fn run(state: &str) {
    let Ok(pane) = env::var("ZELLIJ_PANE_ID") else {
        return;
    };
    let mut input = String::new();
    let _ = io::stdin().read_to_string(&mut input);
    let input: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
    let now = Local::now();
    let summary = summary(&input);
    let file = summary
        .as_ref()
        .and_then(|_| write_detail(&input, &now))
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let mut args = format!("pane={pane},state={state}");
    // 会話の ID。Ctrl+q で終了した後に flock を開き直すと、この会話の続きから起動する
    if let Some(session) = input["session_id"].as_str() {
        args += &format!(",session={session}");
    }
    // 中身は「時刻 タブ 詳細のファイル タブ ログの1行」
    let payload = format!("{}\t{file}\t{}", now.clock(), summary.unwrap_or_default());
    pipe("flock", Some(&args), &payload);
}

/// ログに出す1行。指示なら1行目、ツールなら名前と対象（ファイル名やコマンド）、許可待ちならその文言
fn summary(input: &Value) -> Option<String> {
    match input["hook_event_name"].as_str()? {
        "UserPromptSubmit" => Some(format!("> {}", line(str(&input["prompt"])))),
        "PostToolUse" => {
            let tool_input = &input["tool_input"];
            let target = match tool_input["file_path"]
                .as_str()
                .or(tool_input["notebook_path"].as_str())
            {
                Some(path) => path.rsplit('/').next().unwrap_or(path).to_owned(),
                None => ["command", "pattern", "url", "description"]
                    .iter()
                    .find_map(|key| tool_input[key].as_str())
                    .map(line)
                    .unwrap_or_default(),
            };
            Some(format!("{} {target}", str(&input["tool_name"])).trim_end().to_owned())
        }
        "Notification" => Some(line(str(&input["message"]))),
        _ => None,
    }
}

/// ログの行を押したときに開く詳細。指示は全文、ツールは入力のすべてと結果の先頭
fn detail(input: &Value) -> String {
    match input["hook_event_name"].as_str() {
        Some("UserPromptSubmit") => str(&input["prompt"]).to_owned(),
        Some("PostToolUse") => {
            let mut out = format!("{}\n\n", str(&input["tool_name"]));
            for (key, value) in input["tool_input"].as_object().into_iter().flatten() {
                out += &format!("[{key}]\n{}\n\n", text(value));
            }
            let response: String = text(&input["tool_response"]).chars().take(4000).collect();
            out + "[結果]\n" + &response
        }
        _ => str(&input["message"]).to_owned(),
    }
}

fn write_detail(input: &Value, now: &Local) -> Option<PathBuf> {
    let dir = state_dir().join("events");
    fs::create_dir_all(&dir).ok()?;
    for entry in fs::read_dir(&dir).ok()?.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age > KEEP);
        if old {
            let _ = fs::remove_file(entry.path());
        }
    }
    let path = dir.join(format!("{}-{}.txt", now.stamp(), process::id()));
    fs::write(&path, detail(input)).ok()?;
    Some(path)
}

fn str(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

fn text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// 1行目だけを、タブ区切りの邪魔にならない形で
fn line(s: &str) -> String {
    s.lines()
        .next()
        .unwrap_or_default()
        .replace('\t', " ")
        .chars()
        .take(200)
        .collect()
}

/// 端末のタイムゾーンでの今の時刻
struct Local(libc::tm);

impl Local {
    fn now() -> Local {
        // SAFETY: localtime_r は渡した tm に書き込むだけ
        unsafe {
            let time = libc::time(std::ptr::null_mut());
            let mut tm = std::mem::zeroed();
            libc::localtime_r(&time, &mut tm);
            Local(tm)
        }
    }

    fn clock(&self) -> String {
        format!("{:02}:{:02}", self.0.tm_hour, self.0.tm_min)
    }

    fn stamp(&self) -> String {
        let t = &self.0;
        format!(
            "{}{:02}{:02}{:02}{:02}{:02}",
            t.tm_year + 1900,
            t.tm_mon + 1,
            t.tm_mday,
            t.tm_hour,
            t.tm_min,
            t.tm_sec
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ログの1行は指示の1行目とツールの対象() {
        let prompt = json!({"hook_event_name": "UserPromptSubmit", "prompt": "直して\t欲しい\n2行目"});
        assert_eq!(summary(&prompt).unwrap(), "> 直して 欲しい");
        let edit = json!({"hook_event_name": "PostToolUse", "tool_name": "Edit", "tool_input": {"file_path": "/a/main.rs"}});
        assert_eq!(summary(&edit).unwrap(), "Edit main.rs");
        let bash = json!({"hook_event_name": "PostToolUse", "tool_name": "Bash", "tool_input": {"command": "cargo test\nls"}});
        assert_eq!(summary(&bash).unwrap(), "Bash cargo test");
        let todo = json!({"hook_event_name": "PostToolUse", "tool_name": "TodoWrite", "tool_input": {"todos": []}});
        assert_eq!(summary(&todo).unwrap(), "TodoWrite");
        assert_eq!(summary(&json!({"hook_event_name": "Stop"})), None);
    }

    #[test]
    fn 詳細はツールの入力を順に並べ結果を続ける() {
        let edit = json!({
            "hook_event_name": "PostToolUse", "tool_name": "Edit",
            "tool_input": {"file_path": "/a/main.rs", "old_string": "a\nb"},
            "tool_response": {"ok": true}
        });
        assert_eq!(
            detail(&edit),
            "Edit\n\n[file_path]\n/a/main.rs\n\n[old_string]\na\nb\n\n[結果]\n{\"ok\":true}"
        );
    }
}
