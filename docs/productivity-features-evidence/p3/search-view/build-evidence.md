# P3 standalone search view build evidence

This is build-stage evidence for `src/console_search.rs`, independently of shared
`main.rs` / `ui.rs` integration. No global shortcut, session identity wiring, PTY
routing change, or release/install action is included here. Independent phase
simplification, correctness and rendered integration review remain required.

## Integration contract

- `SearchView::new(classify: fn(&str) -> Option<Severity>)` owns one review worker.
- `capture(SearchSource, moved_screen)` accepts a clone only after the caller has
  used the historical-width-aware snapshot preflight. It releases previous UI
  review data before handing the new screen to its worker.
- `SearchSource` freezes the stable source ID, bounded display label, optional
  device/configuration/outcome labels, and capture time. Callers supply no launch
  arguments or environment values. Source labels and metadata are sanitized and
  bounded when captured.
- `refresh(SearchSource, moved_screen)` requires the same stable source ID and
  preserves query/filter/editing state while accepting updated capture-time
  metadata. A different source passed to `capture` starts a fresh query/filter.
- `show_error(source, message)` exposes preflight failure without cloning a screen.
- `poll()` accepts only the expected source/capture/query ticket. Query results do
  not copy the immutable snapshot. `snapshot()` provides an Arc for exporting the
  entire captured text, including rows hidden by severity filters.
- Global quit, menus, overlays and explicit toolbar/tab actions route first. All
  remaining events must pass through `handle_event` before live selection, copy
  or PTY input handlers. Every event is consumed while review is active. Effects
  explicitly request refresh or resume-live; no event is forwarded to Flutter.
- Enter stops editing. Escape stops editing first, then closes review. Reading
  mode uses n/N, f, /, r, arrows, page keys and Home/End. Mouse drags/clicks/paste
  cannot invoke live selection or copy through this module. Selection/copy is not
  advertised inside this view.
- `render(&mut self, frame, rect)` can overlay ordinary immutable app drawing.
  Integration should supply the region below the toolbar/tabs, including the
  ordinary status/footer region, so a 34×9 review area remains available.
- Historical wrapping/width is preserved; no reflow is claimed. Left/Right pan,
  match reveal and resize bounds use captured cell coordinates. Closing releases
  UI data before cancelling worker state; dropping the view joins its worker.

## Checks

- `cargo test --locked --test console_search_ui -- --include-ignored --nocapture`:
  12 passed, including the explicit viewport measurement.
- `cargo clippy --locked --test console_search_ui -- -D warnings`: passed.
- `rustfmt --edition 2024 --check src/console_search.rs tests/console_search_ui.rs`:
  passed.
- The ignored debug measurement captured 1,200,000 ASCII bytes in 60,000-column
  physical rows and rendered ten 120×40 frames in 20.08 ms total. This measures
  viewport rendering for that fixture, not full capture/search or every workload.

Tests cover input and paste isolation, bounded Unicode queries, long query editing,
source replacement and close, frozen export metadata/scope, filter states, match
wrapping, vertical navigation, historical horizontal pan and resize, lowercase
expansion, combining-mark-only matches, partial-wide-cell clipping, both halves of
wide-cell highlights and tiny rectangles.

## Rendered fixtures

Set `FLIMRU_P3_EVIDENCE_DIR` when running the render-evidence test to regenerate
actual completed TestBackend frame cells as TXT/SVG. PNGs were rendered from SVG
using `rsvg-convert`. The 36 fixture sets cover nine states at 34×12, 80×24 and
120×40, plus each state in the 34×9 review area beneath toolbar/tab placeholders.
The 34×9 fixtures retain only three log rows and still show query, result/filter,
refresh and resume controls.

Builder visual inspection included compact reading/editing/error/pending/filter,
wide-history and capped-result PNGs, 80×24 no-matches and 120×40 reading. The compact
hint was shortened after inspection; combining-only queries gained a dotted-circle
cue; wide continuation backgrounds were aligned with the owning cell. No actionable
UI-polish findings remain in those inspected renders. This is a builder self-check only. Integrated toolbar/mouse/modal behavior and a real-binary PTY
review are not verified by standalone fixtures.

## Artifacts

The requested TXT/SVG/PNG fixtures are durable deliverables in this directory
(about 6 MiB). No disposable directory, app process or server was created. Tests
used the normal project cache; owned worker threads were joined through RAII.

## Required integration event precedence

| Order | Handler | Review behavior |
| --- | --- | --- |
| 1 | Global quit and already-open modal/Actions overlays | Consume through their explicit handlers; review data remains frozen. |
| 2 | Intentional toolbar/tab/device/add/close navigation and explicit process controls | Switch/close deliberately clears review; process controls call direct session/animation helpers rather than recursively synthesizing keys. |
| 3 | Active SearchView | All remaining keys, paste and mouse events are consumed; refresh/resume effects are handled without forwarding the original event. Escape that resumes live remains consumed. |
| 4 | Existing live selection/copy and console/PTY fallback | Reachable only when SearchView reports Inactive. Ctrl+e and mouse selection must not act on live cells behind review. |

In particular, review query `r`/`R` is literal text, reading `r` requests snapshot
refresh, and `Ctrl+a`/`Ctrl+e` are consumed here. Explicit Hot reload/Hot restart/
animation Actions must bypass `App.key` recursion, and integration should hide
misleading process shortcut hints while review owns those keys. The standalone
input test pins this module's behavior; the final integration must test explicit
Actions against the actual PTY/session paths and preserve intentional global
navigation before delegation.
