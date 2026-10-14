# Usage guide

`flutter-run-tui` is a small terminal launcher for Flutter projects. It reads
VS Code style `.vscode/launch.json` files, discovers devices in the background,
and starts Flutter in an interactive Ratatui console. It targets macOS and
Linux. Windows support is currently unqualified.

Install it from this checkout with:

```sh
cargo install --path . --locked
```

Start the picker in a project with:

```sh
flutter-run-tui /path/to/project
cargo run -- /path/to/flutter/project
```

The project argument is optional when run inside a project or one of its child
directories. `--check` validates launch configuration and Flutter SDK
resolution without starting Flutter or entering the TUI. Use `--flutter
/path/to/flutter` to select an executable explicitly, or `--fvm` to force the
FVM wrapper. A project with `.fvmrc` or `.fvm/fvm_config.json` uses its FVM SDK
automatically. `--state /path/to/state.json` overrides the favorites file, and
`--refresh-interval 10` changes the background device refresh interval (in
seconds, minimum 2). `--launch path/to/launch.json` selects a launch file
explicitly; relative paths are resolved from the project. `--no-project-helpers`
disables project-specific OWNIT helpers.

On the device screen, `↑`/`↓` or `j`/`k` selects a device and `Enter` opens its
compatible configurations. Press `f` to toggle a favorite; favorites are
saved immediately and cached devices remain selectable while discovery runs.
`r` requests a refresh, `c` reloads `launch.json`, `/` filters, and `l` opens
the last console. Click a row to select it, click its star to toggle favorites,
and use the buttons to proceed. The wheel moves through picker items.
`Esc` goes back or requests exit from the device screen. `Ctrl+q` and the Quit
button open a confirmation: `y` or clicking Quit confirms; `Enter`, `n`, or
`Esc` cancels. Confirming also stops an active session. `F1` or `?` opens help.
Use `--no-mouse` to leave mouse input to your terminal for selection/copying;
many terminals also allow selecting text while holding Shift.
If the terminal disappears, the launcher automatically stops its owned run
and discovery processes. Mouse hovering does not trigger continuous redraws;
updates are limited to 30 frames per second and idle redraws to once per second.

Every page has an **Actions** button (**Menu** in narrow terminals), also opened
with **F4**. It includes all page commands: filtering, favorites, refreshing,
configuration reload, saved logs, Flutter reload/restart/help/DevTools, animation
speed, colors, and exit. Commands remain reachable at the minimum 34 × 12 size.
Use arrows, Tab/Shift+Tab, Home/End or PageUp/PageDown to navigate the menu;
Enter/Space activates, and Esc/F4 closes. The mouse supports clicks, wheel
scrolling, and scrollbar dragging. Tab also opens the menu from a picker or
saved console; live-console Tab continues to go to Flutter.

