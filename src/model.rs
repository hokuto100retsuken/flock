use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Working,
    Blocked,
    Done,
    Idle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Report {
    Working,
    Blocked,
    Stop,
    Idle,
    End,
}

impl Report {
    pub fn parse(s: &str) -> Option<Report> {
        match s {
            "working" => Some(Report::Working),
            "blocked" => Some(Report::Blocked),
            "stop" => Some(Report::Stop),
            "idle" => Some(Report::Idle),
            "end" => Some(Report::End),
            _ => None,
        }
    }
}

/// zellij から受け取ったペインのうち、一覧に要る情報だけ
#[derive(Clone, Debug)]
pub struct Pane {
    pub id: u32,
    pub tab_position: usize,
    pub title: String,
    pub focused: bool,
}

#[derive(Clone, Debug)]
pub struct Tab {
    pub position: usize,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub pane_id: u32,
    pub status: Status,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub tab_name: String,
    pub rows: Vec<Row>,
}

#[derive(Default)]
pub struct Model {
    statuses: BTreeMap<u32, Status>,
    panes: Vec<Pane>,
    tabs: Vec<Tab>,
}

impl Model {
    pub fn apply(&mut self, pane_id: u32, report: Report) {
        let status = match report {
            Report::End => {
                self.statuses.remove(&pane_id);
                return;
            }
            Report::Working => Status::Working,
            Report::Blocked => Status::Blocked,
            Report::Idle => Status::Idle,
            // 見ている最中に終わったものは「見ていない間に終わった」にしない
            Report::Stop if self.is_focused(pane_id) => Status::Idle,
            Report::Stop => Status::Done,
        };
        self.statuses.insert(pane_id, status);
    }

    pub fn set_panes(&mut self, panes: Vec<Pane>) {
        self.panes = panes;
        self.statuses
            .retain(|id, _| self.panes.iter().any(|p| p.id == *id));
        for pane in self.panes.iter().filter(|p| p.focused) {
            if let Some(status) = self.statuses.get_mut(&pane.id) {
                if *status == Status::Done {
                    *status = Status::Idle;
                }
            }
        }
    }

    pub fn set_tabs(&mut self, tabs: Vec<Tab>) {
        self.tabs = tabs;
    }

    /// タブの位置 → ペインの作成順で並べる。状態が変わっても順番は変えない
    pub fn groups(&self) -> Vec<Group> {
        let mut tabs = self.tabs.clone();
        tabs.sort_by_key(|t| t.position);
        tabs.into_iter()
            .filter_map(|tab| {
                let mut rows: Vec<Row> = self
                    .panes
                    .iter()
                    .filter(|p| p.tab_position == tab.position)
                    .filter_map(|p| {
                        self.statuses.get(&p.id).map(|status| Row {
                            pane_id: p.id,
                            status: *status,
                            title: p.title.clone(),
                        })
                    })
                    .collect();
                if rows.is_empty() {
                    return None;
                }
                rows.sort_by_key(|r| r.pane_id);
                Some(Group {
                    tab_name: tab.name,
                    rows,
                })
            })
            .collect()
    }

    fn is_focused(&self, pane_id: u32) -> bool {
        self.panes.iter().any(|p| p.id == pane_id && p.focused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: u32, tab: usize, focused: bool) -> Pane {
        Pane {
            id,
            tab_position: tab,
            title: format!("p{id}"),
            focused,
        }
    }

    fn model() -> Model {
        let mut m = Model::default();
        m.set_tabs(vec![
            Tab { position: 1, name: "b".into() },
            Tab { position: 0, name: "a".into() },
        ]);
        m.set_panes(vec![pane(5, 1, false), pane(3, 0, false), pane(1, 0, true)]);
        m
    }

    fn ids(m: &Model) -> Vec<u32> {
        m.groups().iter().flat_map(|g| g.rows.iter().map(|r| r.pane_id)).collect()
    }

    #[test]
    fn 状態が変わっても並びは変わらない() {
        let mut m = model();
        for id in [5, 3, 1] {
            m.apply(id, Report::Working);
        }
        assert_eq!(ids(&m), vec![1, 3, 5]);
        m.apply(5, Report::Blocked);
        m.apply(3, Report::Stop);
        assert_eq!(ids(&m), vec![1, 3, 5]);
    }

    #[test]
    fn 状態を報告していないペインは出さない() {
        let mut m = model();
        m.apply(3, Report::Working);
        assert_eq!(ids(&m), vec![3]);
        assert_eq!(m.groups()[0].tab_name, "a");
    }

    #[test]
    fn 見ていないペインの完了はdone_見ているペインはidle() {
        let mut m = model();
        m.apply(1, Report::Stop);
        m.apply(3, Report::Stop);
        let rows = &m.groups()[0].rows;
        assert_eq!(rows[0].status, Status::Idle);
        assert_eq!(rows[1].status, Status::Done);
    }

    #[test]
    fn doneはフォーカスするとidleになる() {
        let mut m = model();
        m.apply(3, Report::Stop);
        m.set_panes(vec![pane(5, 1, false), pane(3, 0, true), pane(1, 0, false)]);
        assert_eq!(m.groups()[0].rows[0].status, Status::Idle);
    }

    #[test]
    fn endを受けたペインは一覧から消える() {
        let mut m = model();
        m.apply(1, Report::Working);
        m.apply(3, Report::Working);
        m.apply(1, Report::End);
        assert_eq!(ids(&m), vec![3]);
    }

    #[test]
    fn 閉じたペインは一覧から消える() {
        let mut m = model();
        m.apply(1, Report::Working);
        m.apply(3, Report::Working);
        m.set_panes(vec![pane(3, 0, true)]);
        assert_eq!(ids(&m), vec![3]);
    }
}
