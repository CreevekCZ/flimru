# P5 standalone delivery-report UI review

Reviewed 2026-11-06. Scope: `src/batch_report.rs` and
`tests/batch_report_ui.rs`, builder handoff and simplification handoff. Product and
test source were not edited by this reviewer. App integration remains a later gate.

## Current verdict

**PASS for the standalone report view after fixes.** Both findings below are
closed by the final source/test and actual PNG recheck recorded at the end of
this report. The initial reproduction and review history remain for traceability.
This verdict does not approve the later App integration gate.

## Findings

1. **MEDIUM — label truncation splits a grapheme and hides shortening.** The
   initial `detail_lines` normalization takes 120 Unicode scalar values before
   grapheme wrapping. A separate temporary Cargo harness imported the actual
   unchanged view and rendered it with `TestBackend` at 120×40. A device label
   consisting of 118 `x` characters followed by `🧑‍💻` produced the incomplete
   visible cell `"🧑\u{200d}"`. A label of 119 `x` characters followed by
   `e\u{301}` rendered plain `e`, losing the accent. Neither shortening is marked.
   These are valid overlong labels accepted by the public report; wrapping cannot
   repair a grapheme already cut by normalization. The dispatch helper has the
   same earlier scalar truncation, so both boundaries need coherent handling.
   Preserve whole graphemes within the deliberate label budget and indicate
   shortened labels with an ellipsis. Add boundary tests for combining and joined
   symbols, rather than relying only on repeated labels whose cutoff happens to
   avoid the boundary. A separate fixer owns this correction.

2. **MEDIUM — three-digit footer counter clipped; source/test fix verified.**
   The initial compact help occupied enough cells to cut the final digit from
   `101-103/103` at 34×12 for eight maximum-width labels. The correction chooses
   the longest help string that fits beside the complete position counter.
   Source inspection confirms the seven Close cells and pointer region are
   unchanged. The independent focused run now passes the new regression for
   both commands, End, 34×12→80×24→34×12 resize, stale pointer rejection and
   complete counters. Final generated maximum-label PNG inspection is pending
   the fixer's render handoff.

## Visual evidence actually inspected

Used `view_image` on **all 36 retained PNG scenes** in `report-view-renders/`,
covering every existing initial, PageDown and End fixture at 34×12, 80×24 and
120×40: mixed reload, mixed restart, all-ended, empty request, long Unicode
label and failure-only. This was an actual image review, supported by reading
source and TestBackend assertions.

- Both command titles remain legible, including the tight 34×12 border title.
- Requested/queued, skipped and failed counts remain fixed while details scroll.
  `Flutter completion is unconfirmed.` remains fully reachable and continuously
  visible at the three supported sizes; no success/completion claim is made.
- Empty requests say there are no session targets. All-ended reports show zero
  queued and eight skipped, with ended-session reasons.
- Both failure categories are readable in yellow and can be read completely by
  scrolling at the minimum size. Status wording, rather than color alone,
  carries each outcome.
- Long CJK, combining and joined-symbol fixtures wrap within the panel. Existing
  PNG font fallback draws the joined emoji with a fallback glyph; actual cell
  tests establish that interior graphemes remain intact. This does not cover the
  truncation boundary described above.
- Close stays visually separate from scrolling content. Hints describe report
  navigation/dismissal and do not invite retry, dispatch, or terminal typing.
  The small layout trades expanded help for space; a visible Close and Esc cue
  remain. Larger layouts show Enter/Esc and PgUp/PgDn explicitly.
- Paged rows may begin with a continued reason or spacer, as expected for physical
  row scrolling; reverse scrolling and the position counter preserve navigation.

## Interaction verification

`cargo test --locked --test batch_report_ui` passed independently first with
**9 tests**, then with **10 tests** after the footer correction. Inspected the
actual tests and event implementation for keyboard/wheel scrolling, clamping,
resize, tiny/zero rectangles, Close hit cells, Enter/Esc dismissal, frozen owned
results and consumption of unknown keys, paste, release events and background
pointer input. The exact seven-cell Close target matches its rendered text.
Resize invalidates pointer regions before redraw. No module access can send a
process command or retry an operation.

## Caller obligations and remaining gate

Integration must route report-modal events before terminal input, actions, paste
and background mouse handling; honor global quit and higher modal precedence;
render after the underlying UI; remove the closed view; and create each view
from the captured dispatch report. Standalone tests cannot prove those App
properties, preservation of the active tab/review snapshot, animation restart,
or stable-ID dispatch. Review those in the later integration gate.

Validation reused the normal project target cache. The boundary probe used only
its own `/private/tmp/flimru-p5-ui.fj4ps61v` manifest/source; its useful failure
inputs and outputs are preserved above. That temporary package and its owned
build artifacts were removed after the command exited. No GUI, selected Xcode,
background process, or other task's temporary directory was touched.

Recap: all existing rendered scenes were reviewed and focused interaction checks
pass. The footer correction is behaviorally verified; final render review and
the confirmed Unicode truncation correction remain before standalone UI approval.


## Final independent recheck after fixes

**Standalone UI verdict: PASS. No unresolved UI findings remain in this scope.**

Read the final `fixes-report-view.md`, inspected the shared
`bounded_device_label` implementation and its dispatch/view call sites, and
reviewed the new helper/view regressions. The source keeps whole graphemes,
marks shortened labels with an ellipsis within the existing scalar budget, and
preserves exact-budget labels. The same normalizer handles dispatch snapshots
and defensive public reports. The footer measures available cells and shortens
optional help before the position counter; dismissal geometry is unchanged.

Used `view_image` to inspect **all 20 new actual PNGs** in
`report-view-fix-renders/`:

- Four `maximum-wide-hot-{reload,restart}-end-{34x12,80x24}.png` scenes visibly
  retain complete `101-103/103` and `32-47/47` counters respectively. The narrow
  footer still shows Close, Esc and arrow hints. Titles, fixed counts and the
  completion caveat remain readable; Close is unobstructed.
- Sixteen `unicode-{joined,accent}-{cut,fit}-{public,dispatch}-end-{34x12,80x24}.png`
  scenes show explicit ellipses when truncation is required, retain the accented
  character when the full grapheme fits, and agree between public-report and
  dispatch-produced paths. Exact-budget joined symbols retain the existing font
  fallback glyph; raw-cell assertions verify the whole joined sequence survives.
  Truncated cases no longer show a stray joined-symbol fragment or a silently
  de-accented final character.

Independently reran `cargo test --locked --test batch_commands --test batch_report_ui`:
**15 helper tests and 11 report UI tests passed, 0 failed**. The tests cover both
command footer titles, narrow/wide/narrow resizing, complete counters, stale
pointer rejection, exact Close cells, both reported Unicode boundaries and both
snapshot paths. This recheck created no temporary directory, background process,
or disposable artifact; normal project target cache was reused.

Both initial MEDIUM findings are closed. Existing standalone scene coverage is
retained above; it was not represented as live App integration evidence. Caller
obligations and the later integration gate remain as listed in this report.

Recap: final corrected PNGs, source and focused tests pass independent UI review.
The standalone delivery-report view is approved for the next integration step.
