# P2 worker foundation simplification

Scope: independent simplification of `src/config_reload.rs` and its internal unit
tests. This evidence covers the worker foundation only; it is not correctness,
UI, integration, or phase approval.

## Changes

- Centralized the accepted-observation state transition in `Debounce::accept`.
  Automatic acceptance after two matching observations and immediate manual
  acceptance now share the same candidate reset and accepted-value update.
- Renamed the captured `before_read` value to `read_generation`, making the stale
  generation guard explicit without changing its lock boundaries or behavior.
- Consolidated identical drop/join assertions into one internal test helper.
- Kept `ConfigReloader::new`, `request_reload`, `poll`, result types, and all public
  constants unchanged. The mailbox, request coalescing, polling/debounce cadence,
  4 MiB plus one-byte read bound, chunk cancellation checks, Unix `O_NONBLOCK`,
  opened-file regular-file check, and teardown logic were left intact.

## Validation

- `cargo test --lib config_reload`: **passed**, 11 tests, 0 failures. Coverage
  includes two-observation debounce, manual acceptance/reset, identical-error
  suppression, stale generation/mailbox rejection, read limits, UTF-8 and
  nonregular files, safe parser errors, same-size/same-mtime updates, atomic
  replacement, delete/recreate, waiting-worker teardown, and FIFO rejection.
- `cargo clippy --lib -- -D warnings`: **passed**.
- `rustfmt --edition 2024 --check src/config_reload.rs`: **passed**.
- `git diff --check -- src/config_reload.rs`: **passed**; the module is currently
  untracked in the shared worktree, so rustfmt is the meaningful owned-file format
  check.
- Re-read the changed acceptance, generation and test-helper sections after the
  checks. No additional fix was needed for these behavior-preserving edits.

## Follow-up boundary

The independent P2 reviewer still needs to review final worker/UI integration and
manual/automatic races. One inherited test robustness limitation is worth noting:
the drop tests join the dropping thread after the two-second receive timeout, so
a real teardown deadlock would still hang the test process after that timeout.
The simplification preserves that existing behavior; the production worker was
not changed to address it.

## Artifact ownership

No new persistent process or disposable directory was created. The tests used
RAII-owned `tempfile` fixtures and completed. Cargo reused the existing project
`target` cache (4.1 GiB at the final check); it is retained for the shared workflow.
The coordinator-owned `/private/tmp/flimru-productivity-p4tfofri` directory was not
modified or removed by this task.

## Recap

Worker foundation simplified within its existing API; all 11 focused tests,
strict library Clippy and the owned-file format check passed. Final P2 review and
integration remain with the coordinator's later stage agents.
