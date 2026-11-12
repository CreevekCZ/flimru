# P5 foundation simplification

Scope: `src/batch_commands.rs` and `tests/batch_commands.rs`, before UI integration.

Reviewed the approved workflow and the complete capability/dispatch helper.
The production helper already has a direct sequence of conservative launch-time
checks and a single dispatch/accounting loop. Left its behavior and structure
unchanged: extracting generic predicates, a dispatch framework, or report builders
would add indirection without removing meaningful complexity.

Simplified the integration test harness to import the registered
`flimru::batch_commands` library module directly. Removed the temporary source-path
module and the redundant configuration alias. The same eleven tests now exercise
the actual public module instead of compiling a second copy of its source.

Review confirmed that the helper records capabilities from launch data, dispatches
by stable run ID, excludes ended/stopping/unsupported targets, attempts each unique
identity once, continues after individual send failures, bounds and sanitizes
per-device details, and reports requested/queued input with unconfirmed Flutter
completion. Existing tests cover these contracts. The tiny bounded application
run count makes the explicit prior-ID scan easy to audit; no collection/framework
was added for it.

Checks completed:

- `rustfmt --edition 2024 src/batch_commands.rs tests/batch_commands.rs` — passed,
  scoped to owned files.
- `cargo test --locked --test batch_commands` — passed, 11 tests. The current
  binary also emitted an unrelated dead-code warning for
  `VisibleEntries::is_empty` in `src/main.rs:141`; that shared file belongs to the
  concurrent integration/fix work and was not changed here.
- `cargo clippy --locked --test batch_commands -- -D warnings` — passed.

Used the normal reusable `target/` build cache. Created no disposable fixture,
background process, isolated build directory, or other temporary artifact.

Recap: the test harness is simpler; production behavior is unchanged. Foundation
checks pass. UI wiring, active-tab preservation and animation integration require
the subsequent integration and review gates.
