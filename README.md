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

### One-time MCP setup

On macOS or Linux, register Flimru once at the user level for your local agent.
Use the absolute path to the same Flimru binary you use to open the launcher.
To build it from source, run `cargo build --locked --release` for
`target/release/flimru`. Run the command for your provider.

```sh
# Codex
codex mcp add flimru -- /absolute/path/to/flimru mcp

# Claude Code
claude mcp add --scope user --transport stdio flimru -- /absolute/path/to/flimru mcp

# OpenCode V2
opencode mcp add flimru --global -- /absolute/path/to/flimru mcp
```

Open `flimru` in the Flutter checkout and start your local agent in that same
checkout or worktree. The MCP server finds the nearest `.vscode/launch.json`
starting from its working directory and searching upward. If it finds none,
it uses the working directory itself. To pin the MCP server to a fixed checkout,
register it with `mcp --project /absolute/path/to/project`.

In T3 Code, use the executable and configuration for the selected local provider
running as your user on the same machine. Start a fresh session or use
**Restart agent session** after registration. [Codex MCP setup](https://developers.openai.com/codex/mcp), [Claude Code MCP setup](https://code.claude.com/docs/en/mcp), [OpenCode V2 MCP setup](https://opencode.ai/docs/mcp-servers/), [T3 Code’s composer guide](https://github.com/pingdotgg/t3code/blob/main/docs/user/composer.md)

Leave the Flimru window open while using its tools. The first window for a
checkout owns its private local socket and runs started through it continue
after a client disconnects. A second window can still be used interactively.
To switch the owner, close the first window, then reopen the one you want to share.

Check for `flimru_targets`, `flimru_sessions`, `flimru_start`, `flimru_logs`, and
`flimru_control`. The tools can be discovered before the Flimru window opens.
Use exact device IDs and configuration names from `flimru_targets`. If a name
is repeated, use its one-based `config_index`. Save the full ID from
`flimru_start` or `flimru_sessions` for later calls.

`flimru_logs` returns up to 8 KiB by default. Omit `cursor` for recent output,
use `0` for the earliest retained output, then pass the returned `cursor` to
read new output. Read `state` and `outcome` for completion, and check `truncated`
for omitted history. Each console retains up to 1 MiB of plain text in memory
while the console is retained. Session inspection reports which controls are
available. Hot reload and restart acknowledge input delivery. Check subsequent
logs for Flutter's result.

Use this instruction in the Flutter project's `AGENTS.md` or `CLAUDE.md`.

> Use Flimru first for Flutter app runs in this checkout. Check `flimru_sessions`
> and reuse a matching run. For a new run, select the target and configuration
> with `flimru_targets` and launch with `flimru_start`. Use `flimru_logs` and
> `flimru_control` for output, hot reload, restart, and stop.
>
> Use T3 Code's Device panel, `agent-device`, or Mobile MCP to open or watch the
> device and interact with the running app. Use Flimru to launch, relaunch, and
> control the app. If MCP is unavailable, use `flimru session` from this checkout.
> If Flimru is closed, ask me to open it in this checkout or worktree.

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
| Anywhere | `Ctrl+q` | Open quit confirmation |
| Quit confirmation | `Enter` / `Y` | Quit Flimru and stop its sessions |
| Quit confirmation | `Escape` / `N` | Cancel and return to the previous view |

Prompts and review views handle their own keys. Use the controls shown there.

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
