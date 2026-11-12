# P3 console review integration build

Status: implemented and ready for separate simplification, correctness and rendered
UI review. This builder report does not close the phase gate. Shared integration
baseline: `/private/tmp/flimru-productivity-p4tfofri/p2-complete`.

## Behavior

Ctrl+s and Actions → Search console capture the selected live, ended or saved
console. Review preserves captured wrapping and width. Query editing, Unicode
case-insensitive literal matches, current/total/capped counts, severity filtering,
next/previous matches, horizontal pan, refresh, resume and source details use the
reviewed SearchView/worker/snapshot implementation. Every search command has an
explicit Actions entry point; command actions do not synthesize ordinary keys.
Ctrl+f still switches consoles. Ordinary typing, slash, paste and selection retain
the previous behavior outside review.

Run IDs are process-lifetime u64 identities, independent of vector positions.
Closing another ended tab preserves review; switching/closing the actual source
clears it. A reviewed single run is deliberately retained as an ended Run when its
child exits, keeping its ID and frozen snapshot. Ordinary single-run completion
transfers the original ID, bounded device/configuration/outcome metadata, maximum
historical width and history capacity into the saved-console path. Reviewed
capture-time metadata stays frozen; Refresh captures current time/outcome.

Session tracks lifetime maximum columns through every successful resize; Run and
LastLog retain it after process teardown. Before any new review Screen clone,
normal-screen capture uses the actual retained history count and lifetime width.
Alternate-screen capture uses configured history capacity because hidden primary
history is unavailable through the active grid. The 128 MiB estimate is checked
before cloning. A wide, empty normal console is accepted; an alternate console
whose hidden-history bound exceeds the budget shows an isolated review error.
Existing live text-selection capture was not broadly rewritten; it is released
before allocating a review clone.

Severity colors and search classification now share the same semantic prefix
classifier in console_style. Existing ANSI rendering remains unchanged. The
search callback maps only recognized errors/warnings; ordinary stack continuation
lines and words buried inside messages remain unclassified.

Explicit menu/button reload, restart, Flutter help, DevTools, animation control,
reconnect, stop and stop-all call direct session/animation helpers. They cannot be
reinterpreted as query input or snapshot refresh. Their process shortcut labels
are omitted while review owns those keys. Requested/failed operation feedback
uses an unobscured bottom notice row; normal review retains the complete 34×9
region, and an operation notice temporarily leaves a 34×8 view. Completion of the
reviewed child produces a visible live completion notice without changing the
capture. Requests are not presented as proof that Flutter completed an operation.

Rendering order is ordinary toolbar/tabs → search/source-details region → global
Actions/help/quit overlays. Covered live cells, selection, status and footer are
not painted. A narrow render_with_focus API suppresses the native query cursor
under parent overlays/top focus, without changing editing state. PTY output keeps
draining for active and hidden sessions, while output-only changes do not request
redraws of a frozen view; meaningful lifecycle/animation/configuration/UI changes
remain observable. Review worker ownership is cancelled/joined on launcher exit.
P4 export dialog wiring is intentionally absent.

## Event precedence

| Priority | Owner | Behavior while review is active |
| --- | --- | --- |
| 1 | Quit confirmation / Ctrl+q | Confirmation consumes input; explicit global quit remains reachable. |
| 2 | Existing global modal / Actions inventory | Menu navigation and paste are isolated. Explicit entries call direct commands; registered global and truthful reading-mode bindings can close the menu and route to their proper owner. Unbound typing cannot edit the query through a menu. |
| 3 | Toolbar focus and intentional console/device/add/close navigation | Focus navigation remains available. Only an actual source switch/close clears review. |
| 4 | Ctrl+s and SearchView | Ctrl+s opens/edits the query. Query r/R is literal text; reading r refreshes. Ctrl+a, Ctrl+e, Ctrl+x, unknown controls, paste, mouse gestures and review navigation are consumed here. Escape first stops editing, then resumes live and remains consumed. |
| 5 | Existing selection/copy and console input | Reachable only outside active review. No hidden live selection or clipboard action runs behind review. |

Mouse overlays route first, then intentional toolbar/tab actions; remaining review
mouse events are consumed before live selection, scrollbar and console handlers.
Source details preserve query/edit mode, filter, offsets and the frozen snapshot.

## Exact checks and evidence

`integration/build-tests.log` records:

```sh
cargo test --locked --bin flimru --lib --test terminal_flow --test console_search_ui --test console_snapshot --test console_review
```

Result: 219 passed, five intentional ignored tests: 57 library, 93 App/worker/render,
7 review-worker, 17 standalone search-view, 17 snapshot and 28 real-binary PTY
passed. The ignored entries are the existing subprocess helper, optional real Dart
VM qualification and existing measurement fixtures. No failure was waived.

After final review tightened Actions fallback so advertised global/reading bindings
remain truthful without permitting query typing through a menu, all nine new App
integration tests passed again (`integration/build-final-focused.log`). They
include the newly pinned Ctrl+s and reading-r menu routes. Strict all-target
Clippy passes (`integration/build-clippy.log`). Scoped rustfmt and scoped
`git diff --check` pass. No whole-project formatter was run.

The two new real-binary PTY tests prove actual Ctrl+s/raw-mode delivery; literal
query/paste/r/R/control and selection isolation; explicit selected-child reload
and restart with an unchanged frozen body; refresh/source details/resume/two-stage
Escape/quit; and quiet terminal output under a continuing child flood. Separate
App tests observe increasing parser revisions for both active and hidden children
while output-only redraw flags remain false. Existing P1/P2 real PTY, manual input
fence, live selection, animation and lifecycle regressions remain passing.

Forty-five integrated App TestBackend TXT/SVG/PNG sets are retained under
`integration/`: fifteen states at 34×12, 80×24 and 120×40, including long metadata,
source details/scrolled details, pending, capture errors, wide retained history,
global overlays, and operation success/error notices. PNGs were rendered using
rsvg-convert. Builder opened compact reading, source details and its scrolled
state, notices, Actions/help/quit; 80×24 wide history; and 120×40 capture error.
The minimum-size source is accessible via its stable ID and bounded fields;
operation feedback and quit remain unobscured. Independent rendered acceptance
and the full phase gate remain required.

## Scope and artifact ownership

Owned integration changes: src/main.rs, src/ui.rs, src/runs.rs, src/session.rs,
src/shortcuts.rs, src/console_style.rs; the approved narrow focus API in
src/console_search.rs and its test in tests/console_search_ui.rs; new
`tests/support/console_search_app.rs`; two appended tests in tests/terminal_flow.rs;
and this P3 evidence. Existing snapshot/review helpers and P4/P5 implementations
were not edited. `integration/build-integration.patch` is a scoped comparison
against the coordinator's P2 snapshot, preserving unrelated dirty work.

Tests used normal reusable Cargo cache and RAII-owned fixtures. Owned children
and review threads exited; no disposable build directory, background server or
language server was retained. Durable render/log/patch evidence stays in this
folder. The existing shared target cache remains available for later phases.
No commit, Xcode selection change, release build, installation, real Flutter app
or physical-device operation occurred. Windows/physical-device acceptance and the
optional real-Dart prerequisite are not qualified by these mock/PTy checks.

Recap: P3 is integrated with stable source identity, bounded preclone ownership,
shared severity semantics, isolated input, direct explicit commands and layered
rendering. Focused regressions and actual renders pass; source is handed off for
separate simplification and independent phase reviews.
