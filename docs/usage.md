# Usage guide

`flimru` is a small terminal launcher for Flutter projects. It reads
VS Code style `.vscode/launch.json` files, discovers devices in the background,
and starts Flutter in an interactive Ratatui console. It targets macOS and
Linux. Windows support is currently unqualified.

Flimru was previously named `flutter-run-tui`. Favorites and last selections
keep their original configuration location, so the rename preserves them.

Install it from this checkout with:

```sh
cargo install --path . --locked
```

Start the picker in a project with:

```sh
flimru /path/to/project
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
`Esc` cancels. Confirming also stops an active session. `?` opens help in pickers;
the Help button and Actions menu work everywhere.
Use `--no-mouse` to leave mouse input to your terminal for selection/copying;
many terminals also allow selecting text while holding Shift.
If the terminal disappears, the launcher automatically stops its owned run
and discovery processes. Mouse hovering does not trigger continuous redraws;
updates are limited to 30 frames per second and idle redraws to once per second.

Function keys are not used. Shortcuts use the **Control** key, not Command (⌘),
and do not require changing macOS keyboard settings.

Every page has an **Actions** button (**Menu** in narrow terminals), also opened
with **Ctrl+p**. It includes all page commands: filtering, favorites, refreshing,
configuration reload, saved logs, Flutter reload/restart/help/DevTools, animation
speed, colors, and exit. Commands remain reachable at the minimum 34 × 12 size.
Use arrows, Tab/Shift+Tab, Home/End or PageUp/PageDown to navigate the menu;
Enter/Space activates, and Esc/Ctrl+p closes. The mouse supports clicks, wheel
scrolling, and scrollbar dragging.

The top action bar supports direct keyboard focus. **Tab/Shift+Tab** enters it
from a picker or saved console; **Ctrl+g** works on every page. **Left/Right** moves
through every top command and clickable navigation step, **Home/End** jumps to
the first/last target, and **Enter/Space** activates. **Down**, **Esc**, **Tab**,
or **Ctrl+g** returns to content without running a command. Up from the first list
item also focuses the top bar. A `›` marker and contrasting background show
focus, including a visible text cue in monochrome. Keyboard-opened menus and
confirmation dialogs restore their trigger focus when dismissed.

In narrow windows the contextual actions scroll into view as focus moves;
Actions/Menu, Help, and Quit stay visible. Click the overflow chevron or scroll
the wheel over the header to reach hidden actions with a mouse. Pointer commands
return to content so mouse and keyboard input can be mixed naturally. Navigation
and paste are not sent to Flutter while the bar has focus; **Tab inside the
unfocused live console still goes directly to Flutter**. Use Ctrl+g to leave or
re-enter its command bar.

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
In single-device mode only the latest console is retained. Starting the next
batch releases previous histories. Background discovery and console output
queues are bounded.

For multiple devices, press **m** in the device picker or use **Actions →
Multi-device**. `--multi` enables this mode at startup. Check devices with
**Space**, their `[ ]` checkbox, or the menu, then press Enter and choose one
launch configuration. Only configurations compatible with **every** checked
device are offered; an explicit `deviceId` remains a constraint. Tests that do
not need a device cannot be duplicated across several devices. Selection uses
device IDs and survives filtering and discovery, including cached devices.

A batch supports up to **eight** instances. Each has its own PTY, bounded output
queue, 10,000-row history, scroll position, and animation control. All instances
receive output processing and terminal resize updates while hidden. Use
**Ctrl+b/Ctrl+f** or the clickable session tabs to switch; the Actions menu
also lists each console directly. Tabs remain available in the device and
configuration pickers while sessions exist. **●** means running and **○** means
ended. The selected console has a highlighted tab. All eight tabs fit in the
minimum terminal width; labels shorten to their number and state as needed.
The console title shows the full device and configuration names. Use Ctrl+g,
Left/Right and Enter to select tabs from the keyboard.
Reload, restart, DevTools, animation changes, typing and paste affect only the
displayed instance.

**q/Esc** stops the displayed instance; pressing it again forces that instance.
Completed consoles remain available for review. **Ctrl+w**, the tab's **×**, or
**Actions → Close ended session** removes an ended tab and releases its history.
Closing the selected tab reveals its neighbor; closing the last tab returns
to the device picker. Ctrl+w leaves running sessions intact: stop one with
q/Esc first, then close its tab. The saved console from a single-device run
also has a tab and can be closed the same way. **Ctrl+d** returns
to the device picker without stopping the batch. **Ctrl+x** or **Actions → Stop all
sessions** stops every instance; Ctrl+x again forces any remaining ones. Quit
confirmation and terminal-loss cleanup include every running instance, even
from the picker. The default single-device flow still returns to the picker
automatically after the session ends.

**Ctrl+n**, the **Add** button in wide consoles, or **Actions → Add session** opens
a fresh device selection while existing sessions keep running. Choose a device
and configuration, or check several devices in multi-device mode, then Run. New
consoles join the existing ones and the first new console becomes active. Each
addition may use a different launch configuration; selection mode can be changed
without affecting running sessions. Ctrl+d also lets you choose additional runs.
Launching the same configuration on a device where it is already running is
blocked to avoid accidental duplicates; a stopped session can be launched again.

There are at most eight active sessions and eight retained console slots. A
launch that would exceed the active limit is rejected before any child starts.
When adding runs, completed consoles are removed oldest first only if slots
are needed; running consoles are preserved. Once all sessions have ended,
starting a new batch replaces the previous histories. An unsuccessful addition
in single-device mode leaves existing sessions and their history unchanged.

Commands are validated before any instance starts. A startup failure is shown
in that device's console, and other instances can continue. Each addition uses
its selected configuration's arguments and environment; fixed application/server
ports in your configuration must support simultaneous runs.

Console ANSI colors (including 256-color and RGB), backgrounds, and emphasis are
preserved in live output and saved history. Plain recognized severity prefixes
receive fallback colors: errors red, warnings amber, info cyan, and debug gray.
This includes Android logcat priorities and `[logger] LEVEL:` / `[LEVEL]` forms.
Lines with supplied ANSI colors keep their original styling; ordinary messages
are not classified from words buried in their text.

**Ctrl+l**, the color button, or **Actions → Colors** toggles output colors without
restarting. `--color auto` is the default and respects nonempty `NO_COLOR`;
`--color always` explicitly enables colors, while `--color never` starts in
monochrome. New sessions request matching child color output and advertise a
truecolor-capable PTY. Explicit `launch.json` environment values are applied last.
Toggling colors affects rendering immediately; child environment changes apply to
the next session. Home/End and the matching menu commands jump to oldest/latest
output in both the live and saved console.

Press **Ctrl+a** or click the animation speed button to toggle normal / **5× slower**,
using the same Flutter `timeDilation` control as DevTools. The button displays
the app's confirmed value, including changes made in DevTools. Each console
remembers its selected factor and restores it when hot restart replaces the
isolate or reconnects the VM Service. Restart polling is temporarily faster;
delayed extension registration shows **restoring 5×…** and transient errors do
not erase the preference. Choosing normal speed cancels slowdown restoration.
It becomes available once a debug/profile app reports its local VM
Service and registers the extension. Release apps and standard Flutter tests
show it as unavailable; connection failures show disconnected rather than
claiming a speed change succeeded.

Restoration uses Flutter's VM Service extension and happens when that extension
becomes callable. It cannot guarantee slowdown before the first frame or before
`main()` runs. Testing those earliest animations at a guaranteed slow speed
requires a debug-only startup hook in the Flutter app which sets `timeDilation`
before `runApp`. A new session starts by observing that app's current value;
animation choices are not shared across consoles or saved globally.

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
ownership, error-handling, and review rules. All checks run locally; GitHub
Actions is disabled for this repository.

The optional real Dart VM Service test can also be run with:

```sh
FLUTTER_TUI_TEST_DART=/path/to/dart cargo test --test terminal_flow real_dart_vm -- --ignored
```

The VM test uses an isolated extension fixture; it does not launch your project.

Further reading: [Ratatui](https://ratatui.rs/) and the [Dart launch
configuration documentation](https://dartcode.org/docs/launch-configuration/).
