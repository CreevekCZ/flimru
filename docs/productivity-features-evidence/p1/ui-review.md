# P1 independent rendered TUI and interaction review

Reviewed 2026-11-06 after the separate simplification stage. Scope: P1 catalog
availability, diagnostic picker, compact explanation, and launch/input isolation.
The independently developed search/export/batch helpers were excluded. Product
source and existing tests were not edited by this reviewer.

Verdict: **two P2 findings require fixes and re-review**. No blocked launch or
modal input escape was observed in the checks below.

## Findings

### P2: Clamp the stored diagnostics scroll position at the actual content end

`src/main.rs:979-981` and `src/main.rs:1487-1491` increment/decrement
`diagnostics_offset` without bounding it to the rendered content. The renderer
clamps only its local scroll argument at `src/ui.rs:1839-1844`. Repeated Down or
wheel-down events therefore accumulate an invisible overscroll position. After
reaching the end, reversing direction can require dozens of Up events before any
text moves. Enlarging the terminal can similarly leave an offset beyond the new
maximum. This makes the compact overlay appear stuck during ordinary navigation.

Confirmed against the real binary with a temporary copy of the existing
`tests/terminal_flow.rs` PTY harness:

1. Use a valid catalog with one rejected configuration named
   `Very long unsupported configuration 界 é`, unsupported `type`, and a compound
   warning. Start the actual debug executable and resize to 34×12.
2. Press `i`, then send Down 50 times. The overlay reaches its last content lines.
3. Send Up once: the entire screen remains identical. Send Home: the screen
   changes back to the top, confirming that there was scrollable content.

The temporary test asserted the current faulty behavior so its passing result
means **reproduction confirmed**, not that the feature meets the desired behavior.
The supplied `blocked-explanation-34x12.png` and
`blocked-explanation-scrolled-34x12.png` also demonstrate why navigation is needed:
the final warning is outside the initial compact viewport.

Expected fix and regression: keep stored offset within the actual wrapped-content
maximum for the current viewport. After many Down/wheel-down events at the bottom,
one Up/wheel-up must move one line immediately. Cover resize from a narrow to a
taller/wider viewport, Home, and content that fits without scrolling. Prefer one
shared content/maximum calculation so rendering and input use the same bounds.

### P2: Give the new ready/blocked count title a readable foreground

The availability counts at `src/ui.rs:1513-1522` are rendered through `panel`,
whose title inherits the border foreground at `src/ui.rs:1408-1413`:
RGB(45,54,68) on the panel's RGB(22,28,39). The calculated luminance contrast is
**1.401:1**. In the actual PNGs the title is barely visible while adjacent body
text is readily legible. This matters particularly when unavailable rows are
hidden: the count is the primary indication that additional blocked entries exist.

Reproducible supplied states include `excluded-hidden-34x12.png`,
`excluded-hidden-80x24.png`, and `excluded-shown-120x40.png`; the same faint style
also affects the new Diagnostics overlay title. Preserve the quiet border while
giving the count/title text a readable explicit foreground.

Expected regression: assert an explicit readable title style for the ready/blocked
counts, regenerate and visually inspect those states at all three sizes, and
confirm the 34-column title still shows both counts without losing the toggle or
blocked-row cues. A textual cell-presence assertion alone does not verify this.

## Rendered evidence actually inspected

All following PNGs were opened with `tools.view_image`; this was not a code-only
review. The README and simplification report were read before inspection.

| Size | Images viewed |
| --- | --- |
| 34×12 | `excluded-hidden-34x12.png`, `excluded-shown-34x12.png`, `no-filter-matches-34x12.png`, `empty-34x12.png`, `rejected-only-34x12.png`, `no-devices-34x12.png`, `no-target-explanation-34x12.png`, `blocked-explanation-34x12.png`, `blocked-explanation-scrolled-34x12.png` |
| 80×24 | `excluded-hidden-80x24.png`, `excluded-shown-80x24.png`, `no-filter-matches-80x24.png`, `empty-80x24.png`, `rejected-only-80x24.png`, `no-devices-80x24.png`, `blocked-explanation-80x24.png`, `blocked-explanation-scrolled-80x24.png` |
| 120×40 | `excluded-shown-120x40.png`, `no-target-explanation-120x40.png`, `blocked-explanation-scrolled-120x40.png` |

