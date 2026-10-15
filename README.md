# flutter-run-tui

A persistent terminal launcher for Flutter, built with Rust and
[Ratatui](https://ratatui.rs/). Pick a device, choose a VS Code launch
configuration, and run. When the session ends, the picker is ready for the next one.

## Features

- Reads `.vscode/launch.json`, including comments and trailing commas.
- Refreshes devices in the background; cached favorites stay selectable.
- Runs Flutter in an interactive console with its usual keyboard shortcuts.
- Supports mouse selection, wheel scrolling, and a draggable console scrollbar.
- Preserves ANSI colors and highlights plain log severity; toggle colors with **F3**.
- Provides every command through the clickable **Actions** menu or **F4**.
- Runs up to eight devices together in **multi-device mode**, with separate consoles.
- Adds sessions while others keep running, with a device and configuration per addition.
- Toggles normal / **5× slower animations** with **F2**, retaining the speed across hot restarts.
- Confirms launcher exit and cleans up owned processes when the terminal closes.

## Get started

Requires Rust, a Flutter SDK, and an interactive terminal. Project FVM SDKs are
detected automatically. Targets macOS and Linux; Windows is unqualified.

From this checkout:

```sh
cargo install --path . --locked
flutter-run-tui /path/to/flutter/project
```

Run inside a Flutter project to omit its path. Use `--check` to validate the
configuration, or `--help` for all options.

## Controls

| Where | Keys | Action |
| --- | --- | --- |
| Picker | `↑` / `↓`, `Enter` | Select device, then configuration |
| Picker | `f`, `r`, `/`, `l` | Favorite, refresh, filter, last console |
| Devices | `m`, `Space` | Toggle multi-device mode, check/uncheck a device |
| Anywhere | `F5` / `F6`, `F7`, `F8` | Switch consoles, device picker, stop all sessions |
| Anywhere | `F9` | Add a session without stopping existing runs |
| Console | `r`, `R`, `h`, `v` | Flutter reload, restart, help, DevTools |
| Console | `F2` | Toggle animation speed in supported debug/profile apps |
| Anywhere | `F3`, `F4` | Toggle colors, open the Actions menu |
| Console | `PgUp` / `PgDn`, `End` | Scroll history, follow output |
| Console | `q` / `Esc` | Stop session and return to the picker |
| Anywhere | `F1`, `Ctrl+q` | Help, confirm launcher exit |

Mouse controls are enabled by default. Use `--no-mouse` for terminal text
selection. Animation control requires Flutter's VM Service extension.
Use `--color always` to override `NO_COLOR`, or `--color never` for monochrome.
Use `--multi` to start in multi-device mode. Check devices, then choose one
compatible configuration to run on all of them. Click the console arrows or use
F5/F6 to switch; Flutter shortcuts affect only the displayed session.
Press **F9** or choose **Add session** from the Actions menu to add another run.

See the [usage guide](docs/usage.md) for configuration support and limitations,
or start with the [example launch file](examples/launch.json).

## Development

```sh
make setup-tools
make check
```

Checks cover formatting, strict Clippy lints, regression tests, and dependency
advisories. See [contributing](CONTRIBUTING.md) for coding standards.

## License

[MIT](LICENSE) © 2026 Jan Kožnárek.
