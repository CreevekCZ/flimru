# P4 standalone export-dialog fixes

Separate fixer for the two P2 findings in `correctness-dialog.md`. Ownership was
limited to `src/export_dialog.rs`, `tests/export_dialog_ui.rs`, this report,
`fixes-dialog-*.log`, `fixes-dialog-renders/`, and `fixes-dialog-ui-renders/`. No backend, shared App/UI,
dependency, or other module was changed. No filesystem work was added to the
dialog, and no commits were created.

## Fixes

- `input_window` in `src/export_dialog.rs` uses Ratatui's existing
  `Span::styled_graphemes` for both width measurement and scroll boundaries.
  Whole joined emoji, modifier sequences, combining sequences, and wide
  graphemes stay intact. A scalar editing cursor inside a grapheme displays at
  that grapheme's start. The End cursor stays inside the field after the visible
  filename suffix. No Unicode dependency was added; editing/render scans remain
  bounded by the existing 4096-byte accepted destination limit.
- `src/export_dialog.rs:75`: constructor validation uses `Path::to_str` and
  rejects non-UTF-8, control-containing, and oversized initial paths. Rejection
  leaves an empty editable destination with a specific visible error. There is
  no lossy conversion, control stripping, or truncation into another filename.
  Accepted UTF-8 paths are retained exactly, including spaces, Unicode, relative
  components, and the exact 4096-byte limit.
- `destination_error` / `ready` in `src/export_dialog.rs`: input validation is independent of capture/worker
  status. Rejected paste/overflow preserves the existing string and disables
  keyboard/mouse submission until a successful explicit insertion, deletion, or
  clear corrects the input. Cursor/focus movement and empty paste do not clear
  rejection. Capture completion and caller `set_error` cannot accidentally
  validate rejected input. Clearing an invalid initial value still requires a
  nonempty destination before submission.

Snapshot/source/metadata ownership, synchronous Saving transition, duplicate
submission blocking, saving edit lockout, worker error retry, cancellation,
modal event consumption, and tiny-layout submission gating remain intact.

## Verification and review

- `cargo test --locked --test export_dialog_ui`: **14 passed, 0 failed**;
  output retained in `fixes-dialog-tests.log`.
- `cargo clippy --locked --test export_dialog_ui -- -D warnings`: passed;
  output retained in `fixes-dialog-clippy.log`.
- `rustfmt --check --edition 2024 src/export_dialog.rs tests/export_dialog_ui.rs`:
  passed; empty successful output retained in `fixes-dialog-fmt.log`.

Regressions reproduce the exact `"/" + "👩‍💻" * 20 + "/tail.txt"` compact
failure and both reported initial path mutations, plus NUL/oversized initial
paths, all initial readiness states, rejected control/overflow paste, immutable
snapshot/metadata, exact accepted ordinary/Unicode paths, and explicit
correction. ZWJ, skin-tone, combining, and wide paths check suffix visibility,
actual TestBackend cursor location, mid-grapheme navigation, and unchanged
submitted bytes at all three requested sizes. A maximum 4096-byte joined-emoji
fixture exercises 64 navigation/render cycles and whole-input overflow
rejection. Every layout in width 0–39/height 0–14 below 30×12 blocks Enter and
has empty mouse regions; every background coordinate at 34×12, 80×24, and
120×40 is consumed.

The fixer reviewed the resulting state transitions and scrolling logic after
implementation. This caught overly long compact validation copy; messages were
shortened and regenerated so the reason and correction instruction both fit
the two status rows at 34×12. This is a fixer self-check, not independent phase
approval.

## Render evidence and resources

`fixes-dialog-renders/` retains **30 TXT/SVG/PNG triplets**: ready, preparing,
saving, worker error, supported long path, reported ZWJ path, modified/combining/
wide path, invalid initial controls, oversized initial path, and rejected paste,
each at 34×12, 80×24, and 120×40. The long supported path fixture was reduced
below 4096 bytes; oversized input now has its own explicit error fixture.

Generation used the actual `ExportDialog::render` TestBackend with
`FLIMRU_P4_EVIDENCE_DIR=docs/productivity-features-evidence/p4/fixes-dialog-renders`
while running the focused tests, followed by `rsvg-convert INPUT.svg -o OUTPUT.png`
for each retained SVG. Original reviewer-era evidence is preserved alongside
this named fixed-state evidence.

Visually inspected actual PNGs: `zwj-path-34x12.png`,
`modified-path-80x24.png`, `zwj-path-120x40.png`,
`invalid-initial-34x12.png`, `oversize-initial-34x12.png`, and
`rejected-paste-34x12.png`. The filename suffix is visible, error prompts fit,
disabled Export is distinguishable, and controls stay within the modal. PNG
emoji glyph appearance follows the local SVG font fallback; byte integrity and
cursor cells are asserted directly against Ratatui rather than inferred from
the font rendering. These are rendered modal checks, not live-terminal or App
integration acceptance.

The evidence directory contains **3,565,248 bytes** (about 3.40 MiB), deliberately
retained as review deliverables. Normal reusable Cargo artifacts were used; no
task-owned disposable build directory, temporary files, background process,
language server, or app instance remains.

