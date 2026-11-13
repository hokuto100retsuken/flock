mod history;
mod hook;
mod layout;
mod setup;

use std::{
    env,
    fs,
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Command, ExitCode, Stdio},
    thread,
    time::{Duration, Instant},
};

const USAGE: &str = "\
Usage:
  flock              start; reattach if running, or reopen the previous tabs with their conversations if it was quit
  flock --fresh      start with a single empty tab instead of reopening the previous tabs
  flock new [DIR] [--name NAME] [--agent claude|agy]
                     start Claude Code (or Antigravity CLI with --agent agy) in a new tab
                     (DIR defaults to the current folder, NAME to the folder name)
  flock focus N      move to agent N in the sidebar
  flock next|prev    move to the next / previous agent
  flock history      list recent Claude Code and Antigravity conversations (used by the sidebar's Resume)
  flock setup        add the hooks that report state to the sidebar to your Claude Code and Antigravity CLI settings
  flock hook STATE   called from the agent's hooks (working|blocked|stop|idle|end)";

type Result<T = ()> = std::result::Result<T, String>;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None => start(false),
        Some("--fresh") => start(true),
        Some("new") => new(&args[1..]),
        Some("focus") => match args.get(1) {
            Some(n) => Ok(pipe("flock-focus", Some(&format!("n={n}")), "flock-focus")),
            None => Err("specify a number".into()),
        },
        Some(step @ ("next" | "prev")) => {
            let name = format!("flock-{step}");
            Ok(pipe(&name, None, &name))
        }
        Some("history") => history::run(),
        Some("setup") => setup::run(),
        Some("hook") => Ok(hook::run(args.get(1).map(String::as_str).unwrap_or_default())),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("flock: {e}");
            ExitCode::FAILURE
        }
    }
}

fn start(fresh: bool) -> Result {
    if env::var_os("ZELLIJ").is_some() {
        return Err("cannot start inside zellij; use flock new to open a new tab".into());
    }
    let session = session();
    match session_state(&session)? {
        Some(layout::SessionState::Running) if fresh => {
            return Err("flock is running; quit it with Ctrl+q before starting fresh".into())
        }
        Some(layout::SessionState::Running) => {
            return Err(Command::new("zellij").args(["attach", &session]).exec().to_string())
        }
        // 終了済みのセッションを復元すると、そのときのレイアウトと設定のまま戻ってしまうので作り直す
        Some(layout::SessionState::Exited) => delete_session(&session)?,
        None => {}
    }
    // zellij は Ctrl+q で終えても最後のタブを閉じて終えても同じく EXITED で残し、終わり方では見分けられない。
    // 空で始めるかは起動するときに決めてもらう
    if fresh {
        let tabs = state_dir().join(format!("{session}.tabs"));
        if tabs.exists() {
            fs::remove_file(&tabs).map_err(|e| format!("{}: {e}", tabs.display()))?;
        }
    }
    let layout = layout::write(&session)?;
    Err(Command::new("zellij")
        .args(["--session", &session, "--new-session-with-layout"])
        .arg(layout)
        // 既定の titles はサイドバーとの境目にも線を引くので、枠なしのペインに線を引かない full にする
        .args(["options", "--pane-frame-style", "full"])
        .exec()
        .to_string())
}

fn session_state(session: &str) -> Result<Option<layout::SessionState>> {
    let listed = Command::new("zellij")
        .args(["list-sessions", "--no-formatting"])
        .output()
        .map_err(|e| format!("cannot start zellij: {e}"))?;
    Ok(layout::session_state(&String::from_utf8_lossy(&listed.stdout), session))
}

fn delete_session(session: &str) -> Result {
    Command::new("zellij")
        .args(["delete-session", session])
        .stdout(Stdio::null())
        .status()
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn new(args: &[String]) -> Result {
    let mut dir = PathBuf::from(".");
    let mut name = None;
    let mut agent = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--name" => name = Some(args.next().ok_or("specify a name")?),
            "--agent" => match args.next().map(String::as_str) {
                Some(a @ ("claude" | "agy")) => agent = Some(a),
                _ => return Err("specify claude or agy for --agent".into()),
            },
            _ => dir = PathBuf::from(arg),
        }
    }
    let dir = dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut message = format!("cwd={}", dir.display());
    if let Some(name) = name {
        message += &format!(",name={name}");
    }
    if let Some(agent) = agent {
        message += &format!(",agent={agent}");
    }
    pipe("flock-new", Some(&message), "flock-new");
    Ok(())
}

fn session() -> String {
    env::var("ZELLIJ_SESSION_NAME").unwrap_or_else(|_| "flock".into())
}

/// zellij pipe でサイドバーへ送る。サイドバーが権限の確認待ちなどで受け取れないと
/// 返ってこないので、2秒で諦める
fn pipe(name: &str, args: Option<&str>, payload: &str) {
    let mut command = Command::new("zellij");
    command
        .env("ZELLIJ_SESSION_NAME", session())
        .args(["pipe", "--name", name]);
    if let Some(args) = args {
        command.args(["--args", args]);
    }
    let Ok(mut child) = command
        .args(["--", payload])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn home() -> PathBuf {
    env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

/// XDG の置き場所。変数がなければ HOME からの既定
fn xdg(var: &str, default: &str) -> PathBuf {
    env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(default))
        .join("flock")
}

/// サイドバーが書くタブの並びと、フックが書くログの詳細の置き場所
fn state_dir() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state")
}
