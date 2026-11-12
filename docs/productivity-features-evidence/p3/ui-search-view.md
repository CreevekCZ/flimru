# P3 standalone SearchView rendered UI review

## Scope and verdict

Independent read-only review of `src/console_search.rs` and its actual rendered
TestBackend cells, after the standalone simplification and row-bounded highlight
fix. This review does **not approve whole P3**. App event precedence, real PTY
isolation, toolbar/tab interactions, live-to-ended session wiring, menus/modals,
and final integrated metadata presentation remain outside this assignment.

**Standalone verdict: Approve the inspected layout/control behavior, with the
MEDIUM source-label and LOW copy findings below still requiring follow-up.** No
HIGH finding was established in the inspected standalone controls. This verdict
does not clear the complete integrated feature's UI gate.

Stack: Rust 2024, Ratatui cell layouts, crossterm input, vt100 captured history.
The established fixed-cell terminal density and RGB palette were retained as
project conventions. Recon included the supplied AGENTS instructions,
`CONTRIBUTING.md`, the P3 workflow requirements, `search-view/build-evidence.md`,
`simplification-search-view.md`, and `fixes-search-view.md`. Review used the
better-interface domain guidance where applicable to a terminal UI; browser DOM,
CSS, touch-target and browser reflow prescriptions are not TUI claims.

## Findings

| Severity | Domain | Location and rendered evidence | Before | Actionable follow-up | Why |
| --- | --- | --- | --- | --- | --- |
| MEDIUM | Layout / source identity | `src/console_search.rs:470–473`; `search-view-review/long-source-query-reading-34x12.png` and `long-source-query-reading-80x24.png` | A frozen source label is silently cut by the border title. At 34 columns the visible title ends after `Physical Pixel 界 ·`; at 80 it stops within the configuration. This view exposes no source-detail action. | Make title overflow explicit and provide a keyboard-reachable way to inspect the full **bounded, sanitized** frozen label/metadata, or an equivalent unambiguous source identifier. Verify this in the later integrated long-source fixture at 34×9. | The device/configuration suffix that distinguishes captured consoles can disappear while review remains active. This concerns source identification, not access to log rows, query text or controls. The existing metadata length/sanitization budget is deliberate and should remain bounded; this finding does not request unbounded raw labels. |
| LOW | Writing | `src/console_search.rs:565–568`; every `search-view/empty-snapshot-*.png` | Empty captured history reports `All · 1 rows · col 1 ↔` above `Snapshot is empty.` | Render `1 row` for a singular physical row. | Isolated grammar issue. The empty-state explanation is otherwise clear and the retained empty terminal row is consistent with the snapshot model. |

## Exact PNG inventory inspected

Every filename below was opened with `tools.view_image`; conclusions were not
inferred from TXT/SVG or source alone. Paths are relative to this report's
directory.

| State | 34×12 | 80×24 | 120×40 | Actual 34×9 review region under placeholders |
| --- | --- | --- | --- | --- |
| Editing | `search-view/editing-34x12.png` | `search-view/editing-80x24.png` | `search-view/editing-120x40.png` | `search-view/editing-34x9-integration.png` |
| Reading | `search-view/reading-34x12.png` | `search-view/reading-80x24.png` | `search-view/reading-120x40.png` | `search-view/reading-34x9-integration.png` |
| No matches | `search-view/no-matches-34x12.png` | `search-view/no-matches-80x24.png` | `search-view/no-matches-120x40.png` | `search-view/no-matches-34x9-integration.png` |
| Empty filter | `search-view/empty-filter-34x12.png` | `search-view/empty-filter-80x24.png` | `search-view/empty-filter-120x40.png` | `search-view/empty-filter-34x9-integration.png` |
| Empty snapshot | `search-view/empty-snapshot-34x12.png` | `search-view/empty-snapshot-80x24.png` | `search-view/empty-snapshot-120x40.png` | `search-view/empty-snapshot-34x9-integration.png` |
| Capture pending | `search-view/pending-34x12.png` | `search-view/pending-80x24.png` | `search-view/pending-120x40.png` | `search-view/pending-34x9-integration.png` |
| Error | `search-view/error-34x12.png` | `search-view/error-80x24.png` | `search-view/error-120x40.png` | `search-view/error-34x9-integration.png` |
| Wide history / combining-only match | `search-view/wide-highlight-34x12.png` | `search-view/wide-highlight-80x24.png` | `search-view/wide-highlight-120x40.png` | `search-view/wide-highlight-34x9-integration.png` |
| Capped results | `search-view/capped-34x12.png` | `search-view/capped-80x24.png` | `search-view/capped-120x40.png` | `search-view/capped-34x9-integration.png` |