The 20 images cover every supplied named state and all three required sizes.
The compact modal wraps the Unicode label and reason, keeps `Esc close` visible,
and reveals the remaining warning after scrolling. Empty catalogs, rejected-only
catalogs, no text-filter matches, and no discovered device have distinct text.
Blocked selected rows expose explanation cues instead of Run; wide details and
the compact modal show corrective guidance without fixture secrets. At 34×12
toolbar/footer truncation is visible, while `u unavailable`, `d diagnostics`, and
the modal dismissal hint remain available.

## Checks actually run

- `cargo test --locked --bin flimru --test terminal_flow diagnostics -- --nocapture`:
  **4 passed**: two application/render tests and two real-binary PTY tests. These
  cover rejected/empty startup, no-device diagnostics access, 34×12 resize,
  keyboard/pointer blocked launch, modal paste isolation, and dismissal.
- `cargo test --locked --bin flimru configuration_rows_and_toolbar_buttons_use_rendered_hit_regions -- --nocapture`:
  **1 passed**, covering rendered row/control hit regions at 34×12, 80×24, 120×40.
- `cargo test --locked --bin flimru every_top_action_is_visible_when_focused_and_matches_its_pointer_target -- --nocapture`:
  **1 passed**, covering keyboard focus visibility and matching pointer targets
  across pages and widths including the required minimum/maximum sizes.
- Temporary copied PTY harness reproduction described above: **1 reproduction
  confirmed**. First compilation lacked two explicit external-crate arguments;
  after supplying them, its initial run confirmed the bug but a later teardown
  wait expected an untruncated toolbar label. The corrected final run passed and
  exited/reaped its launcher through the existing harness. No source fix was made.
- Direct standard-sRGB luminance calculation for the actual title/panel RGB
  values: **1.401:1**.

Normal reusable project `target` storage was used. Three task-owned temporary
harness directories were removed on every outcome; final compiled harness output
was about 3.1 MB before removal. Fixtures used the existing RAII cleanup and
owned-child shutdown. No background process or temporary deliverable is retained.
The supplied PNG/SVG/TXT evidence remains intentionally retained by the P1 builder.

## Recap

Reviewed actual rendered states, compact Unicode explanations, disabled Run cues,
focus/hit regions, no-device access, and modal input isolation. Six existing
focused tests passed. Two actionable UI issues remain: accumulated overscroll and
low-contrast availability counts. Both need a separate fix and independent
re-review; no live Flutter/device, release, or installation qualification occurred.

## Independent re-review after separate fixes — 2026-11-06

**Current UI gate verdict: PASS. Both original P2 findings are closed. No remaining
actionable UI findings were identified in this re-review.** This verdict supersedes
the initial finding status above, which remains as the review/fix history.

Read `fixes.md`, inspected the stable source changes, and opened **all 30 updated
PNG images** with `tools.view_image`. Specifically, each of these ten stems was
viewed with all three `-34x12.png`, `-80x24.png`, and `-120x40.png` suffixes:

- `checking`
- `excluded-hidden`
- `excluded-shown`
- `blocked-explanation`
- `blocked-explanation-scrolled`
- `no-target-explanation`
- `rejected-only`
- `empty`
- `no-filter-matches`
- `no-devices`

The ready/blocked counts and Diagnostics title now have an explicit readable
foreground (`src/ui.rs:1417-1422`). Both counts remain visible at 34×12. Independent
calculation from the actual RGB values confirms **6.681:1** contrast, and the
regenerated images visibly resolve the original faint-title problem. The quiet
border is retained.

Stored scroll offset now uses the same wrapped layout as rendering
(`src/ui.rs:1843-1907`), with clamps before/after keyboard and wheel input
(`src/main.rs:1150-1163`, `src/main.rs:1670-1682`) and during viewport updates
(`src/ui.rs:394-395`). The focused application test verifies immediate one-line
reverse movement after 50 keyboard/wheel events, resize clamping, fitting content,
and Home. The real-binary PTY test independently verifies visible immediate reverse
movement and 34×12 → 120×40 resizing. Updated narrow PNGs retain the scroll/close
hint and expose the remaining warning; wide initial/scrolled PNGs coincide because
the complete content now fits without accumulated overscroll.

