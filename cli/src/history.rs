//! Claude Code と Antigravity CLI の最近の会話を、新しい順に1行1件で出す。サイドバーの Resume が使う。
//! どちらも公開されていない形式なので、読めないファイルや行は飛ばし、一覧全体は止めない
use std::{
    cmp::Reverse,
    fs::{self, File},
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::{home, hook::Local, Result};

/// 出す件数
const LIMIT: usize = 50;

#[derive(Debug, PartialEq)]
struct Entry {
    agent: &'static str,
    id: String,
    /// UNIX 時刻（秒）
    updated: i64,
    cwd: String,
    title: String,
}

pub fn run() -> Result {
    let mut entries = claude(&home().join(".claude/projects"));
    entries.extend(antigravity(&home().join(".gemini/antigravity-cli/conversation_summaries.db")));
    // 消した worktree などのフォルダでは再開するタブが開けない
    entries.retain(|e| Path::new(&e.cwd).is_dir());
    let out: String = newest(entries)
        .iter()
        .map(|e| line(e, &Local::at(e.updated as libc::time_t).date_clock()) + "\n")
        .collect();
    // head などで途中で閉じられても失敗にしない
    let _ = io::stdout().write_all(out.as_bytes());
    Ok(())
}

/// 1行は「エージェント タブ 会話の ID タブ 更新時刻 タブ フォルダ タブ 題」
fn line(e: &Entry, updated: &str) -> String {
    let title = e.title.replace(['\t', '\n', '\r'], " ");
    format!("{}\t{}\t{updated}\t{}\t{}", e.agent, e.id, e.cwd, title.trim())
}

fn newest(mut entries: Vec<Entry>) -> Vec<Entry> {
    entries.sort_by_key(|e| Reverse(e.updated));
    entries.truncate(LIMIT);
    entries
}

/// 会話のファイルを新しい順に読み、出せるものが LIMIT 件たまったら止める。
/// 数 MB のファイルもあるので、古いものまで読まない。
/// 更新時刻は最後の発言の時刻で、ファイルの mtime はその上限として読む順と打ち切りに使う。
/// mtime は発言のない行が後から足されても進むので、そのまま並べると古い会話が上に来る
fn claude(projects: &Path) -> Vec<Entry> {
    let mut files: Vec<(i64, PathBuf)> = fs::read_dir(projects)
        .into_iter()
        .flatten()
        .flatten()
        .flat_map(|project| fs::read_dir(project.path()).into_iter().flatten().flatten())
        .filter_map(|file| {
            let path = file.path();
            let meta = path.metadata().ok()?;
            if !meta.is_file() || path.extension()? != "jsonl" {
                return None;
            }
            let updated = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
            Some((updated as i64, path))
        })
        .collect();
    files.sort_by_key(|f| Reverse(f.0));
    let mut entries = Vec::new();
    for (modified, path) in files {
        if entries.len() >= LIMIT {
            entries = newest(entries);
            if modified < entries[LIMIT - 1].updated {
                break;
            }
        }
        let Ok(file) = File::open(&path) else { continue };
        let Some(session) = read_claude(BufReader::new(file)) else { continue };
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        entries.push(Entry {
            agent: "claude",
            id: id.to_owned(),
            updated: session.spoke.unwrap_or(modified),
            cwd: session.cwd,
            title: session.title,
        });
    }
    entries
}

#[derive(Debug, PartialEq)]
struct Session {
    cwd: String,
    title: String,
    /// 最後の発言の時刻
    spoke: Option<i64>,
}

/// 会話のファイルから、最初に出てくる作業フォルダ、題、最後の発言の時刻を読む。
/// 題は /rename で付けたものを自動で付いたものより優先する。
/// 題のない会話は claude -p のような使い捨てなので出さない。
/// 別の会話に引き継がれた会話は、続きが引き継ぎ先にあるので出さない
fn read_claude(reader: impl BufRead) -> Option<Session> {
    let mut cwd = None;
    let (mut custom, mut ai) = (None, None);
    let mut last_stamped = None;
    for line in reader.split(b'\n') {
        let Ok(line) = line else { break };
        // 大きな行もあるので、要る行だけ JSON として読む。時刻は最後の1行だけ読めばよい
        if contains(&line, b"\"timestamp\"") {
            last_stamped = Some(line.clone());
        }
        let want_cwd = cwd.is_none() && contains(&line, b"\"cwd\"");
        if !want_cwd && !contains(&line, b"-title\"") && !contains(&line, b"\"continued-in\"") {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        if let (None, Some(c)) = (&cwd, value["cwd"].as_str()) {
            cwd = Some(c.to_owned());
        }
        let text = |key: &str| value[key].as_str().filter(|t| !t.trim().is_empty()).map(str::to_owned);
        match value["type"].as_str() {
            Some("custom-title") => custom = text("customTitle").or(custom),
            Some("ai-title") => ai = text("aiTitle").or(ai),
            Some("continued-in") => return None,
            _ => {}
        }
    }
    // 道具の結果の中に入れ子の timestamp が出ることもあるので、行の一番外側の値だけを使う
    let spoke = last_stamped
        .and_then(|line| serde_json::from_slice::<Value>(&line).ok())
        .and_then(|value| epoch(value["timestamp"].as_str()?));
    Some(Session { cwd: cwd?, title: custom.or(ai)?, spoke })
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn antigravity(db: &Path) -> Vec<Entry> {
    // ないときに空のデータベースを作らないよう、読み取り専用で開く
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    Connection::open_with_flags(db, flags)
        .and_then(|conn| query_antigravity(&conn))
        .unwrap_or_default()
}

/// 枝分かれした子の会話と、手順のない空の会話は出さない
fn query_antigravity(conn: &Connection) -> rusqlite::Result<Vec<Entry>> {
    let mut statement = conn.prepare(
        "SELECT conversation_id, title, preview, last_modified_time, workspace_uris
         FROM conversation_summaries
         WHERE parent_conversation_id = '' AND step_count > 0
         ORDER BY last_modified_time DESC LIMIT ?1",
    )?;
    let rows = statement.query_map([LIMIT as i64], |row| {
        Ok([row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?])
    })?;
    Ok(rows.filter_map(|row| antigravity_entry(row.ok()?)).collect())
}

fn antigravity_entry([id, title, preview, updated, uris]: [Option<String>; 5]) -> Option<Entry> {
    let title = [title, preview].into_iter().flatten().find(|t| !t.trim().is_empty())?;
    let uris: Vec<Value> = serde_json::from_str(&uris?).ok()?;
    let cwd = uris.iter().filter_map(Value::as_str).find_map(file_path)?;
    Some(Entry { agent: "agy", id: id?, updated: epoch(&updated?)?, cwd, title })
}

/// file:///a%20b を /a b に
fn file_path(uri: &str) -> Option<String> {
    let path = uri.strip_prefix("file://")?.as_bytes();
    let mut out = Vec::with_capacity(path.len());
    let mut i = 0;
    while i < path.len() {
        let hex = path.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
        match (path[i], hex.and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok().filter(|p| !p.is_empty())
}

/// 2026-11-11 09:58:56.357525+00:00 のような時刻を UNIX 時刻（秒）に。秒より細かい分は捨てる
fn epoch(s: &str) -> Option<i64> {
    let num = |from: usize, len: usize| -> Option<i64> {
        let digits = s.get(from..from + len)?;
        digits.bytes().all(|b| b.is_ascii_digit()).then(|| digits.parse().ok())?
    };
    let (year, month, day) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (hour, minute, second) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    let zone = s[19..].trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let offset = match zone.as_bytes().first() {
        None | Some(b'Z') => 0,
        Some(sign @ (b'+' | b'-')) => {
            let offset = num(s.len() - zone.len() + 1, 2)? * 3600 + num(s.len() - zone.len() + 4, 2)? * 60;
            if *sign == b'-' { -offset } else { offset }
        }
        _ => return None,
    };
    Some(days_from_civil(year, month, day) * 86400 + hour * 3600 + minute * 60 + second - offset)
}

/// 1970-01-01 からの日数。3月始まりの年として数えると閏日が年の最後に来る
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146097 + day_of_era - 719468
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(updated: i64) -> Entry {
        Entry { agent: "claude", id: updated.to_string(), updated, cwd: "/a".into(), title: "t".into() }
    }

    #[test]
    fn claude_の題は最後のai_titleで_フォルダは最初のcwd_時刻は最後の発言() {
        let jsonl = concat!(
            "{\"type\":\"mode\"}\n",
            "{\"type\":\"user\",\"cwd\":\"/a\",\"timestamp\":\"1970-01-01T00:00:01.000Z\"}\n",
            "{\"type\":\"ai-title\",\"aiTitle\":\"古い題\"}\n",
            "壊れた行 \"cwd\" \"ai-title\"\n",
            "{\"type\":\"user\",\"cwd\":\"/b\",\"timestamp\":\"1970-01-01T00:00:02.000Z\"}\n",
            "{\"type\":\"ai-title\",\"aiTitle\":\"新しい題\"}\n",
            "{\"type\":\"atis-latch\"}\n",
        );
        let session = Session { cwd: "/a".into(), title: "新しい題".into(), spoke: Some(2) };
        assert_eq!(read_claude(jsonl.as_bytes()), Some(session));
    }

    #[test]
    fn claude_は名前を付け直した題を優先する() {
        let jsonl = concat!(
            "{\"type\":\"user\",\"cwd\":\"/a\"}\n",
            "{\"type\":\"custom-title\",\"customTitle\":\"付けた題\"}\n",
            "{\"type\":\"ai-title\",\"aiTitle\":\"自動の題\"}\n",
        );
        assert_eq!(read_claude(jsonl.as_bytes()).map(|s| s.title).as_deref(), Some("付けた題"));
    }

    #[test]
    fn claude_の引き継がれた会話は出さない() {
        let jsonl = concat!(
            "{\"type\":\"user\",\"cwd\":\"/a\"}\n",
            "{\"type\":\"ai-title\",\"aiTitle\":\"題\"}\n",
            "{\"type\":\"continued-in\",\"continuedInSessionId\":\"b\"}\n",
        );
        assert_eq!(read_claude(jsonl.as_bytes()), None);
    }

    #[test]
    fn claude_の題がない会話は出さない() {
        let jsonl = "{\"type\":\"user\",\"cwd\":\"/a\",\"message\":\"ai-title の話\"}\n";
        assert_eq!(read_claude(jsonl.as_bytes()), None);
    }

    #[test]
    fn antigravity_は子の会話と空の会話を除き_題がなければプレビューを使う() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE conversation_summaries (conversation_id text, title text, preview text,
                 step_count integer, last_modified_time datetime, workspace_uris text, parent_conversation_id text);
             INSERT INTO conversation_summaries VALUES
                 ('c1', '題', 'p', 3, '2026-11-11 09:00:00.1+00:00', '[\"file:///a\"]', ''),
                 ('c2', '', 'プレビュー', 3, '2026-11-11 10:00:00+00:00', '[\"file:///b\"]', ''),
                 ('c3', '', '', 3, '2026-11-11 11:00:00+00:00', '[\"file:///c\"]', ''),
                 ('c4', '子', 'p', 3, '2026-11-11 12:00:00+00:00', '[\"file:///d\"]', 'c1'),
                 ('c5', '空', 'p', 0, '2026-11-11 13:00:00+00:00', '[\"file:///e\"]', ''),
                 ('c6', 'フォルダなし', 'p', 3, '2026-11-11 14:00:00+00:00', '[]', '');",
        )
        .unwrap();
        let entries = query_antigravity(&conn).unwrap();
        let got: Vec<(&str, &str, &str)> =
            entries.iter().map(|e| (e.id.as_str(), e.cwd.as_str(), e.title.as_str())).collect();
        assert_eq!(got, [("c2", "/b", "プレビュー"), ("c1", "/a", "題")]);
        assert_eq!(entries[0].agent, "agy");
    }

    #[test]
    fn antigravity_の時刻は秒より細かい分を捨て_時差を引く() {
        assert_eq!(epoch("1970-01-01 00:00:00+00:00"), Some(0));
        assert_eq!(epoch("2024-03-01 09:58:56.357525+00:00"), Some(1709287136));
        assert_eq!(epoch("2024-03-01 18:58:56+09:00"), Some(1709287136));
        assert_eq!(epoch("2024-03-01T09:58:56.357Z"), Some(1709287136));
        assert_eq!(epoch("2024-03-01"), None);
    }

    #[test]
    fn ファイルの_uri_はパーセントエンコードを戻す() {
        assert_eq!(file_path("file:///Users/x/my%20repo/%E6%97%A5"), Some("/Users/x/my repo/日".into()));
        assert_eq!(file_path("https://example.com/a"), None);
    }

    #[test]
    fn 新しい順に50件まで() {
        let got = newest((0..60).map(entry).collect());
        assert_eq!(got.len(), 50);
        assert_eq!((got[0].updated, got[49].updated), (59, 10));
    }

    #[test]
    fn 題のタブと改行は空白にする() {
        let e = Entry { title: " 直して\tほしい\r\n2行目 ".into(), ..entry(0) };
        assert_eq!(line(&e, "11/13 09:05"), "claude\t0\t11/13 09:05\t/a\t直して ほしい  2行目");
    }
}
