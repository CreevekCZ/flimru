# P3 App integration independent correctness/performance review

Reviewed the build and simplification reports, their patches, the actual shared
integration source against `/private/tmp/flimru-productivity-p4tfofri/p2-complete`,
the new App tests and both new real-binary PTY tests. Earlier independent
foundation and standalone-view reviews remain applicable, including the closed
wrapped-match rendering finding. This review changed no source or test files.

## Actionable finding

### P2 — Preserve the Console-page guard for the animation shortcut

`src/main.rs:1982-1983` now delegates Ctrl+a to `toggle_animations`, but the
extracted helper at `src/main.rs:845-852` only checks `active_session_mut()`.
The P2 implementation explicitly required `self.page == Page::Console` before
calling the animation controller. That condition was lost during extraction.

A running child remains available through `active_session_mut()` when the user
returns to Devices or Configs. Consequently Ctrl+a in those pickers now invokes
the hidden child's animation controller. If connected and confirmed, the
controller's normal toggle path sends a toggle command; while waiting or
unavailable, this unexpectedly replaces picker status with an animation error.
The picker inventories do not advertise animation control there.

Independent reproduction compiled a temporary copy of the actual current App
with an additional narrow test, leaving repository files unchanged:

1. Create the ordinary test App and add a real RAII-owned live Session running
   `/bin/bash -c 'sleep 30'`.
2. Select `Page::Devices` with that child hidden and set status to
   `picker unchanged`.
3. Call the actual `App::key(Shortcut::Animations.key(), 80, 24)`.
4. Assert that the status remains unchanged, as under the P2 page guard.

The regression test failed. Actual status was:

> Animation control: Animation control is waiting for Flutter's VM Service. Try
> Actions → Reconnect animation control.

This probe demonstrates the unintended controller invocation; it does not claim
to have connected a real Flutter VM or observed an applied timeDilation change.
The connected toggle consequence follows directly from the same controller call.

Restore the Console-page requirement on the keyboard path, or in the helper if
all its explicit callers are intended to stay Console-only. Preserve direct
review Actions routing on Console. Add coverage with a hidden live child on
Devices and Configs; the existing
`function_keys_are_inert_and_console_shortcuts_do_not_edit_picker_state` fixture
has no Session, so it does not detect this regression. A regression should verify
that no animation action/status is produced outside Console while retaining the
existing Console/review explicit-action checks.

## Other integration findings

No other actionable scoped correctness issue found:

- `runs.rs` assigns process-lifetime identities independently of vector indices.
  Switching to a different source clears review, switching to the same source
  preserves it, and removal of a neighboring ended console adjusts the selected
  index without rebinding the review identity. Closing the reviewed source clears
  it. Ordinary single-child completion transfers ID, metadata, capacity and
  maximum historical width to LastLog; completion of the reviewed single child
  deliberately retains its ended Run and frozen snapshot instead of dismissing
  it. Source refresh accepts updated outcome/time without changing identity.
- Session initializes the lifetime width bound and updates it on successful
  resizes; Run retains it before transferring/dropping Session. Preflight at
  `main.rs:728-759` precedes every new review screen clone. Normal-grid captures
  use the maintained actual retained-history count, while alternate-grid captures
  conservatively use configured capacity for the hidden primary history. Both
  use the lifetime width bound. Selection/scrollbar ownership is released before
  cloning, extraction remains worker-owned and capture/query generations remain
  isolated. Saved-console bounds are retained through transfer.
- Global quit, existing overlays, deliberate source navigation and top focus
  precede active review; review precedes selection, copy, process-key fallback
  and PTY input. Query r/R and paste stay in query input, reading r refreshes,
  and Escape that leaves review remains consumed. Ctrl+a/Ctrl+e/Ctrl+x and other
  remaining controls are consumed during review. Mouse gestures in the review
  region cannot start hidden live selection or operate the live scrollbar.
  Overlay paste remains isolated, and the Actions fallback cannot type ordinary
  unbound characters into the query.
- Explicit selected-child reload/restart/help/DevTools, stop and animation menu
  entries use direct helpers rather than synthesized ordinary query keys. Review
  owns the keyboard shortcuts advertised only in reading mode. Process shortcut
  labels are suppressed when review owns their keys. The animation helper's
  outside-Console keyboard regression is the exception identified above.
- Rendering suppresses covered live console/selection/footer drawing, then draws
  review and any operation notice, then global overlays. The narrow focus API
  hides the native query cursor under parent focus/overlays without changing edit
  state. Notices report requests/failures rather than claiming Flutter completed
  an operation. The existing toolbar still derives animation state dynamically.
- `App::tick` polls review once, drains every active/hidden Session and suppresses
  redraw solely for raw output while review is frozen. Lifecycle/configuration
  changes remain observable; the outer loop still compares animation state and
  connection status. No history re-index is added to ordinary frames. Exit clears
  owned children and drops/joins the review worker before terminal restoration.
