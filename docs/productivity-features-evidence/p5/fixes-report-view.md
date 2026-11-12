# P5 delivery-report footer and label fixes

Addressed the independent P3 finding in `correctness-report-view.md` and the
subsequent rendered-UI finding about split Unicode labels. Changes are limited
to `src/batch_report.rs`, `tests/batch_report_ui.rs`, `src/batch_commands.rs`,
`tests/batch_commands.rs`, and this report. Shared integration files were not
edited in this pass.

## Change

Footer help now selects the longest hint that fits the actual terminal cells
remaining after `[Close]`, accounting for the complete range/total counter.
Optional navigation help shortens before the counter can be clipped. Width is
measured with the existing terminal-aware `Span::width` API. The footer and
seven-cell Close region retain their existing geometry; header counts, the
Flutter-completion caveat, detail wrapping, scroll bounds, and dismissal/event
handling are unchanged.

The maximum valid eight-target fixture uses 120 wide `界` label scalars and the
fixed Test exclusion per target. At 34×12 it has 103 wrapped detail rows, and End
now shows the full `101-103/103` counter. The same view at 80×24 rewraps to 47
rows and displays `32-47/47` at End. Returning to 34×12 restores the complete
`101-103/103` counter.

The shared `bounded_device_label` helper now removes controls and truncates at
complete grapheme boundaries with an explicit ellipsis inside the existing
120-scalar output budget. Both dispatch snapshots and the defensive report view
use it, so the view never tries to repair a label whose tail was already lost.
The existing Ratatui grapheme iterator avoids a new dependency. Allocation stays
bounded to 121 sanitized scalars: an untruncated value of at most 120 scalars is
preserved, while a truncated value keeps only whole graphemes totaling at most
119 scalars before appending the ellipsis. A grapheme cut by the lookahead cannot
be retained: it extends to the 121st scalar, beyond that 119-scalar budget.

## Verification and review

- `cargo test --locked --test batch_commands --test batch_report_ui` —
  **15 helper tests and 11 report UI tests passed, 0 failed**. All earlier tests
  are retained; this pass adds one helper and two report regressions.
- `cargo clippy --locked --lib --test batch_commands --test batch_report_ui -- -D warnings`
  — passed; no new lint suppressions.
- `rustfmt --edition 2024 --check src/batch_commands.rs src/batch_report.rs tests/batch_commands.rs tests/batch_report_ui.rs`
  — passed after formatting only these four owned files.

The new regression uses actual `TestBackend` cells for both reload and restart
reports at 34×12, then 80×24, then 34×12 again. It checks the complete initial
and final narrow counters, the complete wider final counter, fixed counts and
unconfirmed completion, exact `[Close]` cells, stale click rejection during
resize, rejection of the cell immediately after Close, and successful dismissal
on the last Close cell. Existing keyboard, mouse, tiny-layout, scrolling,
immutable-data, and Unicode-rendering tests also pass.

Precise label-boundary tests check `x × 118 + 🧑‍💻` and
`x × 119 + e + combining acute` become a whole-prefix label with an ellipsis;
the corresponding exact-120-scalar labels retain the complete emoji/accent.
A single oversized combining cluster is dropped whole, and controls are
removed. Actual `TestBackend` page renders at 34×12 and 80×24 compare raw public
reports against dispatch-produced snapshots, checking that both paths agree,
truncation is marked, and no partial joined/combining character survives.

## Retained render evidence

Ran the same focused test command with
`FLIMRU_P5_REPORT_FIX_EVIDENCE_DIR=docs/productivity-features-evidence/p5/report-view-fix-renders`.
The two new regressions write actual `TestBackend` cells to TXT and SVG. Converted
those 20 SVGs to PNG with the installed `rsvg-convert`, retaining all three
formats in `report-view-fix-renders/` (2,446,455 bytes total). Examples:

- [Maximum narrow counter](report-view-fix-renders/maximum-wide-hot-reload-end-34x12.png)
- [Joined label with marked truncation](report-view-fix-renders/unicode-joined-cut-public-end-34x12.png)
- [Complete boundary emoji](report-view-fix-renders/unicode-joined-fit-dispatch-end-80x24.png)
- [Complete boundary accent](report-view-fix-renders/unicode-accent-fit-dispatch-end-80x24.png)

The set includes both commands' maximum-width counters at both sizes, and all
four Unicode boundary cases at both sizes through both the raw-public-report
and dispatch-snapshot paths. Visually inspected the PNGs for the narrow counter,
truncated joined label, and complete boundary accent; the counter is complete,
the omitted tail has an ellipsis, and the accent remains attached.

Reviewed the final scoped change after validation: hint selection cannot alter
scrolling, report contents, header state, modal routing, or hit regions. This
fix does not approve its own correctness or UI gate. The original independent
reviewers must recheck; shared main/UI integration remains outside this pass.

Checks reused the normal project `target/` cache. No task-owned temporary
directory, separate build cache, background process, or disposable output was
created or left behind. The named 2.33 MiB render directory is retained as the
requested review deliverable. Existing retained render evidence was not overwritten.

## Recap

Complete counters now take priority over optional footer help at narrow widths.
Truncated labels keep complete graphemes and show an ellipsis within the scalar
bound. All 26 focused tests, strict scoped Clippy, and scoped formatting pass.
Source is stable for independent re-review; no commit or gate approval was made.
