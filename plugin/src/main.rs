mod model;
mod resume;

use std::collections::BTreeMap;

use model::{display_title, Line, Model, Pane, Report, Status, Tab};
use resume::{Agent, Conversation, Picker};
use unicode_width::UnicodeWidthChar;
use zellij_tile::prelude::*;

const PIPE_NAME: &str = "flock";
const SYNC_REQUEST: &str = "flock-sync-request";
const SYNC: &str = "flock-sync";
const SEEN: &str = "flock-seen";
const NEW: &str = "flock-new";
const NEXT: &str = "flock-next";
const PREV: &str = "flock-prev";
const FOCUS: &str = "flock-focus";
const LOG_TOGGLE: &str = "flock-log-toggle";
/// 詳細を開いたペインの目印
const DETAIL: &str = "flock-detail";
/// flock history の結果の目印
const HISTORY: &str = "flock-history";

/// タブの並びを書き出す。$1 がセッション名、$2 が中身。flock を起動し直すときに読む
const SAVE_TABS: &str = r#"dir=${XDG_STATE_HOME:-$HOME/.local/state}/flock && mkdir -p "$dir" && printf '%s\n' "$2" >"$dir/$1.tabs""#;

// セッションの中だけで効かせる。zellij の設定ファイルには書かない
const KEYBINDS: &str = r#"
keybinds {
    shared_except "locked" {
        bind "Alt j" { MessagePlugin { name "flock-next"; }; }
        bind "Alt k" { MessagePlugin { name "flock-prev"; }; }
        bind "Alt g" { MessagePlugin { name "flock-log-toggle"; }; }
    }
}
"#;

const NEW_LABEL: &str = "＋ new";
/// ＋ new を押すと出す選択肢。名前と flock new の --agent に当たる値
const AGENTS: [(&str, Option<&str>); 2] = [("Claude Code", None), ("Antigravity", Some("agy"))];
/// エージェントの選択肢の後に出す、過去の会話を再開する選択肢
const RESUME_LABEL: &str = "Resume";
const HINT: &str = "Alt+j/k move · Alt+g log";
const RESUME_HINT: &str = "type to filter · ↑↓ move · Enter resume · Esc close";
/// 再開する会話の一覧で、フォルダ名に取る幅
const FOLDER_WIDTH: usize = 14;
/// working のアイコンをこの順に回す
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SPIN_SECONDS: f64 = 0.1;
/// 右側のログの幅。レイアウトの size と合わせる
const LOG_WIDTH: usize = 40;
/// 戻したログを LOG_WIDTH まで縮めるときに、縮める回数の上限
const FIT_TRIES: u32 = 20;

const RESET: &str = "\u{1b}[0m";
const BOLD: &str = "\u{1b}[1m";
const DIM: &str = "\u{1b}[2m";
const GREEN: &str = "\u{1b}[32m";
/// 選んでいる行の背景。lazygit の既定と同じ青
const SELECTED: &str = "\u{1b}[44m";
/// マウスが乗っている、押せる行
const HOVER: &str = "\u{1b}[100m";

#[derive(Default)]
struct State {
    model: Model,
    manifest: Option<PaneManifest>,
    tabs: Vec<TabInfo>,
    plugin_id: u32,
    configuration: BTreeMap<String, String>,
    initialized: bool,
    /// 右側のログとして動く。設定の mode "log" で決まる
    log_view: bool,
    hidden: bool,
    /// ログに出しているエージェント。エージェント以外のペインに移っても出し続ける
    shown: Option<u32>,
    /// ログを戻した後、幅を縮める残りの回数
    fitting: u32,
    /// 最後に縮めたときの幅。縮めた結果が届く前に重ねて縮めないよう、幅が変わるまで待つ
    fitted_from: Option<usize>,
    /// ログの各行を押したときに開く詳細のファイル
    row_files: Vec<String>,
    session_name: Option<String>,
    /// 最後に書き出したタブの並び。変わったときだけ書き出す
    saved_tabs: String,
    /// working のアイコンの何コマ目か
    frame: usize,
    /// アイコンを回すタイマーを仕掛けている
    spinning: bool,
    /// ＋ new を押して、起動するエージェントの選択肢を出している
    choosing: bool,
    /// マウスが乗っている行。zellij はペインから出たことを知らせないので、最後に乗った行のまま残る
    hover: Option<usize>,
    /// 過去の会話を選んで再開する画面として動く。設定の mode "resume" で決まる
    resume: Option<History>,
    /// 再開する会話の一覧で、いちばん上に出している会話の位置と、出している件数
    resume_top: usize,
    resume_shown: usize,
}

