# P5 full App integration correctness review

Reviewed the builder integration patch against the coordinator's P4 baseline,
the separate simplification patch/handoff, and the actual owning source for all
P5 launch/run, dispatch, Actions, rendering, input, and fixture changes. This
review made no product or test edits and did not run or approve the deferred P6
support module.

## Finding

### P2: Preserve global quit confirmation above the report at tiny sizes

Location: `src/main.rs:754-775`, especially the unconditional report rendering at
`:772-773`, together with `src/ui.rs:1203-1225` and `:1614-1617`.

When a delivery report is open below 34×12, Ctrl+q still sets
`quit_confirmation`, and subsequent `y` requests application exit. However,
`App::draw` first draws the tiny quit prompt through `ui::draw_content`, then
renders the report's full-screen enlargement prompt over it. The final
`ui::draw_overlays` call returns immediately at these sizes, so nothing restores
the higher-priority confirmation.

An independent actual-App `TestBackend` probe at 24×10 reproduced this screen:

```text
Hot reload delivery
report
Enlarge to inspect
results.
Enter/Esc closes.
```

The probe confirmed `quit_confirmation == true`, no visible `Quit Flimru?` text,
and `App::key(y)` returning the exit request. Removing the report while keeping
the same app/quit state restored the ordinary visible quit confirmation. Thus
the displayed dismissal instruction and the actual active modal disagree.

Give the tiny global quit presentation final drawing precedence centrally in
`App::draw`, before child report/export/details rendering can overwrite it.
Exercise visible prompt/buttons, keyboard confirm/cancel, and pointer targets
at representative narrow/short sizes; cancel must return to the existing
report or other covered owner without losing its source/state. The independent
UI reviewer is separately checking hidden tiny pointer targets.

## Other reviewed integration contracts

No other concrete integration correctness finding was identified.

- `FlutterTool::plan` captures capability from launch options and the actual
  Designed-for-iPad adapter condition. SDK/FVM-compatible execution and the
  known secrets guard's exec wrapper do not become unsupported merely because
  of their executable names or shell wrapping. `Run` stores the planned value;
  catalog reload applies no capability mutation to existing runs.
- `Run::batch_target` bounds the copied device label, records its stable run ID
  and live/stopping state, and supplies the same exclusion rule to eligible
  counting and invocation. IDs are generated independently of vector slots.
  Dispatch snapshots all retained targets, resolves each ID again, rejects
  unavailable/newly stopping sessions, and calls the existing `Session::send`
  once per eligible unique ID. Failed sends do not stop later targets. There
  is no launch planning, spawning, revival, or tab-switch call in this path.
- Ordinary `r/R` retain active-session routing. The batch sender uses the same
  `Session::send(b"R")` animation restart hook. Production Session behavior was
  unchanged; the builder's dedicated animation regression is separate evidence
  and was not rerun by this review.
- The private report owns its immutable result and fixed safe categories. App
  key/paste/mouse routing gives it priority before selection and terminal input;
  report navigation/repeat input cannot issue a command. The modal guard also
  prevents reentrant dispatch. Coverage is conservative before first render
  and after resize, and only actually uncovered global Quit cells can bypass
  it at supported sizes. Console ownership suppresses the live/query cursor.
- Batch invocation leaves the frozen review owner, query/filter, source, manual
  offsets, active tab, and existing operation-notice value intact. The covered
  review is rendered with the same notice-height reservation and without focus;
  the batch result does not introduce a new notice or change that geometry.
  Dispatch/report ownership introduces no new worker, queue, child process,
  teardown mechanism, filesystem write, or live-history scan.

## Independent checks

- `cargo test --locked --bin flimru batch_hot_ -- --test-threads=1`:
  **7 passed, 0 failed**, covering stored launch capabilities across actual
  catalog apply, mixed target exclusions plus a real full input queue, stable
  ID reorder/removal/replacement, active-only shortcuts versus explicit batch,
  report reentrancy, frozen review preservation, cursor/modal/global-quit
  ownership, and actual App rendered counts/report scenes.
- `cargo test --locked --lib
  hot_capabilities_follow_launch_options_and_actual_adapter_not_tool_names_or_guards`:
  **1 passed, 0 failed**.
