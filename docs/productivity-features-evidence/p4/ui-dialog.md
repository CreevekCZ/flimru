# P4 standalone export-dialog rendered UI review

Independent read-only review of the actual standalone `ExportDialog` renderer and
events. Read `dialog-review.md`, `simplification-dialog.md`, `fixes-dialog.md`, and
both the original findings and independent re-review in `correctness-dialog.md`.
No product or existing test files were changed. This is **not full P4 approval**.

## Findings

### P3: an empty destination still looks ready to export

Location: `src/export_dialog.rs:139–142`, used for Export styling at `381`.

Reproduction: open a captured/ready dialog, then press Ctrl-U in Destination.
`ui-dialog-probe-renders/empty-destination-34x12.png` shows an empty field with
`Ready · 35 B retained text` and the same cyan Export as a valid destination.
The public-event probe confirms its rendered foreground is RGB(96,210,225).
Enter then produces `Enter a destination filename.` and no Submit action.
The safety gate works, but the displayed enabled state contradicts the known
invalid destination, including after clearing an invalid initial path.

Use a common nonempty-destination condition for the enabled rendering and actual
submit gate, and show a short destination prompt when empty. Keep keyboard/mouse
submission blocked until a nonempty supported path is entered. Add a render/event
regression for Ctrl-U and empty initial destinations.

### P3: the Enter hint contradicts the focused Cancel action

Location: `src/export_dialog.rs:393–395` (fixed footer), with actual Enter dispatch
in `handle_event`.

Reproduction: from Destination press Tab twice. The Cancel focus is clearly
highlighted, but `ui-dialog-probe-renders/cancel-focus-34x12.png` still says
`Enter Export · Esc Cancel`. The public-event probe confirms Enter returns
`DialogAction::Cancel`, discarding the modal. Make this hint focus-aware or use
accurate activation wording that fits the compact layout. Verify both focused
buttons and Destination, including disabled Export.

## Exact rendered evidence inspected

Viewed every following actual fixed PNG with the image tool; all paths below are
relative to this report's directory. Each row lists all three files inspected.

| State | 34×12 | 80×24 | 120×40 |
| --- | --- | --- | --- |
| Ready | `fixes-dialog-renders/ready-34x12.png` | `fixes-dialog-renders/ready-80x24.png` | `fixes-dialog-renders/ready-120x40.png` |
| Preparing | `fixes-dialog-renders/preparing-34x12.png` | `fixes-dialog-renders/preparing-80x24.png` | `fixes-dialog-renders/preparing-120x40.png` |
| Saving | `fixes-dialog-renders/saving-34x12.png` | `fixes-dialog-renders/saving-80x24.png` | `fixes-dialog-renders/saving-120x40.png` |
| Worker error | `fixes-dialog-renders/error-34x12.png` | `fixes-dialog-renders/error-80x24.png` | `fixes-dialog-renders/error-120x40.png` |
| Long source/path | `fixes-dialog-renders/long-path-34x12.png` | `fixes-dialog-renders/long-path-80x24.png` | `fixes-dialog-renders/long-path-120x40.png` |
| Joined emoji path | `fixes-dialog-renders/zwj-path-34x12.png` | `fixes-dialog-renders/zwj-path-80x24.png` | `fixes-dialog-renders/zwj-path-120x40.png` |
| Modifier/combining/wide path | `fixes-dialog-renders/modified-path-34x12.png` | `fixes-dialog-renders/modified-path-80x24.png` | `fixes-dialog-renders/modified-path-120x40.png` |
| Invalid initial path | `fixes-dialog-renders/invalid-initial-34x12.png` | `fixes-dialog-renders/invalid-initial-80x24.png` | `fixes-dialog-renders/invalid-initial-120x40.png` |
| Oversized initial path | `fixes-dialog-renders/oversize-initial-34x12.png` | `fixes-dialog-renders/oversize-initial-80x24.png` | `fixes-dialog-renders/oversize-initial-120x40.png` |
| Rejected paste | `fixes-dialog-renders/rejected-paste-34x12.png` | `fixes-dialog-renders/rejected-paste-80x24.png` | `fixes-dialog-renders/rejected-paste-120x40.png` |

Additional independently generated actual TestBackend renders, also viewed:

- `ui-dialog-probe-renders/empty-destination-34x12.png`
- `ui-dialog-probe-renders/export-focus-34x12.png`
- `ui-dialog-probe-renders/cancel-focus-34x12.png`
- `ui-dialog-probe-renders/disabled-export-focus-34x12.png`
- `ui-dialog-probe-renders/tiny-29x11.png`

Each additional PNG has its actual TXT and SVG alongside it. These use the same
cell-to-SVG conversion as the retained dialog evidence, followed by
`rsvg-convert`; no pixels or dialog state were manually edited.

## Passing checks and limits

- At 34×12, 80×24, and 120×40 the title explicitly says **retained** console log;
  the two scope lines explicitly include query/filter-hidden lines. Source,
  editable destination, status, Export, Cancel, and editing/focus hints all remain
  within the modal. Wider terminals center a bounded modal. The immutable source
  label is a constructor-owned value, not a mutable active-tab lookup.
- Long source labels show an explicit `…`; horizontally scrolled destinations
  show `‹`. Long supported paths show the filename end, and both joined-emoji
  and modifier/combining/wide fixtures retain `/tail.txt` at End in every size.
  PNG emoji appearance follows SVG font fallback; exact bytes, grapheme window,
  and actual terminal cursor cells are verified by TestBackend assertions.
