# P3 App integration UI and interaction review

## Verdict and scope

**Block the P3 integrated UI gate pending the three presentation fixes below.**
Input isolation, source identity/details, minimum-size controls, overlays and
focused process interactions otherwise passed this review. This does not reverse
the earlier standalone SearchView approval. P4/P5 integration remains pending.

Independent read-only review of the stable built/simplified P3 App integration.
Read `build-integration.md`, `simplification-integration.md`, relevant actual
`src/main.rs`, `src/ui.rs`, `src/runs.rs`, SearchView presentation/input code,
`tests/support/console_search_app.rs` and the two real-binary PTY tests. Applied
the previously loaded better-interface domain guidance to the terminal UI.
Established Ratatui density, terminal typography and colors were preserved as
project conventions; no unrelated style changes are proposed.

## Findings

| Severity | Domain | Exact source and rendered evidence | Current behavior | Required fix and user impact |
| --- | --- | --- | --- | --- |
| HIGH | Writing / recovery | `src/main.rs:786` forwards the bare error from `src/console_snapshot.rs:49`; `integration/error-34x12.png`, `error-80x24.png`, `error-120x40.png` | Actual preflight failure says only `retained console screen exceeds the 128 MiB capture estimate limit`. The footer offers refresh/resume, but no action that changes the failing capture condition is explained. | Add concise, real recovery guidance to the App's capture-limit presentation, reachable at the minimum size. For example, start a new session in a narrower terminal before searching it. Do not imply that resizing the existing source alone erases historical width, or that retrying an unchanged source will pass. This follows the error-recovery requirement: the tested real error lacks the instruction present in the earlier synthetic standalone error fixture. |
| HIGH | Layout / error access | `src/ui.rs:1559–1575`; notice storage at `src/main.rs:822–826`, resume overwrites status at `src/main.rs:815`; `integration/operation-error-34x12.png` versus `operation-error-80x24.png` | At 34 columns the sole notice reads `Hot reload: Input queue is full; …`, hiding `wait for Flutter to read input`. The notice has an ellipsis but no details action, scroll path or advertised way to open its complete text. Existing Actions inventory has no operation-message entry. | Keep compact notice shortening, and expose the complete stored operation outcome/error through a clear keyboard-reachable path with a visible cue. Preserve the message long enough to inspect it. The full actionable error must remain accessible at the supported minimum width; resizing the terminal is currently the only way to reveal it, and resuming live replaces the status text. |
| MEDIUM | Writing / mode truthfulness | `src/ui.rs:2107–2108`; `integration-ui-review/help-editing-34x12.png` and `help-source-details-34x12.png`; actual mode handlers at `src/console_search.rs:393–445` and `:455–500` | The single `app.reviewing()` help branch always advertises `n/N matches`, `f filter`, `i source`, `r refresh`, `Esc resume`. In query editing those characters are literal input and Escape stops editing. Inside source details those letters do not run the review commands and Escape returns to the preceding view. | Make the review help reflect editing, reading and source-details modes, or label the reading-only shortcuts explicitly and explain the transition. Retain the truthful `Process controls: Actions` line. Otherwise a person following Help can change their query or fail to leave the mode as described. A review-only reproduction confirms the behavior below. |

The two HIGH findings apply the interface guidance's error-recovery and
inaccessible-truncated-content triggers. They are presentation failures; no
PTY-input leak or incorrect process action was established.

## Exact PNG inventory inspected

Opened **all 45 actual integrated PNGs** using `tools.view_image`, not only source
or TXT/SVG. Paths below are relative to this report's directory.

