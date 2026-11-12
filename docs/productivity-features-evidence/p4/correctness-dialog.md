# P4 standalone dialog correctness review

Independent review of `src/export_dialog.rs`, `tests/export_dialog_ui.rs`,
`dialog-review.md`, `simplification-dialog.md`, and retained text render evidence.
The backend remains covered by the earlier foundation review. No product or
existing test files were edited by this reviewer.

## Findings

### P2: horizontal scrolling uses inconsistent Unicode widths

Location: `src/export_dialog.rs:384–398` (`input_window`).

The initial column count measures the whole string using Ratatui's Unicode
width calculation, but the scrolling loop subtracts independently measured
Unicode scalars. These measurements disagree for multi-scalar emoji such as
`👩‍💻`: the combined grapheme occupies two cells, while its separately measured
woman/laptop scalars each occupy two cells. Scrolling therefore removes too few
visible cells and places the cursor before the actual destination end.

Reproduced through the public constructor and `render` at 34×12 with the ready
destination `format!("/{}{}", "👩‍💻".repeat(20), "/tail.txt")`. The input row shows a
left marker followed entirely by emoji, with **no `/tail.txt` visible**, even
though the cursor starts at End. This contradicts the promised visible end cursor
and makes the filename being edited invisible. Use a consistent grapheme-aware
width calculation/window boundary and add joined-emoji/emoji-modifier scrolling
regressions alongside the existing CJK/combining cases.

### P2: constructor silently changes the destination that will be submitted

Location: `src/export_dialog.rs:71` and `370–381` (`bounded_text`).

The constructor runs filesystem paths through a lossy display conversion and a
helper that drops controls and truncates bytes. These transformations modify the
actual destination used by `Submit`, rather than merely changing its display.
They bypass the backend's rejection of an invalid original path. The public
constructor documents no precondition requiring an already validated UTF-8 path.

Two independent public-API reproductions render a ready dialog and press Enter:

- Initial `/tmp/name\nwith-control.txt` submits `/tmp/namewith-control.txt`.
- On Unix, initial bytes `b"/tmp/invalid-\xff.txt"` submit
  `/tmp/invalid-�.txt` (replacement character).

The same helper silently truncates initial destinations above 4096 bytes. An
export must not write a different path because its original representation was
unsupported. Reject unsupported/control/oversize initial paths with a visible
validation state that prevents submission until explicitly corrected, or preserve
the original supported destination exactly. Keep lossy/clipped transformations
confined to display labels. Add constructor regressions; paste rejection alone
does not cover this path.

## Validation

- `cargo test --locked --test export_dialog_ui`: **6 passed, 0 failed**.
- A task-owned external harness compiled the actual dialog module against the
  normal project library artifacts and exercised only public API/events/rendering.
  Result: **3 passed, 3 failed**. The failures are the joined-emoji suffix check and
  the two initial-path preservation checks above.
- Passing independent checks covered every width 0–39 and height 0–14 (all layouts
  below 30×12 block submission), every coordinate in an 80×24 screen (background
  clicks never submit/cancel), and Unicode editing/navigation/paste lockout while
  Saving followed by explicit error/retry.
- Reviewed readiness, capture completion after a rejected edit, immutable snapshot
  ownership, synchronous Saving transition, repeated-submit rejection, focus
  cycling, empty-path rejection, and cancellation effects. No additional concrete
  state-machine finding was identified.

The external harness directory occupied 2,214,506 bytes before cleanup; it and all
fixtures were removed after its process exited. No additional build cache,
background process, or disposable artifact remains owned by this review.

## Recap

Two P2 findings require a separate fixer and re-review. The existing six tests
pass but do not cover either finding. This is a standalone dialog review;
App integration, worker cancellation/stale-result routing, live PTY draining,
rendered integration validation, and full P4 approval remain pending.

## Independent re-review after the separate dialog fixes

Both P2 findings above are **closed** in the standalone dialog. The initial
findings and evidence are retained as review history; this re-review supersedes
their open disposition. Read the fixed module, expanded tests, and
`fixes-dialog.md`. No implementation/test edits were made by this reviewer.

- `src/export_dialog.rs:424–457` now measures/scans complete rendered graphemes
  consistently. The original compact joined-emoji reproduction exposes
  `/tail.txt`, and the actual TestBackend cursor follows the final `t`. Submitted
  bytes remain identical to the original accepted destination.
- `src/export_dialog.rs:75–88` now rejects unsupported initial paths instead of
  reinterpreting them. `input_error`, checked independently by `ready` at
  `139–142`, remains authoritative across capture completion and caller errors.
  Navigation, empty paste, and ineffective deletion do not authorize submission.
  An explicit supported replacement submits exactly the deliberately entered
  path, including spaces and joined/combining Unicode.

Validation:

- `cargo test --locked --test export_dialog_ui`: **14 passed, 0 failed**.
- An external, task-owned public-API harness reran all six original independent
  checks and two additional correction/retry checks: **8 passed, 0 failed**.
  This includes the three assertions that previously failed, all 600 tested
  terminal sizes, all 1,920 background mouse coordinates at 80×24, Saving edit
  lockout, initial controls/oversize rejection across capture/error transitions,
  rejected control/overflow paste blocking the unchanged old destination, and
  exact submission only after explicit supported correction.
- Visually inspected the actual fixed compact PNGs
  `fixes-dialog-renders/zwj-path-34x12.png` and
  `fixes-dialog-renders/invalid-initial-34x12.png`: the filename suffix and full
  control-path reason/correction guidance are visible. Byte/cursor conclusions
  also come from the actual TestBackend API, independent of PNG font fallback.

No concrete residual standalone-dialog finding was identified. The external
harness occupied 2,224,416 bytes before cleanup and was removed with its fixtures
after the process exited. Normal reusable project artifacts were used; no task
background process or disposable output remains.

Re-review recap: both standalone findings closed; 14 focused tests and eight
independent checks passed. This closes the standalone fixes only. App integration,
worker cancellation/stale-result routing, live PTY validation, and full P4
approval remain pending with the coordinator.
