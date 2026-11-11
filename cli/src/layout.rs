use std::{env, fs, path::PathBuf};

use crate::{state_dir, xdg, Result};

const TEMPLATE: &str = include_str!("layout.kdl");
const PLUGIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/flock.wasm"));

#[derive(Debug, PartialEq, Eq)]
pub enum SessionState {
    Running,
    /// 終了し、復元を待っている
    Exited,
}

/// zellij list-sessions --no-formatting の出力から、そのセッションの状態を読む
pub fn session_state(listed: &str, session: &str) -> Option<SessionState> {
    listed
        .lines()
        .find(|line| line.split_whitespace().next() == Some(session))
        .map(|line| {
            if line.contains("EXITED") {
                SessionState::Exited
            } else {
                SessionState::Running
            }
        })
}

/// プラグインを書き出し、起動に使うレイアウトを作る。
/// サイドバーが書き出したタブの並びがあれば、そのタブを並べる
pub fn write(session: &str) -> Result<PathBuf> {
    let dir = xdg("XDG_DATA_HOME", ".local/share");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // zellij はプラグインの権限をパスごとに覚えるので、版が変わっても同じパスに置く
    let plugin = dir.join("flock.wasm");
    if fs::read(&plugin).ok().as_deref() != Some(PLUGIN) {
        fs::write(&plugin, PLUGIN).map_err(|e| e.to_string())?;
    }
    let saved = fs::read_to_string(state_dir().join(format!("{session}.tabs"))).unwrap_or_default();
    let claude = env::var("FLOCK_CLAUDE_COMMAND").unwrap_or_else(|_| "claude".into());
    let path = dir.join(format!("layout-{session}.kdl"));
    fs::write(&path, render(&plugin.display().to_string(), &claude, &saved))
        .map_err(|e| e.to_string())?;
    Ok(path)
}

/// saved は1行に1タブ「名前 タブ フォルダ タブ 会話の ID」
fn render(plugin: &str, claude: &str, saved: &str) -> String {
    let tabs: Vec<String> = saved
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let mut fields = line.split('\t');
            let (name, cwd) = (fields.next()?, fields.next()?);
            let session = fields.next().unwrap_or_default();
            let focus = if i == 0 { " focus=true" } else { "" };
            let pane = if session.is_empty() {
                "        pane".to_owned()
            } else {
                let resume = match session.strip_prefix("agy:") {
                    Some(id) => format!("exec agy --conversation '{id}'"),
                    None => format!("exec {claude} --resume '{session}'"),
                };
                // エージェントを終了したらペインごと閉じる
                format!(
                    "        pane command=\"sh\" close_on_exit=true {{\n            args \"-c\" {}\n        }}",
                    kdl_string(&resume)
                )
            };
            Some(format!(
                "    tab name={} cwd={}{focus} {{\n{pane}\n    }}",
                kdl_string(name),
                kdl_string(cwd)
            ))
        })
        .collect();
    let tabs = if tabs.is_empty() {
        "    tab name=\"main\" {\n        pane\n    }".to_owned()
    } else {
        tabs.join("\n")
    };
    TEMPLATE
        .replace("{plugin}", &kdl_string(&format!("file:{plugin}")))
        .replace("{claude_command}", &kdl_string(claude))
        .replace("{tabs}", &tabs)
}

fn kdl_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn セッションの状態は名前が一致する行で決まる() {
        let listed = "flock-old [Created 1h ago] (EXITED - attach to resurrect)\nflock [Created 2m ago]\n";
        assert_eq!(session_state(listed, "flock"), Some(SessionState::Running));
        assert_eq!(session_state(listed, "flock-old"), Some(SessionState::Exited));
        assert_eq!(session_state(listed, "other"), None);
    }

    #[test]
    fn 保存したタブは会話があれば続きから_なければシェルで開く() {
        let kdl = render("/p/flock.wasm", "claude", "main\t/a\tabc\nAPI \"x\"\t/b\t\n");
        assert!(kdl.contains("plugin location=\"file:/p/flock.wasm\""));
        assert!(kdl.contains(
            "    tab name=\"main\" cwd=\"/a\" focus=true {\n        pane command=\"sh\" close_on_exit=true {\n            args \"-c\" \"exec claude --resume 'abc'\"\n        }\n    }"
        ));
        assert!(kdl.contains("    tab name=\"API \\\"x\\\"\" cwd=\"/b\" {\n        pane\n    }"));
        let agy = render("/p", "claude", "main\t/a\tagy:c1\n");
        assert!(agy.contains("args \"-c\" \"exec agy --conversation 'c1'\""));
    }

    #[test]
    fn 保存したタブがなければmainタブだけ() {
        assert!(render("/p", "claude", "").contains("    tab name=\"main\" {\n        pane\n    }"));
    }
}