| Scene | 34×12 app | 80×24 app | 120×40 app |
| --- | --- | --- | --- |
| Editing | `integration/editing-34x12.png` | `integration/editing-80x24.png` | `integration/editing-120x40.png` |
| Reading | `integration/reading-34x12.png` | `integration/reading-80x24.png` | `integration/reading-120x40.png` |
| Empty snapshot | `integration/empty-34x12.png` | `integration/empty-80x24.png` | `integration/empty-120x40.png` |
| No matches | `integration/no-matches-34x12.png` | `integration/no-matches-80x24.png` | `integration/no-matches-120x40.png` |
| Empty filter | `integration/empty-filter-34x12.png` | `integration/empty-filter-80x24.png` | `integration/empty-filter-120x40.png` |
| Capture pending | `integration/pending-34x12.png` | `integration/pending-80x24.png` | `integration/pending-120x40.png` |
| Capture error | `integration/error-34x12.png` | `integration/error-80x24.png` | `integration/error-120x40.png` |
| Historical width | `integration/wide-history-34x12.png` | `integration/wide-history-80x24.png` | `integration/wide-history-120x40.png` |
| Source details | `integration/source-details-34x12.png` | `integration/source-details-80x24.png` | `integration/source-details-120x40.png` |
| Source details scrolled | `integration/source-details-scrolled-34x12.png` | `integration/source-details-scrolled-80x24.png` | `integration/source-details-scrolled-120x40.png` |
| Requested operation | `integration/operation-notice-34x12.png` | `integration/operation-notice-80x24.png` | `integration/operation-notice-120x40.png` |
| Failed operation | `integration/operation-error-34x12.png` | `integration/operation-error-80x24.png` | `integration/operation-error-120x40.png` |
| Actions | `integration/actions-34x12.png` | `integration/actions-80x24.png` | `integration/actions-120x40.png` |
| Help | `integration/help-34x12.png` | `integration/help-80x24.png` | `integration/help-120x40.png` |
| Quit confirmation | `integration/quit-34x12.png` | `integration/quit-80x24.png` | `integration/quit-120x40.png` |

Also generated and opened two actual integrated supplementary frames:

- `integration-ui-review/help-editing-34x12.png`
- `integration-ui-review/help-source-details-34x12.png`

The supplied `help-*` scenes start in reading mode. These two additional frames
confirm the mode-specific gap rather than assuming their coverage.

## Coverage and observed behavior

| Concern | Evidence / observation | Result |
| --- | --- | --- |
| Minimum layout | Actual App header/toolbar/tab strip leave 34×9 for review (`ui::review_area`, `src/ui.rs:1545`). All normal compact scenes retain query, status, three body rows and two hint rows. Source hint fits the bottom border. | Clear. |
| Operation layout | Notice scenes reserve one bottom row (`src/main.rs:654–666`), leaving 34×8 and two body rows while retaining query/filter, refresh and resume. `Hot restart requested` is fully readable and does not imply completion. | Geometry and request wording clear; complete-error access finding above. |
| Frozen versus live | Snapshot title persists while review is active; editing footer explicitly says live output continues. Actual source label has ellipsis and the source-details route. Live child completion is covered by the focused App lifecycle test without replacing captured source identity. | Clear for tested integration. |
| Source identity/details | Long device/configuration input is bounded and visibly marked; details show stable ID, label, device, configuration, outcome and UTC capture value. Compact scrolled image reaches outcome/time and keeps `Esc back`; wide images fit all bounded fields. Saved source metadata is independent of later launch-catalog names in the dedicated test. | Clear. |
| Search and state distinctions | Visible `Search ▸` versus `Search:`; active/secondary match hierarchy; no-match body remains captured text; empty filter, empty snapshot, pending and actual budget error are distinct. Empty snapshot uses `1 row`. Historical-width scenes reveal the combining match at columns 109/63/23 for 34/80/120-wide views. | Clear except error guidance finding. |
| Unicode / typography | Wide `界` and complete base-plus-combining cell remain legible in logs and source metadata; dotted-circle query cue is visible. Existing standalone Unicode/cell assertions were already independently reviewed; integration does not reflow history. | Clear for inspected frames. |
| Overlay layering / keyboard focus | Actions, Help and Quit paint above review. App test checks editing native cursor visible, then hidden under top focus, Actions, Help and Quit; cancel restores active editing review. Query/paste stays isolated under overlays. | Clear; not inferred from cursorless PNGs alone. |
| Actions / process shortcut truthfulness | `src/ui.rs:206–229` hides process r/R/h/v/animation shortcuts while review owns input. Reading search actions advertise their current keys; editing/details actions suppress those shortcuts (`:87–92`, `:273–327`). Real selected-child tests observe direct reload/restart without typing into the query. | Clear for inventory and direct actions. Help mode wording remains a separate finding. |
| Keyboard / pointer input isolation | App tests consume controls, paste and drag events before live selection/PTY; real terminal test confirms Ctrl+s survives terminal flow control, no OSC52 copy is emitted, and query/control input reaches neither child while explicit process commands reach only the chosen one. Two-stage Escape and quit confirmation are covered. | Clear in mock-child PTY environment. |
| Quiet frozen output | App test observes both active and hidden parser revisions advance without output-only redraw. Real-binary flood test sees child log growth, unchanged captured screen and less than 128 bytes of terminal redraw during the check interval. Refresh then observes newer history. | Clear for tested workload. |
| Colors / polish | Existing reviewed SearchView palette is unchanged. Toolbar, review, notice and overlay zones have distinct stable geometry; no content/control overlap observed in all 45 scenes. Status meaning uses labels in addition to color. | No additional finding. Terminal palette transformation and assistive-technology behavior remain unverified. |

