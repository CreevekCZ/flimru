# P1 separate fixes and validation

Status: **fixed and ready for independent re-review**. This report does not approve
its own phase gate. Original dirty baseline remains at
`/private/tmp/flimru-productivity-p4tfofri/baseline`.

## Fixes

1. Device-only `App::merge` preserves the selected entry by exact source index,
   including a rejected duplicate-name entry with its explanation open. It no
   longer restores the first matching display name. Catalog-replacement selection
   policy remains the P2 integrator's responsibility.
2. `start_batch` captures the presented assessment's action intent before fresh
   preflight. A blocked **or checking** row opens or refreshes explanation on this
   activation even if external repair now makes it ready. Dismissing the modal
   does not launch; a later explicit ready Run does. Presented ready Run retains
   the fresh, atomic all-target preflight and consumes those plans.
3. Diagnostics input and rendering share `diagnostics_layout`, including actual
   Unicode/word wrapping and content-height calculation. Keyboard, wheel and resize
   clamp the stored offset, so a reverse event immediately moves from the bottom.
4. Panel titles now explicitly use RGB(151,163,180), retaining the quiet border.
   Count and Diagnostics titles have **6.681:1** contrast against RGB(22,28,39),
   compared with the review's 1.401:1. Tests assert actual rendered title-cell
   foreground/background and both ready/blocked counts at all three sizes.
5. Presentation assessment runs on **one owned background worker**, with immutable
   `Arc<LaunchFile>`/tool/target snapshots, a latest-job mailbox, cancellation
   generation, and a bounded two-chunk result channel (128 entries/chunk). Tick
   accepts at most two chunks before returning to PTY/input work. Stale results
   cannot overwrite newer target/catalog state. Shutdown sets the stop predicate
   under the condition-variable mutex, wakes and joins the owned worker; poisoned
   locks are recovered consistently. No filesystem/planner work occurs during
   draw/tick. An OS metadata call on a slow mount may delay the worker (and its
   orderly join at exit), but cannot stall the running UI's metadata preview.
6. The view caches visible indexes and ready/blocked/checking counts by catalog
   allocation, target id/platform/label, adapter, query and unavailable toggle.
   Progress updates maintain the index list incrementally. Cached reads do not
   rescan the full catalog; picker rendering constructs only viewport ListItems.
   Unchanged discovery scans retain completed assessments. Checking is explicit,
   separate from blocked and target-required; it exposes details and no Run.
   There is no entry-count truncation. `LaunchFile` semantic equality is unchanged.

`App.launch` is now `Arc<LaunchFile>`: replacement must install an Arc snapshot,
then use the P2 selection policy and merge. An unchanged Arc plus unchanged targets
retains preview state. Test-only in-place edits use `Arc::make_mut`, giving a new
allocation while the previous worker/view owns its old snapshot. A future owning
source edit must install a new Arc or explicitly invalidate its view.

## Regression evidence

- Rejected duplicate source remains selected after discovery merge while its
  explanation is open; subsequent Enter cannot start its runnable sibling.
- App test uses an executable fixture and file spawn counter for both blocked and
  pending → externally ready. Explanation and modal-dismissal start zero children;
  a later explicit Run starts exactly one child in each case.
- Real-binary PTY repeats the external cwd-repair sequence and checks no process
  log exists before explicit Run; then exactly one spawn is recorded.
- App and real-binary PTY both send 50 Down/wheel-down events, reverse once, and
  verify immediate movement. Resize to 120×40 clamps to zero and fitting content
  cannot overscroll. App test also covers Home and long/wide/combining text.
- Worker tests cover target snapshots with queued stale results, repeated source
  indexes in a replacement catalog, and latest-generation acceptance.
- 100,000 entries under 4 MiB fill in the background while a real owned PTY emits
  heartbeat output. Every entry remains present, target changes reject older
  generations, and unchanged discovery retains its generation and counts.

## Responsiveness measurement

Command: `cargo test --locked --bin flimru large_catalog_presentation_stays_responsive -- --nocapture`.
Actual debug results are in `fixes-performance.log` (100,000 entries, 3,088,910 bytes):