enum History {
    Loading,
    /// flock history が失敗した。標準エラーの1行目
    Failed(String),
    Loaded(Picker),
}

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        self.plugin_id = get_plugin_ids().plugin_id;
        self.log_view = configuration.get("mode").map(String::as_str) == Some("log");
        if configuration.get("mode").map(String::as_str) == Some("resume") {
            self.resume = Some(History::Loading);
            subscribe(&[EventType::Key, EventType::RunCommandResult]);
        }
        self.configuration = configuration;
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::ReadCliPipes,
            PermissionType::ChangeApplicationState,
            PermissionType::MessageAndLaunchOtherPlugins,
            PermissionType::Reconfigure,
            PermissionType::RunCommands,
        ]);
        subscribe(&[
            EventType::PaneUpdate,
            EventType::TabUpdate,
            EventType::PermissionRequestResult,
            EventType::Mouse,
            EventType::CommandPaneExited,
            EventType::ModeUpdate,
            EventType::Timer,
        ]);
    }

    fn update(&mut self, event: Event) -> bool {
        if self.resume.is_some() {
            return self.update_resume(event);
        }
        match event {
            Event::PaneUpdate(manifest) => {
                self.manifest = Some(manifest);
                self.fit();
                // PaneUpdate が届くのは権限が許可された後なので、権限の要る初期化はここで1回だけする
                if !self.initialized {
                    self.initialized = true;
                    // サイドバーはタブごとに別々に動くので、開く前に届いた状態を他のタブからもらう
                    pipe_message_to_plugin(MessageToPlugin::new(SYNC_REQUEST));
                    if !self.log_view {
                        reconfigure(KEYBINDS.to_owned(), false);
                    }
                }
            }
            Event::TabUpdate(tabs) => self.tabs = tabs,
            Event::ModeUpdate(mode) => {
                self.session_name = mode.session_name;
                return false;
            }
            Event::Timer(_) => {
                self.spinning = false;
                self.frame += 1;
                return self.model.any_working();
            }
            // 権限の確認ダイアログに答えられるよう、許可されるまではフォーカスできるままにする
            Event::PermissionRequestResult(PermissionStatus::Granted) => set_selectable(false),
            Event::Mouse(Mouse::LeftClick(line, _)) if self.log_view => {
                self.open_detail(line);
                return false;
            }
            Event::Mouse(Mouse::LeftClick(line, _)) => return self.focus_line(line),
            Event::Mouse(Mouse::Hover(line, _)) => {
                let hover = usize::try_from(line).ok();
                let changed = hover != self.hover;
                self.hover = hover;
                return changed;
            }
            // 詳細は less を閉じたらペインごと閉じる。閉じないと終了したペインが残る
            Event::CommandPaneExited(pane_id, _, context) if context.contains_key(DETAIL) => {
                close_terminal_pane(pane_id);
                return false;
            }
            _ => return false,
        }
        self.sync();
        true
    }

    fn pipe(&mut self, message: PipeMessage) -> bool {
        // 再開する画面はサイドバー向けの操作に加わらない。同じタブにいるので、加わると新しいタブを二重に開く
        if self.resume.is_some() {
            if let PipeSource::Cli(pipe_id) = &message.source {
                unblock_cli_pipe_input(pipe_id);
            }
            return false;
        }
        match (message.name.as_str(), &message.source) {
            (SYNC_REQUEST, PipeSource::Plugin(from)) if *from != self.plugin_id => {
                pipe_message_to_plugin(
                    MessageToPlugin::new(SYNC)
                        .with_destination_plugin_id(*from)
                        .with_payload(self.model.export()),
                );
                return false;
            }
            (SEEN, PipeSource::Plugin(from)) if *from != self.plugin_id => {
                let pane_id = message.payload.as_deref().and_then(|p| p.parse().ok());
                return pane_id.is_some_and(|id| self.model.seen(id));
            }
            (SYNC, _) => {
                self.model.import(message.payload.as_deref().unwrap_or_default());
                return true;
            }
            (LOG_TOGGLE, _) => {
                // どのタブでも同じ見え方にするので、すべてのタブのログが切り替わる
                if self.log_view {
                    self.hidden = !self.hidden;
                    if self.hidden {
                        hide_self();
                    } else {
                        show_self(false);
                        self.fitting = FIT_TRIES;
                        self.fitted_from = None;
                    }
                }
                return false;
            }
            (PIPE_NAME | NEW | NEXT | PREV | FOCUS, _) => {}
            _ => return false,
        }
        // CLI 側はプラグインが受け取るまで待つので、すぐに解放する
        if let PipeSource::Cli(pipe_id) = &message.source {
            unblock_cli_pipe_input(pipe_id);
            // 標準入力が端末でないと、zellij pipe は中身を送った後に中身なしでもう1回送ってくる
            if message.payload.is_none() {
                return false;
            }
        }
        if message.name != PIPE_NAME {
            if self.log_view {
                return false;
            }
            // 操作はすべてのタブのサイドバーに届く。隠れているタブのサイドバーには
            // どのタブが表示中かの更新が届かないので、フォーカス中のペインをその場で問い合わせ、
            // それと同じタブにいるサイドバーだけが動く
            if let Ok((tab, pane)) = get_focused_pane_info() {
                if self.own_pane().map(|(own_tab, _)| own_tab) == Some(tab) {
                    self.run(&message, pane);
                }
            }
            return false;
        }
        let pane_id = message.args.get("pane").and_then(|v| v.parse().ok());
        if let (Some(pane_id), Some(session)) = (pane_id, message.args.get("session")) {
            self.model.set_session(pane_id, session);
        }
        let report = message.args.get("state").and_then(|v| Report::parse(v));
        match (pane_id, report) {
            (Some(pane_id), Some(report)) => {
                // 中身は「時刻 タブ 詳細のファイル タブ ログの1行」。時刻はフック側で付ける
                let payload = message.payload.as_deref().unwrap_or_default();
                let mut fields = payload.splitn(3, '\t');
                let mut field = || fields.next().unwrap_or_default();
                let (time, file, detail) = (field(), field(), field());
                self.model.record(pane_id, &report, time, file, detail);
                // 見ている最中に終わったかを、今フォーカスしているペインで決める
                self.sync();
                self.model.apply(pane_id, report);
                self.save_tabs();
                true
            }
            _ => false,
        }
    }

    fn render(&mut self, rows: usize, cols: usize) {
        // working がいる間だけ描き直し続ける。見えていないタブは描かれないので回らない
        if self.model.any_working() && !self.spinning {
            self.spinning = true;
            set_timeout(SPIN_SECONDS);
        }
        if self.log_view {
            return self.render_log(rows, cols);
        }
        if self.resume.is_some() {
            return self.render_resume(rows, cols);
        }
        // 隠れていたタブに切り替わると、古いフォーカスのまま描かれるので描く前に聞き直す
        self.sync();
        let inner = cols.saturating_sub(2);
        // 上の枠の1行ぶんずらす
        let hovered = self.hover.and_then(|line| line.checked_sub(1));
        let mut body = Vec::new();
        let mut number = 0;
        for line in self.model.lines() {
            match line {
                Line::Tab { name, .. } => {
                    body.push(format!(" {BOLD}{}{RESET}", truncate(&name, inner.saturating_sub(1))))
                }
                Line::Row(row) => {
                    // 番号は flock focus N で選べるエージェントにだけ振る
                    let (label, (icon, color)) = match row.status {
                        Some(status) => {
                            number += 1;
                            (format!("{number:>2}"), status_style(status, self.frame))
                        }
                        None => ("  ".to_owned(), ("-", DIM)),
                    };
                    let title = truncate(&row.title, inner.saturating_sub(7));
                    let focused = self.model.is_focused(row.pane_id);
                    body.push(if focused && hovered != Some(body.len()) {
                        paint(&format!(" {label} {color}{icon}{RESET} {BOLD}{title}{RESET}"), inner, SELECTED)
                    } else if focused {
                        format!(" {label} {color}{icon}{RESET} {BOLD}{title}{RESET}")
                    } else {
                        format!(" {DIM}{label}{RESET} {color}{icon}{RESET} {DIM}{title}{RESET}")
                    });
                }
            }
        }
        // 一覧との間を1行あける。空き行は押しても何も起きないので光らせない
        let blank = body.len();
        body.push(String::new());
        body.push(format!(" {BOLD}{NEW_LABEL}{RESET}"));
        if self.choosing {
            let choices = AGENTS.iter().map(|(name, _)| *name).chain([RESUME_LABEL]);
            body.extend(choices.map(|name| format!("    {name}")));
        }
        if let Some(i) = hovered.filter(|&i| i != blank && i < body.len()) {
            body[i] = paint(&body[i], inner, HOVER);
        }
        let mut lines = boxed(&format!("{BOLD}Agents{RESET}"), GREEN, &body, cols, rows.saturating_sub(1));
        lines.push(format!(" {DIM}{}{RESET}", truncate(HINT, cols.saturating_sub(1))));
        // 最後の行で改行すると1行ずれる
        print!("{}", lines.join("\n"));
    }
}

