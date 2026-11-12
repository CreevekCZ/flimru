# P3 integration review fixes

Addresses the single P2 correctness finding in `correctness-integration.md` and
all three presentation findings in `ui-integration.md`. Source scope is
`src/main.rs`, `src/ui.rs`, `src/console_search.rs`,
`tests/support/console_search_app.rs` and `tests/console_search_ui.rs`.
Previously reviewed snapshot/review helpers, budgets, cell maps and unrelated
P1/P2 implementation were not changed. No commits or subdelegation.

## Corrected behavior

1. **Console-only animation keyboard shortcut.** The Ctrl+a keyboard route now
   checks `Page::Console` before calling the existing direct animation helper.
   The explicit Console Actions route remains direct and unchanged. A new test
   creates a real owned live Session hidden behind each of Devices and Configs,
   invokes the actual shortcut, and verifies picker page/status, animation state
   and stop state remain unchanged. The unavailable controller would otherwise
   produce an observable animation error, so an empty-session fixture cannot
   accidentally pass this regression.
2. **Capture-limit recovery.** The actual preclone CaptureLimit presentation is:
   `128 MiB capture limit. Start a new session in a narrower terminal; history stays wide.`
   It fits all three body rows in the supported 34×12 App / 34×9 review area.
   It requests a new session, and does not imply that resizing existing history
   or retrying an unchanged source removes its retained-width cost.
3. **Complete retained operation outcome.** A clipped notice advertises
   `Ctrl+p details:`. Actions exposes **Operation details**, including after
   Resume, using the full retained latest message. The stored sanitized outcome
   is bounded to 4,096 Unicode scalars. Exceptional oversized messages have an
   explicit `… [shortened]` marker; ordinary errors retain their complete remedy.
   Lifecycle notices use the same retention path, so a details cue always refers
   to its actual message.
4. **Truthful Help.** Query editing explains literal typing/paste, Enter to read,
   and Escape to stop editing. Reading retains the match/filter/source/refresh
   bindings. Source details explains independent scrolling and Escape/Enter/i
   back. Operation details is separately identified as live information. Every
   review Help mode retains `Process controls: Actions`.

## Small details API and state boundaries

The existing bounded SearchView details rendering, grapheme wrapping and scroll
handler now accept a private Source/Operation kind. The only additional public
entry points are:

```rust
view.show_operation_details(message);
view.showing_operation_details();
```

No generic App modal framework was added. Operation details are explicitly
labelled **Latest operation outcome (live; not frozen metadata)**. They preserve
query, filter, edit mode, current match, frozen snapshot and console viewport.
Capture/search results can update their ordinary underlying review state without
replacing the open operation details.

`SearchView::is_active()` and App `reviewing()` continue to mean an actual
source review. Opening retained operation details after Resume leaves both false,
with no source and no snapshot; it does not capture a screen or claim a frozen
console. Closing returns to ordinary live UI. A subsequent Ctrl+s captures
normally. Keyboard/paste/pointer input stays isolated while the details-only view
is open, and global quit remains usable. The existing worker is reused; a first
details request can create it without capturing data, with a nonfatal error
presentation if worker creation fails.

Self-review corrected a compact recovery line that initially wrapped its final
word beyond the three available body rows, matched lifecycle notice retention
to its details cue, preserved exact-width notices without unnecessary clipping,
and kept first-worker creation failure nonfatal. Final affected checks passed
after these corrections.

## Exact checks

```sh
FLIMRU_P3_FIX_EVIDENCE_DIR=docs/productivity-features-evidence/p3/integration-fixes cargo test --locked --bin flimru console_review -- --nocapture
cargo test --locked --bin flimru --lib --test terminal_flow --test console_search_ui --test console_snapshot --test console_review
cargo test --locked --test console_search_ui -- --include-ignored --nocapture
cargo clippy --locked --all-targets -- -D warnings
rustfmt --edition 2024 --check --config skip_children=true src/main.rs src/ui.rs src/console_search.rs tests/console_search_ui.rs tests/support/console_search_app.rs
```

- Final focused App recheck: **12 passed**, zero failures, **0.42 s**. Three new
  App tests pin the keyboard guard, recovery/Help and details-after-Resume path.
  Existing real-child isolation coverage now also opens operation details after
  Resume, types process/stop characters, pastes and clicks, then observes a
  deliberate final input marker. Neither child receives the details input.
- Broader focused integration command: **223 passed**, zero failures, five
  existing intentional ignores: library 57, App 96, worker 7, standalone 18,
  snapshot 17, actual-binary PTY 28. Timings were **2.01 / 13.02 / 0.01 / 0.24 /
  0.02 / 8.88 s** respectively. This includes P1/P2 regressions and the 45 existing
  integrated render scenes. The final narrow recheck above additionally covers
  self-review's lifecycle-retention/error-path adjustments.
- Final standalone command with explicit measurements: **19 passed**, zero
  failures/ignores, **0.62 s**. The new standalone test pins pending-query versus
  operation-details ownership and the no-source/no-snapshot close path.
- Final strict all-target Clippy: passed, **0.75 s**. Scoped formatting passed;
  child modules were excluded with `skip_children=true`. No whole-repository
  formatting was run.
- Both actual-binary console-search PTY tests also passed separately in
  **3.61 s**, covering actual Ctrl+s delivery, process/query isolation and quiet
  frozen output under child flood.
