mod hook;
mod layout;
mod setup;

use std::{
    env,
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Command, ExitCode, Stdio},
    thread,
    time::{Duration, Instant},
};

const USAGE: &str = "\
使い方:
  flock              起動する。起動中なら繋ぎ直し、終了していたら前のタブを会話の続きから開き直す
  flock new [DIR] [--name NAME]
                     新しいタブで Claude Code を起動する（DIR の既定は今のフォルダ、NAME の既定はフォルダ名）
  flock focus N      サイドバーの N 番のエージェントへ移動する
  flock next|prev    次 / 前のエージェントへ移動する
  flock setup        Claude Code の設定に、状態をサイドバーへ送るフックを足す
  flock hook STATE   Claude Code のフックから呼ぶ（working|blocked|stop|idle|end）";

type Result<T = ()> = std::result::Result<T, String>;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None => start(),
        Some("new") => new(&args[1..]),
        Some("focus") => match args.get(1) {
            Some(n) => Ok(pipe("flock-focus", Some(&format!("n={n}")), "flock-focus")),
            None => Err("番号を指定してください".into()),
        },
        Some(step @ ("next" | "prev")) => {
            let name = format!("flock-{step}");
            Ok(pipe(&name, None, &name))
        }
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

fn start() -> Result {
    if env::var_os("ZELLIJ").is_some() {
        return Err("zellij の中では起動できません。flock new で新しいタブを開いてください".into());
    }
    let session = session();
    let listed = Command::new("zellij")
        .args(["list-sessions", "--no-formatting"])
        .output()
        .map_err(|e| format!("zellij を起動できません: {e}"))?;
    match layout::session_state(&String::from_utf8_lossy(&listed.stdout), &session) {
        Some(layout::SessionState::Running) => {
            return Err(Command::new("zellij").args(["attach", &session]).exec().to_string())
        }
        // 終了済みのセッションを復元すると、そのときのレイアウトと設定のまま戻ってしまうので作り直す
        Some(layout::SessionState::Exited) => {
            Command::new("zellij")
                .args(["delete-session", &session])
                .stdout(Stdio::null())
                .status()
                .map_err(|e| e.to_string())?;
        }
        None => {}
    }
    let layout = layout::write(&session)?;
    Err(Command::new("zellij")
        .args(["--session", &session, "--new-session-with-layout"])
        .arg(layout)
        .exec()
        .to_string())
}

fn new(args: &[String]) -> Result {
    let mut dir = PathBuf::from(".");
    let mut name = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--name" => name = Some(args.next().ok_or("名前を指定してください")?),
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