impl State {
    /// 右側のログ。このタブでフォーカスしているエージェントの指示と操作を、新しいものを下に出す
    fn render_log(&mut self, rows: usize, cols: usize) {
        let inner = cols.saturating_sub(2);
        let mut title = format!("{BOLD}Log{RESET}");
        let mut body = Vec::new();
        // 上の枠の行は押しても何も開かない
        self.row_files = vec![String::new()];
        if let Some((pane_id, name)) = self.log_target() {
            let (icon, color) = self
                .model
                .status(pane_id)
                .map(|s| status_style(s, self.frame))
                .unwrap_or(("○", ""));
            title = format!("{color}{icon}{RESET} {BOLD}{}{RESET}", truncate(&name, inner.saturating_sub(5)));
            let entries: Vec<_> = self.model.log(pane_id).collect();
            let room = rows.saturating_sub(2);
            for entry in &entries[entries.len().saturating_sub(room)..] {
                self.row_files.push(entry.file.clone());
                let text = truncate(&entry.text, inner.saturating_sub(entry.time.len() + 2));
                // 指示は操作と見分けやすいよう太字にする
                let style = if text.starts_with("> ") { BOLD } else { "" };
                let line = format!(" {DIM}{}{RESET} {style}{text}{RESET}", entry.time);
                let clickable = !entry.file.is_empty();
                body.push(if clickable && self.hover == Some(self.row_files.len() - 1) {
                    paint(&line, inner, HOVER)
                } else {
                    line
                });
            }
        }
        // 最後の行で改行すると1行ずれる
        print!("{}", boxed(&title, "", &body, cols, rows).join("\n"));
    }