| Operation | Measured duration |
| --- | ---: |
| Initial merge / presentation context | 6.617 ms |
| Pending 80×24 draw | 2.304 ms |
| Target change before first fill | 0.0135 ms |
| Target change with all 100,000 cached assessments | 2.154 ms |
| Maximum observed tick during both fills, PTY draining included | 8.479 ms |
| Maximum observed draw during fill | 2.461 ms |
| Unchanged discovery merge | 5.703 ms |
| Cached 80×24 draw | 0.603 ms |

The supported interactive target is the existing 33 ms frame budget for local
metadata presentation. These measurements satisfy that target on this machine;
regression assertions use a broader 100 ms debug/loaded-CI ceiling per UI operation.
They are not release/hardware/network-filesystem timing guarantees. Background
completion time is deliberately separate from input/PTY responsiveness. Large
catalog parse/reading remains P2's background responsibility.

## Exact final checks

- `cargo test --locked --bin flimru --test availability --test terminal_flow`:
  **67 app/worker/render passed + 3 availability passed + 21 PTY passed**;
  **1 ignored** real-Dart prerequisite. See `fixes-all-focused-tests.log`.
- `cargo clippy --locked --all-targets -- -D warnings`: passed, `fixes-clippy.log`.
- `FLIMRU_P1_EVIDENCE_DIR=docs/productivity-features-evidence/p1 cargo test --locked --bin flimru configuration_diagnostics_render_evidence -- --nocapture`:
  passed, `fixes-render.log`; regenerated real TestBackend SVG/TXT plus PNG via
  `rsvg-convert`. Ten states × three sizes, including checking.
- `cargo fmt --all` and `git diff --check`: passed. No blanket lint suppression
  or relaxed existing assertions. Existing synchronous preview tests now wait
  explicitly for the worker, and launch PTYs wait for the rendered Run control.
- Actually opened final PNGs: excluded-hidden 34×12; excluded-shown 80×24 and
  120×40; checking 34×12 (including the final distinct pending icon/title);
  blocked-explanation-scrolled 34×12; blocked-explanation 80×24. Counts/title text
  is legible, pending says checking, compact modal retains scroll/close hints.

Earlier local logs retain corrected setup/assertion failures: old tests needed
worker synchronization; the new pending assertion initially mistook the count
separator for a blocked reason; a worker test initially expected a per-field code
instead of the parser's existing `configuration` rejection code. All final checks
above pass. No live Flutter/device, release build, installation or independent
security qualification was performed by this fixer.

## Ownership and recap

Owned changes: main/UI availability integration, `src/availability.rs`, the new
`src/availability_worker.rs`, related App/UI tests and two terminal-flow tests.
Independent console snapshot/review/reload/export/batch helpers were not edited.
No commit, installation, selected-Xcode action or real Flutter process occurred.
Fixture child processes are RAII-owned and reaped; their TempDirs clean up on
success/failure. No separate fixer-owned temporary tree or background process is
retained. Normal reusable project `target` is 4.2 GiB and intentionally retained.
P1 render/report evidence is 6.6 MiB. The coordinator's 664 MiB task temporary tree
(including its baseline and audit build) remains coordinator-owned and untouched.

Recap: all five confirmed review findings are addressed with behavioral, rendered,
worker and stress regressions. Independent P1 re-review remains required.

## Async integration follow-up after independent correctness re-review

The separate correctness reviewer closed the original findings and reproduced two
additional integration defects. Both are now fixed; independent re-review is
still required before the gate closes.

**Passive context updates preserve the selected source through asynchronous fill.**
The view retains the desired source across target label/platform/adapter changes
within the same catalog Arc. While that source is absent from the partially
populated hidden-unavailable list, `config_index == usize::MAX` explicitly means
no actionable selection. No other ready row receives its highlight or Run action.
Each bounded result batch reconciles the exact source by binary search; the
anchor survives partial insertion until the source is restored and filling
finishes. A source that remains hidden/blocked retains no actionable fallback.
Unchanged merges also preserve an explicitly cleared selection.

Deliberate navigation and selection clear the deferred anchor: Up/Down/Home/End
(and j/k), query/paste edits, filter clearing, unavailable toggle, opening fresh
diagnostics, starting a fresh device/configuration choice, and mouse row selection.
Missing selection is normalized before arrow arithmetic, avoiding overflow.
Compact partial-fill views explicitly say `Checking selected availability`; the
toolbar says `Checking…`, and wide details name/explain the desired source.

