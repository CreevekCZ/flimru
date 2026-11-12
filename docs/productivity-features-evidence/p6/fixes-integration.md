# P6 integration fixes

## Confirmed causes and changes

- The original cross-feature PTY run reached every catalog reload, frozen search/filter/export, batch hot-command, original-PID and unavailable-configuration explanation assertion. Final quit failed because `App::key` let diagnostics match Ctrl+q as ordinary `q` before global quit handling. Its pointer branch also consumed clicks intended for the visible global confirmation overlay.
- `App::key` now processes the existing global quit confirmation and Ctrl+q immediately after the inert function-key guard, before report/export/diagnostics/selection owners. Existing y/Y confirms and Enter/Esc/n/N cancels semantics remain intact. Removed the duplicate modal-specific Ctrl+q paths and downstream global guards.
- `App::mouse` now admits only actual ConfirmQuit/CancelQuit hit targets while confirmation is open and consumes all other pointer input before underlying owners. It does not expose hidden diagnostic-parent toolbar controls. The P5 tiny-layout draw guard remains intact.
- A targeted follow-up reproduced a second cause: after Search console → Resume live → Operation details, the visible supported-size header Quit was consumed by the body owner. Admit only Quit outside `ui::review_area` at supported sizes; other header/background controls stay consumed. The failing pre-fix regression is retained in `operation-details-quit-before-fix.log`.
- The cross-feature PTY test quits directly from the open explanation rather than sending an adjacent bare Esc before Ctrl+q, eliminating the escape-sequence ambiguity while explicitly asserting the repaired global behavior.

## Validation

- `cargo test --locked --bin flimru productivity_workflow_app_tests -- --nocapture`: **4 passed** (`quit-regressions.log`). Actual-App coverage includes diagnostic normal/minimum/tiny layouts, y/Y and every cancel key, inert function keys, pointer confirm/cancel, consumed background clicks/wheel, preserved diagnostic scroll/source/render, preserved keyboard selection, and visible Quit from operation details after Resume at 80×24 and 120×40.
- `cargo test --locked --bin flimru -- --nocapture`: **119 passed** (`app-input-regressions.log`), including existing console review/export/batch modal and input regressions.
- Required exact cross-feature `terminal_flow` PTY regression: **1 passed**, both original child processes reaped (`cross-flow-runtime.log`). No arbitrary sleep or weakened final assertions.
- `cargo clippy --locked --all-targets -- -D warnings`: passed (`quit-clippy.log`).
- `cargo fmt --all -- --check`: passed (`quit-format.log`).
- Retained actual-App buffers plus SVG/PNG renders at 80×24, 34×12 and 30×10 in `quit-renders/`; all three visually inspected. Confirm/cancel controls remain visible above diagnostics, including the tiny layout.

## Stable handoff

Changes are limited to quit routing in `src/main.rs`, registration of `tests/support/productivity_workflow_app.rs`, that new regression/evidence module, the PTY test final quit step, and P6 evidence. No commit, install, publish, shared-cache removal or unrelated source edit. No task-owned disposable directory/background process was created; fixture temporaries clean up via RAII, and PTY child reaping is asserted. Retained render evidence is approximately 344 KiB. Reusable project `target/` and the coordinator's existing scratch directory were preserved.

Ready for the coordinator's separate simplification, full checks and fresh final review.

## Fresh final-review fixes (F1 and F2)

The final reviewer supplied two reproducible findings in `final-astra-selection-probe.log` and `final-astra-details-navigation-probe.log`. Both were read and addressed before release rebuilding.

### F1: inherited historical-width selection panic

The exact 4×40 parser / twenty lines / resize to 4×80 / oldest history / click column 60 reproduction exposed a retained 40-cell row at an 80-column viewport. `point`, keyboard movement and clipboard slicing assumed every historical row had the current width.

- Missing retained tail cells now behave as blank when positioning, and clipboard iteration clips to actual retained cells. Capture/scroll continue to cache only real cells, avoiding invented padding between original soft-wrapped rows.
- Wrap detection checks whether the captured width includes the entire original row. Old 40-column wraps still join after growth to 80; an 80-column row clipped by a shrink to 40 no longer falsely joins across its missing right half.
- Keyboard movement converts the absolute history endpoint back to a viewport row before indexing the cached rows, preserving its history anchor after scrolling.
- Existing cell/copy limits are unchanged. Three new regressions cover the exact growth panic, absent-tail mouse/keyboard extraction before and after history scrolling, reverse/extreme scrolling, non-history capture of the same retained rows, shrink clipping, real old soft wraps, wide glyphs and combining marks. All **14 selection tests passed** (`selection-resize-regressions.log`).

### F2: deliberate Actions navigation trapped by operation details

The actual Ctrl+p → Device picker reproduction routed the advertised action back through `App::key`, where operation details consumed its synthetic Ctrl+d. Add session / previous / next used the same route; direct switching/closing could also leave a details-only outcome covering the destination.

- Explicit Devices/Add/Previous/Next actions now call navigation directly; ordinary shortcut navigation keeps its existing path and remains owned by open operation details.
- A narrow `SearchView::dismiss_operation_details` API dismisses only that presentation. The dispatch path calls it only for deliberate page/tab/picker navigation and commands. It leaves source details, frozen snapshot Arc, query, severity filter, current match and reading offsets intact on the same source. Existing page departure / actual source replacement / source closure clears review normally.
- Actions opening/selection/dismissal, Help and Quit retain the operation-details context. No global modal dismissal or ordinary-key leakage was introduced.
- Actual-App Actions regressions exercise details-only and details-over-review navigation to Devices/Add/previous/next/switch/close, direct nonactive tab closure and same-source switching, frozen-state preservation, ordinary Ctrl navigation isolation, and menu/Help/Quit cancellation. Picker regressions exercise ToggleMulti / ToggleFavorite / NextOrRun / Refresh on Devices and ToggleUnavailable / Explain / Back on Configs; Refresh asserts receipt of a new discovery scan. These fixtures retain ended consoles and start no PTY children.
- A direct SearchView regression checks exact snapshot identity/query/filter/current-match/offset preservation and proves the API leaves source details alone.

### Final affected validation and handoff

- All actual-App/input tests: **126 passed**.
- Console search UI: **21 passed, 1 pre-existing performance measurement ignored**.
- Combined evidence: `astra-fixes-app-search-regressions.log`.
- All **8** focused productivity App regressions passed (`quit-and-navigation-regressions.log`).
- Required exact productivity PTY cross-flow: **1 passed**, including original child reaping (`cross-flow-runtime.log`).
- Strict all-target Clippy and formatting passed (`astra-fixes-clippy.log`, `astra-fixes-format.log`).
- Reviewed the final selection indexing/extraction and explicit-navigation paths after tests. Expanded source ownership was limited to `src/selection.rs`, the small SearchView API, `src/main.rs`, and precise regression additions in the existing P6 support file / `tests/console_search_ui.rs`. No UI module edit was needed. Existing quit renders remain valid because these fixes do not change quit drawing.
- No new disposable directory, long-lived process or PTY fixture was retained; no install/commit/publish/cache cleanup was performed. Source patch is stable for coordinator full checks/release rebuilding and the reviewer's recheck.
