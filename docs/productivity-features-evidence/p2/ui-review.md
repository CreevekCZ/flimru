# P2 independent rendered UI and interaction review

Reviewed 2026-11-06 after the separate integration builder and simplifier.

**P2 rendered UI gate: PASS. No actionable UI findings identified.** This is an
independent UI verdict; the separate correctness/performance review and final
integration gate remain the coordinator's responsibility.

Read `build-integration.md`, `simplification-integration.md`, the approved phase
requirements in `../../productivity-features-workflow-2026-11-06.md`, and the
settled P1 UI review. Inspected the current reload/apply, header, toolbar, action,
diagnostics, focus, selection and input paths. No product source or test edits,
commits, installation, GUI/Xcode action or subdelegation were performed. Unwired
search/export dialog internals and unrelated dirty work were excluded.

## Actual rendered evidence inspected

All **24 P2 PNGs** below were opened directly with `tools.view_image`. This was
not a code-only review. Corresponding TXT/SVG names use the same stems. The
simplifier's retained comparison establishes byte-identical TXT/SVG output after
its source changes; the independent render test also passed during this review.

| State | Exact PNGs opened in this directory |
| --- | --- |
| Last-good error | `reload-error-34x12.png`, `reload-error-80x24.png`, `reload-error-120x40.png` |
| Error with active filter editing | `reload-error-filter-34x12.png`, `reload-error-filter-80x24.png`, `reload-error-filter-120x40.png` |
| Error diagnostics modal | `reload-error-details-34x12.png`, `reload-error-details-80x24.png`, `reload-error-details-120x40.png` |
| Accepted recovery | `recovered-34x12.png`, `recovered-80x24.png`, `recovered-120x40.png` |
| Accepted empty catalog | `empty-34x12.png`, `empty-80x24.png`, `empty-120x40.png` |
| Removed selection, ready replacement | `no-selection-34x12.png`, `no-selection-80x24.png`, `no-selection-120x40.png` |
| Manual request pending | `manual-pending-34x12.png`, `manual-pending-80x24.png`, `manual-pending-120x40.png` |
| Preserved selected source still checking | `selected-pending-34x12.png`, `selected-pending-80x24.png`, `selected-pending-120x40.png` |

Also opened these **six retained P1 PNGs** directly to check the unchanged shared
row/modal renderer with a long name and wide/combining Unicode:

- `../p1/excluded-shown-34x12.png`, `../p1/excluded-shown-80x24.png`, `../p1/excluded-shown-120x40.png`.
- `../p1/blocked-explanation-34x12.png`, `../p1/blocked-explanation-80x24.png`, `../p1/blocked-explanation-120x40.png`.

The P2 images themselves use `Stable 界 é` and `Replacement 界 é`; they do not
claim an extreme-length reload-specific label fixture. The retained P1 images
show the long label clipped within the 34-column row and wrapped in the modal,
with scroll/close hints intact. The independently rerun diagnostics tests cover
extreme bounded text and the original model's unshortened content.

## Observations and interaction evidence

- **Persistent error and last-good policy:** yellow `Config error` precedes the
  workspace in the header, so the complete marker survives at 34×12. It remains
  visible with filter editing and the diagnostics modal. The accepted rows stay
  visible and Run remains available under an automatic failure. At 120×40 the
  details explicitly distinguish the reload error and last-good catalog from the
  selected row's readiness. At compact sizes `i` opens the explanation; at 34×12
  `Using the last good catalog.` is already visible in its initial viewport and
  the remainder is reachable by scrolling. Error text contains no source secret.
- **Recovery and accepted catalogs:** recovery clears the header error and stale
  error details. Empty catalogs display `Empty configurations array.` rather
  than retaining old rows or presenting an error. The empty state gives repair
  guidance at the larger sizes and a readable empty explanation at 34×12.
- **Manual pending launch guard:** all sizes show `Reloading` in the header and
  a non-Run toolbar state. The footer says to wait and then choose Run explicitly;
  wide details repeat that rule. Existing readiness text underneath describes
  the retained catalog; the pending message is first and toolbar/menu actions
  enforce the guard. The rendered test verifies Run is absent from Actions and
  the toolbar. App and real-binary PTY tests verify an immediate `r` + Enter does
  not start a process and completion does not queue a later launch.
- **Explicit choice after removal/ambiguity:** ready replacement rows have no
  highlight or leading selection marker. The toolbar says `Select a row` (with
  ordinary compact truncation at 34×12), the footer requests a selection, and
  the wide details request an entry. Enter does not launch a replacement. Source
  mapping tests cover removals and ambiguous duplicate names on both sides.
  Asynchronous results preserve an explicitly cleared selection.