- The previous row-bounded highlight traversal is intact, including original
  match indices. The source-details view continues to use frozen metadata and
  independent scrolling. The simplification-only changes preserve metadata
  sentinel bounds, action inventory order and selection-release ordering.

## Independent validation

Ran the complete focused command against the stable integration:

```sh
cargo test --locked --bin flimru --lib --test terminal_flow --test console_search_ui --test console_snapshot --test console_review
```

**219 passed, zero failures; five intentionally ignored tests.** Breakdown:

| Target | Passed | Ignored |
| --- | ---: | ---: |
| Library | 57 | 1 subprocess helper |
| App/worker/render | 93 | 0 |
| Review worker | 7 | 0 |
| Standalone search view | 17 | 1 explicit measurement |
| Snapshot | 17 | 2 explicit measurements |
| Actual binary PTY | 28 | 1 optional real Dart VM qualification |

The two added real-binary search tests passed and exercise actual Ctrl+s delivery,
query/control/paste/selection isolation, explicit child reload/restart,
refresh/source details/resume/quit and quiet output under child flood. New App
tests separately observe parser revisions increasing for both active and hidden
children without raw-output-only redraws. Existing affected P1/P2 tests passed.
App tests exercised 45 actual integrated layout states, while standalone render
tests also ran. Independent image/layout acceptance remains the UI reviewer's
separate gate; this report does not substitute test execution for visual review.

The extra hidden-child picker probe failed as described above despite the green
existing suite. Its temporary directory held a copied App test entry point,
symlinks to existing sources and a compiled harness; `TemporaryDirectory` removed
it on exit. The owned Session and discovery worker were dropped normally, and no
probe child, temporary cache or background server remains. Normal reusable
project `target` storage was used; other agents' retained snapshots, evidence and
build cache were not cleaned or altered.

## Disposition and recap

Full P3 correctness approval is pending the single P2 shortcut regression fix
and targeted recheck. No other scoped source change is requested. The separate
rendered integration review remains required. Real Flutter VM behavior,
physical-device operation and Windows acceptance are not established by the
mock-child/PTy validation above.

Recap: 219 focused tests passed; an independent actual-App probe exposed one lost
Console-page guard. Restore that guard and pin hidden-child picker behavior
before closing the P3 correctness gate.

## Fix recheck — original finding closed; details-only residuals

Reviewed `fixes-integration.md` and the held main/UI/SearchView delta, including
the new focused tests. The original animation finding is **closed**:
`main.rs:2023-2027` restores the Console-page guard on Ctrl+a, while the explicit
Console Actions helper remains direct. The new owned-Session regression covers
both Devices and Configs and passed independently.

The new operation-details view correctly leaves `is_active()`/`reviewing()` false
and source/snapshot absent after Resume; it does not create a frozen capture.
Closing it allows subsequent Ctrl+s to capture normally. Existing source review
retains query/filter/current match/offsets and its Arc across details navigation.
Pending worker results do not replace the details kind/message. Latest operation
messages are sanitized, bounded to 4,096 scalars, explicitly marked on exceptional
shortening and retained through Resume. Reviewed lifecycle notices now use that
same retention path. The original row-bounded rendering fix remains intact.

Two independently confirmed residuals prevent closure of the new details-only
ownership boundary:

### P2 — Ctrl+e starts hidden selection before details-only input routing

At `main.rs:1877-1883`, the SelectText shortcut runs before the operation-details
guard at `:1942-1945` and excludes only source review through `!reviewing()`.
`begin_text_selection` at `:2319-2322` has the same incomplete guard. Operation
details after Resume intentionally have no source and therefore satisfy both
conditions: Ctrl+e captures underlying console text and enables keyboard
selection behind the outcome panel.

An independent temporary copy of the actual App reproduced this with a Console
fixture, a retained operation outcome, `OperationDetails`, and the actual
`Shortcut::SelectText.key()`. Assertions confirmed the details-only preconditions
(not reviewing, source absent, snapshot absent, details visible), but
`app.selection.is_none()` failed immediately after Ctrl+e.

Subsequent arrows and Enter also precede details routing in the keyboard-selection
branch at `main.rs:1816-1845`; thus hidden selection can steal details scrolling
and copy underlying text. This consequence follows from the actual selection
state and those existing handlers; the probe deliberately did not trigger a real
clipboard write. Restore modal precedence or exclude operation details from
selection creation/keyboard selection paths, including the direct selection
helper. Pin Ctrl+e plus following navigation/Enter/Escape while details-only is
open, with no hidden selection or clipboard request and normal behavior after
closing.

### P3 — Details-only rendering exposes the covered live PTY cursor

`main.rs:655-660` draws ordinary content before rendering operation details when
there is no source review. With a live Console, `ui.rs:1361-1376` sets the native
PTY cursor because its focus predicate does not exclude operation details.
Passing `focused=false` to SearchView only suppresses its own query cursor; it
does not clear an already-set underlying cursor.