    fn open_detail(&self, line: isize) {
        let Some(file) = usize::try_from(line)
            .ok()
            .and_then(|i| self.row_files.get(i))
            .filter(|f| !f.is_empty())
        else {
            return;
        };
        open_command_pane_floating(
            CommandToRun::new_with_args("less", vec!["-R", file]),
            floating_coordinates(),
            BTreeMap::from([(DETAIL.to_owned(), String::new())]),
        );
    }

    /// 隠したペインを戻すと、zellij は隣のペインを半分に割った幅で戻すので、元の幅まで縮める
    fn fit(&mut self) {
        if self.fitting == 0 {
            return;
        }
        let Some((_, pane)) = self.own_pane() else {
            return;
        };
        if pane.is_suppressed {
            return;
        }
        if pane.pane_columns <= LOG_WIDTH {
            self.fitting = 0;
            return;
        }
        if self.fitted_from == Some(pane.pane_columns) {
            return;
        }
        self.fitted_from = Some(pane.pane_columns);
        self.fitting -= 1;
        resize_pane_with_id(
            ResizeStrategy::new(Resize::Decrease, Some(Direction::Left)),
            PaneId::Plugin(self.plugin_id),
        );
    }

    fn log_target(&mut self) -> Option<(u32, String)> {
        let (tab, _) = self.own_pane()?;
        let agents: Vec<&PaneInfo> = self
            .manifest
            .as_ref()?
            .panes
            .get(&tab)?
            .iter()
            .filter(|p| !p.is_plugin && self.model.status(p.id).is_some())
            .collect();
        let shown = match agents.iter().find(|p| p.is_focused) {
            Some(focused) => Some(focused.id),
            None if self.shown.is_some_and(|id| agents.iter().any(|p| p.id == id)) => self.shown,
            None => agents.first().map(|p| p.id),
        };
        let title = display_title(&agents.iter().find(|p| Some(p.id) == shown)?.title);
        self.shown = shown;
        shown.map(|id| (id, title))
    }