The original 36 images use the short fixed label `Pixel 界 · development`.
To cover the requested long-source boundary without changing product/test files,
this review additionally generated and inspected:

- `search-view-review/long-source-query-editing-34x12.png`
- `search-view-review/long-source-query-reading-34x12.png`
- `search-view-review/long-source-query-editing-80x24.png`
- `search-view-review/long-source-query-reading-80x24.png`
- `search-view-review/long-source-query-editing-120x40.png`
- `search-view-review/long-source-query-reading-120x40.png`
- `search-view-review/filter-errors-34x9.png`
- `search-view-review/filter-warnings-34x9.png`
- `search-view-review/wide-cell-active-34x12.png`

Supplemental frames use the existing `write_evidence` helper and actual completed
TestBackend frames. A temporary copy of the existing test module included the
unchanged actual `console_search.rs` by absolute path, plus one supplemental test.
It was compiled with `rustc --edition 2024 --test` against the current project
dependency rlibs. Only `supplemental_review_frames --exact --nocapture` ran.
SVGs were rasterized by `rsvg-convert`. Long-source input was
`Physical Pixel 界 · ` + four repetitions of `long configuration segment ` +
` · ended success`; the query was 506 `a` characters followed by `needle`.
Populated Errors/Warnings filters used `ERROR first request`, `WARN second request`
and `INFO third request`; the wide-cell image searched `界` in `İ i 界 é`.

## Coverage and observations

| Domain / concern | Evidence and observed result | Result |
| --- | --- | --- |
| Keyboard accessibility and mode clarity | Editing uses `Search ▸` and a positioned terminal cursor; reading uses `Search:`. Every inspected editing render shows `Enter read · Esc stop editing`; every reading render shows `/ edit` and `Esc resume live`. Input tests cover two-stage Escape and consumed mouse/paste/control keys. | Clear for standalone input; terminal screen-reader behavior and integrated focus/modal routing not verified. |
| Layout / minimum dimensions | All nine states at all four requested areas. The 34×9 body has three rows; both header rows and both hint rows survive. The sample error wraps into exactly three body rows without covering hints. | Clear for inspected controls; source-label finding above. |
| Writing / state distinctions | `No matches` leaves captured rows visible; `No lines in this filter.` replaces a severity-empty body; `Snapshot is empty.` differs from `Capturing…` and `Review unavailable`. Error names the 128 MiB limit and says to refresh after reducing retained history. | Clear except singular-row finding. |
| Typography / long queries | Supplemental 512-character query retains its `needle` tail while editing; reading shows its start, and `/ edit` provides a route back to editing the query. Terminal monospace numbers keep count positions stable. | Clear for the inspected terminal rendering. Actual terminal font/zoom and screen-reader pronunciation not verified. |
| Color | Palette inspected in `src/console_search.rs:21–26` and verified against PNG fills. Computed WCAG sRGB contrast: TEXT/PANEL 13.80:1; MUTED/PANEL 6.68:1; ACCENT/PANEL (and reverse current-match pair) 9.60:1; ERROR/PANEL 7.10:1; TEXT/MATCH 8.88:1. Mode, filter, count and error meaning have text cues. Current match is also bold. | Clear for actual inspected pairs; user terminal color transformation not verified. |
| Match hierarchy and caps | Reading/editing PNGs show active `request` with bright background and secondary match with subdued background. `1/10000+ capped` remains legible even at 34 columns, including the 34×9 image. Populated Errors and Warnings supplemental images each show `1/1` and the expected sole row. | Clear. |
| Historical width / horizontal reveal | Wide-history frames show `col 72 ↔` at 34 columns, `col 26 ↔` at 80, `col 1 ↔` at 120. Same captured match is revealed at the right side in each viewport, with no claim of reflow. Focused tests verify Left/Right and resize bounds. | Clear for inspected reveal/pan behavior. |
| Unicode complete-cell highlighting | Existing wide-history PNGs visibly highlight the complete `é` cell for a combining-mark-only query and show `◌́` in the query. Supplemental wide-cell PNG visibly fills both cells of `界`. Automated assertions additionally inspect both wide-cell backgrounds, partial-wide clipping, lowercase expansion and long wrapped combining boundaries. | Clear for tested cell/style mapping. |
| UI polish | Consistent one-cell border, aligned query/status/body/footer zones, no overlap in any inspected state; no animation in this component. | Clear. |
| Frozen source metadata | Tests establish stable source identity, preserved capture metadata and refresh behavior; normalization test strips controls and enforces the 96-character label budget. Supplemental images test a genuinely long label. | Rendering finding above; full App metadata presentation remains for integration. |

## Verification and resource ownership

- `cargo test --locked --test console_search_ui -- --include-ignored --nocapture`:
  **14 passed, 0 failed, 0 ignored**, 0.62 s test execution. This includes all
  existing fixture-state assertions and the new overlapping-query regression.
