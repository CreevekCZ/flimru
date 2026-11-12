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
