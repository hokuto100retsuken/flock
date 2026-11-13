/// 新しいタブの名前にする、会話のタイトルの文字数
const TAB_NAME_CHARS: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Agy,
}

/// 再開できる過去の会話1件。flock history の1行
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conversation {
    pub agent: Agent,
    pub id: String,
    /// 最後に更新した日時（MM/DD HH:MM）
    pub updated: String,
    pub cwd: String,
    pub title: String,
}

impl Conversation {
    /// 一覧に出すフォルダ名。パスの最後の部分
    pub fn folder(&self) -> &str {
        self.cwd.rsplit('/').find(|s| !s.is_empty()).unwrap_or(&self.cwd)
    }

    /// 新しいタブの名前。タイトルがなければフォルダ名
    pub fn tab_name(&self) -> String {
        match self.title.trim() {
            "" => self.folder().to_owned(),
            title => title.chars().take(TAB_NAME_CHARS).collect(),
        }
    }

    /// 会話を再開するコマンド。復元のときに起動コマンドから会話の ID を読むので、ID は ' で囲む
    pub fn command(&self, claude_command: &str) -> String {
        match self.agent {
            Agent::Claude => format!("exec {claude_command} --resume '{}'", self.id),
            Agent::Agy => format!("exec agy --conversation '{}'", self.id),
        }
    }
}

/// flock history の出力を読む。1行に1件「エージェント タブ ID タブ 日時 タブ フォルダ タブ タイトル」で、読めない行は飛ばす
pub fn parse(stdout: &str) -> Vec<Conversation> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(5, '\t');
            let agent = match fields.next()? {
                "claude" => Agent::Claude,
                "agy" => Agent::Agy,
                _ => return None,
            };
            let (id, updated, cwd, title) = (fields.next()?, fields.next()?, fields.next()?, fields.next()?);
            // ID は ' で囲んでシェルに渡すので、' を含むものは再開できない
            if id.is_empty() || id.contains('\'') || cwd.is_empty() {
                return None;
            }
            Some(Conversation {
                agent,
                id: id.to_owned(),
                updated: updated.to_owned(),
                cwd: cwd.to_owned(),
                title: title.to_owned(),
            })
        })
        .collect()
}

/// 入力した文字で絞り込み、上下で選ぶ
#[derive(Default)]
pub struct Picker {
    conversations: Vec<Conversation>,
    query: String,
    /// 絞り込んだ一覧の中での位置
    selected: usize,
}

impl Picker {
    pub fn new(conversations: Vec<Conversation>) -> Picker {
        Picker {
            conversations,
            ..Picker::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.conversations.is_empty()
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    // 絞り込むと前の位置は別の会話を指すので、先頭に戻す
    pub fn push(&mut self, c: char) {
        self.query.push(c);
        self.selected = 0;
    }

    pub fn pop(&mut self) {
        self.query.pop();
        self.selected = 0;
    }

    /// 端で止まる
    pub fn move_by(&mut self, step: isize) {
        let last = self.filtered().len().saturating_sub(1) as isize;
        self.selected = (self.selected as isize + step).clamp(0, last) as usize;
    }

    /// タイトルかフォルダ名に、大文字小文字を区別せず入力した文字を含むもの
    pub fn filtered(&self) -> Vec<&Conversation> {
        let query = self.query.to_lowercase();
        self.conversations
            .iter()
            .filter(|c| {
                c.title.to_lowercase().contains(&query) || c.folder().to_lowercase().contains(&query)
            })
            .collect()
    }

    pub fn selected_entry(&self) -> Option<&Conversation> {
        self.filtered().get(self.selected).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picker() -> Picker {
        Picker::new(parse(
            "claude\tc1\t11/13 10:00\t/Users/me/src/flock\tログイン画面を直す\n\
             agy\ta1\t11/12 09:00\t/Users/me/src/Shop\t在庫の集計\n\
             claude\tc2\t11/11 08:00\t/Users/me/src/blog\tREADME を書く\n",
        ))
    }

    fn ids(p: &Picker) -> Vec<&str> {
        p.filtered().iter().map(|c| c.id.as_str()).collect()
    }

    #[test]
    fn 読めない行は飛ばす() {
        let conversations = parse(
            "claude\tc1\t11/13 10:00\t/Users/me/src/flock\tタブ\tを含むタイトル\n\
             codex\tx1\t11/13 10:00\t/tmp\t知らないエージェント\n\
             agy\ta1\t11/13 10:00\t/tmp\n\
             claude\t\t11/13 10:00\t/tmp\tID なし\n\
             claude\tc'2\t11/13 10:00\t/tmp\t' を含む ID\n",
        );
        assert_eq!(
            conversations,
            vec![Conversation {
                agent: Agent::Claude,
                id: "c1".into(),
                updated: "11/13 10:00".into(),
                cwd: "/Users/me/src/flock".into(),
                title: "タブ\tを含むタイトル".into(),
            }]
        );
    }

    #[test]
    fn タイトルとフォルダ名を大文字小文字を区別せず絞り込む() {
        let mut p = picker();
        assert_eq!(ids(&p), vec!["c1", "a1", "c2"]);
        p.push('s');
        p.push('H');
        assert_eq!(ids(&p), vec!["a1"]);
        p.pop();
        p.pop();
        // フォルダの途中（/Users など）には当てない
        p.push('u');
        p.push('s');
        assert!(ids(&p).is_empty());
    }

    #[test]
    fn 選択は端で止まり_絞り込むと先頭に戻る() {
        let mut p = picker();
        p.move_by(-1);
        assert_eq!(p.selected(), 0);
        p.move_by(5);
        assert_eq!(p.selected_entry().unwrap().id, "c2");
        p.push('集');
        assert_eq!(p.selected_entry().unwrap().id, "a1");
        p.push('x');
        assert_eq!(p.selected_entry(), None);
        p.move_by(1);
        assert_eq!(p.selected(), 0);
    }

    #[test]
    fn 再開コマンドから_ctrl_qの後の復元で会話を読み戻せる() {
        let p = picker();
        let all = p.filtered();
        // zellij はペインの起動コマンドを sh -c 付きで返す
        let restored = |c: &Conversation| crate::model::resumed_session(&format!("sh -c {}", c.command("claude --model opus")));
        assert_eq!(restored(all[0]).as_deref(), Some("c1"));
        assert_eq!(restored(all[1]).as_deref(), Some("agy:a1"));
    }

    #[test]
    fn タブの名前は文字単位で切り詰め_タイトルがなければフォルダ名() {
        let mut c = picker().filtered()[0].clone();
        c.title = "あ".repeat(30);
        assert_eq!(c.tab_name(), "あ".repeat(TAB_NAME_CHARS));
        c.title = " ".into();
        assert_eq!(c.tab_name(), "flock");
    }
}
