# P5 standalone delivery-report correctness review

Reviewed `src/batch_report.rs`, `tests/batch_report_ui.rs`,
`report-view-build.md`, and `simplification-report-view.md` after the separate
simplification pass. This review did not edit product or test sources, repeat
the unchanged helper review, or authorize shared main/UI integration.

## Finding

### P3: Keep the complete position counter visible at the minimum supported size

Location: `src/batch_report.rs:205-213`.

The compact footer assumes that navigation help plus the counter fits in the
space remaining after `[Close]`. At 34×12, the remaining rectangle is only 21
columns. A valid eight-target report with each device label set to
`"界".repeat(120)` and each outcome set to
`Skipped(Unsupported(HotExclusion::Test))` wraps to 103 physical detail rows.
At End, the intended counter `101-103/103` is clipped to `101-103/10`:

```text
│ [Close] Esc ↑↓ Pg 101-103/10 │
```

This makes the displayed total disagree with the current range, despite all
details remaining reachable and the actual scroll bound being correct. The
fixture stays within the application's eight-session and 120-scalar label bounds;
it requires no forged counts or oversized public API input.

Size the compact hint against its actual remaining width, shortening/removing
the help before truncating the position counter. Add a minimum-size maximum-wide-
label regression asserting the complete final counter. Preserve exact Close hit
cells and the fixed header counts/completion caveat.

## Independent validation and observations

- `cargo test --locked --test batch_report_ui`: **9 passed, 0 failed**.
- An external source-path harness exercised the unchanged view with actual
  `TestBackend` rendering across all widths 34–120 and heights 12–40:
  **2,523 layouts**. Each layout rendered the initial view, jumped to End,
  rendered again, rejected further downward movement, and returned to Home.
  No panic or scroll-bound error occurred. The 34×12 final render above
  independently reproduced the footer issue with offset 100 and total 103.
- Inspected retained compact initial/end TXT renders to compare the ordinary
  two-digit pager case with the maximum-width-label reproduction. No retained
  evidence was modified.

The view owns its report and exposes no mutation of its counts/details, session
lookup, send, retry, process, or filesystem API. It renders fixed requested/queued,
skipped, and failed counts with unconfirmed Flutter completion. Per-device error
descriptions come from fixed helper categories. The renderer processes at most
eight details with at most 120 sanitized label scalars each, and bounded fixed
reason strings. Rewrapping these ordinary helper-produced snapshots per render
does not scan live console histories or mutable sessions. No performance timing
claim is made by this review.

Every handled event returns Consumed or Closed. Unknown keys, paste, release
events, focus events, and background pointer input cannot escape through this
API. Global quit and modal precedence are explicitly caller responsibilities.
Close hit testing uses only the rendered seven-cell region. Resize invalidates
old pointer regions, subsequent rendering recomputes wrapping and clamps the
offset, and tiny/zero layouts reset scrolling and remain keyboard-dismissable.
The focused tests cover these behaviors, both command labels, failure categories,
immutable source ownership, Unicode wrapping, and detail reachability.

Validation reused normal `target/` artifacts. The task-owned external harness
source/binary directory occupied 1.4 MiB before its EXIT trap removed it after
the child exited. No disposable artifact or background process remains owned by
this review.

## Remaining gates

The P3 footer issue needs a separate fix and original-reviewer recheck. Actual
rendered UI review remains a separate gate. Later authorized shared integration
must establish module registration, launch-time capabilities and stable IDs,
Actions eligibility/explanations, dispatch via `Session::send`, modal routing,
active-tab/review preservation, global quit, animation behavior, and real-binary
PTY flows. Standalone view review cannot approve those caller contracts or P5.

## Recap

One P3 finding: compact pager clipping can display an incorrect total for valid
maximum-width labels. All nine focused tests and navigation across 2,523 layouts
pass. No serious standalone correctness issue identified; fix/re-review and later
integration/UI gates remain open.

## Independent re-review after separate footer and Unicode fixes

The P3 footer finding above is **closed**. Inspected the final report renderer,
the shared `bounded_device_label` function in `src/batch_commands.rs`, both
focused test files, and the footer fix handoff. The historical finding and
nine-test results above describe the original view; this section records the
current standalone correctness verdict.

The footer measures its actual remaining cells and chooses the longest optional
help that fits beside the complete ASCII position counter. At the minimum
supported width, the bounded report's position counter fits even after the
seven-cell Close region. The exact reported maximum-wide-label fixture now
renders `101-103/103` at 34×12. Both command variants have regression coverage
across narrow→wide→narrow resize, including full counters, fixed header/caveat,
unchanged Close cells, stale pointer rejection, and dismissal.

Also independently reviewed the Unicode-boundary followup reported by the UI
reviewer. Dispatch snapshots and the defensive presentation path now share one
sanitizer. It retains at most 121 non-control scalars as lookahead, returns
untruncated labels up to 120 scalars unchanged, and otherwise keeps only complete
graphemes within a 119-scalar prefix before appending an ellipsis. The grapheme
that crosses the source lookahead boundary cannot enter the retained prefix,
because its end already exceeds that prefix budget. Output remains at most 120
scalars, visibly signals truncation, and is stable under the view's second pass.
No dispatch, capability, count, failure, event-routing, or process behavior was
changed by this label correction.

Independent checks:

- `cargo test --locked --test batch_commands --test batch_report_ui`:
  **15 helper tests and 11 view tests passed, 0 failed**.
- An external harness compared the actual shared sanitizer with segmentation of
  the complete sanitized source across **5,292 boundary/control/idempotence
  cases**. Prefix lengths 0–125 covered combining accents, joined emoji,
  skin-tone modifiers, family emoji, paired regional indicators, wide glyphs,
  and multi-mark clusters, with/without removed controls and varied suffixes.
  All outputs matched, stayed within the scalar budget, and remained identical
  after a second sanitizer pass. A separate 10,000-mark single grapheme produced
  the bounded explicit ellipsis as expected.

No remaining concrete standalone helper/report correctness finding was
identified. The existing normal build cache was reused; the external harness's
620 KiB task-owned source/binary directory was removed by its EXIT trap after
the child exited. No disposable artifact or background process remains.

Recap: footer and Unicode-boundary fixes pass independent re-review. Standalone
correctness gates are clear. Rendered UI re-review and the later authorized main
integration/caller/PTY gates remain separate; this does not approve full P5.