- Explicit measurement in that invocation: ten 120×40 wide-history/no-query
  frames took **19.50 ms**. Ten overlapping frames at widths 1/32/120 took
  **48.36 / 51.28 / 70.85 ms** in the explicit measurement. These are local debug
  fixture observations, not release latency guarantees.
- Supplemental test: **1 passed**, 0.05 s; all nine supplemental PNGs inspected.
- Computed all five foreground/background pairs above from the actual declared
  RGB values using WCAG relative luminance; none falls below 4.5:1.
- Product source, repository test source, main/UI files and export dialog were
  not edited. No subdelegation, app launch, PTY process, server or language server.
- The supplemental harness and binary were task-owned in a unique system
  temporary directory; **3,008,837 bytes** were measured before automatic scope
  cleanup after child processes exited. No disposable directory remains.
- Supplemental PNG/SVG/TXT deliverables in `search-view-review/` are retained
  evidence: **1,377,434 bytes** (about 1.4 MiB on disk). Normal tests used the
  reusable project `target` cache; it was not deleted.

## Integration follow-up

Resolve the long-source presentation finding, then render the real integrated
34×9 area with long device/configuration/outcome labels. Recheck actual App
source capture and same-session completion, intentional tab/process Actions,
menu/modal precedence, Escape consumption, selection/copy isolation, and a real
binary PTY before approving whole P3. The placeholders in the supplied 34×9
fixtures do not establish those behaviors.

> Recap: 45 actual PNGs inspected; 14 existing tests and one review-only fixture
> test passed. Compact controls, state wording, contrast and Unicode cells passed
> the scoped review. Two follow-ups remain: long-source identity presentation and
> singular-row copy. Full P3 App integration remains unapproved.

## Independent source-UI fix recheck

This appended recheck supersedes the earlier open-finding status and conditional
standalone verdict. Read `fixes-source-ui.md`, the changed source presentation and
input paths, and the exact tests. Product and test source were not edited during
this recheck.

**Verdict: Approve the standalone SearchView UI. Both original findings are
resolved; no actionable residual was found in this recheck. Whole P3 App
integration remains unapproved.**

### Resolution evidence

- Source titles at `src/console_search.rs:567–573` use `clipped_title`, whose
  implementation at `src/console_search.rs:995` walks complete graphemes and
  reserves a cell for an explicit ellipsis. The long-source PNGs at 34, 80 and
  120 columns visibly disclose clipping/normalization without corrupting the
  wide glyph or border. The reading bottom border advertises `i source`, while
  both normal footer hint lines remain intact even at 34×9.
- Reading `i` at `src/console_search.rs:433` and the explicit
  `show_source_details` API at `src/console_search.rs:141` open the frozen source
  details. Editing `i` stays literal via the existing query-character handler.
  Source fields come from the stored `SearchSource` at
  `src/console_search.rs:752–807`; no live tab lookup is used. Details show source
  ID, label, device, configuration, outcome and capture time.
- The 34×9 details top/page/end PNGs show an independent scrollable body and
  persistent `↑↓ PgUp/PgDn · Esc back`. The top page exposes the 96/128 budget
  notice and explains the ellipsis; the end page shows outcome and the complete
  `Captured: Unix UTC 1700000000 s` value. At 80×24 and 120×40 all fields fit,
  so PageDown/End correctly leave the same complete presentation visible.
- `src/console_search.rs:50–62` and `:985–994` preserve the existing sanitized
  scalar budgets, marking stored shortening with an ellipsis and a details
  notice. Display clipping does not mutate the stored label. This is access to
  the full **bounded** metadata, with explicit notice for omitted extreme input.
- Source event handling at `src/console_search.rs:455–500` consumes details input
  and returns on Escape, Enter or unmodified `i`; scroll state is separate from
  the review viewport. The regression test at
  `tests/console_search_ui.rs:356` scrolls through all bounded fields at 34×9,
  then checks the same query, filter, current match, review offsets, snapshot Arc,
  capture time and exact pre-details rendered buffer after returning. The test
  at `tests/console_search_ui.rs:466` checks literal editing `i`, direct actions,
  extreme metadata bounds/notices and Enter returning to the editing mode.
- The singular branch at `src/console_search.rs:667` now renders `1 row`.
  It is visibly corrected in all four newly inspected empty-snapshot sizes.

### Exact new PNG inventory inspected

Opened these **32** actual PNGs with `tools.view_image`, from the fixer's
56-layout `search-view-source-ui/` evidence set. The other 24 regenerated
ordinary-size layouts were not opened in this recheck; the original complete
state review above and current fixture assertions remain separate evidence.

