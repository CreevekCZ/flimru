# P4 integrated App UI/input review

Independent review after the App integration builder and simplifier. Read
`build-integration.md`, `simplification-integration.md`, the actual App/UI/dialog
integration, all six App tests in `tests/support/log_export_app.rs`, and the
actual-binary export test in `tests/terminal_flow.rs`. Previously cleared
standalone dialog/path/Unicode/tiny matrices were not repeated. No product or
existing test files were edited. **The integrated UI gate awaits one fix below.**

## Finding

### P3: the visible global Quit button is inert during export

Location: `src/main.rs:2650–2653`, the early modal mouse routing.

The integrated ready/error/review renders at 80×24 and 120×40 retain the normal
visible toolbar Quit button. Its visual styling and hit-test action are unchanged,
but `App::mouse` routes that click to the dialog and returns before global Quit
handling. It produces no feedback or confirmation. `Ctrl+q` does open the normal
quit confirmation, so the global operation is supported while the dialog is open.
`docs/usage.md:55–57` describes both Ctrl+q and the Quit button as confirmation
entry points. Ordinary background buttons are intentionally blocked; the visible
global exit affordance should follow the explicit global quit behavior.

An independent task-owned actual-App harness rendered the real App and located
the visible `Q` cell whose `ui::mouse_action` is `MouseAction::Quit`:

- At **80×24**, coordinate **(75, 1)** renders `Q` and hit-tests Quit, but calling
  `App::mouse` leaves `quit_confirmation == false` and the export dialog open.
- At **120×40**, coordinate **(115, 1)** has the same result.
- In both sizes Ctrl+q then sets `quit_confirmation == true`; Enter dismisses
  confirmation and retains the original export dialog.

Permit the visible Quit pointer action to open the same confirmation before
ordinary modal mouse routing. Keep toolbar navigation, selection, background
clicks, and all other covered actions consumed. Add App-level pointer regressions
for the visible Quit control, confirmation cancellation returning to the same
export state, and ordinary background controls remaining blocked. No immediate
quit or extra export submission should result from that pointer entry point.

## Actual App PNGs inspected

Viewed all nine files with the image tool, relative to this report's directory:

| App state | 34×12 | 80×24 | 120×40 |
| --- | --- | --- | --- |
| Ready | `integration/ready-34x12.png` | `integration/ready-80x24.png` | `integration/ready-120x40.png` |
| Worker error | `integration/error-34x12.png` | `integration/error-80x24.png` | `integration/error-120x40.png` |
| Covered review | `integration/review-34x12.png` | `integration/review-80x24.png` | `integration/review-120x40.png` |

The retained-log title, explicit full query/filter-hidden scope, frozen source
label, editable destination, ready/error status, Export/Cancel, and accurate
focus hint remain visible and within the modal. Long sources use an explicit
ellipsis and long default destinations use the left-scroll marker while exposing
their filename suffix. At 34×12 the dialog nearly fills the screen and retains
its controls; at larger sizes it is centered over the App/review. The compact
error identifies the collision and asks for another filename; wider layouts
also show the no-overwrite explanation.

The covered review remains visible around the dialog in the larger renders.
The screenshots do not contain the native cursor, so cursor conclusions below
use the actual App draw API and event tests rather than image inference.

## Passing input/outcome checks

- `cargo test --locked --bin flimru log_export -- --nocapture`: **6 passed,
  0 failed**, 97 filtered out. The tests exercise actual App source capture,
  frozen review Arc/full unfiltered text, invocation metadata, no pre-submit
  filesystem work, duplicate submission, editable retry, retained outcome,
  cancel/publication winner/source close/shutdown, modal input isolation and
  App renders.
- `cargo test --locked --test terminal_flow console_export_real_binary --
  --nocapture`: **1 passed, 0 failed**, 29 filtered out, approximately 3.35 seconds.
  Actual Actions invocation, query/severity-filtered review, bracketed paste into
  destination, explicit submission, full retained saved UTF-8 content, complete
  outcome through Operation details, child input isolation, and normal quit pass.
- The independent visible-Quit App harness passed its reproduction assertions
  at both larger sizes and independently confirmed Ctrl+q/cancel behavior. It
  compiled the actual source with current Cargo dependencies, without changing
  production/test files.

Source and test inspection confirm the following within this UI scope:

- The modal routes keys before early selection, Actions/top navigation, review
  query handling, and PTY fallback. Paste and mouse have matching early modal
  handling; the paste bridge preserves an oversized sentinel instead of silently
  submitting a shortened path. Ctrl+q precedes the dialog and the confirmation
  overlay owns subsequent input. The visible Quit pointer exception is the
  finding above.
- `console_ui_owned` suppresses the live console cursor/selection/raw-output
  rendering. The parent review renders with focus false, so its query editor
  cannot claim the cursor; the destination alone renders focused. The actual
  live-child App test observes the destination row as the native cursor row and
  a hidden cursor while quit confirmation covers the dialog.
- Actual `r`, `R`, `q`, selection/menu shortcuts, paste, and background mouse
  input remain inside the dialog, with no child input until cancellation. The
  child test receives exactly the deliberately sent post-cancel `z` and no
  covered modal text. No stop request is raised by plain modal `q`.
