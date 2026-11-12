# P1 simplification stage

Reviewed the P1 changes against the captured baseline and the builder-stage
snapshot. Changes are limited to `src/main.rs`, the P1 explanation interactions
in `tests/terminal_flow.rs`, and this evidence. Unrelated dirty work and helper
modules were preserved.

- Removed the raw allocation address used as a catalog cache identity. Catalog
  replacement already passes through `App::merge`, which clears assessments;
  this explicit invalidation is now documented. Target ID/platform/name and
  adapter changes still invalidate the presentation cache. Run still performs
  fresh all-target preflight and consumes its plans.
- Moved the blocked launch assessment into the presentation cache rather than
  cloning its reason/command data before clearing the plans.
- Extended the cache regression to update a configuration while preserving its
  source index and allocation, then verify merge exposes the new blocking reason.
- Fixed two P1 PTY test timing races found during validation. Each now waits for
  the picker to render after Escape dismisses Diagnostics before sending another
  control shortcut. Without that acknowledgement, Escape and Ctrl+q could be
  decoded together and merely dismiss the modal.

## Validation

- `cargo test --locked --bin flimru --test availability --test terminal_flow`:
  **83 passed, 1 ignored** (61 application, 3 availability, 19 PTY). The ignored
  test requires a real Dart executable. See `simplification-tests.log`.
- `cargo clippy --all-targets -- -D warnings`: passed;
  `simplification-clippy.log`.
- Owned files pass rustfmt checking and `git diff --check`.
- Reviewed the final stage diff: no visual rendering code or control semantics
  changed, so the builder's 27 rendered evidence images were not regenerated.

The broad PTY runs initially exposed the two dismissal races above. The isolated
multi-session retry passed before synchronization, confirming timing dependence;
the final complete affected run passed after both fixes. The retained
`simplification-tests-before-synchronization.log` records the second failure and
`simplification-retry.log` records that isolated retry.

## Handoff and limits

Future catalog apply paths, including P2 automatic/manual reload, must call merge
or explicitly clear `availability_view` before reading assessments for the new
catalog. A vector allocation address is deliberately no longer a fallback.
No new feature, production UI/layout change, live Flutter/device qualification,
release build, or installation was performed. Independent correctness and
rendered-UI review remain the coordinator's next gates.

Normal reusable `target` storage was used. This stage created no disposable
build/cache directory or persistent background process; retained logs are small
and the existing approximately 6 MiB P1 render evidence remains intentional.

Recap: simplified cache ownership and blocked preflight handling, strengthened
catalog-refresh coverage, synchronized modal-dismissal PTY tests, and passed the
affected test/strict-lint checks.
