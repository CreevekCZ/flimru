# P3 standalone search view independent review

Scope: `src/console_search.rs`, `tests/console_search_ui.rs`, standalone build
evidence and the separate simplification disposition. The previously reviewed
snapshot/review foundation is unchanged. No shared App integration is assessed
here. Only this report was added; implementation and tests were not edited.

## Finding requiring a fix

### P2 — Clip match coordinates to the visible row before enumerating them

At `src/console_search.rs:677-680`, `render_rows` obtains cell ranges for an
entire matching byte range, then filters that iterator down to `row_index`.
This runs once for every candidate match on every visible row. For overlapping
long queries spanning narrow soft-wrapped rows, each candidate unnecessarily
enumerates all the other rows in the match. The sorted match-byte clipping above
this loop limits candidate matches, but does not limit this coordinate traversal.

An independent scratch harness imported the actual implementation, created a
24-row vt100 screen with history capacity 20,000 and 12,000 repeated `a` cells,
captured it, searched for 512 repeated `a` characters and waited for completion.
After drawing once to establish the 120×40 viewport, it entered reading mode and
sent PageDown until the view was near byte 5,000. Timing one subsequent actual
TestBackend render produced:

| Captured physical columns | Top physical row | Stored matches | One debug frame |
| --- | --- | --- | --- |
| 1 | 4,998 | 10,000 | 410.5 ms |
| 32 | 136 | 10,000 | 19.7 ms |
| 120 | 34 | 10,000 | 12.4 ms |

These machine-specific debug measurements are not release benchmarks. The
width-one case demonstrates the avoidable scaling directly; width 32 is also
representative of a compact console. Rendering runs synchronously, so this work
can delay the next input/PTY-drain iteration when integrated. This is not a
capture/search worker issue.

Suggested correction: intersect `found.bytes` with `row.bytes` before calling
`cell_ranges`, retaining the original match enumeration index for current-match
styling. The byte/cell mapping already expands a partial combining-character
intersection to its owning physical cell. This avoids traversing unrelated
wrapped rows while preserving selection and highlight behavior.

Regression suggestion: include a long overlapping query over many narrow
soft-wrapped rows, scroll into the middle, and verify both current and secondary
highlights. Add the same shape to the explicit rendering measurement. Avoid a
machine-dependent elapsed-time assertion; the implementation should make the
row-bounded traversal evident, and the measurement should document the result.
The existing no-query wide-row measurement does not cover this workload.

## Other reviewed behavior

No additional actionable correctness finding in this standalone scope:

- Capture releases prior UI review data, preserves query/filter/editing for the
  same stable ID, and resets those controls for a new source. Refresh accepts
  updated capture-time metadata only for the same ID. Poll compares complete
  expected tickets and source IDs before replacing the view.
- Close drops the UI Arc, invalidates expected work and clears the owned worker.
  `snapshot()` shares the exact full immutable capture even when severity hides
  lines; pending query changes do not mutate its text or metadata.
- All remaining events are consumed while active, including key releases,
  modified keys, reading-mode paste and mouse events. Enter stops editing and
  Escape stops editing before a subsequent Escape resumes live. Effects request
  refresh/resume without sending the original event elsewhere.
- Query input is bounded in Unicode scalar values and removes control characters;
  Backspace removes a complete scalar. Rendering derives whole-cell byte
  boundaries through the snapshot mapping, avoiding UTF-8 slicing at arbitrary
  bytes. Combining-only matches retain their base cell, partial wide cells at the
  viewport edges are omitted, and complete wide glyphs style both cells.
- Match candidates are sorted and restricted to the visible byte interval;
  overlapping highlight ranges merge while the current match keeps distinct
  styling. Horizontal reveal uses captured historical coordinates, and filtered
  row navigation maps snapshot rows back to displayed rows. Resize clamps both
  offsets and reveals the current match when a viewport is available.
- Pending, error, empty snapshot, empty filter and no-match states remain
  distinct. Layout dimensions use saturating/bounded arithmetic for small areas.

## Validation

`cargo test --locked --test console_search_ui -- --include-ignored --nocapture`
passed **12 tests**, including the existing explicit viewport measurement and
the render fixture generator's 36 layouts. The unchanged wide-row fixture
rendered ten 120×40 frames from 1,200,000 bytes at 60,000 captured columns in
31.3 ms total on this run. This passing case does not negate the finding above.

The additional scratch timing harness passed and exposed the overlapping-query
scaling issue using production code and actual TestBackend drawing. It used a
task-owned `TemporaryDirectory`, removed automatically after compilation and
execution. Normal reusable `target` dependencies were used. No scratch cache,
background process or disposable output remains.

## Caller obligations for integration

- Use stable RunIds rather than vector indices. Transfer review deliberately
  through live-to-saved transitions, and clear deliberately on explicit source
  changes/close; never let another source reuse an existing review identity.
- Apply the historical-width-aware clone preflight before constructing every
  screen clone, retaining the lifetime width bound through saved-console transfer.
  Provide only display labels and correctly refreshed capture metadata.
- Route intentional global quit/modal/toolbar/tab/process actions first, then
  delegate every remaining event to active review before live selection, copy or
  PTY fallback. Consume refresh/resume effects without redispatching the original
  key. Keep explicit process Actions on direct helpers rather than recursive key
  synthesis, and avoid advertising process shortcuts owned by review.
