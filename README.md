# flock

A zellij sidebar that shows the state of the coding agents running in each pane. Works with [Claude Code](https://claude.com/claude-code) and Antigravity CLI (`agy`).

![Agent list on the left, Claude Code in the middle, and the focused agent's log on the right](docs/screenshot.png)

- The right column shows the log at the top and the agent list at the bottom. The pane you work in is on the left.
- The agent list shows every tab and every pane in it. Rows are ordered by tab position, then by pane creation order. They don't move when a state changes.
- States: `⠋` working / `●` blocked (Claude Code only) / `✓` done (finished while you weren't looking) / `○` idle / `-` no agent running (a plain shell, for example)
- States come from the agents' hooks through `zellij pipe`. Only panes running an agent get a number.
- Click a row to jump to that pane. Click a tab name to switch to that tab. `＋ new` lets you pick Claude Code or Antigravity and starts it in a new tab in the current pane's folder.
- `Alt+j` / `Alt+k` move to the next / previous agent (only inside a flock session; use `Alt+↓` / `Alt+↑` to move between panes vertically).
- The log shows the focused agent's prompts and actions (tools and the files or commands they touched) with timestamps. Click a row to open the full prompt or the tool's input and output (`q` to close). `Alt+g` toggles it.

## Install

With Homebrew (macOS / Linux):

```sh
brew install hokuto100retsuken/tap/flock
flock setup        # add the hooks to your Claude Code and Antigravity CLI settings
```

Use the full name: plain `brew install flock` installs an unrelated file-locking tool.

Or download the binary for your platform from [Releases](https://github.com/hokuto100retsuken/flock/releases) and put it on your PATH. The sidebar plugin is embedded in the binary, so the only other requirement is [zellij](https://zellij.dev/) 0.45 or later.

```sh
# macOS (Apple Silicon)
curl -L https://github.com/hokuto100retsuken/flock/releases/latest/download/flock-aarch64-apple-darwin.tar.gz | tar xz -C ~/.local/bin
flock setup        # add the hooks to your Claude Code and Antigravity CLI settings
```

To build from source: `cargo install --git https://github.com/hokuto100retsuken/flock flock`. The Rust version and the WASM target are pinned in `rust-toolchain.toml` (rustup installs them automatically).

## Usage

```sh
flock              # start; reattach if running, or reopen the previous tabs with their conversations if it was quit
flock --fresh      # start with a single empty tab instead of reopening the previous tabs
flock new [DIR] [--name NAME] [--agent agy]  # start Claude Code (or Antigravity CLI) in a new tab (DIR defaults to the current folder, NAME to the folder name)
flock focus N      # move to agent N in the sidebar
flock next|prev    # move to the next / previous agent
```

On first launch, and whenever the plugin needs new permissions, the agent list and the log each ask for permission: click each one and press `y`. Reorder tabs with zellij's `Alt+i` / `Alt+o`; the agent list follows.

The previous tabs are reopened however the session ended, whether with Ctrl+q or by closing the last tab, because zellij leaves both the same way. Use `flock --fresh` to start empty.

flock starts zellij with `pane_frame_style full`, so the pane you work in gets a frame and no lines are drawn around the agent list and the log.

### With Antigravity CLI

1. Install Antigravity CLI and run `agy` once, so that `~/.gemini` exists.
2. Run `flock setup`. It adds the hooks to `~/.gemini/config/hooks.json`. It skips Antigravity CLI when `~/.gemini` doesn't exist, so run it again if you installed `agy` later.
3. Start an agent with `＋ new` → `Antigravity` in the sidebar, or with `flock new --agent agy`. An `agy` you start yourself in a pane is listed once you send it the first prompt.
4. After Ctrl+q, `flock` reopens the tab and resumes the conversation with `agy --conversation <ID>`.

Claude Code and Antigravity CLI can run side by side in the same session. See [Hooks](#antigravity-cli) for what Antigravity CLI can't report.

On start, the plugin is written to `~/.local/share/flock/flock.wasm`. The tab layout and log details are kept in `~/.local/state/flock/`.

## Development

```sh
cargo build --release   # target/release/flock; build.rs compiles the plugin to WASM and embeds it
cargo test
```

## Hooks

### Claude Code

`flock setup` adds hooks to `~/.claude/settings.json` that call `flock hook <state>` on each event, using the full path of the binary so it doesn't clash with util-linux's `flock` on Linux (the original is saved as `settings.json.bak`). Outside zellij the hooks do nothing.

| Event | State sent |
|---|---|
| `SessionStart` | idle |
| `UserPromptSubmit` / `PostToolUse` | working |
| `Notification` (`permission_prompt`) | blocked |
| `Stop` | stop (idle if you're watching, done otherwise) |
| `SessionEnd` | end (shown as `-`) |

Resuming after a permission prompt is shown by switching back to working when the next tool call finishes (`PostToolUse`).

### Antigravity CLI

If `~/.gemini` exists, `flock setup` also adds a hook named `flock` to `~/.gemini/config/hooks.json`.

| Event | State sent |
|---|---|
| `SessionStart` | idle |
| `PreInvocation` / `PostToolUse` | working |
| `Stop` | stop |

Antigravity CLI has no hook for permission prompts or for the end of a session, so it never shows as blocked, and it keeps its last state after `/exit` until the pane closes. Its conversations are reopened with `agy --conversation` after Ctrl+q.
