# flock

zellij のサイドバーに、各ペインで動いているコーディングエージェントの状態を一覧表示するプラグイン。

- 並びはタブの位置 → ペインの作成順で固定。状態が変わっても行は動かない
- 状態: `⠋` working / `●` blocked / `✓` done（見ていない間に終わった）/ `○` idle
- 状態は Claude Code のフックから `zellij pipe` で受け取る。状態を報告したペインだけが一覧に出る
- 行をクリックするとそのペインへ移動する。タブ名をクリックすると名前を変えられる。`＋ new` で今のペインと同じフォルダに新しいタブを開き、Claude Code を起動する
- `Alt+j` / `Alt+k` で次 / 前のエージェントへ移動する（flock のセッションの中だけ。上下のペイン移動は `Alt+↓` / `Alt+↑`）
- 右側に、フォーカスしているエージェントの指示と操作（ツールと対象のファイル・コマンド）を時刻付きで出す。行を押すと指示の全文やツールの入力・結果を開く（`q` で閉じる）。`Alt+g` で出し入れする

## ビルド

Rust のバージョンと WASM のターゲットは `rust-toolchain.toml` で決めている（rustup が自動で入れる）。

```sh
cargo build --release   # target/wasm32-wasip1/release/flock.wasm
cargo test --target aarch64-apple-darwin
```

## 起動と操作

`bin/flock` を PATH の通った場所にリンクして使う。

```sh
flock              # 起動する。起動中なら繋ぎ直す
flock new [DIR] [--name NAME]  # 新しいタブで Claude Code を起動する（DIR の既定は今のフォルダ、NAME の既定はフォルダ名）
flock focus N      # サイドバーの N 番のエージェントへ移動する
flock next|prev    # 次 / 前のエージェントへ移動する
```

初回と権限が増えたときは、サイドバーに権限の確認が出るのでクリックしてから `y` で許可する。

## Claude Code のフック

`~/.claude/settings.json` の `hooks` に、各イベントで `hooks/flock-state` を呼ぶ設定を入れる（dotfiles の `claude/settings.json` に入れてある）。zellij の外で動いているときは何もしない。

| イベント | 送る状態 |
|---|---|
| `SessionStart` | idle |
| `UserPromptSubmit` / `PostToolUse` | working |
| `Notification`（`permission_prompt`） | blocked |
| `Stop` | stop（見ていれば idle、見ていなければ done） |
| `SessionEnd` | end（一覧から消す） |

承認待ちのあと再開したことは、次のツール実行が終わった時点（`PostToolUse`）で working に戻して表す。