| State | 34×12 | 80×24 | 120×40 | 34×9 review area |
| --- | --- | --- | --- | --- |
| Long source editing | `search-view-source-ui/long-source-editing-34x12.png` | `search-view-source-ui/long-source-editing-80x24.png` | `search-view-source-ui/long-source-editing-120x40.png` | `search-view-source-ui/long-source-editing-34x9-integration.png` |
| Long source reading | `search-view-source-ui/long-source-reading-34x12.png` | `search-view-source-ui/long-source-reading-80x24.png` | `search-view-source-ui/long-source-reading-120x40.png` | `search-view-source-ui/long-source-reading-34x9-integration.png` |
| Source details top | `search-view-source-ui/source-details-top-34x12.png` | `search-view-source-ui/source-details-top-80x24.png` | `search-view-source-ui/source-details-top-120x40.png` | `search-view-source-ui/source-details-top-34x9-integration.png` |
| Source details page | `search-view-source-ui/source-details-page-34x12.png` | `search-view-source-ui/source-details-page-80x24.png` | `search-view-source-ui/source-details-page-120x40.png` | `search-view-source-ui/source-details-page-34x9-integration.png` |
| Source details end | `search-view-source-ui/source-details-end-34x12.png` | `search-view-source-ui/source-details-end-80x24.png` | `search-view-source-ui/source-details-end-120x40.png` | `search-view-source-ui/source-details-end-34x9-integration.png` |
| Empty snapshot / singular row | `search-view-source-ui/empty-snapshot-34x12.png` | `search-view-source-ui/empty-snapshot-80x24.png` | `search-view-source-ui/empty-snapshot-120x40.png` | `search-view-source-ui/empty-snapshot-34x9-integration.png` |

Also inspected all other regenerated minimum-area states:

- `search-view-source-ui/editing-34x9-integration.png`
- `search-view-source-ui/reading-34x9-integration.png`
- `search-view-source-ui/no-matches-34x9-integration.png`
- `search-view-source-ui/empty-filter-34x9-integration.png`
- `search-view-source-ui/pending-34x9-integration.png`
- `search-view-source-ui/error-34x9-integration.png`
- `search-view-source-ui/wide-highlight-34x9-integration.png`
- `search-view-source-ui/capped-34x9-integration.png`

The minimum-area rerenders preserve the three log rows, editing/reading Escape
semantics, no-match/filter/empty/pending/error distinction, readable error
recovery, `1/10000+ capped`, dotted-circle combining-query cue and complete-cell
highlight. Adding the source hint to the border does not displace these controls.
The palette is unchanged from the measured pairs above.

### Independent focused verification and remaining boundary

Compiled a fresh temporary copy of the **current unmodified**
`tests/console_search_ui.rs`, changing only its module path to the absolute
current `src/console_search.rs` path. Command shape:

```text
rustc --edition 2024 --test <owned-temp>/review.rs \
  -L dependency=<project>/target/debug/deps \
  --extern flimru=<existing-project-rlib> \
  --extern crossterm=<existing-project-rlib> \
  --extern ratatui=<existing-project-rlib> \
  --extern vt100=<existing-project-rlib> -o <owned-temp>/review
<owned-temp>/review --include-ignored --nocapture
```

Dependencies used: `libflimru-dcb6ad8ccfc8eab8.rlib`,
`libcrossterm-e89cffe83455fffa.rlib`, `libratatui-dc3126ece5eb2a9b.rlib`,
`libvt100-0b2058cecb6aba00.rlib`. **17 passed, 0 failed, 0 ignored**, 0.62 s,
including all three new source UI tests and preserved overlap/Unicode tests.
Explicit ten-frame debug measurements: wide-history/no-query **18.61 ms**;
overlap widths 1/32/120 **43.11 / 46.87 / 66.25 ms**. These are local fixture
measurements, not release or full-App guarantees.

This verifies the standalone module and tests against existing dependency
artifacts. It is **not a fresh Cargo/full-App build gate**. The coordinator
reported an expected concurrent shared-App integration compile error; this
recheck did not rebuild or repair shared App sources. Integrated Actions routing,
toolbar/tab/menu/modal precedence, PTY isolation, real source identity wiring,
and live-to-ended lifecycle still require the later integrated review. Terminal
screen-reader behavior remains unverified.

No new durable render output, server, PTY or language server was created by this
recheck. The unique owned temporary harness/binary measured **3,181,156 bytes**
before cleanup; the child test process exited and the directory was removed.
Only this report was appended.

> Recap: independently inspected 32 affected PNGs and passed all 17 current
> standalone tests. Both source-identity and grammar findings are resolved.
> Standalone UI approved; full P3 App/Cargo/PTY/menu/modal gates remain pending.