The new pending state is clear at all three sizes: count text says `checking`,
row text says `checking availability`, the pending marker differs from the blocked
marker, and the toolbar offers `Checking…` rather than Run. The 120×40 details
explicitly request a later Run after completion. At 34×12, the count and pending
description fit sufficiently to identify the state, while Enter/i explanation
cues remain visible. Pending and blocked activation preserve explanation intent
even when a fresh external check becomes ready; the independently rerun spawn-count
tests confirm dismissal itself starts no child and a later explicit ready Run
starts exactly one.

Checks actually rerun for this re-review:

- `cargo test --locked --bin flimru --test terminal_flow diagnostics -- --nocapture`:
  **6 passed** (3 application/render, 3 real-binary PTY). Includes the corrected
  stored-scroll/resize regression, regenerated-state render assertions including
  actual count foreground/background, no-device access, blocked keyboard/pointer
  launch, modal paste isolation, and dismissal.
- `cargo test --locked --bin flimru blocked_or_pending_enter_only_explains_then_a_later_ready_run_spawns_once -- --nocapture`:
  **1 passed**, including both blocked and pending → ready activation paths.
- `cargo test --locked --test terminal_flow displayed_blocked_enter_after_external_repair_explains_before_any_process_starts -- --nocapture`:
  **1 passed**, confirming external cwd repair requires a subsequent explicit Run.
- Direct standard-sRGB calculation: title contrast **6.681:1**.

No product source/test edits, new temporary harness, real Flutter/device run,
release build, or installation was performed. Normal reusable `target` storage
was reused; there are no new task-owned disposable build directories or retained
background processes from this re-review. The existing rendered evidence remains
intentional.

Recap: both UI findings are independently verified fixed; all 30 updated images
were inspected; eight focused tests passed; pending availability is readable and
does not offer implicit launch. P1 passes the independent rendered UI gate within
the automated/mock validation scope.

## Affected-UI re-review after async selection follow-up — 2026-11-06

**Affected-UI verdict: PASS. No actionable findings in the new deferred-selection
state or reviewed transitions.** The earlier scroll/contrast closures remain
unchanged; this re-review is limited to the async selection follow-up described
in the appended `fixes.md` section.

Opened these three new actual PNGs with `tools.view_image`:

- `partial-fill-selection-34x12.png`
- `partial-fill-selection-80x24.png`
- `partial-fill-selection-120x40.png`

In each image, `Ready first` is present without a selection highlight or marker;
the heading reports one checking/one ready entry; the toolbar says `Checking…`
and offers no Run. The footer explicitly says `Checking selected availability`
and offers arrows to choose. These cues remain legible at 34×12. At 120×40 the
details identify `Selected second` and explain its pending assessment, so the
ready sibling is not presented as the pending source's identity or action.

Reviewed source selection/restore helpers (`src/main.rs:820-846`), deliberate
navigation normalization (`src/main.rs:1521-1544`), paste/filter and row-selection
paths, and unselected list rendering (`src/ui.rs:1587-1592`). The absence of an
actionable selection remains explicit until the desired source becomes visible
or the user deliberately chooses another row. Arrow handling normalizes the
absent index before arithmetic, and pointer hit testing uses bounded visible-row
coordinates. Deliberate row/query/toggle choices clear the deferred anchor so a
later worker batch cannot restore an unwanted earlier selection.

Checks actually rerun, all with `cargo test --locked --bin flimru <name> -- --nocapture`:

- `hidden_unavailable_selection_survives_target_context_and_partial_async_fill`:
  **1 passed**. Holds actual worker results, exposes only the ready sibling with
  no highlight/Run, checks Enter starts no process, restores exact source1, repeats
  a target-platform change, then confirms the explicit fixture Run uses
  `Selected second`.
- `deliberate_picker_transitions_release_a_deferred_source_without_overflow`:
  **1 passed**. Exercises Up/Down/Home/End, query paste, unavailable toggle,
  diagnostics entry and mouse row activation; later results preserve the user's
  deliberate source0 choice.
- `source_selection_apply_helper_preserves_an_explicit_empty_selection_through_merge`:
  **1 passed**. Empty selection survives merge/worker update without Run; an
  explicit Down selects the first visible row.