The P2 integration has two explicit helper contracts:

- `set_config_source_selection(Some(source_index))`: after installing the chosen
  catalog Arc, retain that exact source through pending availability. It becomes
  actionable only if present in the visible list. `None` clears selection and
  keeps it non-actionable through merge/worker updates until a deliberate choice.
- `set_config_selection(index)`: explicit picker-index choice; clears any deferred
  source anchor. P2 should apply its unique-name/content selection policy through
  these helpers rather than editing the view's `RefCell` fields.

**Fresh synchronous preflight metadata wins over queued previews.** The view now
fences every synchronously assessed entry for its current generation. Worker
results cannot replace that entry's newer assessment, in either direction. New
catalog/target/adapter context resets the fence and may assess it again. Fresh
Run still plans all targets at the actual launch boundary; pending/blocked Enter
still opens explanation and starts no child.

Deterministic tests drain actual queued worker results into a held batch without
presenting it, then apply it after fresh preflight. They cover queued ready →
fresh blocked and queued blocked → fresh ready, retaining exact fresh reasons,
counts, modal/no-spawn behavior and the fence reset on a new context. Selection
regression applies source0's ready result while source1 is still pending,
asserts no highlight/Run, then restores source1; both label and platform discovery
changes are covered, and a later `/bin/echo` fixture Run uses `Selected second`.
Further tests cover all deliberate transitions and P2's explicit empty-selection
helper across merge.

Actual `partial-fill-selection-{34x12,80x24,120x40}.{txt,svg,png}` evidence was added
from the real TestBackend UI using held actual worker results. All three final
PNGs were opened and inspected: ready source0 remains unhighlighted, Run is
absent, checking counts/footer remain legible, and wide details identify pending
source1. These are distinct from the earlier all-checking render. The generation
now produces **11 named states × 3 sizes**.

Final follow-up checks:

- `cargo test --locked --bin flimru --test availability --test terminal_flow`:
  **71 app/worker/render + 3 availability + 21 PTY passed; 1 Dart ignored**.
  See `fixes-async-followup-tests.log`.
- `cargo clippy --locked --all-targets -- -D warnings`: passed,
  `fixes-async-followup-clippy.log`.
- `FLIMRU_P1_EVIDENCE_DIR=docs/productivity-features-evidence/p1 cargo test --locked --bin flimru configuration_diagnostics_render_evidence -- --nocapture`:
  passed; all SVG/PNG/TXT evidence regenerated. See
  `fixes-async-followup-render.log`.
- `cargo test --locked --bin flimru large_catalog_presentation_stays_responsive -- --nocapture`:
  passed. Latest 100,000-entry / 3,088,910-byte debug measurement:
  initial merge **6.395 ms**, pending draw **2.071 ms**, target change before fill
  **0.0088 ms**, full-cache target change **2.050 ms**, maximum tick **9.018 ms**,
  maximum draw **0.897 ms**, unchanged merge **3.749 ms**, cached draw **0.600 ms**.
  See `fixes-async-followup-performance.log`; original timing caveats still apply.
- Targeted rustfmt with `skip_children=true` and `git diff --check`: passed.

The follow-up touches only main/UI cache integration and its tests. No independent
helper module was changed. Fixture processes and TempDirs remain RAII-owned and
cleaned; no new disposable tree was retained. Evidence is now about 7.3 MiB.

Recap: both reproduced async gaps are fixed, deterministic regressions and actual
partial-fill renders are included, and the source is held stable for the original
correctness/UI reviewers. This report does not approve its own gate.

## Rejected-catalog diagnostics follow-up

The correctness reviewer closed both async defects, then reproduced a remaining
UI workload proportional to all rejected entries: selected diagnostics and device
details cloned/wrapped the complete legacy aggregate warning vector. That path
is now fixed; the source is held for independent correctness/UI re-review.

`SourceEntry.warnings` records warnings from that exact source position, including
duplicate-name entries. `LaunchFile.file_warnings` records only whole-file
warnings such as unsupported compounds. The existing `LaunchFile.warnings`
remains the complete aggregate with exactly the prior strings/order, including
skipped-entry errors, for `--check` compatibility. Classification happens during
parsing; no warning-prefix prose is parsed to infer ownership. Parser acceptance,
rejection, launch semantics and semantic equality policy are unchanged.

