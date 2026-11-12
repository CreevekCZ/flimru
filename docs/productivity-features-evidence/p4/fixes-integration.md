# P4 integration fixes

Separate fix pass for the three P2 findings in `correctness-integration.md` and
the visible-Quit P3 in `ui-integration.md`. Read those reports and the builder /
simplifier handoffs. Scope remained four confirmed fixes; no broad refactor,
new feature, dependency, backend change, P5/P6 edit, commit, or installation.

## Changes

- `src/main.rs:2651`: before export mouse routing, admit only the actual
  `ui::mouse_action` Quit action when its cell is outside the dialog's last
  rendered coverage. Route it through the existing `activate(Quit)` confirmation
  path. Other toolbar/background input stays consumed. `ExportDialog::covers`
  (`src/export_dialog.rs:127`) records the exact cleared modal rectangle, or the
  full screen for its tiny-layout fallback; before its first draw the background
  is treated as covered. This prevents a hidden compact/tiny Quit hitbox from
  activating through the modal, while retaining a visible right-edge Quit even
  in the shallow 120×12 layout. No standalone appearance or editing state changed.
- `src/console_search.rs:618`: a height-only review viewport change no longer
  marks an old current match for reveal. Existing clamping still handles actual
  available rows; width changes still reveal the current match, and deliberate
  query/match navigation still requests reveal. The first operation-notice row
  therefore preserves a manually selected review offset after export cancellation.
- `src/console_search.rs:28`: raise the shared bounded operation-message budget
  to **8192 characters**, sufficient for an accepted 4096-byte destination plus
  success prefix and byte count. Generic exceptional outcomes remain bounded
  with the existing shortened marker. `docs/usage.md:203` documents that bound.
- `src/main.rs:1069`: `shutdown_export` returns the sanitized retained
  `last_operation` outcome after reporting it. The final post-restoration print
  now receives the same safe message as status / Operation details, including a
  canonical path reached through a clean input symlink whose target contains
  terminal controls. Success/byte-count truth is retained when publication won.

The existing-notice covered/uncovered viewport behavior was already correct and
was left intact. No changes were made to capture identity, worker ownership,
filesystem publication, query/filter scope, or explicit submission semantics.

## Regressions and checks

Four focused actual-App regressions were added in
`tests/support/log_export_app.rs`:

1. Locate the actual rendered `Q` cell and its real Quit hit action at 80×24,
   120×40, and 120×12. Click it, verify ordinary confirmation (no immediate exit
   or export), cancel by the actual Cancel hit region and by Enter, and verify
   the identical destination / snapshot Arc survives. Explicit confirmation still
   returns the existing exit signal. Ordinary Help/background input stays
   consumed; obscured Quit at 34×12 and tiny 29×11 does not activate.
2. Capture 100 rows with the sole match at row zero, manually End-scroll away,
   open/render export, cancel into the first operation notice, and render again.
   Offset, query, and frozen Arc remain unchanged; explicit next-match returns
   to the current match. Affected resumed-review render is retained.
3. Format and retain a successful **4096-byte ASCII absolute path** made of 31
   128-byte directory components and a 96-byte filename, with `usize::MAX` as
   the byte count. The complete exact outcome survives status / retained state
   and scrollable rendered Operation details. An over-8192-character outcome
   still gets a bounded explicit marker. This synthetic successful-result
   boundary does not claim macOS can create a 4096-byte filesystem path.
4. Actually export through a clean symlink whose canonical parent contains
   ESC `[31m`, wait for publication without polling away the completion, then
   shut down. The returned truthful success equals retained status, includes
   filename/count, has no control characters, and the saved content is correct.

`tests/console_search_ui.rs:325` separately exercises height-only shrink/grow
with the current match outside manual scroll, followed by explicit next-match
and query changes. Existing width-resize / horizontal reveal tests remain green.
The existing dialog hit-region regression now also checks fail-closed coverage
before first draw and exact background/field/button coverage after drawing.

Final validation:

- `cargo test --locked --bin flimru log_export -- --nocapture`: **10 passed**,
  zero failures; `fixes-integration-tests.log`.
- `cargo test --locked --test console_search_ui --test export_dialog_ui`:
  **20 search tests + 16 dialog tests passed**, zero failures; the existing
  explicit measurement test remains ignored. `fixes-integration-ui-tests.log`.
- `cargo clippy --locked --bin flimru --tests -- -D warnings`: passed;
  `fixes-integration-clippy.log`.
- Scoped `rustfmt --edition 2024 --check --config skip_children=true` for
  `src/main.rs`, `src/export_dialog.rs`, `src/console_search.rs`,
  `tests/support/log_export_app.rs`, `tests/console_search_ui.rs`, and
  `tests/export_dialog_ui.rs`: passed; `fixes-integration-fmt.log`.

Self-review preserved actual visible-versus-obscured mouse coverage and normal
confirmation semantics. An initial retained-details assertion assumed an
unwrapped numeric count; it now joins actual viewport rows without border or
padding cells, so normal terminal wrapping is accepted while complete data is
still checked. Strict Clippy caught the new coverage accessor being unused in
the standalone test build; meaningful coverage assertions resolved it without
suppressing lint. Current-source checks above pass without warnings. Prior
unchanged backend / 209-test evidence was not repeated.

## Render evidence and resources

`fixes-integration-renders/` retains exactly four actual App TestBackend
TXT/SVG/PNG triplets: `quit-confirmation-80x24`,
`quit-confirmation-120x40`, `quit-confirmation-120x12`, and
`cancel-notice-manual-review-80x24`. All four PNGs were opened and visually
inspected: the standard quit confirmation covers the export dialog with its
actual Quit/Cancel controls; the shallow scene leaves the global Quit visible
outside the centered dialog; the resumed review shows the manually chosen late
rows with the cancellation notice rather than jumping to the old first match.

Generated with `FLIMRU_P4_APP_EVIDENCE_DIR` in the focused App command and
`rsvg-convert`. Redundant unchanged ready/error/review scenes emitted by the
existing fixture were removed only from this task-owned new evidence directory;
the builder's original nine scenes and standalone evidence remain untouched.
The four retained triplets total **1,008,017 bytes** (about 0.96 MiB).

Normal reusable Cargo artifacts were used. RAII temporary fixture roots, symlink
targets, child sessions, and workers were cleaned up after tests. No task-owned
temporary build directory, background process, language server, or app instance
remains. Source edits are limited to the six Rust paths above plus the one usage
bound paragraph; `src/ui.rs`, export backend, and P5/P6 files were not edited.

## Handoff / recap

All four confirmed integration findings have passing targeted regressions.
Shared source is stable and released for the original correctness and UI
reviewers to recheck. This fixer does not approve P4; the coordinator retains
independent gate decisions, live acceptance, and any later installation.
