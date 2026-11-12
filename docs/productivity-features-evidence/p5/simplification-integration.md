# P5 full-integration simplification

Reviewed the builder handoff, the integration patch against P4, and the owning
source for capability planning/storage, target capture, dispatch, Actions,
delivery-modal routing and the focused regression coverage. The coordinator's
`/private/tmp/flimru-productivity-p4tfofri/p5-built` snapshot is the before-edit
baseline for this stage.

One narrow production cleanup was warranted: `App::hot_eligible_count` now maps
the run iterator directly through `Run::batch_target` and the existing exclusion
predicate instead of collecting a temporary target vector first. Each run still
uses exactly the same launch-time metadata, live/stopping checks and bounded
label capture. The immutable invocation-time target vector remains in the actual
dispatch path, where it provides the required snapshot. No eligibility rule is
duplicated and no new abstraction is introduced.

Verified the earlier allocation concern was already addressed by the builder:
`Run::batch_target` calls `batch_commands::bounded_device_label`, rather than
cloning the full device name. That helper and its existing idempotence coverage
were left unchanged. The count cleanup removes one transient vector allocation;
it does not alter layout, labels, command ordering or modal semantics.

No other cleanup justified changing the reviewed integration. In particular,
stable-ID re-resolution, `Session::send`, fixed failure categories, frozen report
ownership, review preservation and conservative pointer coverage remain as built.
The deferred P6 support module was neither edited nor registered.

## Checks

- `rustfmt --edition 2024 src/main.rs` — passed. Comparison against every file in
  the stage snapshot verified that only the intended `src/main.rs` count function
  changed; no child-module or unrelated source changes were introduced.
- `FLIMRU_P5_APP_EVIDENCE_DIR=<owned-temp>/renders cargo test --locked --bin flimru
  batch_hot_ -- --test-threads=1` — **7 passed**, 107 unrelated tests filtered out.
  This covers eligibility counts, stored capabilities across reload, partial
  send failure, stable identities, explicit versus local commands, frozen review
  preservation and modal input/cursor/quit ownership.
- `cargo clippy --locked --bin flimru -- -D warnings` — passed.
- `rustfmt --edition 2024 --check --config skip_children=true src/main.rs` — passed.
- `git diff --check` — passed.
- Compared regenerated App TestBackend output with retained builder evidence:
  all **18 TXT/SVG files across 9 scenes matched exactly**, including mixed/zero
  counts and report over review at 34×12, 80×24 and 120×40. Existing PNG evidence
  was not modified.

The stage patch is `simplification-integration.patch`, generated directly against
`p5-built`. Prior builder results for the broader 239-test affected set and
all-target Clippy remain separate evidence in `build-integration.md`; this stage
did not repeat that broad set or claim new animation/PTY validation.

## Resources and handoff

Validation reused the normal `target/` cache. The only new temporary directory,
`/var/folders/p1/0ny0fgnj1kg148ynjs9yn6zw0000gn/T/flimru-p5-integration-simplify.AiABVy`,
held 1,977,959 bytes of comparison evidence. After tests exited, the directory was
removed and its absence verified. No owned process or disposable artifact remains.
The coordinator's baseline snapshots were preserved.

Recap: removed an unnecessary count-path vector, preserved exact rendered output,
and passed focused checks. Source is stable and released for independent
correctness and UI review; those gates remain pending.
