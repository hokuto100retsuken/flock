mod model;

use std::collections::BTreeMap;

use model::{Line, Model, Pane, Report, Status, Tab};
use unicode_width::UnicodeWidthChar;
use zellij_tile::prelude::*;

const PIPE_NAME: &str = "flock";

const RESET: &str = "\u{1b}[0m";
const BOLD: &str = "\u{1b}[1m";
const DIM: &str = "\u{1b}[2m";

#[derive(Default)]
struct State {
    model: Model,
    manifest: Option<PaneManifest>,
    tabs: Vec<TabInfo>,
}

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, _configuration: BTreeMap<String, String>) {
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::ReadCliPipes,
            PermissionType::ChangeApplicationState,
        ]);
        subscribe(&[
            EventType::PaneUpdate,
            EventType::TabUpdate,
            EventType::PermissionRequestResult,
            EventType::Mouse,
        ]);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::PaneUpdate(manifest) => self.manifest = Some(manifest),
            Event::TabUpdate(tabs) => self.tabs = tabs,
            // 権限の確認ダイアログに答えられるよう、許可されるまではフォーカスできるままにする
            Event::PermissionRequestResult(PermissionStatus::Granted) => set_selectable(false),
            Event::Mouse(Mouse::LeftClick(line, _)) => {
                self.focus_line(line);
                return false;
            }
            _ => return false,
        }
        self.sync();
        true
    }

    fn pipe(&mut self, message: PipeMessage) -> bool {
        if message.name != PIPE_NAME {
            return false;
        }
        // CLI 側はプラグインが受け取るまで待つので、すぐに解放する
        if let PipeSource::Cli(pipe_id) = &message.source {
            unblock_cli_pipe_input(pipe_id);
        }
        let pane_id = message.args.get("pane").and_then(|v| v.parse().ok());
        let report = message.args.get("state").and_then(|v| Report::parse(v));
        match (pane_id, report) {
            (Some(pane_id), Some(report)) => {
                self.model.apply(pane_id, report);
                true
            }
            _ => false,
        }
    }

    fn render(&mut self, _rows: usize, cols: usize) {
        for line in self.model.lines() {
            match line {
                Line::Tab(name) => println!("{BOLD}{}{RESET}", truncate(&name, cols)),
                Line::Agent(row) => {
                    let (icon, color) = status_style(row.status);
                    let title = truncate(&row.title, cols.saturating_sub(4));
                    println!(" {color}{icon}{RESET} {DIM}{title}{RESET}");
                }
            }
        }
    }
}

impl State {
    fn focus_line(&self, line: isize) {
        let Ok(index) = usize::try_from(line) else {
            return;
        };
        if let Some(Line::Agent(row)) = self.model.lines().get(index) {
            focus_terminal_pane(row.pane_id, false, false);
        }
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