## Stable handoff / recap

Both reported P2 cases now have passing public constructor/event/render
regressions. Review the constructor/input-error gate and grapheme window in the
two owned files, rerun the retained commands, and compare the named fixed-state
render triplets. The dialog API and caller obligations documented in
`dialog-review.md` are unchanged. The original correctness reviewer must
independently re-review these fixes; shared App/UI integration, worker
cancellation/stale-result routing, live PTY validation, and full P4 approval
remain with the coordinator.

## Standalone UI follow-up

Read `ui-dialog.md` and inspected the independent probe PNGs for empty
destination, Cancel focus, and disabled Export focus. Fixed both P3 findings in
the same two owned files; shared App/UI, backend, workers, and dependencies were
left untouched.

`destination_error` now supplies the shared destination condition for status,
enabled Export styling, and the submission readiness gate. Empty initial paths,
Ctrl-U, deletion of the last character, or clearing an unsupported initial path
show `Enter a destination.` and muted Export immediately. Enter and mouse
Export remain consumed until a supported nonempty path is deliberately entered.
A pending capture with an empty field still says `Preparing snapshot…` alongside
the destination prompt; capture completion cannot turn that empty field into a
ready-looking export. Constructor/rejected-input errors, worker retry errors,
Preparing, and Saving keep their separate states.

The Enter footer now matches focus and action: `Enter Export; Esc Cancel` for
enabled Destination/Export, `Enter Cancel; Esc Cancel` for Cancel in every
state, and `Preparing · Esc Cancel`, `Saving · Esc Cancel`, or
`Edit path · Esc Cancel` for disabled Destination/Export. Each footer fits even
the actual 30×12 minimum editable layout. Saving also replaces the editing hint
with `Tab focus`, because edits are locked until the caller reports an error.

Added render/event tests for empty initial paths and Ctrl-U from both valid and
unsupported initial paths; muted ordinary and focused Export; blocked Enter and
mouse submission; exact valid correction; all three focus positions across
ready, preparing, saving, empty, rejected-input, and worker-retry states; enabled
Export activation; Cancel Enter behavior in every state; and empty capture
completion. These checks run at **30×12**, 34×12, 80×24, and 120×40. Existing
Unicode/path-intent and frozen snapshot/metadata regressions remain intact.

Final standard-project validation:

- `cargo test --locked --test export_dialog_ui`: **16 passed, 0 failed**;
  retained in `fixes-dialog-ui-tests.log`.
- `cargo clippy --locked --test export_dialog_ui -- -D warnings`: passed;
  retained in `fixes-dialog-ui-clippy.log`.
- `rustfmt --check --edition 2024 src/export_dialog.rs tests/export_dialog_ui.rs`:
  passed; empty successful output in `fixes-dialog-ui-fmt.log`.

The first standard attempt hit the concurrent shared integration's temporary
non-exhaustive MouseAction match. An interim library-only temporary manifest
pointing directly to the actual test file passed all 16 tests and strict Clippy;
its 64,520-byte manifest/lock directory was automatically removed after the
processes exited. After the coordinator announced a compile-stable checkpoint,
the standard project commands above passed. Final retained logs are from those
standard commands, and no workaround source or configuration was retained.

The final `fixes-dialog-ui-renders/` contains **54 actual TXT/SVG/PNG triplets**:
all ten preceding states regenerated for the changed footer, plus empty
destination, Export focus, Cancel focus, preparing/disabled Export focus,
empty/disabled Export focus, saving Cancel focus, preparing with an empty path,
and deliberately corrected path. Each is rendered at 34×12, 80×24, and 120×40.
Generation uses the existing `FLIMRU_P4_EVIDENCE_DIR` test mechanism and
`rsvg-convert` as above; original and earlier fixer/reviewer evidence remains
preserved for comparison.

Viewed actual compact PNGs `empty-destination-34x12.png`,
`empty-export-focus-34x12.png`, `cancel-focus-34x12.png`,
`disabled-export-focus-34x12.png`, `preparing-empty-34x12.png`,
`saving-34x12.png`, `saving-cancel-focus-34x12.png`, and
`corrected-destination-34x12.png`, plus `cancel-focus-80x24.png` and
`empty-destination-120x40.png`. The destination prompt is readable, disabled
Export stays muted with and without focus, enabled correction looks ready,
Cancel's Enter hint agrees with its action, and pending states no longer
advertise an enabled export or saving edits. Self-review also checked that
input validation remains blocking through capture/error transitions and that
valid correction re-enables the same exact destination.

The final UI evidence totals **6,344,731 bytes** (about 6.05 MiB), intentionally
retained for independent review. The earlier 3,565,248-byte fixed evidence is
also retained as historical comparison. Normal reusable Cargo artifacts were
used; no task-owned temporary directory or background process remains.

### Follow-up handoff / recap

Both standalone UI findings now have passing output/event regressions. Source
is stable for the original UI reviewer: review `destination_error`, `ready`,
`enter_hint`, status rendering, the two new regression tests, and the final
`fixes-dialog-ui-renders/` evidence. This fixer does not grant standalone UI or
P4 phase approval. Independent UI re-review and shared integration/live-terminal
acceptance remain with the coordinator.
