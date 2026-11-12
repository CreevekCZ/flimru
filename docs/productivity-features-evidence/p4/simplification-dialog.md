# P4 standalone dialog simplification

Read `dialog-review.md` and inspected the complete modal and its focused tests.
Ownership was limited to `src/export_dialog.rs`, `tests/export_dialog_ui.rs`, and
this report. Shared App/UI/backend/search code was left untouched.

The modal already has a narrow, readable state and event flow. No production
behavior simplification was warranted. Applied scoped formatting and clarified
the test evidence generator: named the SVG font-weight fragment and split the
dense long-Unicode fixture construction into source/destination values. There
are no API, visual, filesystem, input, state, or budget changes.

Checked readiness/submission, Saving input lockout, retryable errors, immutable
snapshot/source retention, bounded path insertion, UTF-8 cursor boundaries,
focus cycling, mouse hit regions, tiny-layout handling, and event consumption.
No concrete issue requiring a separate fix was found during this pass.

Validation:

- `cargo test --test export_dialog_ui`: **6 passed, 0 failed**.
- `cargo clippy --test export_dialog_ui -- -D warnings`: passed, exit 0.
- `rustfmt --check --edition 2024 src/export_dialog.rs tests/export_dialog_ui.rs`:
  passed.
- Reran `render_evidence_all_states_and_unicode_paths` with
  `FLIMRU_P4_EVIDENCE_DIR` pointing to a task-owned temporary directory: **1
  passed**, 5 filtered out. All **30 generated TXT/SVG artifacts matched the
  retained evidence byte for byte**. Existing fifteen PNGs can therefore be reused.

Normal Cargo cache was reused. The task-owned render directory occupied 1.3 MiB
and was removed by an EXIT trap after comparison. No background processes or
temporary artifacts remain from this pass.

Stable handoff: the standalone modal preserves the API documented in
`dialog-review.md`; callers still own capture/export work, stale-result handling,
global quit precedence, and cancellation/join on close. This simplification is
not phase approval. App integration, independent correctness review, and live
PTY validation remain with the coordinator.
