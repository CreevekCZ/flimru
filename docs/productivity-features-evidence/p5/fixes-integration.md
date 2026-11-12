# P5 integration tiny-quit fix

Addressed the independently reproduced global-quit finding in
`correctness-integration.md` and `ui-integration.md`. This pass edits only
`src/main.rs`, `tests/support/batch_commands_app.rs`, this report, and its named
render evidence. No P6 module was registered or run.

## Change

`App::draw` now renders the existing global tiny-layout UI and returns before
subordinate modal drawing whenever quit confirmation owns a terminal below
34×12. The six-line central guard prevents delivery reports, export dialogs,
and operation-details views from clearing the active quit prompt/buttons.
Supported-size layering, cursor handling, modal state, input routing, pointer
geometry, and command dispatch code remain unchanged.

The view shown at a tiny size now matches the existing active global input
owner: `Quit Flimru?`, `y quits · Enter/Esc cancels`, and the visible
`[Quit]`/`[Cancel]` controls. Cancel restores the covered modal, rather than
dismissing it or changing its content.

## Verification and review

Added one actual-App regression covering all nine combinations of batch report,
export dialog, and operation details at 33×12, 34×11, and 24×10. It uses the real
`App::draw`, key routing, mouse routing, and `ui::mouse_action`, with actual
`TestBackend` buffers. The matrix verifies:

- Without confirmation, the underlying tiny Quit hit cell is consumed by the
  existing modal and cannot open confirmation.
- Ctrl+q directly at the tiny size and shrinking an already-rendered 80×24
  confirmation both keep the global prompt visible and native cursor hidden.
- Every actionable ConfirmQuit and CancelQuit cell spells exactly its visible
  `[Quit]` or `[Cancel]` button. No blank confirmation target remains actionable
  in these rendered states.
- Enter, Esc, and pointer Cancel dismiss only confirmation and restore the
  exact previous modal buffer; keyboard `y` and the visible pointer Quit return
  the normal explicit exit request.
- Opening/canceling these prompts does not write a console export.

Checks:

- `cargo test --locked --bin flimru batch_hot_ -- --test-threads=1` —
  **8 passed, 0 failed**, including all seven existing batch integration tests.
- `cargo test --locked --bin flimru log_export_ -- --test-threads=1` —
  **10 passed, 0 failed**.
- `cargo test --locked --bin flimru console_review_operation_details -- --test-threads=1`
  — **1 passed, 0 failed**.
- `cargo clippy --locked --bin flimru --tests -- -D warnings` — passed, with
  no lint suppressions added.
- `rustfmt --edition 2024 --config skip_children=true --check src/main.rs tests/support/batch_commands_app.rs`
  — passed after formatting only the two owned files, with child-module
  formatting disabled.

Reviewed the final scoped implementation and test after these checks: the
new production branch is conditional on an already-active global confirmation
and the existing tiny-layout boundary. It calls the existing UI renderer and
cannot mutate the covered report/export/details owner. No additional confirmed
finding appeared in the finalized review reports. This fix does not approve
its own integration correctness or UI gate; original-reviewer rechecks remain
required.

## Retained actual render evidence

Ran the batch integration command with
`FLIMRU_P5_APP_FIX_EVIDENCE_DIR=docs/productivity-features-evidence/p5/integration-fix-renders`.
The regression retained nine actual App TXT/SVG scenes and converted all nine
SVGs to PNG using the installed `rsvg-convert`. The named directory contains
266,721 bytes across the three formats. Examples visually inspected:

- [Batch report, 33×12](integration-fix-renders/tiny-quit-batch-33x12.png)
- [Export dialog, 34×11](integration-fix-renders/tiny-quit-export-34x11.png)
- [Operation details, 24×10](integration-fix-renders/tiny-quit-details-24x10.png)

Each image displays the full prompt and visible Quit/Cancel controls, including
the wrapped cancellation hint at 24×10. Other existing retained scenes were not
overwritten. Validation reused normal `target/` output; fixture directories and
workers are owned by the test's existing RAII lifetimes. No task-owned background
process, separate build cache, or disposable artifact remains. The named render
directory is retained as requested review evidence.

## Recap

Global tiny quit confirmation now draws above all three affected modal paths.
All 19 targeted App tests, strict Clippy, and scoped formatting pass; nine actual
PNG scenes are retained. Awaiting original independent reviewers; no gate
approval, P6 registration, commit, installation, or release was performed.
