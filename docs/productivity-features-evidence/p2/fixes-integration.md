# P2 integration queued-input fix

Status: **fix complete; held for the original independent reviewer**. This report
closes no phase or UI gate. Scope is `src/main.rs`, `src/ui.rs`, related P2 App,
render and `tests/terminal_flow.rs` tests, plus this stage's evidence. The reviewed
foundation/watchdog and P1 policy were preserved. No search/export helper,
running-session lifecycle, parser/availability implementation, or unrelated dirty
file was changed.

## Behavior

Manual reload completion and Run re-arming now have separate state. A manual
request guards Run immediately, including worker-start failure. Its matching or
newer completing result clears the generation-pending state and arms an input
fence on success **or** failure, including automatic work that supersedes a manual
result. Stale generations cannot change either state.

The main event loop acknowledges input idle only when its existing positive-timeout
`event::poll` reports false. The 128-event cap and five-millisecond work cap do not
acknowledge idle. Until that acknowledgement, queued Enter/Run stays blocked at
`start_batch`; all other keys are processed normally. Idle acknowledgement forces
a redraw and ends that input pass, so a ready frame is drawn before later input
can request Run. No queue is flushed, delayed launch is stored, sleep is added,
or confirmation dialog is introduced. Existing sessions remain untouched.

Run actions/toolbars, header, footer and diagnostics use the same guard. Pending
worker work still says `Reloading`; completed work waiting for idle says
`Finishing reload…`, followed by `Choose Run when it appears`.
Self-review caught the guard-owned status lingering after idle; idle now restores
the reload outcome only if that exact guard message is still displayed. Other
status messages are preserved.

## Input-backend check

Inspected the selected Cargo dependency, crossterm 0.29.0's `event/read.rs` and
`event/source/unix/tty.rs`, using this repository's `use-dev-tty` feature. The
positive-timeout tty source checks already-parsed events before kernel polling
and retries interrupted polls. A zero timeout can skip that parser check, so the
existing minimum one-millisecond positive poll was retained. There is no new
independent read, kernel-only idle probe or event-stream reader.

## Regression coverage

- App regression covers successful/failed manual completion and newer automatic
  completion superseding a manual request. Idle observed while pending cannot
  re-arm Run; stale results preserve the fence; 256 queued F1 events and Enter
  remain blocked across simulated budget boundaries. Queued navigation, filter
  editing and quit/cancel continue to work. Explicit idle acknowledgement exposes
  Run without scheduling it, and restores truthful success/error status.
- New real-binary PTY regressions cover keyboard and toolbar entry points, each
  with successful and failed completion. One single-write burst uses 256 inert
  F1 sequences before Enter, crossing the 128-event budget. A separate single-write
  burst uses 64 visibility toggles against a 100,000-entry catalog, below the event
  cap and crossing the five-millisecond work budget. Across all eight cases no
  queued Enter launches a child. A fresh Enter after the ready frame launches
  exactly once with the fresh arguments on success or last-good arguments on
  failure. Existing adjacent `r`+Enter and menu-route PTY tests remain intact;
  existing App tests retain all five manual entry routes.
- A temporary source-copy probe instrumented only the existing five-millisecond
  break with append-only logging, then ran the actual new time-budget PTY test
  against that copied binary. **Passed** all four route/outcome cases; recorded
  **130 time-budget breaks after only 1–5 events**, confirming this is not merely
  event-count coverage. No instrumentation remains in repository source.
  `fixes-integration-budget-probe.log` records the run and measured break points.

## Validation

- `cargo test --locked --bin flimru --lib --test availability --test terminal_flow`:
  **passed**, 57 library + 83 binary/App/render + 3 availability + 26 real-PTY
  tests; **169 passed**, 2 intentionally ignored entries total.
  `fixes-integration-tests.log` retains the results.
- After strengthening the App fence/status assertions during final review,
  `cargo test --locked --bin flimru manual_reload_requires_post_completion_input_idle_before_run_rearms`:
  **passed**. No production behavior changed after the full focused suite.
- Final `cargo clippy --locked --all-targets -- -D warnings`: **passed**;
  `fixes-integration-clippy.log`. No lint suppression added.
- Scoped `rustfmt --check --edition 2024 --config skip_children=true src/main.rs src/ui.rs tests/terminal_flow.rs`
  and scoped `git diff --check`: **passed**.
- `configuration_reload_render_evidence_and_run_gating`: **passed**. Retained new
  `input-drain` and `input-drain-error` TXT/SVG/PNG renders at 34×12, 80×24 and
  120×40. The actual compact success/error and 80×24 error PNGs were opened for
  self-review: Run is absent, the input guard remains readable in the footer,
  and the persistent error marker is visible on failure. The compact toolbar
  truncates its label as expected; the footer carries the complete explanation.
  Independent rendered UI review is still required.

## Ownership and recap

Cargo reused the normal shared `target` cache, approximately **4.4 GiB**, retained
for ongoing workflow validation. Parent/PTY fixtures retain their existing RAII
cleanup and quit/reap paths. The owned render directory (5.6 MiB before removal)
was removed by its EXIT trap; the source-copy probe directory (18,749,688 bytes)
was removed in Python `finally` after compiler/test processes exited. Its useful
measurements and the selected render deliverables are retained beside this report.
No task-owned process or disposable directory remains. No commit, installation,
Xcode operation or real Flutter/device run occurred.

Recap: reload completion keeps precompletion queued Run intent blocked until the
backend observes idle and a new ready frame can be drawn. Success/failure,
event-count/time-budget, explicit fresh Run, lint and affected render checks pass.
Shared source is held for the original reviewer's independent verification.


## UI wording follow-up

Addressed the separate low-priority wording finding in `ui-review.md` without
changing the independently reviewed fence behavior. Header, toolbar, guard,
footer and details now describe **Finishing reload…** and instruct **Choose Run
when it appears**. User-facing queue/backend terminology was removed; success
and failure outcome strings and the persistent error marker are unchanged.
Only these P2 strings, their matching assertions, six affected render stems and
this report/check log changed in this follow-up.

Regenerated and directly opened all six PNGs: success and failure at 34×12, 80×24
and 120×40, with matching TXT/SVG files. The 34-column toolbar uses its existing
ellipsis clipping, while the footer displays the complete reload label and
instruction. Larger sizes show the same copy in header/toolbar/footer/details;
error states retain `Config error` and the reload-failure outcome. Self-review
found no further correction. Original UI reviewer verification remains pending.

Affected checks, recorded in `fixes-integration-wording.log`:

- `manual_reload_requires_post_completion_input_idle_before_run_rearms`: passed.
- `configuration_reload_render_evidence_and_run_gating`: passed, including explicit
  assertions for the reload label, Run hint and absence of queued-input wording.
- `cargo clippy --locked --bin flimru -- -D warnings`: passed.
- `cargo clippy --locked --bin flimru --tests -- -D warnings`: passed. An attempted
  direct `-- --test` checker invocation could not resolve the library crate;
  the standard Cargo test-target selection above resolved that checker setup
  issue without a source or behavior change.
- Scoped rustfmt and diff-whitespace checks for main/UI: passed.

The command-owned render directory (5.6 MiB) was removed by its EXIT trap after
copying the six requested deliverables. No task-owned background process or
throwaway build directory remains. The normal shared target cache is retained
at approximately 4.4 GiB for ongoing validation. No broad behavior suite was
repeated for this wording-only follow-up.

Recap: product copy now names the reload transition, all six affected images were
inspected, and scoped tests/lint/formatting pass. Source is held for the original
UI reviewer's string/image recheck; no phase gate is approved by this fixer.
