# P2 foundation test-watchdog fix

Scope: the small test robustness finding in `correctness-foundation.md` only.
Changes are confined to the `src/config_reload.rs` test module and this report.
Production worker behavior, API, main/UI and other agents' files were not changed.
This is a fix-stage report; independent re-review and the later P2 integration
review remain required.

## Fix

Replaced the thread/channel watchdog followed by an unconditional blocking join
with an owned subprocess watchdog. The parent launches the current library test
executable with the exact ignored child-helper name and explicit case/fixture
environment values. Each child runs one worker scenario.

The parent owns its temporary fixture and child-output log. Its RAII child guard
kills and waits for its own child on early return or unwinding; the timeout path
also kills and waits before fixture cleanup. No detached test thread is left in
the parent, and the child creates no descendant processes. Log output uses a
parent-owned file rather than an unread pipe that could block child teardown.

The original two-second drop timeout starts when the child writes its drop marker.
Setup separately preserves each existing three-second result wait (six update
results, two FIFO results, one deadline result), plus one second for child startup.
There is no extra sleep to establish the poll-deadline state. All worker-owning
tests use the subprocess path, including the update and public-constructor tests,
so their implicit error-path drops cannot leave the suite blocked either.

Original behavior assertions and test names remain. Successful drop additionally
asserts shutdown, an empty mailbox, and that both the reloader handle and joined
worker released their `Arc` ownership. A deterministic regression test supplies a
worker that parks forever: the parent observes drop start, times out, kills and
reaps the process, observes unsuccessful exit, and explicitly closes its fixture.
That test passed in approximately two seconds with no stuck worker in the suite.

## Review and validation

A self-review caught and corrected the setup budget for the update scenario so
all six original result waits retain their individual bound. Ownership review
checked cleanup ordering on normal exit, child failure, timeout and parent error.
This self-review does not approve the independent review gate.

- `cargo test --lib config_reload`: **passed**, 12 passed, 0 failed, 1 ignored
  child-helper entry. The helper is executed by its parent scenarios with
  `--exact ... --ignored --nocapture`.
- `cargo clippy --lib -- -D warnings`: **passed**.
- `cargo clippy --lib -- --test -D warnings`: **passed**, including the test
  module without selecting unrelated binary test targets.
- `rustfmt --check --edition 2024 src/config_reload.rs`: **passed**.

## Artifact ownership

Cargo reused the shared project `target` cache (4.2 GiB at validation completion);
that directory remains for the ongoing workflow. No isolated DerivedData/module
cache/build tree was created. Parent-owned `tempfile::TempDir` fixtures and their
logs were removed after each child was reaped, including the deliberate timeout.
No task-owned process or disposable directory remains.

## Recap

The watchdog now demonstrates bounded failure for a real stuck join while keeping
worker and fixture ownership explicit. Focused library tests, strict production
and test-module Clippy, and scoped formatting passed. Independent re-review is
pending; no P2 integration gate is approved by this report.