- Collision errors preserve the existing file and frozen source; editing the
  destination and retrying writes the retained text. Full absolute saved path
  and byte-count outcome is kept in `last_operation`/Operation details. The PTY
  check accepts normal terminal wrapping of the filename rather than requiring
  its suffix on one line.
- The review export reuses its exact snapshot, source and capture time. Query,
  severity filter, Arc, and offsets remain stable in the standard App regression;
  outside-review capture also preserves its source and saved-console offset
  through later output/tab changes. Pending review says to wait; failed review
  exposes the actual failure and narrower/new-session recovery. Export does not
  silently replace either with a fresh capture.

The coordinator/correctness reviewer is separately investigating review
viewport/offset behavior with an existing operation notice. The standard test
does not establish that boundary case, and this UI report neither assumes a
finding nor clears it. Other worker lifecycle/backend acceptance remains with
the corresponding correctness gate. P5 and separately prepared P6 work are
outside this review.

## Resources and recap

Normal Cargo artifacts and existing RAII test fixtures were reused. The owned
external harness occupied **13,456,918 bytes** before Python's temporary-directory
context removed it after completion. An initial harness compile lacked the
Cargo version environment value and also cleaned its owned directory; the
successful probe supplied that build-time value. No task-owned background
process, app, language server, disposable build tree, or temporary fixture
remains. Existing nine PNG triplets were inspected without regeneration.

Recap: all nine integrated App PNGs inspected; six App tests and one actual-binary
PTY flow passed. One actionable visible-Quit UI finding needs a separate fix and
re-review. Prior standalone UI findings remain closed. This report does not
grant full P4 approval or clear later phases.

## Independent integrated UI re-review after the separate fixes

The visible-Quit P3 above is **closed**. This disposition supersedes the earlier
open UI gate. Read `fixes-integration.md`, reviewed the actual App pointer guard,
dialog rendered-coverage accessor, and targeted regressions. No product or test
files were edited. Unchanged original nine App scenes and standalone matrices
were not repeated.

Before modal mouse routing, the App now admits only a real Quit hit action whose
cell lies outside the dialog's last rendered coverage. It uses the existing
confirmation action rather than exiting or exporting immediately. Coverage is
fail-closed before first draw, the exact modal rectangle in normal layouts, and
the full screen in the tiny fallback. Thus 80×24, 120×40, and shallow 120×12
retain an actionable visible Quit, while hidden Quit at 34×12 and 29×11 cannot
activate through the dialog. Ordinary Help/toolbar/background clicks remain
consumed. Confirmation cancellation keeps the original export prompt and data.

Viewed all four exact affected actual-App PNGs under
`fixes-integration-renders/`:

- `quit-confirmation-80x24.png`
- `quit-confirmation-120x40.png`
- `quit-confirmation-120x12.png`
- `cancel-notice-manual-review-80x24.png`

The standard confirmation visibly covers the dialog with clear Quit/Cancel
controls and `y quit · Enter/Esc cancel` guidance. The shallow 120×12 scene keeps
the global Quit visible outside the centered export rectangle and the complete
confirmation controls inside the terminal. The resumed review shows late rows
085–098, query `needle`, and the explicit cancellation notice rather than
jumping to the old first match.

Targeted current-source checks rerun during this re-review:

- `cargo test --locked --bin flimru log_export_only_visible_quit_opens_confirmation_and_cancel_retains_the_modal -- --nocapture`: **1 passed**. Reviewed its actual rendered `Q` discovery, coverage checks at all five sizes, blocked ordinary controls/background, no pre-submit writes, pointer/Enter cancellation, identical destination/Arc, and explicit confirmation exit signal.
- `cargo test --locked --bin flimru log_export_cancel_notice_preserves_manual_review_away_from_current_match -- --nocapture`: **1 passed**. Query, frozen Arc and manually chosen offset survive open/draw/cancel/first-notice render; deliberate next-match still reveals the match.
- `cargo test --locked --test console_search_ui height_only_viewport_changes_preserve_manual_scroll_but_navigation_reveals -- --nocapture`: **1 passed**. Height-only shrink/grow preserves manual scroll; match/query navigation still reveals deliberately.
- An independent temporary actual-App harness: **1 passed**. Starting with review query `needle`, the exact review Arc, and edited destination `/tmp/explicit-界.txt`, visible-Quit confirmation followed by actual pointer Cancel and then Enter cancellation preserves all three. Confirmation paste is ignored, its cursor is hidden, the same dialog remains open, and no export directory is created.

The shared 8192-character outcome budget and safe final-outcome fixes remain with
the independent correctness gate; their tests/source were read as context rather
than re-running that separate boundary matrix. The previous UI report's
notice/viewport uncertainty is resolved for the affected presentation by the
passing height-only/manual-review checks and actual late-row PNG above.

Normal Cargo artifacts were reused. The independent harness occupied
**13,558,586 bytes** and was automatically removed after its process exited.
No task-owned temporary output or process remains; existing four render triplets
are retained deliverables.

Final UI recap: **P4 standalone and integrated UI/input gates pass within the
reviewed scope; no residual actionable UI finding remains.** All three earlier UI
findings are closed. This verdict covers the export UI and its App integration;
the coordinator retains overall P4 correctness/phase acceptance and later P5/P6
or installation decisions.