## Independent checks

- `cargo test --locked --bin flimru console_review -- --nocapture`:
  **9 passed, 0 failed**, 0.39 s. Includes full inventory/source identity,
  preclone bounds, lifecycle, cursor/overlays, selected-child actions, frozen
  output drain and all 45 actual integrated render layouts.
- `cargo test --locked --test terminal_flow console_search_ -- --nocapture`:
  **2 passed, 0 failed**, 3.64 s. These are the actual Ctrl+s/input-fence/process
  Actions and continuing-child-flood tests, not a static route inspection.
- Read-only supplementary reproduction: compiled a temporary copy of the current
  App test crate with absolute module paths and the current support test module,
  adding one review-only test. Current product modules were included unchanged.
  Used `rustc --edition 2024 --test`, existing normal project dependency rlibs,
  and `CARGO_PKG_VERSION=0.1.0`, then ran
  `console_search_app_tests::independent_mode_help_reproduction`:
  **1 passed**, 0.02 s. It opened Help during editing, saved an actual frame,
  closed Help and pressed `r`, verifying the query changed from `needle` to
  `needler`. It then opened source details and Help, saved another actual frame,
  closed Help and pressed `r`, verifying details stayed open and query stayed
  unchanged. These observations confirm the advertised refresh command is
  inapplicable in those modes. PNGs used the existing actual-cell `evidence`
  helper and `rsvg-convert`.

No source/test edits, subdelegation, app installation, Xcode use, release build or
real Flutter/device operation. Test-owned workers/children exited through their
existing RAII paths. The review-only harness and binary occupied **13,083,761
bytes** in one unique system temporary directory before child processes exited
and automatic cleanup removed the directory. The two supplementary PNG/SVG/TXT
sets are durable review evidence (**145,461 bytes**, about 152 KiB on disk).
Normal Cargo checks used the shared reusable `target` cache and did not create a
disposable build tree. Builder/simplifier lint and broader checks remain their
separate evidence; this review does not claim to have rerun them.

> Recap: inspected all 45 integrated PNGs plus two reproduced mode-specific Help
> frames; nine App tests, two real-binary PTY tests and one review-only
> reproduction passed. Three presentation findings remain: actionable capture
> recovery, full compact operation-message access, and mode-aware Help. P3 UI
> approval waits for their independent recheck; P4/P5 integration is pending.

## Independent integration-fix recheck

**The three original UI findings are closed. The full P3 UI gate remains
unapproved pending the details-only boundary findings below.** Read
`fixes-integration.md`, the actual changed App/SearchView routes, notice retention,
Actions inventory and the new focused tests. No product/test source edits.

### Original finding closure and exact inspected images

Opened all eight affected actual PNGs using `tools.view_image`; unchanged earlier
scenes were not reopened:

- `integration-fixes/capture-recovery-34x12.png`
- `integration-fixes/help-editing-34x12.png`
- `integration-fixes/help-reading-34x12.png`
- `integration-fixes/help-source-details-34x12.png`
- `integration-fixes/operation-notice-34x12.png`
- `integration-fixes/operation-details-34x12.png`
- `integration-fixes/operation-details-end-34x12.png`
- `integration-fixes/operation-details-after-resume-34x12.png`

The actual capture error at `src/main.rs:803` now explains a condition-changing
action: start a new session in a narrower terminal, and says history stays wide.
All three body rows fit at the minimum App size. It does not promise that resizing
existing captured history alone fixes the budget.