- Final explicit ten-frame debug measurements: overlapping captured widths
  1/32/120 took **45.564208 / 49.782042 / 71.087709 ms**; the unchanged 1.2 MB
  wide-row/no-query case took **18.623166 ms**. Existing cell/performance budgets
  pass. These local timings are not release guarantees.

## Affected rendered evidence and resources

`integration-fixes/` contains only eight affected compact scenes, each retained
as actual TestBackend TXT/SVG and rasterized PNG. All eight PNGs were opened and
visually inspected using the image-viewing tool:

- `capture-recovery-34x12`: all three recovery lines are visible.
- `help-editing-34x12`, `help-reading-34x12`,
  `help-source-details-34x12`: mode-specific instructions and process Actions.
- `operation-notice-34x12`: visible keyboard details cue.
- `operation-details-34x12`, `operation-details-end-34x12`: complete input-queue
  error and remedy with a persistent scroll/back hint.
- `operation-details-after-resume-34x12`: retained live outcome with no snapshot
  claim and ordinary console chrome underneath.

The 24 durable files total **587,992 bytes** (about 632 KiB allocated). Existing
evidence was preserved. Normal reusable `target` storage measured **4.7 GB** and
is retained for project checks. No task-owned disposable build tree, scratch
directory, installed app, server or language server was created. All tool runs
exited; test-owned children/workers and temporary files use existing RAII cleanup.

Recap: all four confirmed integration findings are fixed; focused App,
standalone, broad integration/PTY and strict checks pass. Stable source is held
for the original correctness/UI reviewers. Their independent gate recheck and
later P4/P5 acceptance remain separate.

## Residual details-only ownership and Unicode-boundary pass

The original reviewers confirmed that details-only mode still bypassed the
source-only selection guard, retained the covered live native cursor, and
advertised unavailable Ctrl+s in Actions. This pass fixes that common boundary
with `App::console_ui_owned()`: actual source review **or** operation details.
`reviewing()`, `reviewed_source()` and snapshot ownership remain source-only.

The shared UI/input predicate now protects the early keyboard-selection branch,
Ctrl+e entry, direct selection helper, covered console rendering, selection and
scroll actions, and raw-output-only redraw policy. It also supplies truthful
process-stop guidance and the action inventory's covered-console rules. Operation
details suppress all unsupported shortcut hints; the supported Ctrl+p menu cue
and global Ctrl+q remain truthful. Explicit menu commands remain available. No
new navigation behavior or modal framework was introduced.

One added real-child App regression observes a visible normal PTY cursor before
opening details and its suppression while covered, then normal restoration on
close. It invokes Ctrl+e plus direct selection helpers, arrows/Shift+arrows,
Home/End, Enter and Escape without creating selection or requesting a clipboard
copy. Normal Ctrl+e works after closing. A deliberate final live input marker
proves none of the details selection/navigation input reached the child. Existing
details-after-Resume coverage now inspects the actual Actions inventory/menu and
operation Help: Search console has no Ctrl+s claim, Select text is absent, and
unsupported commands carry no shortcut. Source/snapshot remain absent.

The requested small Unicode correction is local to SearchView `bounded_label`.
It keeps the existing sanitized one-scalar lookahead, retains only complete
graphemes within the same scalar budget, then adds ellipsis. It drops an entire
boundary combining/ZWJ cluster rather than popping one scalar from a partial
cluster. The App source-label sentinel is unchanged. Added tests cover 96/128/512
budgets, combining marks, ZWJ emoji, wide characters and unchanged exact fits.
Existing bounded metadata tests allow a shorter complete-grapheme prefix instead
of requiring every scalar slot to be filled.

Residual validation:

```sh
FLIMRU_P3_FIX_EVIDENCE_DIR=docs/productivity-features-evidence/p3/integration-fixes cargo test --locked --bin flimru console_review -- --nocapture
cargo test --locked --test console_search_ui -- --include-ignored --nocapture
cargo clippy --locked --all-targets -- -D warnings
rustfmt --edition 2024 --check --config skip_children=true src/main.rs src/ui.rs src/console_search.rs tests/console_search_ui.rs tests/support/console_search_app.rs
```

- Final affected App recheck: **13 passed**, **0.39 s**, including owned live
  child/cursor/selection and source-less menu assertions.
- Standalone: **20 passed**, **0.65 s**, including explicit measurements and new
  grapheme boundaries. Ten overlapping frames at widths 1/32/120 measured
  **49.169250 / 52.231958 / 72.954875 ms**; wide/no-query **18.359958 ms**.
- Strict all-target Clippy passed, **1.31 s** including shared-cache lock wait;
  scoped formatting passed. The previous broad 223-test result is historical
  evidence above; it was not redundantly rerun for these residual changes.

Three additional actual-cell TXT/SVG/PNG scenes were generated and all three
PNGs opened with the image-viewing tool: `details-only-actions-34x12`,
`help-operation-details-34x12`, `details-only-live-34x12`. The corrected compact
menu visibly omits Ctrl+s, operation Help advertises only supported controls,
and the live scene retains the scroll/back hint. Native cursor visibility is
verified by the real TestBackend assertion, not inferred from its PNG.
These nine additional deliverables total **211,615 bytes**. No new disposable
tree/server was created; the test child, workers and temp input log use RAII and
all checks exited. Normal shared `target` cache is retained.

Recap: residual selection/cursor/shortcut ownership and bounded-label graphemes
are corrected; thirteen affected App tests, twenty standalone tests, strict
Clippy and scoped formatting pass. Stable source is held for the original
correctness/UI reviewers' final residual recheck before P4/P5 writes.
