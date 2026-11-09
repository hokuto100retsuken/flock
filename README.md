# flock

zellij のサイドバーに、各ペインで動いているコーディングエージェントの状態を一覧表示するプラグイン。

- 並びはタブの位置 → ペインの作成順で固定。状態が変わっても行は動かない
- 状態: `⠋` working / `●` blocked / `✓` done（見ていない間に終わった）/ `○` idle
- 状態は Claude Code のフックから `zellij pipe` で受け取る。状態を報告したペインだけが一覧に出る

## ビルド

Rust のバージョンと WASM のターゲットは `rust-toolchain.toml` で決めている（rustup が自動で入れる）。

```sh
cargo build --release   # target/wasm32-wasip1/release/flock.wasm
cargo test --target aarch64-apple-darwin
```

## 起動

```sh
zellij --layout layouts/flock.kdl
```

初回はサイドバーに権限の確認が出るので `y` で許可する。

## Claude Code のフック

フックの設定は `hooks/claude-settings.json` にある。zellij の外で動いているときは何もしない。

- 試すとき: `claude --settings ~/src/github.com/hokuto100retsuken/flock/hooks/claude-settings.json`
- 常に使うとき: 中身を `~/.claude/settings.json` の `hooks` に足す

承認待ちのあと再開したことは、次のツール実行が終わった時点（`PostToolUse`）で working に戻して表す。