Click **Filter /** or press `/` to edit the list filter. Type or paste the
query, then click **Apply** or press Enter. **Clear** or Esc clears it. Search
input is bounded to 512 characters. The compact help fits the smallest supported
terminal; even smaller windows retain a clickable exit when space permits.

In the console, Flutter receives normal typing directly. `r`, `R`, `h`, and
`v` are forwarded to Flutter for reload, restart, help, and DevTools. `q` or
`Esc` requests a graceful stop; builds and tests receive an interrupt after
one second if they do not exit. A second press force-stops it, and a stop is
forcefully completed automatically after eight seconds. `PgUp`/`PgDn` scroll a
10,000-line console history and `End` follows current output. The mouse wheel
scrolls the console, and its scrollbar supports clicks and dragging.
Only the latest console is retained, and a new successful run releases the
previous history. Background discovery and console output queues are bounded.

Console ANSI colors (including 256-color and RGB), backgrounds, and emphasis are
preserved in live output and saved history. Plain recognized severity prefixes
receive fallback colors: errors red, warnings amber, info cyan, and debug gray.
This includes Android logcat priorities and `[logger] LEVEL:` / `[LEVEL]` forms.
Lines with supplied ANSI colors keep their original styling; ordinary messages
are not classified from words buried in their text.

**F3**, the color button, or **Actions → Colors** toggles output colors without
restarting. `--color auto` is the default and respects nonempty `NO_COLOR`;
`--color always` explicitly enables colors, while `--color never` starts in
monochrome. New sessions request matching child color output and advertise a
truecolor-capable PTY. Explicit `launch.json` environment values are applied last.
Changing F3 affects rendering immediately; child environment changes apply to
the next session. Home/End and the matching menu commands jump to oldest/latest
output in both the live and saved console.

Press **F2** or click the animation speed button to toggle normal / **5× slower**,
using the same Flutter `timeDilation` control as DevTools. The button displays
the app's confirmed value, including changes made in DevTools and hot-restart
resets. It becomes available once a debug/profile app reports its local VM
Service and registers the extension. Release apps and standard Flutter tests
show it as unavailable; connection failures show disconnected rather than
claiming a speed change succeeded.

## `launch.json`

The launcher accepts JSONC, including comments and trailing commas. Supported
configuration fields are `name`, `type`, `request`, `cwd`, `program`, `args`,
`toolArgs`, `env`, `flutterMode`, `deviceId`, `preLaunchTask`,
`runTestsOnDevice`. The `presentation` field is accepted but its sorting and
grouping options are ignored. `type: "dart"` and
`request: "launch"` are the supported values. `toolArgs` is preferred for
Flutter command options. For compatibility, `args` is also forwarded as
Flutter CLI flags; these are not VS Code or Dart debugger application args.
Test configurations use a `program` such as `test` and run `flutter test`.

The supported variables are `${workspaceFolder}` (also
`${workspaceRoot}`), `${workspaceFolderBasename}`, and `${env:NAME}`.
Configurations containing unsupported variables are skipped with a warning.
Debugger breakpoints are not
implemented. A non-empty `preLaunchTask` blocks that configuration because the
launcher cannot execute the task. `customTool`, `customToolReplacesArgs`,
`vmAdditionalArgs`, and `dartAdditionalArgs` are unsupported and block the
configuration when supplied.

`deviceId` pins a configuration to its matching target, so it is offered only
when that device is selected. Widgetbook configurations commonly pin a nested
`cwd` to `widgetbook` and use the `web-server` target. On OWNIT projects, the
automatic adapter preserves `scripts/check_secrets.sh` for
`--dart-define-from-file` and `scripts/run_mac_designed_for_ipad.sh` for My Mac
(Designed for iPad) on Apple Silicon. Solana configurations are offered only
to Android devices. Helpers are enabled only when both scripts are present.

State is stored as JSON in the platform configuration directory. If the file
is corrupt, the launcher warns and disables persistence so the original file
is preserved until repaired.

For project checks, install the audit tool with `make setup-tools`, then run
`make check`. This enforces formatting, all-target Clippy warnings, regression
tests, and dependency advisories (including warnings). Rust's `rustfmt` and
`clippy` components are required. The lint configuration denies production
`unwrap`/`expect`, explicit panics, undocumented unsafe blocks,
`std::sync::mpsc::channel`, `std::mem::forget`, and direct process exit.
See [development standards](../CONTRIBUTING.md) for the
ownership, error-handling, and review rules. A GitHub Actions workflow applies
the checks on macOS and Linux when the project is hosted there; hosted runs
have not been verified locally.

The optional real Dart VM Service test can also be run with:

```sh
FLUTTER_TUI_TEST_DART=/path/to/dart cargo test --test terminal_flow real_dart_vm -- --ignored
```

The VM test uses an isolated extension fixture; it does not launch your project.

Further reading: [Ratatui](https://ratatui.rs/) and the [Dart launch
configuration documentation](https://dartcode.org/docs/launch-configuration/).