    /// 押した行に応じて動かす。選択肢の開け閉めで描き直しが要るなら true
    fn focus_line(&mut self, line: isize) -> bool {
        let was_choosing = std::mem::take(&mut self.choosing);
        // 上の枠の1行ぶんずらす
        let Ok(index) = usize::try_from(line - 1) else {
            return was_choosing;
        };
        let lines = self.model.lines();
        // 一覧、空き行、＋ new、選択肢の順に並ぶ
        let new_line = lines.len() + 1;
        match lines.get(index) {
            Some(Line::Row(row)) => focus_terminal_pane(row.pane_id, false, false),
            // switch_tab_to は1始まり
            Some(Line::Tab { position, .. }) => switch_tab_to(*position as u32 + 1),
            None if index == new_line => self.choosing = !was_choosing,
            None if was_choosing => match index.checked_sub(new_line + 1) {
                Some(i) if i < AGENTS.len() => self.open_new_tab(None, None, AGENTS[i].1),
                Some(i) if i == AGENTS.len() => self.open_resume(),
                _ => {}
            },
            None => {}
        }
        was_choosing || self.choosing
    }

    fn run(&self, message: &PipeMessage, focused: PaneId) {
        let current = match focused {
            PaneId::Terminal(id) => Some(id),
            PaneId::Plugin(_) => None,
        };
        let target = match message.name.as_str() {
            NEW => {
                return self.open_new_tab(
                    message.args.get("cwd").cloned(),
                    message.args.get("name").cloned(),
                    message.args.get("agent").map(String::as_str),
                )
            }
            NEXT => self.model.neighbor(current, 1),
            PREV => self.model.neighbor(current, -1),
            FOCUS => message
                .args
                .get("n")
                .and_then(|n| n.parse().ok())
                .and_then(|n| self.model.nth(n)),
            _ => None,
        };
        if let Some(pane_id) = target {
            focus_terminal_pane(pane_id, false, false);
        }
    }

    /// 新しいタブでエージェント（指定がなければ Claude Code）を起動する。
    /// フォルダの指定がなければ今いるペインと同じフォルダ、名前の指定がなければフォルダ名
    fn open_new_tab(&self, cwd: Option<String>, name: Option<String>, agent: Option<&str>) {
        let cwd = cwd.or_else(|| {
            let (_, pane_id) = get_focused_pane_info().ok()?;
            Some(get_pane_cwd(pane_id).ok()?.display().to_string())
        });
        let Some(cwd) = cwd else {
            return;
        };
        let name = name.unwrap_or_else(|| {
            cwd.rsplit('/')
                .find(|s| !s.is_empty())
                .unwrap_or("new")
                .to_owned()
        });
        let command = match agent {
            Some("agy") => "agy",
            _ => self.claude_command(),
        };
        self.open_tab(&name, &cwd, &format!("exec {command}"));
    }

