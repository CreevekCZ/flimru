# P3 frozen source identity and singular-row fixes

The two standalone findings in `ui-search-view.md` are addressed in
`src/console_search.rs` and its focused UI tests. Shared App/UI/Run/Session files
and the reviewed snapshot/review helpers were not edited by this fix.

## Result and API handoff

- Border titles use a cell-width-aware, complete-grapheme ellipsis when the
  frozen source does not fit. Rendering never replaces or further truncates the
  stored metadata. Existing sanitization and scalar budgets remain: 96 label
  characters, 128 for each other supplied display field, including the explicit
  ellipsis when shortening is necessary.
- Reading `i` opens **Frozen source details**, advertised on the bottom border.
  Editing `i` remains literal query text. An explicit Actions call opens details
  in either mode. Details show the stable source ID, bounded label, device,
  configuration, outcome and capture time in Unix UTC seconds. Extreme values
  have a clear shortening notice and omitted-character marker. No current-tab
  lookup, launch arguments or environment are involved.
- Details wrap by complete graphemes, scroll with arrows/PageUp/PageDown,
  Home/End and mouse wheel, and retain a visible `Esc back` hint at 34×9.
  Escape, Enter or unmodified `i` returns to the previous review mode. All
  details events are consumed, including paste and unsupported shortcuts.
  Query, filter, current match, frozen snapshot Arc and review offsets remain
  unchanged when opening, scrolling and closing the details.
- Status grammar now distinguishes `1 row` from plural rows.

Direct public Actions API, sharing the same implementations as ordinary keys:

```rust
view.edit_query();
view.next_match(false);      // next; existing boolean API remains compatible
view.previous_match();      // equivalent to existing next_match(true)
view.cycle_filter();         // existing implementation is now public
view.set_filter(filter);     // existing specific-filter action
view.show_source_details();
view.showing_source_details(); // query details visibility
view.refresh(source, screen);  // existing validated same-source capture
view.close();                 // existing explicit resume/close operation
```

These actions require no synthesized `n`, `f`, `r` or `/` query keystrokes.
App callers still own deliberate global/menu/modal precedence, capture preflight
and refresh/resume handling. Standalone fixes do not establish integrated PTY or
menu isolation.

## Validation and rendered evidence

```sh
rustfmt --edition 2024 src/console_search.rs tests/console_search_ui.rs
FLIMRU_P3_EVIDENCE_DIR=docs/productivity-features-evidence/p3/search-view-source-ui cargo test --locked --test console_search_ui -- --include-ignored --nocapture
cargo clippy --locked --test console_search_ui -- -D warnings
rustfmt --edition 2024 --check src/console_search.rs tests/console_search_ui.rs
```

- **17 tests passed**, zero failures, including ignored explicit measurements;
  final test execution **0.64 s**. All previous fourteen tests remain and three
  source-identity/control/render tests were added.
- Strict scoped Clippy passed, **0.36 s**. Scoped formatting passed; no
  whole-repository formatter was run.
- A final focused rerun of
  `cargo test --locked --test console_search_ui frozen_source_details_are_scrollable_and_preserve_review_mode_and_position -- --exact`
  also passed after adding the explicit preserved-filter assertion. Its normal
  binary build reported three unused-item warnings in concurrently edited
  `console_style.rs`/`runs.rs`; these are outside this fix's ownership. No
  search-view warning was emitted.
- Regression tests cover readable complete bounded multi-byte identity across
  scrolled 34×9 pages; control stripping and 96/128-character ellipsis bounds;
  literal editing input; direct next/previous/filter/edit/source actions;
  source capture time; exact query/match/filter/offset/snapshot ownership; and
  return to an identical prior TestBackend buffer. Details also render safely
  across 1–11 columns and 1–9 rows.
- Long-overlap performance/cell regressions remain passing. Final regular ten
  frame timings at captured widths 1/32/120 were **54.891584 / 52.170000 /
  71.513208 ms**. Explicit measurement: **47.850750 / 51.557667 / 73.552083 ms**.
  Existing wide-row/no-query case: **18.429292 ms** for ten frames. The generous
  two-second loaded-debug bound passes; these local timings are not release
  guarantees.

`search-view-source-ui/` retains **56 actual TestBackend TXT/SVG/PNG layouts**:
the original 36 states regenerated for the ellipsis, source hint and singular
copy, plus twenty long-source reading/editing/details-top/page/end layouts at
34×12, 80×24, 120×40 and an actual 34×9 review rectangle under toolbar/tab
placeholders. PNGs were rasterized from each corresponding actual-cell SVG with
`rsvg-convert`, using four bounded concurrent subprocesses that all exited.
These new fixtures supersede the earlier presentation evidence where wording or
title overflow changed; the earlier review images were preserved.

Visual self-review opened these ten PNGs with the image-viewing tool:

- `long-source-reading-34x9-integration.png`
- `long-source-reading-80x24.png`
- `long-source-reading-120x40.png`
- `long-source-editing-34x12.png`
- `source-details-top-34x9-integration.png`
- `source-details-page-34x9-integration.png`
- `source-details-end-34x9-integration.png`
- `source-details-top-80x24.png`
- `source-details-top-120x40.png`
- `empty-snapshot-34x9-integration.png`

Self-review shortened the notice and reordered the capture-time wording so each
reads clearly at compact widths. The 34×9 final pages show the complete bounded
identity through scrolling, the retained outcome and capture time at the end,
and a persistent return hint. No further standalone issue was found in this
inspection; original independent correctness/UI rechecks remain pending.

## Resource ownership and scope

No task-owned temporary directory, server, PTY, language server or background
process was created. Tests used the normal reusable `target` cache, measured
at **4.4 GB** and retained for ongoing project builds. The 168
retained evidence files total **9,541,313 bytes**; these are requested review
deliverables, not disposable build output. Other agents' sources and artifacts
were left in place.

Recap: source overflow is explicit, full bounded frozen details are accessible,
singular grammar is fixed, direct Actions methods are ready, and seventeen tests
plus strict scoped checks pass. Stable source is handed back for the original
reviewers; whole P3 App acceptance remains outside this standalone fix.