- `configuration_diagnostics_render_evidence_and_minimum_size_interactions`:
  **1 passed**. Includes all three partial-fill TestBackend sizes, no Run action,
  actual unhighlighted row background, count checks, and the existing modal and
  secret-safety assertions. Existing evidence files were not rewritten by this run.

No product source/test edits or extra temporary output were created. Normal
reusable project build storage was used; no review-owned process or disposable
directory remains.

Recap: inspected the three affected renders and reran four targeted tests. The
deferred source is clearly pending, ready siblings stay unselected, deliberate
navigation is safe, and worker completion restores the correct source. The P1
rendered UI gate remains passed within the existing automated/mock scope.

## Affected-UI re-review after warning-layout follow-up — 2026-11-06

**Final affected-UI verdict: PASS. No actionable UI findings were identified in
the warning ownership, summary, bounded presentation, or scrolling changes.**
Read the appended rejected-catalog diagnostics section in `fixes.md` and reviewed
the associated selected/file warning paths. Earlier scroll/contrast and deferred
selection closures remain in force; this was not a repeated all-state sweep.

Opened all nine affected PNGs with `tools.view_image`: each of
`warning-explanation`, `warning-summary-explanation`, and
`warning-limit-explanation-scrolled` at `34x12`, `80x24`, and `120x40`.

Observed behavior:

- Selected explanations retain the selected configuration name and actionable
  invalid-argument reason. At the larger sizes, `Configuration warnings:` and
  `File warnings:` clearly separate the selected unknown-field warning from the
  compound warning. No sibling name, sibling rejected-type value, or fixture
  secret appears in these images; the render test also asserts this boundary.
- The no-selection modal asks for an entry and reports ready/blocked/checking and
  total-warning counts. It directs full-catalog inspection to `--check`, without
  showing sibling warning content. At 34×12 the initial viewport contains the
  selection guidance and catalog summary, with the scroll/close hint retained.
- The scrolled omission state shows **`24 more warnings; use --check for full
  output.`** at all three sizes. At 34×12 the complete notice wraps into two
  readable lines and remains above `↑↓ scroll · Esc close`. Omission disclosure
  therefore remains reachable at the minimum supported size.
- Presentation is bounded before layout: selected names, reason/target text and
  individual warnings receive explicit shortening markers; each warning group
  has its own displayed-count limit. Source labels and all original warning
  records remain intact for CLI access.

Evidence boundary: the supplied omission PNG fixture uses the ordinary label
`Many warnings` and 40 short unknown-field warnings. Its images visibly verify
the **24-more** notice. Extreme repeated wide/combining Unicode label and warning
shortening are verified by the application test and source review below; a
`shortened` marker is not visible in these nine supplied PNGs, and is not claimed
as screenshot evidence here.

Checks actually rerun:

- `cargo test --locked --bin flimru --test terminal_flow diagnostics -- --nocapture`:
  **8 passed** (5 application/render, 3 real-binary PTY). Includes selected/sibling
  warning isolation with duplicate display names, file-warning inclusion,
  no-selection/device summaries, extreme text bounds, full original model
  retention, compact warning-end/reverse navigation, actual count styling,
  secret-safe rendered states, ordinary keyboard/wheel reverse and resize,
  no-device access, and blocked/modal input isolation.
- That run also executes the owned-heartbeat-PTY rejected-heavy responsiveness
  regression. On this independent run, 100,000 rejected entries / 2,300,037 bytes
  measured modal open **1.959 ms**, maximum draw **2.395 ms**, Down **0.349 ms**,
  wheel **0.368 ms**, tick **8.595 ms**, and resize/draw **3.691 ms**. This confirms
  the tested local debug workload remains responsive; it does not qualify other
  hardware or slow network filesystem behavior.
- `cargo test --locked --lib warning_scope_is_structural_and_legacy_aggregate_order_is_preserved -- --nocapture`:
  **1 passed**, confirming structural warning ownership and unchanged full CLI
  aggregate strings/order.

No product source/test edits, replacement render files, new temporary harness,
release build, installation, or real Flutter/device run were performed. Normal
reusable build storage was used; no review-owned background process or temporary
directory remains.

Recap: inspected nine affected renders and reran nine focused tests, all passing.
Selected and file warnings remain distinguishable, sibling content is isolated,
the minimum-size omission notice is reachable, and ordinary scrolling remains
correct. The P1 rendered UI gate remains passed within the stated evidence scope.
