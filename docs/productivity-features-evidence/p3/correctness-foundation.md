# P3 foundation independent correctness and performance review

Reviewed `src/console_snapshot.rs`, `src/console_review.rs`, their focused tests,
the P3 product contract, and the foundation simplification disposition. This is
a read-only implementation review; only this report was added. UI integration,
PTY event routing, stable RunId ownership and rendered acceptance are deferred to
the required integration follow-up. **This report does not close the P3 gate.**

## Findings

No actionable correctness or performance defect found in the scoped foundation.
No source changes or fixes are requested by this review.

## Bounds and retained text

- `console_snapshot.rs:136` uses saturating arithmetic, the larger of current
  and caller-supplied historical width, both drawing grids, configured history
  capacity and row/container overhead for the 128 MiB preclone estimate. vt100
  0.16.2 cells have an enforced 32-byte size and inline character storage. Its
  cloned rows copy their cell lengths; this estimate is conservative for the
  requested screen clone when callers uphold the documented width/history bounds.
- `console_snapshot.rs:184` clamps caller limits to the 16 MiB text and 32 MiB
  coordinate ceilings. String growth and all three coordinate vectors account
  for allocated capacity, reject before accepting oversized output and return an
  explicit error instead of publishing a truncated snapshot. Text offsets fit the
  stored u32 runs under the text ceiling.
- `console_snapshot.rs:165` probes the actual retained row length. Capture does
  not assume that resize changes historical widths: the original width governs
  padding, trailing trimming, wrap joins and each row's cell mapping. The public
  maximum captured width also includes historical rows. Shrinking the drawing
  grid can destroy cells in vt100; the foundation correctly preserves available
  retained data and does not claim to reconstruct destroyed cells.
- `console_snapshot.rs:196` keeps the cursor row plus populated rows beneath it,
  omits unused bottom grid rows and retains explicit cursor-advanced blanks.
  Soft wraps join; hard line boundaries remain newlines. The source viewport is
  isolated because only a moved screen clone receives scrollback changes.
- `console_snapshot.rs:369` maps original bytes through row-local coordinate
  runs. ASCII compression remains contiguous, combining substrings select their
  owning cell and wide-cell highlights include the continuation within that
  row's captured width. Empty, newline-only and out-of-range spans yield no
  highlight. The u16 maximum width and final-column boundary were checked
  independently, including retained width 65535 after a shrink.

## Search and worker state

- `console_snapshot.rs:415` bounds input at 512 Unicode scalar values, lowercases
  each character literally and uses streaming KMP with O(query length) scratch.
  Overlapping matches and failure-prefix fallback preserve original UTF-8 ranges,
  including partial matches within lowercase expansions. This is the stated
  lowercase behavior, without normalization or full Unicode case folding.
  Match storage stops at 10,000 and `capped` becomes true only on another distinct
  original-byte match; exact-cap results remain uncapped.
- `console_review.rs:414` performs severity filtering on complete logical lines,
  includes all their physical rows and reuses that eligibility for search. All
  rows require no index allocation; filtered u32 indices check actual capacity
  against 4 MiB and remain sorted for reverse lookup.
- `console_review.rs:185`, `:226` and `:325` separate extraction cancellation
  from query cancellation. Query changes coalesce without restarting capture;
  replacement captures cancel both old operations. Captures are installed only
  for the matching source/snapshot, and results publish only for the current
  generation. The mailbox mutex covers replacement and publication decisions.
- Clear takes priority over queued capture/search work, invalidates unconsumed
  results and drops the worker's Arc on wake. A following capture still runs.
  Shutdown cancels, wakes and joins the owned thread. The classifier's documented
  quick/nonblocking requirement is necessary for bounded cancellation latency.
  Search/filter updates reuse the exact immutable snapshot Arc; externally owned
  export Arcs may outlive clear without retaining worker ownership.

## Independent validation

`cargo test --locked --test console_snapshot --test console_review` passed:
**17 snapshot tests and 7 worker tests**, with two measurement tests intentionally
ignored by the routine command. The earlier simplifier's concurrent binary-build
failure was not reproduced by this review's completed Cargo run.

Ran the two ignored tests separately using the freshly built snapshot harness:

| Debug measurement on this machine | Observed |
| --- | --- |
| 16,660,000 text bytes, 12,640,256 coordinate capacity bytes: capture | 145.3 ms |
| Same near-limit snapshot, 512-character query | 570.8 ms |
| Oversized text rejection | 146.0 ms |
| 10,024 retained rows at 120 columns: conservative clone estimate | 39,231,488 bytes |
| Same realistic history: screen clone | 17.4 ms |
| Same realistic history: extraction | 192.3 ms |

Both measurement tests passed. These are debug, machine-specific observations,
not a frame-time guarantee. Extraction and search belong on the owned worker;
the preflighted clone remains a synchronous integration cost. The later UI review
must assess that cost and verify continued active/hidden PTY draining under load.

An additional temporary harness importing the actual snapshot implementation
passed two independent tests covering six explicit Unicode/KMP expectations
(partial lowercase-expansion matches, overlapping expansion matches, prefix
fallback and deliberate lack of normalization), plus retained width 65535 and
the final-cell mapping after shrink. These tests use fixed expected ranges rather
than reproducing the matcher. An initial compile selected an old Rust 1.97
dependency artifact; selecting the current Rust 1.99 dependency resolved that
harness-only mismatch. The final harness passed.

## Required integration follow-up

Before any live/saved screen clone, pass its configured history upper bound and
a source-lifetime maximum width to
`estimate_screen_capture_with_max_columns`. Initialize that width at parser
creation, update it for every resize, and transfer it with saved consoles.
`estimate_screen_capture` alone is explicitly unsuitable after a historical
shrink. The capture-stage check cannot retroactively protect a prior clone.

Integration must also discard superseded UI data, compare complete tickets and
stable RunIds, retain review deliberately across live-to-saved transfer, preserve
the same Arc for export and exercise input/paste/focus precedence and lifecycle
races. Those caller requirements are documented foundation contracts, not claims
that unfinished UI integration has already satisfied them.

## Artifact ownership and recap

Used the normal reusable `target` cache. Both scratch compile attempts used their
own `TemporaryDirectory`, removed on success or failure; the successful scratch
directory held approximately 1.4 MiB before removal. No disposable cache,
background process or large temporary output remains owned by this review.

Recap: no scoped defects found; 24 routine tests, two measurements and two extra
independent boundary tests passed. Foundation review is complete; P3 integration
review and acceptance remain required.
