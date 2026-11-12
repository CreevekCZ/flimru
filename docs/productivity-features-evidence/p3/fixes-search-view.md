# P3 standalone search-view rendering fix

The P2 finding in `correctness-search-view.md` is corrected locally at
`src/console_search.rs:677–681`. Each candidate match is intersected with the
current `SnapshotRow.bytes` before `cell_ranges` enumerates coordinates. The
existing original enumeration index still selects current-match styling. No
helper API, snapshot/review implementation, budgets, cell map, shared App source,
or event/lifecycle behavior changed.

The intersection selects the same row's cells as the former full-match mapping
followed by a row filter. Partial combining-character intersections still pass
through the existing mapping, which expands them to the complete owning cell.
The code no longer traverses unrelated wrapped rows for every candidate.

## Regression and review

Two additional tests preserve all twelve existing tests:

- A real 120×40 TestBackend draws 12,000 repeated ASCII cells, a 512-character
  overlapping query and 10,000 stored matches at captured widths 1, 32 and 120.
  It scrolls near byte 5,000, verifies every visible secondary highlight, then
  navigates to original match index 5,000 and verifies visible current/secondary
  backgrounds, bold state and text. This also exercises horizontal reveal at
  width 120. Ten timed draws have a deliberately loose two-second debug bound
  per width, roughly 27–40 times this run's measured work.
- A width-three history repeats `e\u{301}界`; its overlapping query begins
  inside a combining-character cell and ends at a combining mark over 100 rows
  later. Assertions inspect both boundaries, the complete base-plus-mark cell,
  both wide-glyph cells, and distinct current/secondary styles.

The existing ignored viewport measurement retains its 1.2 MB / 60,000-column
case and now measures the overlapping-query workload too. During self-review,
new test expectations were corrected to account for the normal-width query's
horizontal reveal and to keep the secondary-only inspection before the capped
match coverage ends. Final source and assertions were reviewed after those
corrections; no additional standalone implementation issue was found.

## Exact validation

All commands used the normal reusable project `target` cache:

```sh
rustfmt --edition 2024 src/console_search.rs tests/console_search_ui.rs
cargo test --locked --test console_search_ui -- --include-ignored --nocapture
cargo clippy --locked --test console_search_ui -- -D warnings
rustfmt --edition 2024 --check src/console_search.rs tests/console_search_ui.rs
cargo test --locked --test console_snapshot --test console_review
```

- Search-view tests: **14 passed**, zero failures, including the explicit
  measurement and existing 36 render layouts; final run took **0.65 s**.
- Strict scoped Clippy: passed, **0.99 s** including build-directory lock wait.
- Scoped formatting: passed. No whole-repository formatting was run.
- Unchanged review foundation: **7 passed** in **0.01 s**.
- Unchanged snapshot foundation: **17 passed**, two existing heavy measurements
  ignored, in **0.03 s**. This includes budgets, cell mapping, Unicode expansion,
  historical widths and overlap/cap checks.

Final debug timing from that search-view test invocation (ten actual frames):

| Captured columns | Top physical row | Regular regression | Explicit measurement |
| --- | --- | --- | --- |
| 1 | 5,000 | 49.650917 ms | 56.691667 ms |
| 32 | 156 | 52.692542 ms | 53.383667 ms |
| 120 | 41 | 73.301291 ms | 74.106291 ms |

The unchanged wide-row/no-query measurement took **19.039916 ms** for ten
120×40 frames. These debug timings are machine-specific evidence, not release
benchmarks. The independent pre-fix width-one measurement was **410.5 ms for
one frame**, at top row 4,998; the corrected measurement is near the same byte
position and uses ten frames.

No render fixtures were regenerated: this correction changes coordinate work,
not the selected cells, styles, text or layout. The unchanged fixture cases all
executed successfully, and the added assertions directly inspect the affected
long-overlap and Unicode output.

No task-owned temporary directory, disposable output, background process or
language server was created. All invoked checks exited. The existing shared
`target` directory measured **4.4 GB** and remains the reusable project cache;
it was not deleted or treated as this fix's disposable output.

Recap: local row-bounded mapping is complete; fourteen search-view tests, strict
Clippy, scoped formatting and twenty-four foundation tests pass. Source is held
for the original independent reviewer's recheck. Shared App integration remains
outside this standalone fix's approval scope.
