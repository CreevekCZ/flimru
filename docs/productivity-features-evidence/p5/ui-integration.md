# P5 independent integrated UI/input review

Reviewed 2026-11-06 after the integration builder and separate simplifier. Read
`build-integration.md`, `build-integration.patch`,
`simplification-integration.md`, `simplification-integration.patch`, and owning
App/UI/report source and focused tests. No product or test source was edited.
Standalone report gates remain separate evidence.

## Verdict

**PASS — full P5 UI/input gate after the central tiny Quit correction.**
The initial HIGH/P1 finding below is closed by the final actual-PNG, source and
focused-test recheck at the end of this report. The initial reproduction remains
for traceability. Cross-feature checks and final independent review remain later
gates; this UI verdict does not authorize installation or release.

## Finding: hidden quit confirmation remains actionable below minimum size

Location: `App::draw` in `src/main.rs:753–776`, especially report rendering at
772–774; `ui::draw_content` in `src/ui.rs:1203–1228`; tiny early return in
`ui::draw_overlays` at `src/ui.rs:1615–1617`.

When the delivery report is open, pressing Ctrl+q sets `quit_confirmation`.
Below 34×12, `draw_content` first paints the global tiny Quit confirmation and
buttons. The report's tiny-layout render then clears the entire screen and
replaces them with its enlargement prompt. `draw_overlays` returns immediately
for the tiny size, so it never restores the higher-priority confirmation.
Meanwhile the App mouse report guard is bypassed while `quit_confirmation` is
true, and tiny `mouse_action` still exposes ConfirmQuit coordinates.

Verified with an owned temporary copy of the unchanged App source and a small
appended probe using the existing `crate::tests::app()` and actual
`App::draw`, `App::key`, `App::mouse`, and `ui::mouse_action` methods. The probe
ran three sizes; it did not mock the dispatch or modal draw/input functions:

| Actual terminal | Visible after Ctrl+q | Hidden ConfirmQuit cell | Rendered cell | App mouse result |
| --- | --- | --- | --- | --- |
| 33×12 | Delivery report / enlarge / Enter-Esc closes | (1, 11) | blank | `true` (exit) |
| 34×11 | Delivery report / enlarge / Enter-Esc closes | (1, 10) | blank | `true` (exit) |
| 24×10 | Wrapped delivery report / enlarge / Enter-Esc closes | (1, 9) | blank | `true` (exit) |

`quit_confirmation` was true and `Quit Flimru?` was absent from the actual
TestBackend output in all three cases. A user can therefore click a visibly
blank cell and quit the launcher while its confirmation is hidden. The report
text also falsely says Enter/Esc closes the report while those keys currently
cancel the unseen quit confirmation.

Required fix: enforce the global tiny quit confirmation's draw priority centrally
before subordinate report/export/review layers, with visible controls matching
hit testing. Regression coverage should render after Ctrl+q and after shrinking
an open confirmation, verify visible Quit/Cancel ownership, and verify clicking
its actual controls. Include no-confirmation report behavior so covered or
unproven underlying Quit cells still cannot bypass the report. A separate fixer
owns product/test corrections and independent reviewers must recheck.

## Actual integrated PNG review

Used `view_image` to inspect all nine PNGs in `integration-renders/`: mixed
eligibility Actions, zero eligibility Actions, and report over frozen review,
each at 34×12, 80×24 and 120×40.

- At 34×12, both `Reload all eligible (1)` and `Restart all eligible (1)` remain
  complete; zero-eligible menus likewise retain both `(0)` counts. Batch entries
  have no broadcast shortcut hint. The selected restart footer says
  `Hot restart · input requests` and is readable within the compact panel.
  Source inspection confirms the reload selection's matching hot-reload footer.
- Wider menus retain the same meaning and show ordinary per-session hot actions
  separately. Retained ended runs still offer zero-eligible batch actions.
- Actual report-over-review scenes show one requested/queued, two skipped, zero
  failures, explicit ended-session reasons, and `Flutter completion is
  unconfirmed.` At 34×12 the detail list scrolls beneath fixed counts/caveat;
  the wide scenes expose the queued target as well. The displayed delivery
  content does not claim Flutter completed a hot command.
- The large scene preserves the frozen review/query underneath. The report has
  a distinct panel and its own Close/navigation footer. No new visible layout
  issue was found at supported sizes.

## Input/state checks and source review

Independently ran:

```sh
cargo test --locked --bin flimru batch_hot_ -- --test-threads=1
```

**7 passed, 0 failed; 107 unrelated tests filtered out.** Inspected the actual
assertions rather than treating test names as proof.

App input routing places the report ahead of selection, ordinary shortcuts,
Ctrl+p/Actions, review edits, mouse/drag/wheel routing, and paste. Report dismissal
returns before underlying handlers. Unknown r/R/q keys cannot trigger another
send; reentrant batch requests return without dispatch. The native live cursor
is suppressed by console ownership; frozen review renders without focus. Tests
confirm both live and query cursor restoration after closure.

