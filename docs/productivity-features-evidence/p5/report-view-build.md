# P5 frozen delivery report view — build handoff

Built 2026-11-06. This is a disjoint P5 presentation foundation. It does not close
the full P5 phase gate or integrate the feature into App/Actions.

## Owned changes

- `src/batch_report.rs`: immutable captured report plus viewport/dismissal state.
- `tests/batch_report_ui.rs`: standalone path harness and actual TestBackend
  rendering evidence generator.
- `docs/productivity-features-evidence/p5/report-view-renders/`: 36 actual rendered
  scenes, each retained as TXT, SVG, and PNG, at 34×12, 80×24, and 120×40.

No existing main/UI/runs/session/shortcuts/core helper files were changed. There
were no new dependencies, subprocess runners, retry paths, or filesystem paths in
the view. The evidence generator performs filesystem work only in the test when
`FLIMRU_P5_REPORT_EVIDENCE_DIR` is explicitly set.

## Settled API and caller contract

`BatchReportView::new(HotCommand, BatchReport)` owns its captured source. Its
read-only report data never changes in response to live tabs, session completion,
configuration changes, or vector movement. The controller exposes:

- `render(&mut self, &mut Frame)` renders the centered overlay and records its
  current viewport and exact seven-cell `[Close]` hit region.
- `handle_event(&Event) -> ReportAction::{Consumed, Closed}` consumes all events.
  Enter/Esc and a left click on actual Close cells dismiss it. Up/Down,
  PageUp/PageDown, Home/End and wheel input inside the detail viewport scroll;
  offset is clamped to the rendered physical rows.
- `is_open()`, `offset()`, and `close_region()` provide read-only state.

Integration should register this module, create the view from the completed
dispatch report, and route modal input before ordinary terminal input, actions,
paste, or background mouse handling. Global quit and higher modal precedence
remain the caller's responsibility. On `Closed`, remove the view and render the
previous UI. Render the report after its underlying UI. No event returned from
this view authorizes another dispatch, retry, confirmation, or process command.

Resize clears old pointer regions immediately, then render recomputes wrapping
and clamps the offset. Below 34×12 a safe enlargement prompt remains dismissible
with Enter/Esc; detailed inspection resumes when the terminal is large enough.

## Presentation and behavior

Reload and restart have distinct `Hot reload delivery report` / `Hot restart
delivery report` titles. Requested/queued, skipped, and failed counts and the
`Flutter completion is unconfirmed.` caveat remain fixed above the scrollable
device details at all three supported review sizes. Per-device failure text uses
the helper's fixed categories; the view accepts no opaque send-error message.

Details use restrained existing panel, text, accent, muted, and queued colors,
with yellow for failed delivery. The title has explicit normal text contrast.
Word-preferred grapheme wrapping preserves wide, combining, and joined characters
and supplies the same physical rows to rendering and scroll bounds. Publicly
constructed reports are defensively restricted to eight details and 120
control-free device-label scalars per detail, matching the dispatch helper's
existing bounds.

Unknown keys including `r`/`R`, release events, paste, focus changes, and background
clicks/wheel/drag input are consumed. The view cannot access sessions or commands.

## Validation

Final checks after the implementation self-review:

```sh
rustfmt --edition 2024 src/batch_report.rs tests/batch_report_ui.rs
cargo test --test batch_report_ui
cargo clippy --test batch_report_ui -- -D warnings
rustfmt --edition 2024 --check src/batch_report.rs tests/batch_report_ui.rs
```

Result: **9 tests passed**, targeted Clippy passed, owned-file formatting passed.
Tests cover both command titles; mixed requested/skipped/failed delivery; zero
eligible/all-ended and empty requests; both fixed failure categories; every
bounded device reason reachable at 34×12; reverse and clamped keyboard/wheel
scrolling; resize and stale pointer rejection; tiny/zero rectangles; Enter/Esc
and exact Close-cell dismissal; frozen owned results; Unicode grapheme integrity;
public-input sanitization/bounds; background/unknown/paste event isolation.

Evidence was generated with:

```sh
FLIMRU_P5_REPORT_EVIDENCE_DIR=docs/productivity-features-evidence/p5/report-view-renders \
  cargo test --test batch_report_ui
```

Each generated SVG was converted with `rsvg-convert INPUT.svg -o OUTPUT.png`.
The 36 scenes include reload, restart, all-ended, empty, long device, and
failure-only reports, plus distinct PageDown/End views when scrolling changes
the viewport. TXT/SVG are serialized from TestBackend cells rather than mock
content. PNGs were visually inspected at 34×12 (failure details), 80×24 (mixed
restart), and 120×40 (long Unicode labels). Counts, caveat, title contrast,
wrapping, failure emphasis, and Close affordance were readable. Emoji appearance
in SVG/PNG depends on the renderer's font fallback; TestBackend grapheme-cell
tests independently verify intact combining and joined symbols.

Self-review checked that every detail remains reachable at 34×12; scrolling does
not move the delivery caveat/counts; stale Close coordinates cannot act after a
resize; background input cannot fall through; the source cannot resolve a
different live vector entry; and the module has no process or filesystem access.
The defensive public-report sanitization/bounds regression was added during
review, then focused checks were rerun successfully.

## Artifacts and remaining work

Retained render evidence totals **5,410,210 bytes** across 108 files. Its durable
purpose is phase review and reproducible layout comparison. Validation reused
the normal project `target` cache. No task-owned temporary build directory or
background process remains.

Pending: independent view review/fixes, module registration, Actions integration,
launch-time capability storage, stable-ID dispatch through `Session::send`,
animation restart regression, cross-feature App/PTy tests, and the full P5 gate.
The view itself does not establish any of those integration guarantees.