- **Deferred pending source:** the ready sibling remains unhighlighted while the
  catalog heading reports one checking/one ready source. Toolbar/footer show
  Checking; wide details identify the pending `Other` source. No ready sibling
  inherits its identity or Run action. The worker/app regressions verify the
  preserved source and explicit navigation behavior.
- **Filter, modals and session continuity:** error/filter renders retain the query
  and Apply/Clear row. App tests independently confirm apply preserves page,
  filter/edit state, modal flags, marked targets, favorites, unrelated status,
  and retained console selection. Modal keyboard/paste precedence tests pass.
  PTY tests confirm active output and child identity survive failed/corrected
  file saves while console shortcuts remain session input.
- **Keyboard/focus/pointer and resize:** independently rerun TestBackend checks
  verify row/control hit rectangles at exactly 34×12, 80×24 and 120×40; focus
  visibility and non-overlapping toolbar targets across the minimum and larger
  widths; the last-drawn toolbar viewport; and filter Apply/Clear targets. Real
  binary compact Actions navigation/pointer, filter/favorites/resize, and modal
  keyboard/wheel reverse/resize tests pass. The existing P1 readable title style
  and bounded diagnostics scrolling remain intact. Compact toolbar/footer text
  still uses the established clipping behavior; essential state and dismissal
  cues remain visible, and focused actions are brought into view.

## Independently executed checks

All commands completed successfully using the normal shared Cargo cache.
`ui-review-checks.log` retains output for the seven interaction commands below.
The reload command's complete result was observed directly in this review's tool
output; its pre-existing builder/simplifier logs remain separate evidence.

| Command | Passed |
| --- | --- |
| `cargo test --locked --bin flimru --test terminal_flow reload -- --nocapture` | 10 App/render tests + 4 real-binary PTY tests |
| `cargo test --locked --bin flimru --test terminal_flow pointer -- --nocapture` | 6 App/render tests + 1 real-binary PTY test |
| `cargo test --locked --bin flimru --test terminal_flow diagnostics -- --nocapture` | 5 App/render tests + 3 real-binary PTY tests |
| `cargo test --locked --bin flimru --test terminal_flow filter -- --nocapture` | 5 App/render tests + 1 real-binary PTY test |
| `cargo test --locked --bin flimru top_actions -- --nocapture` | 1 App test |
| `cargo test --locked --bin flimru every_top_action -- --nocapture` | 1 render/hit-region test |
| `cargo test --locked --bin flimru action_menu_is_modal -- --nocapture` | 1 App test |
| `cargo test --locked --bin flimru configuration_rows_and_toolbar_buttons -- --nocapture` | 1 hit-region test |

These are **39 passing test executions**, with overlap between filters; they are
not claimed as 39 distinct regressions. No ignored test was selected. The reload
run also measured the existing 100,000-entry regression: semantic duplicate apply
19.464 ms, reordered/changed apply 23.776 ms, next heartbeat-draining tick 0.403 ms.
This is observed local debug timing, not a new platform qualification.

## Ownership, limits and recap

No new review-owned temporary directory or background process was created.
Existing test fixtures own/reap their children and remove fixture storage. The
normal shared `target` cache is about **4.4 GiB** and intentionally remains for
the ongoing workflow; this reviewer did not create or remove an isolated cache.
Retained P2 evidence is about **5.5 MiB**. Only this report and the named check log
were added by this review; the existing renders were not overwritten.

Evidence is TestBackend rendering and real-binary PTY interaction with fixture
tools. It does not qualify physical Flutter devices, arbitrary network filesystem
timing, platform font rasterization, release packaging or installation.

Recap: inspected all 24 P2 images and six shared-renderer long/Unicode images;
independent reload, input, focus, pointer, modal and resize checks passed. No P2
UI fix is required. The rendered UI gate passes within the stated automated/mock
scope, with the settled P1 style and diagnostics behavior preserved.

## Affected-state review after queued-input fix — 2026-11-06

**Affected UI verdict: one P3 wording finding; behavior checks pass.** The earlier
all-state pass predates this follow-up. No unsafe Run activation, persistent-error
regression, input loss or new modal was observed. The low-priority finding below
requires a separate wording fix/re-review or explicit coordinator disposition;
this section does not independently close the new affected-state UI gate.

Read `fixes-integration.md`, inspected the new guard/idle acknowledgement and
affected presentation paths, and opened all six new actual PNGs directly with
`tools.view_image`:

- `input-drain-34x12.png`, `input-drain-80x24.png`, `input-drain-120x40.png`.
- `input-drain-error-34x12.png`, `input-drain-error-80x24.png`, `input-drain-error-120x40.png`.