Clipped notices visibly advertise `Ctrl+p details:`. The explicit Operation
details entry (`src/ui.rs:106–107`) exposes the entire ordinary queue error and
its remedy through a separate scroll/back view. `last_operation` retention at
`src/main.rs:854` survives Resume and does not claim a new snapshot. The minimum
details images visibly label the message live rather than frozen metadata.
The bounded 4,096-scalar policy has an explicit shortening marker for extreme
messages. These changes close the full-message-access finding.

Help at `src/ui.rs:2121–2132` now branches on operation details, source details,
editing and reading. The three affected Help PNGs correctly describe literal
query typing, Enter to read, Escape to stop editing, reading navigation, or
details scrolling/back as applicable; the operation branch's source wording is
also mode-specific. The original unconditional reading-help finding is closed.

### Confirmed residual findings

| Severity | Location | Reproduction and current behavior | Required correction |
| --- | --- | --- | --- |
| MEDIUM | `src/ui.rs:88` uses actual snapshot review as the Actions hint-ownership predicate; `src/main.rs:1942–1944` consumes keys in operation details | After Resume, open Operation details, then Actions. The actual menu advertises `Search console — Ctrl+s` (`integration-fixes-review/details-only-actions-search-80x24.png`). Sending that exact Ctrl+s closes Actions but leaves operation details open with `reviewing=false`, no source and no capture. Explicitly selecting Search console with Enter instead captures normally. | Make shortcut hints match the details-only input owner, or intentionally route advertised global commands before modal consumption while preserving the input fence. Keep explicit menu selections available. The mismatch is demonstrated for Ctrl+s; other process/navigation labels need the same ownership audit, not an assumed claim that every one is broken. |
| MEDIUM | `src/main.rs:655–659` draws ordinary live content before operation details; live cursor at `src/ui.rs:1374–1376` is not suppressed | With an actual live child, Resume then open retained Operation details. A real TestBackend draw leaves `cursor_visible()` true beneath the details panel. The supplementary reproduction expected it hidden and failed that assertion. The ended-console version stays hidden, explaining why the supplied post-Resume fixture does not expose it. | Suppress the underlying live native cursor whenever operation details owns input, including after Resume when `reviewing=false`. Preserve the current separate snapshot identity semantics. |

The separate correctness reviewer also owns the confirmed Ctrl+e hidden-selection
bypass in this same new boundary; this UI appendix does not claim to have
independently reproduced that selection issue. The coordinator is routing one
coherent boundary fix.

### Independent checks and reproduction limits

- `cargo test --locked --bin flimru console_review -- --nocapture`:
  **12 passed**, 0.42 s.
- `cargo test --locked --test console_search_ui -- --include-ignored --nocapture`:
  **19 passed**, 0.63 s, including native query-focus, complete Unicode cells,
  operation-details ownership and the explicit measurement.
- `cargo test --locked --test terminal_flow console_search_ -- --nocapture`:
  **2 passed**, 3.46 s. Existing raw-terminal/process/flood coverage remains
  passing; these tests do not prove every new Actions hint works.
- A fresh read-only temporary App harness reproduced the Ctrl+s menu mismatch,
  then proved explicit Search console selection works. It next used an owned
  actual live child and failed its assertion that the native cursor should be
  hidden under details. It stopped there; its intended subsequent process-r menu
  experiment did **not run**, and is not reported as a reproduced defect.
- The harness used a temporary copy of current main/support test source with
  absolute paths to unchanged product modules and existing dependency rlibs.
  Python's `TemporaryDirectory` cleaned the scratch tree and test binary on the
  failed-test exception; test-owned child/process state used RAII. No temporary
  artifact is retained. The actual Ctrl+s menu frame was preserved as
  PNG/SVG/TXT under `integration-fixes-review/` (about 240 KiB allocated),
  rasterized with `rsvg-convert` and opened with `tools.view_image`.

No full acceptance is inferred from the passing existing checks: both residuals
are specific to the new no-snapshot operation-details path. Fix and recheck this
boundary before closing the full P3 UI gate; later P4/P5 integration remains
pending.

> Recap: all three original UI findings are resolved in eight inspected affected
> PNGs; 12 App, 19 standalone and two real PTY tests pass. A supplementary actual
> menu interaction confirmed an advertised Ctrl+s mismatch, and a live-child draw
> exposed the underlying native cursor. P3 UI approval awaits their boundary fix.

## Final independent details-boundary closure