Selected explanations/details now show the selected entry's reasons and warnings
plus file warnings. Every row retains its own unavailability reasons, including
parser reasons while filesystem-backed assessment is still checking. Sibling
entries' warnings are not copied into this path. No-selection diagnostics give
ready/blocked/checking and warning counts and ask the user to select an entry.
Device details give an O(1) aggregate count summary and file warnings. These paths
never iterate the aggregate warning vector.

Presentation has explicit bounds before wrapping: 16 displayed warnings per
selected/file group, 512 characters per reason/warning, and 256 per configuration
label. An omitted-warning count or a shortened-text notice points to `--check`
for full output. All source strings and warnings remain in the model/CLI stream;
reason lists and catalog entries are not dropped. Picker row labels use the same
bounded presentation, so one extreme label cannot make a draw wrap megabytes.
The compact modal still scrolls the selected content and reaches the explicit
omission notice. No full-catalog warning layout/cache is constructed on input.

Regression coverage now includes structural per-entry ownership with identical
names, file-warning inclusion, exact legacy warning order, selected/sibling
isolation, no-selection/device summaries, 40 warnings and extreme Unicode label
bounds, explicit omission notices and compact end/reverse scrolling. A new
100,000-rejected-entry / **2,300,037-byte** fixture keeps a real owned heartbeat
PTY draining while opening/drawing Diagnostics, pressing Down, using the wheel,
and resizing across 34×12, 80×24 and 120×40. All 100,000 source diagnostics remain
present; selected first/middle/last rows expose their own reason and file warning.

Latest debug timing (`fixes-warning-performance.log`):

| Rejected-catalog UI operation | Maximum measured |
| --- | ---: |
| Open modal | 2.430 ms |
| Draw | 2.999 ms |
| Down key (both shared-layout clamps included) | 0.514 ms |
| Wheel down (both clamps included) | 1.007 ms |
| Tick including live PTY drain and worker results | 9.123 ms |
| Resize plus actual draw | 4.146 ms |

These actual debug measurements are below the 33 ms local target and the test's
broader 100 ms loaded-CI ceiling. They qualify this rejected-heavy fixture,
not hardware/release/network-filesystem timing. The previous ready-heavy probe
remains in the suite.

Final checks:

- `cargo test --locked --lib --bin flimru --test availability --test terminal_flow`:
  **57 library + 73 app/worker/render + 3 availability + 21 PTY passed**;
  **2 marked ignored** (watchdog-only subprocess child and real-Dart prerequisite).
  See `fixes-warning-tests.log`.
- `cargo clippy --locked --all-targets -- -D warnings`: passed,
  `fixes-warning-clippy.log`.
- `cargo test --locked --bin flimru rejected_catalog_diagnostics_are_bounded -- --nocapture`:
  passed, `fixes-warning-performance.log`.
- Real TestBackend render generation passed with
  `FLIMRU_P1_EVIDENCE_DIR=docs/productivity-features-evidence/p1 cargo test --locked --bin flimru configuration_diagnostics_render_evidence -- --nocapture`;
  see `fixes-warning-render.log`. All SVG/PNG/TXT files regenerated via
  `rsvg-convert`. Added selected-warning, no-selection warning-summary, and
  scrolled omission-notice states: **14 states × 3 sizes**.
- Actually opened final PNGs: warning-explanation 34×12 and 80×24;
  warning-summary-explanation 80×24; warning-limit-explanation-scrolled 34×12 and
  80×24. The selected reason remains scrollable, selected and file warnings are
  distinct, siblings are absent, and the compact bottom visibly says
  `24 more warnings; use --check for full output.`
- Targeted rustfmt with `skip_children=true` and `git diff --check`: passed.

This follow-up changes main/UI/config and their tests only. No independent helper
module, installed binary or Xcode instance was changed. Test children/TempDirs
remain owned and cleaned; no fixer-owned disposable tree is retained. Normal
reusable project target cache is now about 4.3 GiB; retained P1 evidence is about
9.5 MiB. The coordinator's temporary baseline/audit tree remains untouched.

Recap: selected diagnostics now have bounded relevant work for both ready and
rejected-heavy catalogs, full warning compatibility/access is retained, and
independent re-review is required before closing the P1 gate.
