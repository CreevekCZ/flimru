# P2 integration simplification

Status: **simplification complete; ready for independent correctness and UI
review**. This is a scoped simplifier handoff, not phase approval.

## Scope and changes

Read the actual P2 integration delta in `build-integration.patch` against the
coordinator's `p1-complete` snapshot, the builder report, the current reload
worker, and the integration source/tests. Compared the final edits with the
coordinator's `p2-built` snapshot to verify the stage stayed inside the P2 delta.

The new flow is already compact. Its manual-request/generation, last-good error,
semantic duplicate, and both-catalog identity branches represent distinct
required behavior, so they were retained. The simplification makes only these
changes:

- `src/main.rs`: use explicit count/iterator/entry names in source preservation;
  rename the duplicate fingerprint predicate to `same_entry`; combine the error
  marker replacement and previous-value comparison with `Option::replace`; test
  `previous_error.is_none()` directly instead of maintaining a redundant alias.
- `src/ui.rs`: share the identical Run-action exclusion assertion across the
  three P2 render states that need it. This changes only the internal render test.
- `src/config_reload.rs` and `tests/terminal_flow.rs`: inspected but unchanged in
  this integration simplification stage. The bounded startup loader and reviewed
  worker/subprocess watchdog implementation remain intact.

No public API, catalog equality check, iteration order, lock scope, worker cadence,
selection semantics, status text, UI layout, session handling, or PTY scenario was
changed. In particular, equality remains short-circuited for manual completion and
error recovery; both sides of duplicate source mapping still require uniqueness.
No console search/export module or unrelated baseline file was edited.

## Checks

All commands ran in the shared normal Cargo cache. New evidence logs are retained
beside this report.

| Command | Result |
| --- | --- |
| `cargo test --locked --bin flimru --lib --test availability --test terminal_flow` | Passed: 57 library, 82 App/worker/render, 3 availability, 24 real-binary PTY tests; 166 passed, 2 intentionally ignored total. `simplification-focused-tests.log` |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed. `simplification-clippy.log` |
| `rustfmt --check --edition 2024 --config skip_children=true src/main.rs src/ui.rs src/config_reload.rs tests/terminal_flow.rs` | Passed; only these files were formatted. |
| `git diff --check -- src/main.rs src/ui.rs src/config_reload.rs tests/terminal_flow.rs` | Passed. |
| `cargo test --locked --bin flimru reload_large_catalog_apply -- --nocapture` | Passed. `simplification-performance.log` |
| `cargo test --locked --bin flimru configuration_reload_render_evidence -- --nocapture` | Passed. `simplification-render.log` |

The optional real-Dart regression and subprocess-only watchdog helper remain the
two intentional ignores in the focused suite; parent watchdog tests ran normally.
The source change is behavior-preserving, so no duplicate regression test was
added.

### Apply cost and rendering

The repeated 100,000-entry test measured semantic duplicate apply/release at
26.214 ms, reversed/changed identity mapping plus merge at 20.034 ms, and the
following live-heartbeat drain tick at 0.397 ms. All were below the existing 33 ms
interactive frame target in this run and the established 100 ms debug assertion.
These measurements include normal shared-machine timing variation and are not
new filesystem or platform qualification.

Regenerated all eight P2 states at 34×12, 80×24 and 120×40 into one owned temporary
directory with `FLIMRU_P2_EVIDENCE_DIR`. Compared every generated artifact with the
builder's retained evidence: **all 24 TXT and 24 SVG files matched byte for byte**.
The existing PNGs and builder visual inspection therefore remain applicable.
`simplification-render-comparison.log` records the passing render test and
comparison. No production UI rendering code changed.

## Self-review and handoff

Re-read the final stage delta after validation. The `Option::replace` expression
returns the same change flag and leaves the same error marker; removing the
recovery alias preserves short-circuit order; naming and test assertion changes
preserve their original comparisons. No further simplifier fix was necessary.
Independent final P2 correctness/performance and rendered UI review remain
required, including the connected manual/automatic apply and live-session flows.

The earlier foundation report's inherited drop-test timeout limitation has since
been addressed by the coordinator's reviewed subprocess watchdog implementation;
this stage inspected and preserved that settled foundation.

## Ownership and recap

No background process remains. The render comparison directory was created under
`/private/tmp/flimru-productivity-p4tfofri/p2-simplification-render.XXXXXX`, owned by
the command, and removed by its EXIT trap after the render test and comparison
finished. All test fixtures use their existing RAII ownership. The normal shared
`target` cache is about 4.4 GiB and remains available to the ongoing workflow; no
isolated build cache was created or removed. Other coordinator temporary files
and dirty work were preserved. No commit, installation, release build or Xcode
operation occurred.

Recap: P2 integration simplified with small source/test changes; 166 focused
tests, strict all-target Clippy, formatting, live apply timing and byte-identical
render evidence checks pass. Shared source is ready for separate phase reviewers.