- Poll worker updates on the ordinary UI tick, keep every live/hidden PTY draining,
  supply the documented review rectangle below toolbar/tabs, and preserve snapshot
  Arc/metadata ownership for export. A standalone passing input test cannot prove
  this shared event precedence or live lifecycle behavior.

Recap: one scoped P2 rendering-performance issue requires correction; 12 existing
tests passed. Standalone view approval is pending that fix and targeted recheck.
Full P3 approval additionally requires the later shared App integration review,
rendered integration acceptance and real-binary PTY checks.

## Independent fix recheck — finding closed

Rechecked the separate fix at `src/console_search.rs:677-681`, both new
regressions in `tests/console_search_ui.rs` and `fixes-search-view.md`. The prior
pending disposition above is superseded by this recheck: **the standalone P2
finding is closed, with no residual scoped finding.**

The implementation intersects each match's byte range with the current physical
row before enumerating coordinates. Candidate filtering already ensures that
these ranges overlap. Mapping therefore visits only this row and still expands
partial combining-mark matches to complete owning cells. Original `enumerate`
indices remain intact, so clipping does not renumber the current match or change
highlight selection. No source or test edits were made during this recheck.

Independently reran
`cargo test --locked --test console_search_ui -- --include-ignored --nocapture`:
**14 passed, zero failures**, including the new long-overlap current/secondary
output checks, wrapped combining/wide boundary checks and all existing layouts.
The new regression visits original match index 5,000 after skipping many earlier
candidates and verifies actual visible text, backgrounds and bold styling.

Observed corrected debug times for ten frames were 48.8 ms at retained width 1,
53.3 ms at width 32 and 73.5 ms at width 120. The explicit measurement repeated
those shapes at 48.4, 56.0 and 75.5 ms respectively; its original wide-row/no-query
fixture took 19.6 ms for ten frames. The former width-one probe required 410.5 ms
for one frame. These results and the narrowed traversal support closing the
reported scaling defect without claiming universal frame-time guarantees.

This recheck used the normal reusable project cache and created no temporary
directory or background process. The previously stated caller obligations still
apply. **Standalone foundation/view correctness review is complete; full P3 App
integration, real PTY routing/lifecycle acceptance and integrated rendered review
remain pending.**

Recap: row-bounded coordinate mapping verified; the P2 finding is closed;
fourteen search-view tests passed; full P3 approval is still outside this scope.

## Independent frozen-source/UI delta recheck

Reviewed the held `console_search.rs` changes, focused tests and
`fixes-source-ui.md` before shared App handoff. **No actionable scoped correctness
finding.** The standalone correctness disposition remains approved within scope;
the full P3 App gate remains open.

- The new title clipping at `console_search.rs:995` measures complete graphemes
  in terminal cells and reserves room for an explicit ellipsis. It never slices
  UTF-8 at arbitrary bytes or mutates the stored identity while drawing. Stored
  metadata retains the existing scalar budgets: 96 for the label and 128 per
  optional field. Shortening now includes a bounded ellipsis and an explicit
  details notice; metadata control characters remain stripped.
- Details derive ID, labels, outcome and capture timestamp exclusively from the
  captured `SearchSource`. They do not consult current tab/session state. The
  additional wrapping/scrolling allocation is bounded by these small metadata
  limits. Complete-grapheme wrapping and the same line list govern scroll bounds
  and actual drawing; oversized graphemes in exceptionally narrow areas use a
  visible marker. The 34×9 supported review area retains the return hint.
- Details use independent offsets and viewport state. Opening, scrolling and
  returning do not clear query/filter/current-match/log-offset state or replace
  the immutable snapshot Arc. Escape, Enter and unmodified `i` return to the
  previous editing/reading mode while leaving review active. Every details event
  is consumed, including paste, key releases and unsupported shortcuts. Editing
  `i` remains literal outside details.
- Public edit/previous/filter/source actions call the existing state transitions
  directly, without synthetic query input. Inactive guards remain effective.
  Capture/refresh/clear reset details deliberately; generation and worker
  publication handling are unchanged. Caller-owned Actions precedence and real
  PTY isolation still require integration checks.
- The earlier rendering fix remains intact at `console_search.rs:870-874`:
  match bytes are intersected with the physical row before coordinate traversal,
  and original match indices still select the current highlight. Singular row
  grammar is now explicit without changing row counts.

Validation: the requested focused Cargo invocation encountered a transient
concurrent App build error in `main.rs:2428` because its `MouseAction` match did
not yet cover the new search actions. This was outside the held standalone
files and was not modified or counted as a passing Cargo command. After verifying
that the compiled focused harness postdated both scoped source/test files, ran
`target/debug/deps/console_search_ui-b408ed45ad9073a3 --include-ignored --nocapture`
directly: **17 passed, zero failures**, including the 56 fixture layouts and new
source-details/control assertions. This checks actual rendered buffers and
frozen metadata, not the final integrated App event path.

The long-overlap regressions still passed. Ten regular frames measured
51.7/53.3/75.0 ms at retained widths 1/32/120; the explicit measurement repeated
those shapes at 70.4/54.3/74.0 ms, and the original wide-row/no-query case took
19.5 ms. These machine-specific debug figures preserve the previous finding's
closure. This recheck created no temporary directory, subprocess server or
disposable output and left implementation/tests unchanged.

Recap: frozen-source identity/details and direct Actions API reviewed; no new
scoped issue; seventeen focused tests passed through the current harness.
Shared App wiring, integrated rendering and actual PTY lifecycle/input acceptance
remain required before full P3 approval.
