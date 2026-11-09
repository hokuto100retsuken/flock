mod model;

use std::collections::BTreeMap;

use model::{Line, Model, Pane, Report, Status, Tab};
use unicode_width::UnicodeWidthChar;
use zellij_tile::prelude::*;

const PIPE_NAME: &str = "flock";
const SYNC_REQUEST: &str = "flock-sync-request";
const SYNC: &str = "flock-sync";
const NEW: &str = "flock-new";
const NEXT: &str = "flock-next";
const PREV: &str = "flock-prev";
const FOCUS: &str = "flock-focus";
const LOG_TOGGLE: &str = "flock-log-toggle";
/// 詳細を開いたペインの目印
const DETAIL: &str = "flock-detail";

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
/// 右側のログの幅。レイアウトの size と合わせる
const LOG_WIDTH: usize = 40;
/// 戻したログを LOG_WIDTH まで縮めるときに、縮める回数の上限
const FIT_TRIES: u32 = 20;

const RESET: &str = "\u{1b}[0m";
const BOLD: &str = "\u{1b}[1m";
const DIM: &str = "\u{1b}[2m";

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
    /// 一度隠して戻した
    restored: bool,
    /// ログの各行を押したときに開く詳細のファイル
    row_files: Vec<String>,
}

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        self.plugin_id = get_plugin_ids().plugin_id;
        self.log_view = configuration.get("mode").map(String::as_str) == Some("log");
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
        ]);
    }

    fn update(&mut self, event: Event) -> bool {
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
            // 権限の確認ダイアログに答えられるよう、許可されるまではフォーカスできるままにする
            Event::PermissionRequestResult(PermissionStatus::Granted) => set_selectable(false),
            Event::Mouse(Mouse::LeftClick(line, _)) if self.log_view => {
                self.open_detail(line);
                return false;
            }
            Event::Mouse(Mouse::LeftClick(line, _)) => {
                self.focus_line(line);
                return false;
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
        match (message.name.as_str(), &message.source) {
            (SYNC_REQUEST, PipeSource::Plugin(from)) if *from != self.plugin_id => {
                pipe_message_to_plugin(
                    MessageToPlugin::new(SYNC)
                        .with_destination_plugin_id(*from)
                        .with_payload(self.model.export()),
                );
                return false;
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
                        self.restored = true;
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
        let report = message.args.get("state").and_then(|v| Report::parse(v));
        match (pane_id, report) {
            (Some(pane_id), Some(report)) => {
                // 中身は「時刻 タブ 詳細のファイル タブ ログの1行」。時刻はフック側で付ける
                let payload = message.payload.as_deref().unwrap_or_default();
                let mut fields = payload.splitn(3, '\t');
                let mut field = || fields.next().unwrap_or_default();
                let (time, file, detail) = (field(), field(), field());
                self.model.record(pane_id, &report, time, file, detail);
                self.model.apply(pane_id, report);
                true
            }
            _ => false,
        }
    }

    fn render(&mut self, rows: usize, cols: usize) {
        if self.log_view {
            return self.render_log(rows, cols);
        }
        let mut number = 0;
        for line in self.model.lines() {
            match line {
                Line::Tab { name, .. } => println!("{BOLD}{}{RESET}", truncate(&name, cols)),
                Line::Agent(row) => {
                    number += 1;
                    let (icon, color) = status_style(row.status);
                    let title = truncate(&row.title, cols.saturating_sub(6));
                    println!("{DIM}{number:>2}{RESET} {color}{icon}{RESET} {DIM}{title}{RESET}");
                }
            }
        }
        // 一覧との間を1行あける
        println!();
        println!("{BOLD}{NEW_LABEL}{RESET}");
    }
}

impl State {
    /// 右側のログ。このタブでフォーカスしているエージェントの指示と操作を、新しいものを下に出す
    fn render_log(&mut self, rows: usize, cols: usize) {
        // 隠して戻したペインは枠なしの指定が外れて zellij が区切り線を引くので、自分では引かない
        let bar = if self.restored { String::new() } else { format!("{DIM}│{RESET}") };
        let width = cols.saturating_sub(2);
        let mut lines = Vec::new();
        self.row_files.clear();
        if let Some((pane_id, title)) = self.log_target() {
            let (icon, color) = self
                .model
                .status(pane_id)
                .map(status_style)
                .unwrap_or(("○", ""));
            let title = truncate(&title, width.saturating_sub(2));
            lines.push(format!("{color}{icon}{RESET} {BOLD}{title}{RESET}"));
            lines.push(String::new());
            let entries: Vec<_> = self.model.log(pane_id).collect();
            let room = rows.saturating_sub(lines.len());
            self.row_files = vec![String::new(); lines.len()];
            for entry in &entries[entries.len().saturating_sub(room)..] {
                self.row_files.push(entry.file.clone());
                let text = truncate(&entry.text, width.saturating_sub(entry.time.len() + 1));
                // 指示は操作と見分けやすいよう太字にする
                let style = if text.starts_with("> ") { BOLD } else { "" };
                lines.push(format!("{DIM}{}{RESET} {style}{text}{RESET}", entry.time));
            }
        }
        // 区切り線を下まで引くため、空いた行にも出す。最後の行で改行すると1行ずれる
        lines.resize(rows, String::new());
        let lines: Vec<String> = lines.iter().map(|l| format!("{bar} {l}")).collect();
        print!("{}", lines.join("\n"));
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
            FloatingPaneCoordinates::new(
                Some("10%".into()),
                Some("10%".into()),
                Some("80%".into()),
                Some("80%".into()),
                None,
                None,
            ),
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
        let title = agents.iter().find(|p| Some(p.id) == shown)?.title.clone();
        self.shown = shown;
        shown.map(|id| (id, title))
    }

    fn focus_line(&self, line: isize) {
        let Ok(index) = usize::try_from(line) else {
            return;
        };
        let lines = self.model.lines();
        match lines.get(index) {
            Some(Line::Agent(row)) => focus_terminal_pane(row.pane_id, false, false),
            // タブ名を押したら、そのタブへ切り替えて名前の入力状態にする（Ctrl+t → r と同じ）。
            // 右クリックは zellij がプラグインに渡さないので左クリックにしている
            Some(Line::Tab { position, .. }) => {
                // switch_tab_to と rename_tab は1始まり
                let tab = *position as u32 + 1;
                switch_tab_to(tab);
                // 入力状態に入った時点の名前が Esc で戻す先になるので、空にするのはその後
                switch_to_input_mode(&InputMode::RenameTab);
                rename_tab(tab, "");
            }
            None if index == lines.len() + 1 => self.open_new_tab(None, None),
            _ => {}
        }
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

    /// 新しいタブで Claude Code を起動する。フォルダの指定がなければ今いるペインと同じフォルダ、
    /// 名前の指定がなければフォルダ名
    fn open_new_tab(&self, cwd: Option<String>, name: Option<String>) {
        let cwd = cwd.or_else(|| {
            let (_, pane_id) = get_focused_pane_info().ok()?;
            Some(get_pane_cwd(pane_id).ok()?.display().to_string())
        });
        let Some(cwd) = cwd else {
            return;
        };
        let Some(url) = self.own_pane().and_then(|(_, pane)| pane.plugin_url.clone()) else {
            return;
        };
        let name = name.unwrap_or_else(|| {
            cwd.rsplit('/')
                .find(|s| !s.is_empty())
                .unwrap_or("new")
                .to_owned()
        });
        let command = self
            .configuration
            .get("claude_command")
            .map(String::as_str)
            .unwrap_or("claude");
        let config: String = self
            .configuration
            .iter()
            .map(|(k, v)| format!("{k} {}; ", kdl_string(v)))
            .collect();
        // 新しいタブにもサイドバーを付けるので、レイアウトの default_tab_template と同じ形にする
        new_tabs_with_layout(&format!(
            r#"layout {{
    tab name={name} cwd={cwd} focus=true {{
        pane split_direction="vertical" {{
            pane size=32 borderless=true {{
                plugin location={url} {{ {config}}}
            }}
            pane command="sh" focus=true {{
                args "-c" {command}
            }}
            pane size={LOG_WIDTH} borderless=true {{
                plugin location={url} {{ {config}mode "log"; }}
            }}
        }}
        pane size=1 borderless=true {{
            plugin location="compact-bar"
        }}
    }}
}}"#,
            name = kdl_string(&name),
            cwd = kdl_string(&cwd),
            url = kdl_string(&url),
            command = kdl_string(&format!("exec {command}")),
        ));
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
        let active_tab = self.tabs.iter().find(|t| t.active).map(|t| t.position);
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
                        focused: p.is_focused && active_tab == Some(*tab_position),
                    })
            })
            .collect();
        self.model.set_panes(panes);
        self.model.set_tabs(
            self.tabs
                .iter()
                .map(|t| Tab {
                    position: t.position,
                    name: t.name.clone(),
                })
                .collect(),
        );
    }
}

fn status_style(status: Status) -> (&'static str, &'static str) {
    match status {
        Status::Working => ("⠋", "\u{1b}[36m"),
        Status::Blocked => ("●", "\u{1b}[31m"),
        Status::Done => ("✓", "\u{1b}[32m"),
        Status::Idle => ("○", "\u{1b}[90m"),
    }
}

fn kdl_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
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