    /// 新しいタブを開き、command を sh -c で動かす。開けたら true
    fn open_tab(&self, name: &str, cwd: &str, command: &str) -> bool {
        let Some(url) = self.own_pane().and_then(|(_, pane)| pane.plugin_url.clone()) else {
            return false;
        };
        // 再開する画面から開くときも、新しいタブのプラグインはサイドバーとログとして動かす
        let config: String = self
            .configuration
            .iter()
            .filter(|(k, _)| *k != "mode")
            .map(|(k, v)| format!("{k} {}; ", kdl_string(v)))
            .collect();
        // 新しいタブにもサイドバーを付けるので、レイアウトの default_tab_template と同じ形にする
        let opened = new_tabs_with_layout(&format!(
            r#"layout {{
    tab name={name} cwd={cwd} focus=true {{
        pane split_direction="vertical" {{
            pane command="sh" focus=true close_on_exit=true {{
                args "-c" {command}
            }}
            pane size={LOG_WIDTH} split_direction="horizontal" {{
                pane borderless=true {{
                    plugin location={url} {{ {config}mode "log"; }}
                }}
                pane borderless=true {{
                    plugin location={url} {{ {config}}}
                }}
            }}
        }}
        pane size=1 borderless=true {{
            plugin location="compact-bar"
        }}
    }}
}}"#,
            name = kdl_string(name),
            cwd = kdl_string(cwd),
            url = kdl_string(&url),
            command = kdl_string(command),
        ));
        !opened.is_empty()
    }

    fn claude_command(&self) -> &str {
        self.configuration
            .get("claude_command")
            .map(String::as_str)
            .unwrap_or("claude")
    }

    /// 過去の会話を選ぶ画面を、このプラグインを mode "resume" にしたフローティングペインで開く
    fn open_resume(&self) {
        let Some(url) = self.own_pane().and_then(|(_, pane)| pane.plugin_url.clone()) else {
            return;
        };
        let mut config = self.configuration.clone();
        config.insert("mode".to_owned(), "resume".to_owned());
        // open_plugin_pane_floating は zellij 0.45.1 で返事が来ずにプラグインごと落ちるので、
        // 返事を待たない「送り先がなければ起動する」メッセージで開く
        let mut message = MessageToPlugin::new("flock-resume")
            .with_plugin_url(url)
            .with_plugin_config(config)
            .new_plugin_instance_should_float(true)
            .new_plugin_instance_should_be_focused();
        if let Some(coordinates) = floating_coordinates() {
            message = message.with_floating_pane_coordinates(coordinates);
        }
        pipe_message_to_plugin(message);
    }

    fn update_resume(&mut self, event: Event) -> bool {
        match event {
            Event::PaneUpdate(manifest) => {
                self.manifest = Some(manifest);
                // 権限が許可された後に届くので、ここで1回だけ一覧を取りに行く。
                // 文字を打てるよう、サイドバーと違ってフォーカスできるままにする
                if !self.initialized {
                    self.initialized = true;
                    let flock = self.configuration.get("flock_path").map(String::as_str).unwrap_or("flock");
                    run_command(&[flock, "history"], BTreeMap::from([(HISTORY.to_owned(), String::new())]));
                }
                false
            }
            Event::RunCommandResult(exit_code, stdout, stderr, context) if context.contains_key(HISTORY) => {
                self.resume = Some(if exit_code == Some(0) {
                    History::Loaded(Picker::new(resume::parse(&String::from_utf8_lossy(&stdout))))
                } else {
                    let stderr = String::from_utf8_lossy(&stderr);
                    History::Failed(stderr.lines().next().unwrap_or_default().to_owned())
                });
                true
            }
            Event::Key(key) => self.resume_key(key),
            Event::Mouse(Mouse::LeftClick(line, _)) => {
                // 上の枠と入力欄の2行ぶんずらす
                let Some(index) = usize::try_from(line - 2).ok().filter(|&i| i < self.resume_shown) else {
                    return false;
                };
                let conversation = match &self.resume {
                    Some(History::Loaded(picker)) => picker.filtered().get(self.resume_top + index).map(|c| (*c).clone()),
                    _ => None,
                };
                if let Some(conversation) = conversation {
                    self.open_conversation(&conversation);
                }
                false
            }
            _ => false,
        }
    }

    /// 押したキーに応じて動かす。描き直しが要るなら true
    fn resume_key(&mut self, key: KeyWithModifier) -> bool {
        if key.is_key_without_modifier(BareKey::Esc) {
            close_self();
            return false;
        }
        let Some(History::Loaded(picker)) = &mut self.resume else {
            return false;
        };
        let typed = key.has_no_modifiers() || key.has_only_modifiers(&[KeyModifier::Shift]);
        match key.bare_key {
            BareKey::Enter if typed => {
                if let Some(conversation) = picker.selected_entry().cloned() {
                    self.open_conversation(&conversation);
                }
                return false;
            }
            BareKey::Up if typed => picker.move_by(-1),
            BareKey::Down if typed => picker.move_by(1),
            BareKey::Backspace if typed => picker.pop(),
            BareKey::Char(c) if typed && !c.is_control() => picker.push(c),
            _ => return false,
        }
        true
    }

    /// 会話のフォルダで新しいタブを開いて再開し、選ぶ画面を閉じる
    fn open_conversation(&self, conversation: &Conversation) {
        let command = conversation.command(self.claude_command());
        if self.open_tab(&conversation.tab_name(), &conversation.cwd, &command) {
            close_self();
        }
    }

    fn render_resume(&mut self, rows: usize, cols: usize) {
        let inner = cols.saturating_sub(2);
        // 下に操作の案内を1行出すので、枠はその分低くする
        let height = rows.saturating_sub(1);
        // 枠の上下と入力欄を除いた、会話を出せる行数
        let room = height.saturating_sub(3);
        let mut body = Vec::new();
        let (mut top, mut shown) = (0, 0);
        match &self.resume {
            Some(History::Loaded(picker)) if picker.is_empty() => body.push(format!(" {DIM}No conversations{RESET}")),
            Some(History::Loaded(picker)) => {
                body.push(format!(" {BOLD}>{RESET} {}", truncate(picker.query(), inner.saturating_sub(3))));
                let conversations = picker.filtered();
                let selected = picker.selected();
                // 選んでいる会話が見える範囲に入るよう、はみ出したときだけずらす
                top = self.resume_top.min(selected).max((selected + 1).saturating_sub(room));
                for (i, conversation) in conversations.iter().enumerate().skip(top).take(room) {
                    let line = resume_row(conversation, inner);
                    body.push(if i == selected { paint(&line, inner, SELECTED) } else { line });
                }
                shown = body.len() - 1;
                if conversations.is_empty() {
                    body.push(format!(" {DIM}No matches{RESET}"));
                }
            }
            Some(History::Failed(error)) => {
                body.push(" Couldn't load history".to_owned());
                body.push(format!(" {DIM}{}{RESET}", truncate(error, inner.saturating_sub(1))));
            }
            _ => body.push(format!(" {DIM}Loading…{RESET}")),
        }
        self.resume_top = top;
        self.resume_shown = shown;
        let mut lines = boxed(&format!("{BOLD}Resume{RESET}"), GREEN, &body, cols, height);
        lines.push(format!(" {DIM}{}{RESET}", truncate(RESUME_HINT, cols.saturating_sub(1))));
        // 最後の行で改行すると1行ずれる
        print!("{}", lines.join("\n"));
    }

    fn own_pane(&self) -> Option<(usize, &PaneInfo)> {
        self.manifest.as_ref()?.panes.iter().find_map(|(tab, panes)| {
            panes
                .iter()
                .find(|p| p.is_plugin && p.id == self.plugin_id)
                .map(|p| (*tab, p))
        })
    }

    fn sync(&mut self) {
        // 隠れているタブのサイドバーにはタブやペインの更新が届かないので、見ているペインは zellij に聞く
        let focused = get_focused_pane_info().ok();
        let active_tab = focused
            .map(|(tab, _)| tab)
            .or_else(|| self.tabs.iter().find(|t| t.active).map(|t| t.position));
        let focused_terminal = match focused {
            Some((_, PaneId::Terminal(id))) => Some(id),
            _ => None,
        };
        let panes = self
            .manifest
            .iter()
            .flat_map(|m| m.panes.iter())
            .flat_map(|(tab_position, panes)| {
                panes
                    .iter()
                    .filter(|p| !p.is_plugin)
                    .map(move |p| Pane {
                        id: p.id,
                        tab_position: *tab_position,
                        title: p.title.clone(),
                        command: p.terminal_command.clone().unwrap_or_default(),
                        focused: match focused_terminal {
                            Some(id) => p.id == id,
                            None => p.is_focused && active_tab == Some(*tab_position),
                        },
                    })
            })
            .collect();
        // 隠れているタブのサイドバーには、タブを切り替えても更新が届かないことがあるので、見たことを知らせる
        for pane_id in self.model.set_panes(panes) {
            pipe_message_to_plugin(MessageToPlugin::new(SEEN).with_payload(pane_id.to_string()));
        }
        self.model.set_tabs(
            self.tabs
                .iter()
                .map(|t| Tab {
                    position: t.position,
                    name: t.name.clone(),
                })
                .collect(),
        );
        self.save_tabs();
    }

    /// Ctrl+q で終了した後に作り直せるよう、タブの名前・フォルダ・会話の ID を1行に1タブで書き出す。
    /// 隠れているタブのサイドバーにはタブの更新が届かず古い並びのままなので、表示中のタブのサイドバーだけが書く
    fn save_tabs(&mut self) {
        if self.log_view {
            return;
        }
        let Ok((focused_tab, _)) = get_focused_pane_info() else {
            return;
        };
        if self.own_pane().map(|(tab, _)| tab) != Some(focused_tab) {
            return;
        }
        let Some(session_name) = self.session_name.clone() else {
            return;
        };
        let tabs: Vec<String> = self
            .model
            .saved_tabs()
            .into_iter()
            .filter_map(|tab| {
                let cwd = get_pane_cwd(PaneId::Terminal(tab.pane_id)).ok()?;
                Some(format!(
                    "{}\t{}\t{}",
                    tab.name,
                    cwd.display(),
                    tab.session.unwrap_or_default()
                ))
            })
            .collect();
        let tabs = tabs.join("\n");
        if tabs.is_empty() || tabs == self.saved_tabs {
            return;
        }
        run_command(
            &["sh", "-c", SAVE_TABS, "sh", &session_name, &tabs],
            BTreeMap::new(),
        );
        self.saved_tabs = tabs;
    }
}

