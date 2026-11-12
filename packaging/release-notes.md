# Flimru 0.1.0

First release of Flimru, the Flutter Interactive Multi-device Runner.

- Pick a device and a VS Code launch configuration; keep the launcher open between runs.
- Discover devices in the background and keep favorites ready to select.
- Run up to eight sessions with independent consoles and clickable tabs.
- Use keyboard or mouse controls, colored output, scrollback, and a draggable scrollbar.
- Slow animations by 5× and restore the setting after hot restart in supported debug/profile apps.
- Close ended tabs, add sessions during a run, and confirm before quitting.

## Installation (after publication)

The first binary package is for Apple Silicon Macs. Rust is not required to install it.
A Flutter SDK or project FVM SDK and `.vscode/launch.json` are required to run apps.

```sh
brew install CreevekCZ/tap/flimru
flimru /path/to/flutter/project
```

Intel Macs and Linux can build from source; prebuilt packages for those platforms
are not included in this release. Windows is unqualified.

## Limits

Animation control requires Flutter's VM Service extension. Flimru restores slowdown
after hot restart, but a TUI-only solution cannot guarantee it applies before the
app's first animation frame. The initial binary is tested locally on the release
builder's macOS version; the minimum deployment target is not a claim of runtime
testing on every older macOS version.
