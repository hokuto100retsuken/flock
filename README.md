# flock

zellij のサイドバーに、各ペインで動いているコーディングエージェントの状態を一覧表示するプラグイン。

- 並びはタブの位置 → ペインの作成順で固定。状態が変わっても行は動かない
- 状態: `⠋` working / `●` blocked / `✓` done（見ていない間に終わった）/ `○` idle
- 状態は Claude Code のフックから `zellij pipe` で受け取る。状態を報告したペインだけが一覧に出る
- 行をクリックするとそのペインへ移動する。`＋ new` で今のペインと同じフォルダに新しいタブを開き、Claude Code を起動する
- `Alt+j` / `Alt+k` で次 / 前のエージェントへ移動する（flock のセッションの中だけ。上下のペイン移動は `Alt+↓` / `Alt+↑`）

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
flock new [DIR]    # 新しいタブで Claude Code を起動する（DIR の既定は今のフォルダ）
flock focus N      # サイドバーの N 番のエージェントへ移動する
flock next|prev    # 次 / 前のエージェントへ移動する
```

初回と権限が増えたときは、サイドバーに権限の確認が出るのでクリックしてから `y` で許可する。

## Claude Code のフック

フックの設定は `hooks/claude-settings.json` にある。zellij の外で動いているときは何もしない。

- 試すとき: `claude --settings ~/src/github.com/hokuto100retsuken/flock/hooks/claude-settings.json`
- 常に使うとき: 中身を `~/.claude/settings.json` の `hooks` に足す

承認待ちのあと再開したことは、次のツール実行が終わった時点（`PostToolUse`）で working に戻して表す。