fn status_style(status: Status, frame: usize) -> (&'static str, &'static str) {
    match status {
        Status::Working => (SPINNER[frame % SPINNER.len()], "\u{1b}[36m"),
        Status::Blocked => ("●", "\u{1b}[31m"),
        Status::Done => ("✓", "\u{1b}[32m"),
        Status::Idle => ("○", "\u{1b}[90m"),
    }
}

/// 再開する会話の1行。エージェント、日時、フォルダ名、タイトルの順で、フォルダ名までは桁を揃える
fn resume_row(conversation: &Conversation, width: usize) -> String {
    let agent = match conversation.agent {
        Agent::Claude => "CC",
        Agent::Agy => "agy",
    };
    let folder = truncate(conversation.folder(), FOLDER_WIDTH);
    let folder = format!("{folder}{}", " ".repeat(FOLDER_WIDTH - visible_width(&folder)));
    let head = format!(" {agent:<3} {DIM}{}{RESET} {folder} ", conversation.updated);
    let title = truncate(&conversation.title, width.saturating_sub(visible_width(&head) + 1));
    format!("{head}{title}")
}

/// フローティングペインを開く位置。画面の真ん中に、縦横 80% の大きさで
fn floating_coordinates() -> Option<FloatingPaneCoordinates> {
    FloatingPaneCoordinates::new(
        Some("10%".into()),
        Some("10%".into()),
        Some("80%".into()),
        Some("80%".into()),
        None,
        None,
    )
}

