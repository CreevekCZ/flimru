# Flimru

**Flutter Interactive Multi-device Runner** — a persistent terminal launcher
for Flutter, built with Rust and
[Ratatui](https://ratatui.rs/). Pick a device, choose a VS Code launch
configuration, and run. When the session ends, the picker is ready for the next one.

## Features

- Reads `.vscode/launch.json`, including comments and trailing commas, and reloads changes automatically.
- Explains unavailable launch configurations and keeps the last good configuration on file errors.
- Refreshes devices in the background; cached favorites stay selectable.
- Runs Flutter in an interactive console with its usual keyboard shortcuts.
- Shares the open Flimru window's sessions with local MCP clients and shell commands.
- Supports drag-to-copy text selection, wheel scrolling, and a draggable console scrollbar.
- Preserves ANSI colors and highlights plain log severity; colors are always enabled.
- Searches and filters a frozen console snapshot and exports retained logs as text.
- Requests hot reload or restart across eligible debug sessions from Actions.
- Provides every command through the clickable **Actions** menu or **Ctrl+p**.
- Navigates the top actions with **Tab/Ctrl+g**, arrow keys, and **Enter/Space**.
- Runs up to eight devices together in **multi-device mode**, with separate consoles.
- Adds sessions while others keep running, with a device and configuration per addition.
- Switches consoles with clickable session tabs; closes ended tabs with **Ctrl+w** or **×**.
- Toggles normal / **5× slower animations** with **Ctrl+a**, retaining the speed across hot restarts.
- Captures each run's VM Service address and offers **Reconnect** to recover animation control.
- Confirms launcher exit and cleans up owned processes when the terminal closes.

## Get started

Requires a Flutter SDK and an interactive terminal. Project FVM SDKs are detected
automatically. Targets macOS and Linux; Windows is unqualified.

Homebrew packaging is **in preparation** for Apple Silicon Macs. Once the release
is published, installation will be:

```sh
brew install CreevekCZ/tap/flimru
flimru /path/to/flutter/project
```

The Homebrew package will not require Rust. Until publication, build from this
checkout with Rust installed:

```sh
cargo install --path . --locked
flimru /path/to/flutter/project
```

Run inside a Flutter project to omit its path. Use `--check` to validate the
configuration, or `--help` for all options.

## Share sessions with an agent

On macOS or Linux, open `flimru /absolute/path/to/project` in a terminal and
leave it running. The window owns every Flutter process started through it,
including agent launches. A client can disconnect while Flutter keeps running.
The first window for a checkout owns its private local socket. A second window
can still be used interactively. To switch the owner, close and reopen the window
you want to share after closing the first one.

### MCP in T3 Code

Build with `cargo build --locked --release` to get `target/release/flimru`.
Use the same build for the launcher and MCP server. In T3 Code, select a local
provider running as your user on the machine with Flimru. Register the following
stdio command in that provider's configuration. Use the exact checkout open in
Flimru, including its worktree path if applicable.

```sh
/absolute/path/to/flimru mcp --project /absolute/path/to/project
```

- For an installed Codex provider, run `codex mcp add flimru -- /absolute/path/to/flimru mcp --project /absolute/path/to/project` using that provider's Codex executable and config home. You can also put a `[mcp_servers.flimru]` table with `command` and `args` in a trusted project config. [T3 Code’s Codex setup](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-codex.md), [Codex MCP configuration](https://developers.openai.com/codex/mcp)
- For a Claude provider, run `claude mcp add --scope local --transport stdio flimru -- /absolute/path/to/flimru mcp --project /absolute/path/to/project` from the checkout using that provider's Claude executable and config directory. [T3 Code’s Claude setup](https://github.com/pingdotgg/t3code/blob/main/docs/user/providers-claude.md), [Claude Code MCP configuration](https://code.claude.com/docs/en/mcp)

Start a fresh T3 agent session or use **Restart agent session**. Check for
`flimru_targets`, `flimru_sessions`, `flimru_start`, `flimru_logs`, and
`flimru_control`. The tools can be discovered before the Flimru window opens.
Use exact device IDs and configuration names from `flimru_targets`. If a name
is repeated, use its one-based `config_index`. Save the full ID from
`flimru_start` or `flimru_sessions` for later calls. [T3 Code’s composer guide](https://github.com/pingdotgg/t3code/blob/main/docs/user/composer.md)

`flimru_logs` returns up to 8 KiB by default. Omit `cursor` for recent output,
use `0` for the earliest retained output, then pass the returned `cursor` to
read new output. Read `state` and `outcome` for completion, and check `truncated`
for omitted history. Each console retains up to 1 MiB of plain text in memory
while the console is retained. Session inspection reports which controls are
available. Hot reload and restart acknowledge input delivery. Check subsequent
logs for Flutter's result.

Example instruction for a Flutter project's `AGENTS.md` or `CLAUDE.md`.

> Use Flimru MCP for Flutter runs in this checkout. Check `flimru_sessions`
> for an existing run first. Use `flimru_targets` and `flimru_start` when a new
> run is needed. Read `flimru_logs` with the previous cursor and use
> `flimru_control` for reload, restart, and explicit stop. Keep runs active for
> the user unless asked to stop. If Flimru is closed, ask me to open it in this
> checkout. Use `flimru session` as the shell equivalent when MCP is unavailable.

### Shell commands

Run these commands from the Flutter checkout with Flimru open.

```sh
flimru session targets
flimru session list
flimru session start --device DEVICE_ID --config 'Debug launch name'
flimru session inspect SESSION_ID
flimru session logs SESSION_ID
flimru session logs SESSION_ID --follow
flimru session reload SESSION_ID
flimru session restart SESSION_ID
flimru session control SESSION_ID help
flimru session stop SESSION_ID
```

Use `--project /absolute/path/to/project` to select a checkout explicitly.
`Ctrl+c` ends a shell log follower. The run stays active in Flimru.

## Controls

| Where | Keys | Action |
| --- | --- | --- |
| Picker | `↑` / `↓`, `Enter` | Select device, then configuration |
| Picker | `/`, `l`, `c` | Filter, last console, reload launch configurations |
| Devices | `f`, `r`, `d` | Favorite, refresh devices, configuration diagnostics |
| Configurations | `r`, `u`, `i` | Reload configurations, show unavailable, explain selection |
| Devices | `m`, `Space` | Toggle multi-device mode, check/uncheck a device |
| Anywhere | `Ctrl+b` / `Ctrl+f`, `Ctrl+d`, `Ctrl+x` | Switch consoles, device picker, stop all sessions |
| Anywhere | `Ctrl+n` | Add a session without stopping existing runs |
| Console | `r`, `R`, `h`, `v` | Flutter reload, restart, help, DevTools |
| Console | `Ctrl+a` | Toggle animation speed in supported debug/profile apps |
| Console | `Ctrl+s` | Search a frozen console snapshot |
| Snapshot reading | `/`, `n` / `N`, `f`, `r`, `i` | Edit query, navigate matches, filter severity, refresh, show details |
| Ended console | `Ctrl+w` / tab `×` | Close the session tab and release its history |
| Anywhere | `Ctrl+p` | Open the Actions menu |
| Picker / saved console | `Tab`, `Shift+Tab` | Focus the top actions |
| Anywhere | `Ctrl+g` | Focus the top actions, including live consoles |
| Top actions | `←` / `→`, `Enter` / `Space` | Choose and activate an action |
| Top actions | `↓`, `Esc`, `Tab` | Return to the list or console |
| Console | `PgUp` / `PgDn`, `End` | Scroll history, follow output |
| Console / saved output | Mouse drag | Select text; releasing automatically copies it |
| Console / saved output | `Ctrl+e`, arrows, `Shift+arrows`, `Enter` | Position, select, copy text (`Esc` clears) |
| Console | `q` / `Esc` | Stop session and return to the picker |
| Picker | `?` | Open help (also available from Help or Actions everywhere) |
| Anywhere | `Ctrl+q` | Confirm launcher exit |

Prompts and review views handle their own keys; use the controls shown there.
`Ctrl+q` opens launcher exit confirmation from any view.

Function keys are not used. `Ctrl` means Control, not Command (⌘).
Mouse controls are enabled by default. Use `--no-mouse` for terminal text
selection. Animation control requires Flutter's VM Service extension.
Console colors are always enabled, including when the shell sets `NO_COLOR`.
Use `--multi` to start in multi-device mode. Check devices, then choose one
compatible configuration to run on all of them. Click a session tab or use
Ctrl+b/Ctrl+f to switch; Flutter shortcuts affect only the displayed session.
Press **Ctrl+n** or choose **Add session** from the Actions menu to add another run.
Actions also offers explicit reload/restart requests for eligible sessions and
export of retained console logs.

See the [usage guide](docs/usage.md) for configuration support and limitations,
or start with the [example launch file](examples/launch.json).

## Development

```sh
make setup-tools
make check
```

Checks run locally and cover formatting, strict Clippy lints, regression tests,
and dependency advisories. GitHub Actions is disabled for this repository.
See [contributing](CONTRIBUTING.md) for coding standards and the
[release guide](docs/releasing.md) for local packaging and the Homebrew handoff.

## License

[MIT](LICENSE) © 2026 Jan Kožnárek.