### P3: Describe the reload transition in user-facing terms

The new header/toolbar says `Finishing input` / `Finishing input…`
(`src/ui.rs:483-491`, `src/ui.rs:1138-1146`), while the guard/footer/details say
`Finishing queued input. Choose Run when ready.` (`src/main.rs:868-873`). All six
images expose the event-queue implementation rather than naming the reload
transition that temporarily disables Run. In the compact success image the
header clips to `Finishing inp`; the footer preserves the complete queue wording.
The outcome status already says the launch configurations reloaded, so the extra
input operation is especially hard to interpret without knowing the fence design.

Use a clear reload-progress label such as `Finishing reload…` in the header and
toolbar, and a guard/footer instruction such as `Finishing reload…` / `Choose Run
when it appears.` Keep the truthful success/failure outcome and persistent error
marker. This is a copy issue, not a request to change the input fence or flush
input. Expected verification: regenerate the six affected renders, inspect all
three sizes, and keep the existing Run-guard/fresh-Run regressions passing.

### Verified affected behavior

- Success and failure drain states have no Run in toolbar or Actions. The
  presentation uses the same guard as `start_batch`; the toolbar action is
  ExplainConfiguration rather than NextOrRun. Existing focus reconciliation
  removes an invalid former Run target, and rendered controls/hit regions share
  the same generated button list. No new pointer rectangle calculation was added.
- The error header remains `Config error`, including at 34×12; the wider details
  retain the last-good explanation. The guard message precedes the selected
  catalog's readiness text. Success retains the reloaded outcome status. On idle,
  an exact guard-owned status is restored to success/failure, and unrelated status
  is preserved.
- Worker-pending remains Reloading. Completion alone does not expose Run; the
  main loop acknowledges actual idle, ends that pass and redraws the ready state
  before consuming later Run input. No delayed launch or confirmation dialog is
  introduced. Queued navigation/filter/quit-cancel remain actionable.

Independently executed after this fix:

- `cargo test --locked --bin flimru --test terminal_flow manual_reload -- --nocapture`:
  **3 App + 3 real-binary PTY tests passed**. Covers ordinary keyboard/menu routes,
  all five App entry points, successful/failed completion, automatic supersession,
  stale generation, event-count/time-budget queued bursts through keyboard and
  toolbar, no queued launch, and a fresh Run starting exactly one child with the
  accepted arguments or last-good arguments after failure.
- `cargo test --locked --bin flimru configuration_reload_render_evidence_and_run_gating -- --nocapture`:
  **1 passed**, including all new drain success/failure states and all original
  P2 states at the required sizes, with no Run action while guarded.

No source/test edit, overwritten image, new temporary build directory or retained
background process was created. Cargo reused the normal ongoing-workflow cache.

Recap: all six new PNGs inspected; seven independent focused tests passed; the
new safety transition works. One P3 copy correction remains: describe finishing
the reload rather than finishing queued input.

## Final wording re-review — 2026-11-06

**Final P2 rendered UI gate: PASS. The P3 wording finding is closed; no remaining
actionable UI findings were identified.** This verdict supersedes the pending
affected-state verdict above. The original all-state review and independently
passed queued-input/fresh-Run behavior checks remain applicable.

Read the separate fixer's `UI wording follow-up` in `fixes-integration.md` and
inspected the current strings. Header/toolbar now use `Finishing reload…`; the
guard/details say `Finishing reload… Choose Run when it appears.`; footer shows
the same instruction on two lines. The success/failure outcome strings and
`Config error` marker remain unchanged. No former input-queue wording remains in
the affected product labels.

Directly reopened all six regenerated actual PNGs with `tools.view_image`:

- `input-drain-34x12.png`, `input-drain-80x24.png`, `input-drain-120x40.png`.
- `input-drain-error-34x12.png`, `input-drain-error-80x24.png`, `input-drain-error-120x40.png`.

The footer displays the complete reload label and `Choose Run when it appears`
at all three sizes, including 34×12. Ordinary compact header/toolbar clipping
remains, with full explanatory copy visible below. Larger sizes also show the
reload label in header/toolbar and the complete instruction in details. No Run
toolbar action appears while guarded. The failure images retain the persistent
error marker and failure outcome; the wide last-good explanation is intact.

This was the requested image/string-only recheck. No unchanged behavior suite
was repeated; the preceding seven independent affected tests and the separate
fixer's scoped tests/lint/format evidence cover the preserved fence behavior.
Only this report was appended; no product edit, new temporary artifact or
background process was created.

Recap: all six changed images and product strings verified; the reload transition
is clear, success/error outcomes are preserved, and the P3 finding is closed.
The P2 rendered UI gate passes within the existing automated/mock scope.