- External copied-main harness with the actual source and existing dependency
  artifacts: **tiny quit reproduction confirmed** as described above. Its
  second, optional existing-notice probe did not reach the preservation assertion:
  the independent fixture's `ERROR needle` text lacked the severity classifier's
  colon, so the filtered source had no manual-scroll range. This is a probe
  fixture failure, not product evidence or an additional finding. The existing
  no-notice App regression passed; existing-notice preservation was reviewed in
  the owning source and can be exercised with a corrected fixture on recheck.

The broad 239-test builder run, all-target Clippy, real-binary PTY and animation
results, and exact nine-scene simplifier comparison remain attributable to their
respective handoffs; this review did not repeat or relabel them as independent
checks. The first external harness compile needed `CARGO_PKG_VERSION` for Clap;
that setup failure was corrected in the second attempt. Both task-owned harness
directories were removed by EXIT traps after children exited, and their absence
was checked. Normal build/dependency artifacts were reused. No owned disposable
artifact or background process remains.

## Recap

One P2 finding blocks the integration correctness gate: tiny quit confirmation
is active but hidden behind the report. Eight focused project tests pass, and
an independent App probe confirms the finding. Separate fix/original-reviewer
recheck and UI approval are required; full P5 and P6 approval remain open.

## Original-reviewer recheck after separate integration fix

The P2 tiny-quit finding above is **closed**. Reviewed the finalized six-line
`App::draw` guard, the new actual-App regression, and `fixes-integration.md`.
The initial finding/validation sections are historical; this section records
the current final P5 correctness verdict.

When global quit confirmation owns a terminal below 34×12, `App::draw` now
calls the existing shared UI renderer and returns before report/export/details
layers can clear it. The condition is limited to the established tiny boundary
and an already-active confirmation. Its draw-only early return does not mutate
the covered owner, dispatch commands, change pointer geometry, or affect the
supported-size overlay order. The ordinary shared tiny UI displays the prompt,
confirmation/cancellation buttons, and instructions while retaining the existing
input semantics.

Independently reran the focused affected checks:

- `cargo test --locked --bin flimru batch_hot_ -- --test-threads=1`:
  **8 passed, 0 failed**. The new regression exercises batch/export/details at
  33×12, 34×11, and 24×10; direct tiny confirmation and supported→tiny resize;
  prompt text and hidden native cursor; exact visible ConfirmQuit/CancelQuit
  cells; hidden underlying Quit consumption; keyboard/pointer confirm/cancel;
  exact covered-modal restoration; and no implicit export write.
- `cargo test --locked --bin flimru log_export_ -- --test-threads=1`:
  **10 passed, 0 failed**.
- `cargo test --locked --bin flimru
  console_review_operation_details_preserve_review_and_remain_reachable_after_resume
  -- --test-threads=1`: **1 passed, 0 failed**.

Also corrected the optional independent existing-notice probe's fixture and
ran it against a task-owned copy of the actual App/test source without editing
repository product/tests. The existing successful no-notice preservation
regression was strengthened in that copy by inserting
`report_operation("Existing operation notice")` before the first review render
and manual PageDown, and by requiring the same notice after dismissal. Its
original classified `ERROR: needle` source provides a nonzero manual offset.
The enhanced probe **passed**, preserving the same frozen Arc, source identity,
query, severity filter, manual offsets, active tab, and existing notice while
the report consumes input. This closes the original probe's validation gap.

All 19 targeted project tests plus the enhanced independent probe pass. No
additional P5 correctness finding was identified. No shared cache was cleaned
or modified as cleanup. The external copied-source harness occupied 13 MiB,
was removed by its exact task-owned EXIT cleanup after the child exited, and
its absence was verified. No owned disposable artifact or background process
remains. The fixer's named render evidence is retained by its owner.

Final P5 correctness verdict: **clear**. The helper, report, and full App
integration findings are closed after separate fixes and original-reviewer
rechecks. Independent rendered UI approval, coordinator phase acceptance, and
the later P6 cross-feature/fresh final-review gates remain separate. This
review did not register or run P6 support tests, approve installation, or claim
new broad-suite/animation/real-binary validation beyond the cited handoffs.

Recap: tiny global quit precedence is fixed across all three covered owners;
19 affected tests and the existing-notice preservation probe pass; no remaining
P5 correctness finding.
