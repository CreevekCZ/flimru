# P3 standalone search-view simplification

Reviewed the complete `src/console_search.rs` implementation and
`tests/console_search_ui.rs`, alongside the standalone search-view build evidence.
Only `src/console_search.rs` required changes; tests, snapshot/review helpers,
shared App/UI files and export UI were left unchanged.

## Changes

- Retained the incoming capture source ID directly before moving its metadata into
  the view. Worker capture no longer rereads the newly assigned `Option` with an
  unreachable fallback source ID of zero.
- Computed the Ctrl/Alt modifier predicate once in keyboard routing and reused it
  for editing Backspace, typed characters and reading shortcuts. The previous
  three copies expressed the same policy; Escape, Enter and the exact Ctrl+u
  override retain their original precedence and behavior.

Reviewed the final edits after validation: source normalization does not alter
the ID, and the shared modifier predicate is identical to all three original
guards. No API, layout, text, frozen source metadata, worker lifecycle, query
budget, snapshot ownership, search algorithm, cell mapping or viewport clipping
behavior changed. Further reductions would obscure the distinct pending/capture,
editing/reading and byte/cell-coordinate responsibilities, so they were avoided.

## Validation

- `cargo test --locked --test console_search_ui -- --include-ignored --nocapture`:
  **12 passed, 0 failed, 0 ignored**. See
  `simplification-search-view-tests.log`.
- The included debug viewport measurement rendered ten 120×40 frames from
  1,200,000 ASCII bytes captured in 60,000-column physical rows in **27.55 ms**.
  This fixture measurement does not establish every workload's performance.
- `cargo clippy --locked --test console_search_ui -- -D warnings`: passed; see
  `simplification-search-view-clippy.log`.
- `rustfmt --edition 2024 --check src/console_search.rs tests/console_search_ui.rs`:
  passed. Scoped whitespace validation passed.
- The render-evidence test exercised its 36 standalone/minimum-integration
  fixture layouts during this run. Rendering code was untouched, so the builder's
  existing TXT/SVG/PNG artifacts remain applicable and were not regenerated.

Normal reusable project `target` storage was used. This stage created no
disposable build/cache directory, background process or media output. The two
retained validation logs are approximately 8 KiB on disk; test-owned review
workers and fixture screens were released through RAII.

## Handoff

The standalone search-view API remains stable for later App integration. This
stage is a simplification pass and self-check, not independent correctness or
rendered integration acceptance. Final P3 integration still needs its affected
tests, simplification, correctness/performance and rendered-UI gates.

Recap: removed an unreachable capture-ID fallback and repeated modifier checks;
12 tests, strict scoped Clippy and formatting passed; visual behavior and API
remain unchanged.
