# P3 App integration simplification

Reviewed the builder integration patch against the completed P2 baseline,
`build-integration.md`, the actual integration source and affected App/standalone
search/PTY tests. Compared the final stage edits with the coordinator's saved
`/private/tmp/flimru-productivity-p4tfofri/p3-built` snapshot. This stage changed
only `src/main.rs` and `src/ui.rs`; all other scoped source/test files and unrelated
dirty work remain unchanged.

## Changes

- Reused `source_label` for device, configuration and outcome metadata, plus the
  saved-console fallback configuration. The helper performs the same control
  filtering and retains the same 129-character bound. Its extra character is now
  documented as the sentinel that lets SearchView mark truncated metadata.
- Removed duplicate selection and scrollbar reset assignments after capture
  preflight. Both remain explicitly released before any Screen clone; neither can
  be repopulated between that release and handing the capture to SearchView.
- Replaced the four repeated Flutter action additions with one small ordered
  table. Labels, shortcut visibility, command characters and inventory order are
  identical in live and review modes.
- Added the nonreview Select text entry alongside Search console in the existing
  console-page branch, removing a second equivalent page/review condition while
  preserving its exact inventory position.

The final source was reviewed again after edits and checks. No API, feature,
layout, input precedence, process-command routing, source identity, lifecycle,
capture/search budget or byte/cell mapping changed. Explicit cancellation,
normal/alternate history bounds, source-switch rules and overlay layering were
left in their existing focused paths. The earlier foundation/view simplification
passes remain applicable.

## Validation

- `cargo test --locked --bin flimru --test console_search_ui`: **110 passed**
  (93 App/worker/render and 17 standalone search tests), **one existing viewport
  measurement ignored**. `integration/simplification-tests.log`.
- `cargo test --locked --test terminal_flow console_search_`: **2 passed**;
  27 unrelated PTY tests filtered out. These exercise actual Ctrl+s delivery,
  review input/selection isolation, explicit process actions, refresh/resume,
  quit and quiet output during child flood. `integration/simplification-pty.log`.
- `cargo clippy --locked --all-targets -- -D warnings`: passed;
  `integration/simplification-clippy.log`.
- Scoped `rustfmt --edition 2024 --config skip_children=true --check` and
  `git diff --check` passed for all permitted integration source/test files.
  No whole-project formatter was used.
- The App render test exercised all **45 integrated layouts**, and the
  standalone render tests also passed. Rendering code, inventory content and
  order are unchanged, so the builder's retained TXT/SVG/PNG artifacts remain
  applicable and were not regenerated.

The final simplification-only patch is retained at
`integration/simplification.patch`. No new tests were added for these
behavior-preserving reductions. Foundation tests, unrelated PTY tests and
measurements were not repeated; their builder/earlier-stage evidence remains
separate from this affected-check run.

## Handoff and ownership

Source is coherent and ready for independent P3 correctness/performance and
rendered integration reviews. This report does not close those gates or
authorize P4/P5 integration. No commit, release, installation or physical-device
qualification was performed.

Checks used the normal reusable project `target` cache and RAII-owned test
fixtures. No disposable build directory, background process, language server or
new media artifact was created. Only small durable validation logs and the stage
patch were added; the coordinator's existing snapshots and builder render
deliverables remain intentionally retained.

Recap: simplified bounded source metadata and action inventory construction,
removed duplicate capture resets, preserved all behavior, and passed 112 affected
tests, strict all-target Clippy and scoped formatting/whitespace checks.
