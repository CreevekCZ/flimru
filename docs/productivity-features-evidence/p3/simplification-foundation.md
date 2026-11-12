# P3 foundation simplification

Inspected the complete new implementation in `src/console_snapshot.rs` and
`src/console_review.rs`, together with their focused integration tests. This
stage covers only the snapshot/review-worker foundation; the later P3 UI
integration requires its own final simplification pass and independent review.

## Disposition

No source or test changes were needed. The current code is already focused:

- Historical-width probing and the caller-maintained maximum-width preflight
  serve different points in capture ownership. Combining them would weaken the
  clone bound or require moving the live screen viewport.
- The coordinate runs, streaming lowercase KMP matcher, explicit growth checks
  and cancellation points preserve the documented text, coordinate and capture
  budgets without constructing large intermediate arrays.
- Capture cancellation and desired-query cancellation have distinct lifetimes:
  replacing a query preserves extraction, while capture replacement invalidates
  both. The mailbox flags distinguish clear, capture and coalesced search work.
- Review rows retain a zero-allocation All variant and a bounded filtered index;
  snapshots remain shared through immutable `Arc` ownership for review/export.

Reducing these structures or rewriting the proven algorithms would add churn
without a useful simplification. Public API, behavior, performance strategy and
budgets are unchanged. No unrelated dirty work was edited.

## Validation

- `rustfmt --edition 2024 --check src/console_snapshot.rs
  src/console_review.rs tests/console_snapshot.rs tests/console_review.rs` passed.
- `cargo clippy --locked --lib --test console_snapshot --test console_review
  -- -D warnings` passed; see `simplification-foundation-clippy.log`.
- `cargo test --locked --test console_snapshot --test console_review` compiled
  the focused harnesses but failed while building the binary because concurrent
  P1 main/UI edits were incomplete. The diagnostics concern availability tuple
  arity, `Arc<LaunchFile>` assignment and `VisibleEntries` borrow lifetimes; see
  `simplification-foundation-tests.log`. This is not reported as a passing Cargo
  test command or a foundation failure.
- Executed those newly built focused harnesses directly after checking they were
  newer than all four scoped source/test files:
  `target/debug/deps/console_snapshot-78fc4e01ef3b61c0` passed **17 tests**, with
  **2 measurement tests ignored**; `target/debug/deps/console_review-314abec9a56052c1`
  passed **7 tests**. See `simplification-foundation-snapshot-harness.log` and
  `simplification-foundation-review-harness.log`. The builder's separate completed
  measurements were not repeated because this stage changed no implementation.

Normal reusable `target` storage was used. This stage created no disposable build
directory, temporary cache or background process. Retained evidence logs are
small (the P3 evidence directory was 16 KiB before this report).

## Handoff

The existing foundation API is ready for the coordinator's integration work.
This is a simplification disposition, not the independent correctness/performance
review or rendered UI acceptance. Full P3 integration still needs its affected
tests, final simplification and review gates after concurrent edits stabilize.

Recap: no needless simplification found; source/API unchanged; scoped formatting,
strict Clippy and 24 routine foundation tests passed, with two measurements
intentionally ignored and the concurrent binary-build failure recorded.
