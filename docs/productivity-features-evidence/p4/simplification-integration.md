# P4 App integration simplification

Read `build-integration.md`, the scoped `integration.patch` against
`p3-complete`, and the actual integration source/tests. Compared edits to the
coordinator's `p4-built` snapshot. Ownership covered only the six integration
paths documented by the builder; P1–P3 behavior, separately owned P5/P6 work,
and unrelated dirty work were preserved.

Simplified `App::export_event` in `src/main.rs`: borrow the pending export and
dialog in place, match the concrete dialog action, and convert an absent snapshot
directly to its existing validation error. This removes the redundant open check,
nested optional action/request handling, and repeated take/restore of the owner.
Submission, error retry, cancellation, and consumed-event semantics remain the
same. The existing cancellation helper still owns cancellation and pending-writer
acknowledgement.

Reviewed the source capture, invocation metadata, frozen review Arc reuse,
capture/writer ownership, result polling and shutdown, modal event precedence,
covered cursor rendering, and source-close hooks. No additional simplification
was warranted in the settled backend/dialog or the narrow UI integration.
No concrete issue requiring a separate fix was found during this pass. This is
the simplification stage, not independent correctness or phase approval.

Validation after the edit:

- `cargo test --locked --bin flimru log_export -- --nocapture`: **6 passed**, 97
  filtered out; 0.08 seconds. Includes the actual App render scenes and live-child
  input/cursor checks.
- `cargo clippy --locked --bin flimru --tests -- -D warnings`: passed, exit 0.
- Scoped `rustfmt --edition 2024 --check --config skip_children=true` across the
  six integration paths: passed.
- `cargo test --locked --test terminal_flow console_export_real_binary -- --nocapture`:
  passed; actual-binary export, full filtered snapshot and modal-input isolation.

Only `src/main.rs` changed in this stage. No renderer, layout, label, or fixture
changed, so the builder's nine actual App PNGs remain valid. The full 209-test
affected baseline and standalone backend/dialog gates were not repeated because
their implementation was unchanged. Normal Cargo cache and existing RAII test
fixtures were reused; this pass created no task temporary directory or retained
background process.

`simplification-integration.patch` records this stage's complete scoped diff
against `p4-built` (only the handler hunk). Shared source is coherent and released
for independent correctness and rendered UI review; installation remains pending
the coordinator's later workflow.

Recap: one handler made simpler, six App checks and the real-binary flow passed,
strict lint/format checks passed, and visuals stayed unchanged.