- Preparing and Saving visibly distinguish disabled Export from active Cancel.
  Invalid controls, oversized constructor paths, and rejected paste show a
  yellow reason and correction instruction with muted Export, including compact
  renders. These invalid input states remain blocked through capture/error
  transitions until explicit correction. Accepted paths retain exact bytes.
- Ready Export and Cancel focus have distinct bold/background treatment.
  Focused disabled Export remains muted; Enter is consumed without submission.
  Existing event tests check both button regions and consume background input at
  all three requested sizes. Tiny layouts below 30×12 expose no action regions
  and block Enter; the independent 29×11 PNG displays the enlarge/cancel guidance.
- `cargo test --locked --test export_dialog_ui`: **14 passed, 0 failed** during
  this review. Coverage includes exact accepted paths/4096-byte limit, explicit
  correction, immutable snapshot/metadata, grapheme/end-cursor behavior, Saving
  edit lockout, duplicate submits, 600 small-size combinations, and background
  coordinates at all requested sizes.
- A task-owned external harness compiled the actual module with the current
  Cargo artifacts and exercised five extra UI/event states in one test:
  **1 passed**, with the two findings positively reproduced. The other states
  confirm valid focused Export submits, disabled focused Export does not, and
  tiny-layout Enter/Esc behavior. No existing test or product source was edited.

The compact worker-error fixture displays `Error: Destination already` /
`exists. Choose another`; its longer explanation is visible at 80×24 and
120×40. The fixed two-row status area still clamps longer worker messages, as
already documented in `dialog-review.md`; this review does not establish that
arbitrary caller error text will always expose its complete corrective action.

These PNGs omit the hardware cursor and use font fallback. Real terminal fonts,
bracketed paste transport, live PTY draining, global quit precedence, action-menu
integration, frozen capture selection while tabs change, cancellation/join,
stale-result rejection, successful save outcomes, and error routing remain App
integration/live-terminal acceptance work for the coordinator. The standalone
API and fixtures cannot prove those behaviors.

## Resources and recap

Reused normal Cargo artifacts. The external harness occupied 2,601,362 bytes and
was removed by Python's temporary-directory context after its process exited.
An earlier harness setup rejected ambiguous cached library selection before
compilation and also cleaned its owned temporary directory. No task-owned
background process, language server, or disposable build directory remains.
The five new review-evidence triplets total 334,096 bytes and are retained as
named deliverables; existing fixed evidence was left untouched.

Recap: all 30 fixed PNGs inspected; 14 focused tests and the independent five-state
probe passed their assertions; two small UI findings require a separate fix and
re-review. Unicode suffix/cursor and fail-closed path fixes remain intact. This
review does not grant standalone UI acceptance yet or whole-phase P4 approval.

## Independent UI re-review after the separate UI fixes

Both P3 findings above are **closed**. This disposition supersedes the earlier
open findings and standalone-acceptance limitation; the original reproduction
evidence remains intact. Read the appended standalone UI follow-up in
`fixes-dialog.md`, then reviewed the actual `destination_error`, `ready`,
`enter_hint`, submission, and status/button rendering changes. No source or test
files were edited during this re-review.

The same nonempty, accepted destination condition now drives readiness,
submission, status, and enabled Export styling. An empty initial destination,
Ctrl-U, and deleting the final character immediately show a destination prompt
and muted Export. Keyboard and mouse Export are consumed while empty; deliberate
valid correction restores Ready/cyan Export and submits the exact entered path.
Capture completion preserves the empty-path gate. Invalid input rejection
continues to take precedence until explicit correction.

Enter guidance now matches the action for Destination, Export, and Cancel.
Focused Cancel says `Enter Cancel; Esc Cancel` in ready, preparing, saving,
invalid, empty, and retry states. Disabled Destination/Export shows preparing,
saving, or edit-path guidance. Saving no longer advertises editing keys.

Viewed these exact final PNGs under `fixes-dialog-ui-renders/`:

- `empty-destination-34x12.png`
- `empty-export-focus-34x12.png`
- `cancel-focus-34x12.png`
- `disabled-export-focus-34x12.png`
- `preparing-empty-34x12.png`
- `saving-34x12.png`
- `saving-cancel-focus-34x12.png`
- `corrected-destination-34x12.png`
- `export-focus-34x12.png`
- `rejected-paste-34x12.png`
- `cancel-focus-80x24.png`
- `empty-destination-120x40.png`

The compact status/hints fit at 34×12 without clipping. Disabled focused Export
keeps its muted text and distinct focus background; correction and Cancel focus
are clear. Larger affected examples retain the same behavior and layout.

Reran `cargo test --locked --test export_dialog_ui`: **16 passed, 0 failed**.
Read the two added regressions: they directly assert muted Export and no Ready
status for empty paths, blocked Enter/mouse activation, exact valid correction,
final-character deletion, invalid-path clearing, all three focus positions in
six readiness/error states, and capture completion while empty. These checks run
at 30×12, 34×12, 80×24, and 120×40; both enabled keyboard activation contexts and
Cancel activation agree with their displayed hints. Earlier Unicode, retained
scope, cursor, input-rejection, and snapshot tests remain passing.

No residual actionable standalone UI finding was identified in the affected
states. Normal Cargo cache was reused; no task-owned temporary output or process
was created in this re-review. Existing final renders are retained evidence.

Re-review recap: **standalone dialog UI passes within the reviewed scope**, and
both UI findings are closed. Full P4 App integration, worker cancellation/join
and stale-result routing, successful/error outcomes in the App, and live PTY
acceptance remain pending with the coordinator. This is not whole-phase approval.