**Full P3 integrated UI gate: APPROVE.** This supersedes the earlier UI holds.
All original findings and the subsequently reproduced details-only shortcut and
cursor findings are closed. No actionable UI residual was found in this final
delta. This approval covers P3's inspected integrated UI and interaction scope;
later P4/P5 integration, real Flutter/device qualification and assistive-technology
testing remain separate.

Read the appended residual pass in `fixes-integration.md`, actual changed
ownership/hint/render paths, the live-child regression and targeted grapheme
tests. Opened exactly the three new affected actual PNGs with
`tools.view_image`; unchanged earlier scenes were not repeated:

- `integration-fixes/details-only-actions-34x12.png`
- `integration-fixes/help-operation-details-34x12.png`
- `integration-fixes/details-only-live-34x12.png`

The compact menu visibly retains explicit commands, including selected
**Search console**, without claiming Ctrl+s or other unsupported keyboard
shortcuts. The Ctrl+p/Escape menu chrome remains visible. The operation Help
render describes only its scrolling/back behavior, live-message meaning,
supported menu access and global quit. The live details render keeps the full
ordinary error/remedy and persistent scroll/back hint at 34×12. Cursor visibility
is established by the backend assertion below, not inferred from that PNG.

Resolution checks:

- `App::console_ui_owned()` at `src/main.rs:649–650` explicitly covers actual
  snapshot review **or** operation details. Snapshot/source identity APIs remain
  source-only, so retained operation details after Resume still claim no capture.
- `ui::available_actions` at `src/ui.rs:87–105` suppresses unsupported shortcut
  labels for operation details, preserving the supported Quit shortcut. Select
  text is omitted while the console UI is owned. This closes the reproduced
  Ctrl+s advertisement mismatch without weakening modal input consumption.
- Explicit menu activation remains direct: Search console calls
  `search_console()` at `src/main.rs:2703–2705`; process actions call the direct
  Flutter helper immediately above it. This final inspection confirms the hint
  correction did not convert explicit selections into synthetic key input.
  Prior direct-menu execution evidence remains applicable.
- `src/ui.rs:1334` now skips covered live console drawing for either owner,
  preventing the live native cursor from being scheduled beneath details.
  The new actual live-child test
  `console_review_details_only_owns_live_selection_cursor_and_input_until_closed`
  at `tests/support/console_search_app.rs:332` proves a visible live cursor before
  details, hidden cursor during details, and restored cursor after closing.
- That same independently rerun test invokes Ctrl+e and direct selection
  helpers while details owns input, plus arrows/Shift+arrows, Home/End and Enter.
  No selection appears or clipboard request changes status. After close, normal
  Ctrl+e selection works again; the owned child's log contains only the final
  deliberately entered live `v`. The shared guard therefore closes the adjacent
  hidden-selection boundary as well as the UI cursor defect.
- Inspected the local complete-grapheme `bounded_label` change at
  `src/console_search.rs:1024–1050` and independently ran
  `bounded_source_and_error_labels_preserve_boundary_graphemes_and_exact_fits`
  (`tests/console_search_ui.rs:533`). It preserves complete combining/ZWJ/wide
  boundaries within the existing 96/128/512 scalar budgets, explicit ellipsis,
  and unchanged exact fits. No broader text-policy or layout change was inferred.

Independent final affected checks:

- `cargo test --locked --bin flimru console_review -- --nocapture`:
  **13 passed, 0 failed**, 0.37 s.
- `cargo test --locked --test console_search_ui -- --include-ignored --nocapture`:
  **20 passed, 0 failed, 0 ignored**, 0.63 s. Includes explicit measurements,
  details ownership, grapheme boundaries and existing Unicode/highlight cases.

The unchanged 45-scene review, two real-binary PTY checks and earlier broader
validation remain the separately dated evidence above; they were not relabeled
as fresh final runs. This recheck made no product/test edits, generated no new
renders, created no disposable tree/server/language server, and used the normal
shared Cargo cache. Test-owned children/workers and temporary logs were released
by the existing RAII lifecycle.

> Recap: inspected the three affected actual PNGs and independently passed 13 App
> plus 20 standalone tests. Details-only hints, native cursor and selection
> ownership are corrected; no UI findings remain. P3 integrated UI approved;
> P4/P5 integration remains pending.
