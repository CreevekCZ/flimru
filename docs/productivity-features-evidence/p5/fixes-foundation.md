# P5 foundation correctness fix

Addressed the independent finding in `correctness-foundation.md`. Changes are
limited to `src/batch_commands.rs`, `tests/batch_commands.rs`, and this report.
No main/UI/session/launch-planning or single-session input code was changed.

## Change

Added fixed typed exclusions and safe descriptions for machine mode,
nonresident runs, and startup tracing. Launch-time capability detection now
excludes `--machine` / `--machine=true`, `--no-resident` / `--resident=false`,
and `--trace-startup` / `--trace-startup=true` in either `args` or `toolArgs`.
The dispatcher therefore never submits ordinary terminal `r` / `R` input to
these snapshots.

The policy intentionally remains conservative across duplicates and conflicts:
any recognized incompatible occurrence excludes the launch, regardless of
argument order or source. This does not emulate Flutter's last-option-wins
parser. Explicit `--machine=false`, `--trace-startup=false`, and
`--resident=true` alone preserve normal implicit/explicit-debug capabilities;
they cannot override an incompatible occurrence. The implementation comment
and regression tests record this policy. Existing mode, test, custom-runner,
and hot-disabled exclusions retain their behavior.

## Verification and review

- `cargo test --locked --test batch_commands` — **14 passed, 0 failed**,
  preserving all 11 existing tests and adding three regression tests.
- `cargo clippy --locked --lib --test batch_commands -- -D warnings` — passed;
  no lint suppression added.
- `rustfmt --edition 2024 --check src/batch_commands.rs tests/batch_commands.rs`
  — passed after formatting only those two files.

The regression matrix confirms all six incompatible spellings are forwarded by
the actual launch planner from either argument source (12 cases), receive the
expected typed exclusion, and never invoke the sender for either reload or
restart (24 dispatch checks). Safe explicit boolean values preserve dispatch in
implicit and explicit debug launches through both sources. Conflicting options
remain excluded in both orders within either source and across sources.

Reviewed the final scoped code after the checks: the change only adds capability
exclusions, keeps fixed bounded descriptions, and leaves dispatch identity,
counts, byte protocol, and failure handling unchanged. This implementation
review does **not** approve its own foundation gate. Separate independent
re-review and later main/UI integration validation remain required.

Validation reused the normal project `target/` cache. No isolated build cache,
task-owned temporary directory, background process, or disposable artifact was
created or left to clean up.

## Recap

The reproduced unsupported-mode eligibility gap is fixed with three typed
reasons and regression coverage. All 14 focused tests, strict scoped Clippy,
and scoped formatting pass. Awaiting the original reviewer's re-review; no
phase approval or commit was made.
