# flock

zellij のサイドバーに、各ペインで動いているコーディングエージェントの状態を一覧表示するプラグイン。

- 並びはタブの位置 → ペインの作成順で固定。状態が変わっても行は動かない
- 状態: `⠋` working / `●` blocked / `✓` done（見ていない間に終わった）/ `○` idle
- 状態は Claude Code のフックから `zellij pipe` で受け取る。状態を報告したペインだけが一覧に出る
- 行をクリックするとそのペインへ移動する。タブ名をクリックすると名前を変えられる。`＋ new` で今のペインと同じフォルダに新しいタブを開き、Claude Code を起動する
- `Alt+j` / `Alt+k` で次 / 前のエージェントへ移動する（flock のセッションの中だけ。上下のペイン移動は `Alt+↓` / `Alt+↑`）
- 右側に、フォーカスしているエージェントの指示と操作（ツールと対象のファイル・コマンド）を時刻付きで出す。行を押すと指示の全文やツールの入力・結果を開く（`q` で閉じる）。`Alt+g` で出し入れする

## インストール

[Releases](https://github.com/hokuto100retsuken/flock/releases) から、使う環境のバイナリを取って PATH の通った場所に置く。サイドバーのプラグインはバイナリに入っているので、ほかに要るのは [zellij](https://zellij.dev/)（0.45 以降）だけ。

```sh
# macOS（Apple Silicon）の例
curl -L https://github.com/hokuto100retsuken/flock/releases/latest/download/flock-aarch64-apple-darwin.tar.gz | tar xz -C ~/.local/bin
flock setup        # Claude Code の設定にフックを足す
```

ソースから入れるときは `cargo install --git https://github.com/hokuto100retsuken/flock flock`。Rust のバージョンと WASM のターゲットは `rust-toolchain.toml` で決めている（rustup が自動で入れる）。

## 起動と操作

```sh
flock              # 起動する。起動中なら繋ぎ直し、Ctrl+q で終了していたら前のタブを会話の続きから開き直す
flock new [DIR] [--name NAME]  # 新しいタブで Claude Code を起動する（DIR の既定は今のフォルダ、NAME の既定はフォルダ名）
flock focus N      # サイドバーの N 番のエージェントへ移動する
flock next|prev    # 次 / 前のエージェントへ移動する
```

初回と権限が増えたときは、サイドバーと右側に権限の確認が出るので、それぞれクリックしてから `y` で許可する。タブの順番は zellij の `Alt+i` / `Alt+o` で変えられ、サイドバーの並びもそれに合わせて変わる。

起動するときにプラグインを `~/.local/share/flock/flock.wasm` に書き出す。タブの並びとログの詳細は `~/.local/state/flock/` に置く。

## 開発

```sh
cargo build --release   # target/release/flock。build.rs がプラグインを WASM にビルドして埋め込む
cargo test
```

## Claude Code のフック

`flock setup` が `~/.claude/settings.json` の `hooks` に、各イベントで `flock hook <状態>` を呼ぶ設定を足す（元の設定は `settings.json.bak` に残す）。zellij の外で動いているときは何もしない。

| イベント | 送る状態 |
|---|---|
| `SessionStart` | idle |
| `UserPromptSubmit` / `PostToolUse` | working |
| `Notification`（`permission_prompt`） | blocked |
| `Stop` | stop（見ていれば idle、見ていなければ done） |
| `SessionEnd` | end（一覧から消す） |

承認待ちのあと再開したことは、次のツール実行が終わった時点（`PostToolUse`）で working に戻して表す。
