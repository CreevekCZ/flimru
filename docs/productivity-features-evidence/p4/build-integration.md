# P4 App integration handoff

Integrated explicit retained-console export into the actual App. Scope is
`src/main.rs`, `src/ui.rs`, the narrow final-shutdown addition in
`src/log_export.rs`, the covered-cursor entry point in `src/export_dialog.rs`,
new `tests/support/log_export_app.rs`, and one actual-binary regression in
`tests/terminal_flow.rs`. No P5 helper/report, P6 prepared flow, Cargo settings,
installed binary or unrelated source was changed. No commits or subdelegation.
`integration.patch` is the scoped textual diff against the coordinator's
`/private/tmp/flimru-productivity-p4tfofri/p3-complete` baseline; it excludes
separately owned work and the already-reviewed standalone implementation.

## Source, ownership and filesystem boundary

- **Export retained console log** is an explicit Actions entry for console and
  saved-log pages, including ended runs. A ready review passes the exact frozen
  `Arc<ConsoleSnapshot>` and its original `SearchSource` metadata. Query/severity
  never restrict exported text. Pending review asks the user to wait; failed
  review retains its actual error/recovery rather than suggesting waiting.
- Review/export share one narrow `capture_console` helper: source identity/time
  and bounded labels are frozen at invocation, selection-owned history is released,
  historical-width/alternate-grid preflight runs before the Screen clone, and the
  clone is moved into one owned ReviewWorker with empty query/All. The capture
  worker is joined/released as soon as its matching result is acknowledged.
- One `PendingExport` holds source identity, workspace, optional dialog/capture
  worker and the writer. Capture and writing are sequential. Tab changes, later
  output, completion and catalog changes never replace its metadata or snapshot.
  Review query/filter/offset and retained console offset are preserved.
- Preparing/opening/editing creates no directories or files. Explicit Submit
  builds the already-reviewed capped request and starts the writer; the dialog
  immediately enters Saving so repeated Enter cannot submit twice. Existing
  destinations remain errors; editing and retrying uses the same captured source.
- Cancel removes the prompt and cancels work. An active writer remains owned and
  polled until its truthful result arrives. If publication won, the App reports
  the absolute saved path and actual byte count. Closing that captured source
  cancels its export; switching tabs does not. CPU-only capture cancellation uses
  the existing worker's cancellation checkpoints and joined shutdown.
- Final App teardown invokes the narrow `ExportWorker::shutdown` API: cancel,
  join, return the actual result. The App retains its operation outcome and, on
  an observable connected terminal, prints it after terminal restoration. Normal
  UI cancellation uses cancel/poll and never waits for writer acknowledgement.

## Event and cursor precedence

Global Ctrl+q and the quit-confirmation overlay precede export input. While the
dialog is visible, every remaining key, paste and mouse event routes to it before
early selection handling, Ctrl+e, normal Ctrl+p/top navigation, review controls or
PTY fallback. Background clicks are consumed. Paste is bounded before allocating
a second event payload, with an oversize sentinel so no truncated path can submit.

`console_ui_owned` now also covers the export dialog. It suppresses covered live
render/cursor, selection and raw-output-only redraw. The parent review renders
unfocused in its existing viewport; the destination editor alone owns the cursor.
Quit overlays suppress that cursor too. Review/query/source state survives closing
the prompt. Complete saved path/bytes and sanitized errors use `report_operation`
and the existing retained **Operation details** path, including after Resume.

## Review and fixes

Self-review kept the parent's review viewport dimensions unchanged while covered,
bounded the App paste bridge before cloning, and closed stale diagnostic state on
modal invocation. Coordinator review identified pending-versus-failed review and
outside-review CaptureLimit copy: one shared `capture_error_message` now preserves
the concrete narrower/new-session recovery in both flows. Existing App tests pin
both distinctions. No broad new capture framework or separate search state was
introduced.

One initial PTY assertion failed because it expected an unwrapped filename in
Operation details. The actual full message correctly wrapped `.txt` onto the next
line. The assertion now accepts the real terminal wrapping; the filesystem and
App retained-outcome checks still verify the complete path and byte count.

## Validation and evidence

```sh
FLIMRU_P4_APP_EVIDENCE_DIR=docs/productivity-features-evidence/p4/integration cargo test --locked --bin flimru log_export -- --nocapture
cargo test --locked --test terminal_flow console_export_real_binary -- --nocapture
cargo test --locked --bin flimru --lib --test console_search_ui --test export_dialog_ui --test log_export
cargo clippy --locked --all-targets -- -D warnings
rustfmt --edition 2024 --check --config skip_children=true src/main.rs src/ui.rs src/log_export.rs src/export_dialog.rs tests/support/log_export_app.rs tests/terminal_flow.rs
```

- Six focused App tests passed. They cover exact Arc/full filtered text and
  metadata; invocation-time capture/tab stability/offset/no pre-submit files and
  duplicate submit; retry and full retained outcome; cancellation/publication
  winner/source close/final shutdown; real-child input/selection/cursor/quit
  isolation; and actual App rendered presentation.
- The real-binary PTY flow passed: launch the owned fake Flutter child, generate
  mixed-severity output, freeze/query/filter, invoke export through actual Actions,
  edit and submit the destination, inspect the saved UTF-8 header/full retained
  text and Operation details, verify no modal input reached the child, and quit.
  The focused successful run took 3.07 seconds.
- Affected regression command: **209 passed**, zero failures; library 57, App 103,
  search UI 19, dialog UI 16, export backend 14. Two existing intentional ignores
  remain (subprocess-only watchdog child and explicit render measurement).
  Timings were 2.01 / 13.88 / 0.25 / 0.16 / 0.84 seconds respectively.
- Strict all-target Clippy and scoped formatting passed. The previously cleared
  standalone matrices were not expanded; their affected checks were rerun for
  the small shutdown/cursor API changes.
- `integration/` retains nine actual App TestBackend scenes (ready, error, review
  at 34×12, 80×24, 120×40), each as TXT/SVG/PNG. All nine PNGs were opened and
  visually inspected. PNGs were rasterized with `rsvg-convert`. Compact source/path
  clipping remains explicit; error controls and the full-snapshot scope remain
  visible. Native cursor ownership is verified by the live-child TestBackend test.

The new App render evidence occupies about 2.1 MiB. Builds used the normal shared
Cargo cache. Test fixtures, children and workers use RAII; no disposable build
tree, server, language server or task-owned temporary directory remains. The
coordinator's baseline under `/private/tmp` was read only and preserved.

Recap: P4 App export is implemented and locally reviewed; affected regressions,
actual-binary PTY, strict checks and nine App renders pass. Source is ready for
the separate simplifier/correctness/UI gates. No installation was performed.