The reviewed source conservatively marks coverage unknown before first draw and
after resize. At supported sizes, the focused test proves covered toolbar Quit
cells are consumed while a truly uncovered Quit opens confirmation; after resize
a stale pointer cannot open it. Ctrl+q remains the global keyboard escape. The
initial no-draw coverage rule follows directly from the empty `area` predicate;
the tests exercise resize invalidation but do not explicitly perform a first-draw
pointer click.

The state regression preserves the active tab, captured source identity, frozen
snapshot Arc, query, severity filter and nonzero manual review offsets after
modal input and close; no review operation notice is added. The mixed-target
regression includes a real rejected nonblocking queue and continues to later
eligible targets. Stable-ID and ordinary active-only r/R regressions pass.
Earlier standalone failure/Unicode/wrapping evidence was not rerun or recounted
as integration coverage.

## Resources and scope limits

The probe's temporary manifest/source lived only at
`/private/tmp/flimru-p5-ui-integration._pqfm7s2`; it reused the normal project
`target` cache. Failure inputs/results are retained above. Its exact package build
artifacts and owned temporary source directory were removed after the probe
exited. Cleanup unexpectedly reported 67,186 files / 4.0 GiB removed from the
shared target cache by `cargo clean -p flimru_p5_ui_integration_probe`; the copied
manifest named its library `flimru`. This exceeded intended probe-only cleanup
and may require broader rebuilds. The coordinator was informed immediately; no
product/test source was deleted, and no further cache cleanup was attempted.
No GUI/Xcode or external service was touched.

This review does not claim a P6 cross-feature gate, real Flutter/device
acceptance, final fresh-model review, installation, or release. Deferred P6
support tests were neither registered nor run by this reviewer.

Recap: supported integrated rendering, modal isolation and restoration checks pass.
A confirmed hidden global Quit control at tiny sizes blocks integrated UI
approval until fixed and independently rechecked.


## Final independent recheck — central tiny Quit correction

**Full P5 UI/input verdict: PASS. The HIGH/P1 finding is closed; no unresolved UI
or input findings remain in the reviewed P5 scope.**

Inspected the central `App::draw` early branch: when quit confirmation is active
and the terminal is below 34×12, the App renders the global tiny UI and returns
before any report, export, or review/details layer can clear it. This preserves
higher modal priority at the common owner rather than adjusting each child view.
The ordinary supported-size path and no-confirmation report path remain intact.

Used `view_image` on **all nine corrected actual App PNGs** in
`integration-fix-renders/`: batch report, export dialog, and operation details,
each at 33×12, 34×11 and 24×10. Every scene visibly shows `Quit Flimru?`, the
keyboard confirmation/cancellation instructions, `[Quit]`, and `[Cancel]`.
Instructions wrap cleanly at 24×10. The lower modal no longer obscures those
controls or misstates the active Enter/Esc behavior.

Read the complete new regression
`batch_hot_tiny_global_quit_is_visible_above_batch_export_and_operation_details`.
It exercises the actual App for all nine modal/size combinations, with direct
Ctrl+q at the tiny size and opening confirmation at 80×24 before shrinking.
It compares every ConfirmQuit/CancelQuit hit cell against the exact visible
`[Quit]`/`[Cancel]` text, verifies the native cursor is hidden, exercises Enter,
Esc, pointer Cancel, keyboard y and pointer Quit, and compares the restored lower
modal buffer exactly against its initial buffer. It also checks that a covered
Quit location without confirmation remains consumed and that merely opening the
export dialog creates no export output.

Independently reran after the shared-cache incident:

```sh
cargo test --locked --bin flimru batch_hot_ -- --test-threads=1
```

**8 passed, 0 failed; 107 unrelated tests filtered out.** This includes the new
nine-combination regression and the existing supported-size pointer coverage,
native live/query cursor restoration, frozen review-state preservation, delivery
isolation and explicit dispatch tests. The nine earlier supported-size scenes
were not re-inspected or recounted as new visual evidence in this focused pass.
No new temporary artifact, background process, product/test edit, or cache
cleanup was created/performed during this recheck.

### Cleanup incident correction

The earlier probe cleanup command was exactly:

```sh
cargo clean --offline --manifest-path /private/tmp/flimru-p5-ui-integration._pqfm7s2/Cargo.toml --target-dir /Users/jankoznarek/Documents/Development/Rust/flutter-run-tui/target -p flimru_p5_ui_integration_probe
```

It reported `Removed 67186 files, 4.0GiB total`, exceeding the intended package
cleanup. The coordinator was informed, verified source and the installed regular
executable remained intact, and reported the current shared target cache size as
3.2 GiB. Required checks were rebuilt afterwards. The correction for future
review work is to use existing harnesses or an exact task-owned source/probe
directory and delete only that owned directory after processes exit; never run
`cargo clean` against the shared target. No additional shared-cache deletion was
performed during this final recheck.

Recap: the supported-size P5 review remains valid, all corrected tiny Quit scenes
are visible and match their hit regions, focused tests pass, and the P5 UI/input
gate is approved for cross-feature checks and the final independent review.