fn kdl_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// lazygit のように角の丸い枠で囲み、上の枠にタイトルを入れる。中身が足りない行は空ける
fn boxed(title: &str, color: &str, body: &[String], width: usize, height: usize) -> Vec<String> {
    let inner = width.saturating_sub(2);
    let fill = inner.saturating_sub(visible_width(title) + 3);
    let mut lines = vec![format!("{color}╭─{RESET} {title} {color}{}╮{RESET}", "─".repeat(fill))];
    for i in 0..height.saturating_sub(2) {
        let line = body.get(i).map(String::as_str).unwrap_or_default();
        let pad = inner.saturating_sub(visible_width(line));
        lines.push(format!("{color}│{RESET}{line}{}{color}│{RESET}", " ".repeat(pad)));
    }
    lines.push(format!("{color}╰{}╯{RESET}", "─".repeat(inner)));
    lines
}

/// 選んでいる行として、幅いっぱいに背景を塗る
/// 行の幅いっぱいを背景色で塗る
fn paint(line: &str, width: usize, background: &str) -> String {
    let pad = width.saturating_sub(visible_width(line));
    // 途中の RESET で背景が消えないよう塗り直す
    let line = line.replace(RESET, &format!("{RESET}{background}"));
    format!("{background}{line}{}{RESET}", " ".repeat(pad))
}

/// 色の指定を除いた表示幅
fn visible_width(s: &str) -> usize {
    let mut width = 0;
    let mut escape = false;
    for c in s.chars() {
        if escape {
            escape = !c.is_ascii_alphabetic();
        } else if c == '\u{1b}' {
            escape = true;
        } else {
            width += c.width().unwrap_or(0);
        }
    }
    width
}

/// 表示幅に収まるよう切り詰める
fn truncate(s: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > width {
            break;
        }
        used += w;
        out.push(c);
    }
    out
}