An independent actual-App probe started an owned live Session printing `ready`,
opened an operation outcome without capturing a source, and drew an 80×24
TestBackend frame. `!terminal.backend().cursor_visible()` failed while the
operation panel covered that cursor location. Existing details-after-Resume
render coverage uses an ended Run, so the live cursor predicate is skipped.

Suppress live cursor placement while operation details owns the region, or avoid
setting underlying content focus before drawing this modal. Add a live-Console
cursor visibility assertion while open and verify normal cursor restoration on
close. No query/source/snapshot state change is needed to fix this presentation
regression.

### Recheck validation and ownership

Independent affected checks passed: **12 App tests**, **19 standalone tests**
including explicit measurements, and **2 real-binary search PTY tests**. These
green tests do not cover the two failing boundary probes above. No unchanged
219-test broad rerun was performed during this recheck.

Both additional probes compiled task-owned copies of the actual App entry point
and referenced existing source/dependencies; repository product/tests were not
edited. The live cursor probe's owned child and workers dropped normally. Scratch
artifacts measured approximately 13.2 MiB per directory before automatic
`TemporaryDirectory` removal. Neither directory, probe child nor disposable cache
was retained.

Recap: the original Ctrl+a regression is closed. New operation-details state and
message retention are otherwise coherent, but the Ctrl+e selection bypass and
covered native cursor need correction and targeted recheck. Full P3 correctness
approval remains pending those fixes plus the separate original UI review gate.

## Final residual recheck — correctness/performance gate approved

Reviewed the final residual pass in `fixes-integration.md`, its actual
main/UI/SearchView changes and the targeted regressions. **Both confirmed
details-only findings are closed. No remaining actionable correctness or
performance finding in the reviewed P3 scope. The P3 correctness/performance gate
is approved.** The original animation regression remains closed; separate
rendered/UI signoff is still the original UI reviewer's responsibility.

`App::console_ui_owned()` at `main.rs:649-650` separates covered UI ownership
from source identity. It is true for source review or operation details, while
`reviewing()`, `reviewed_source()`, `SearchView::is_active()`, source and snapshot
remain source-only. This is the appropriate distinction for the no-capture
details-after-Resume path.

- Early keyboard selection/movement/copy, Escape selection handling and Ctrl+e
  now exclude this UI owner (`main.rs:1822-1888`). The direct selection helper
  has the same guard (`:2325-2327`), as do selection/scroll Actions. Details input
  therefore reaches its own handler without creating a hidden selection or
  allowing subsequent arrows/Enter to operate the old console. Normal selection
  remains available after close.
- `ui.rs:1334-1335` returns after ordinary chrome when the console UI is owned,
  preventing live content and its native cursor from being placed beneath
  operation details. The new real-child regression explicitly observes native
  cursor visible → hidden → visible, no hidden selection after Ctrl+e/direct
  helpers/navigation, and normal Ctrl+e selection after closing. A final deliberate
  live input marker confirms that the modal sequence did not reach the child.
- Raw-output-only redraw suppression uses the same ownership predicate while
  all Sessions continue ticking. Lifecycle redraws and source identity checks
  remain unchanged. Operation-only Actions suppress unsupported shortcut hints
  and omit Select text; explicit menu commands and global quit remain reachable.
- Source-less details retain no source/snapshot, close cleanly, and do not turn
  later Ctrl+s into query editing against an absent capture. Pending worker
  updates cannot replace the open details kind/message. Latest bounded message
  retention and reviewed lifecycle notices preserve their earlier fixes.
- The additional `bounded_label` correction sanitizes bounded lookahead, reserves
  the ellipsis scalar and appends only complete graphemes. A boundary combining
  or ZWJ cluster is dropped in full; unchanged exact fits still pass through.
  It preserves the 96/128/512 scalar budgets and the caller's shortening sentinel,
  without changing search matching, coordinates or snapshot memory limits.

Independent focused validation, without repeating the unchanged broad suite:

```sh
cargo test --locked --bin flimru console_review -- --nocapture
cargo test --locked --test console_search_ui bounded_source_and_error_labels_preserve_boundary_graphemes_and_exact_fits -- --exact
cargo test --locked --test console_search_ui operation_details_survive_pending_search_and_close_without_claiming_a_snapshot -- --exact
cargo test --locked --test console_search_ui explicit_review_actions_do_not_type_shortcuts_and_extreme_metadata_is_marked -- --exact
```

**13 affected App tests and all three targeted standalone tests passed**, zero
failures. These include the actual live-child ownership/cursor regression,
original hidden-child animation guard, source-less details/recapture behavior,
worker-result/details separation and Unicode boundaries. Earlier independent
219-test/PTY and standalone performance evidence remains applicable; unchanged
checks were not repeated. This recheck edited only this report, used the reusable
project cache and created no temporary directory, probe child, server or
disposable output outside existing RAII test fixtures.

Recap: original animation guard and both details-only residuals are closed;
focused regressions pass; **P3 correctness/performance approved**, with separate
UI gate closure required before the coordinator declares the whole phase done.
