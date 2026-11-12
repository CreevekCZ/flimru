# P5 full integration — builder handoff

2026-11-06. Built against coordinator baseline
`/private/tmp/flimru-productivity-p4tfofri/p4-complete` after the P4 gate passed.
This handoff is ready for independent simplification, correctness, and UI review;
it does **not** close those gates or authorize installation.

## Result

Actions now offers **Reload all eligible (N)** and **Restart all eligible (N)**
whenever retained runs exist, including N=0. Both counts remain complete at
34×12; the selected row's footer explains the hot-command meaning and input
request scope. Neither action has a keyboard broadcast binding. Ordinary `r`/`R`
retain their existing active-console behavior.

`FlutterTool::plan` records `HotCapabilities` from the launch configuration and
the actual known Designed-for-iPad adapter branch. Normal SDK/FVM execution,
explicit user-supplied Flutter-compatible CLI executables, and the secrets-guard
exec wrapper retain normal command support. The custom runner is excluded by its
real adapter selection, independently of executable/configuration labels. `Run`
stores the resulting launch-time value; editing/reloading the catalog does not
change existing runs' capabilities.

Invocation snapshots retained run IDs, bounded device labels, launch-time
capabilities, and live/stopping state. Dispatch resolves each stable ID again,
rejects disappeared/newly-stopping sessions, and submits one `r` or `R` byte via
the established `Session::send` path. A failed input submission does not stop
later targets. No process is spawned, replanned, revived, or selected by mutable
vector index. Production `Session` behavior is unchanged.

The resulting immutable, binary-private delivery modal shows requested/queued,
skipped, failed, fixed per-device reasons, and unconfirmed Flutter completion.
It intercepts ordinary keys, selection, paste, and mouse input, owns the native
cursor, and closes back to the original tab/review/query/filter/manual offset.
Batch delivery does not add a review-operation notice or change its geometry.
Global Ctrl+q and actually uncovered Quit cells remain available; covered or
unproven pointer cells (before first draw/after resize) cannot bypass the report.

## Scope

- `src/tool.rs`, `src/runs.rs`: planned/stored capabilities and bounded target
  snapshots; existing command/run fixtures receive explicit metadata.
- `src/main.rs`, `src/ui.rs`: Actions, stable-ID dispatch, immutable result owner,
  input/cursor/quit routing, resize invalidation, and private module registration.
- `src/batch_report.rs`: binary import path, modal coverage predicate, test-only
  inspection accessors; established report content/wrapping semantics preserved.
- `tests/support/batch_commands_app.rs`: six behavioral App regressions plus one
  actual App rendering fixture. Existing P3/P4 fixture callers are mechanically
  updated for the launch/run metadata contract.
- `tests/animation_control.rs`: direct `Session::send(b"R")` speed restoration.
- `tests/terminal_flow.rs`: one real-binary two-device batch delivery flow.

No new dependencies or library view API were added. The deferred P6 support file
was not edited, registered, or run. No commits, release build, or installation.

## Validation

Affected suites passed:

| Target | Passed | Ignored |
| --- | ---: | ---: |
| Library unit tests | 58 | 1 subprocess watchdog child |
| Binary/App/UI unit tests | 114 | 0 |
| Batch helper integration | 15 | 0 |
| Standalone report UI | 11 | 0 |
| Session resources | 1 | 0 |
| Animation controls | 10 | 0 |
| Real-binary terminal flows | 30 | 1 optional real Dart VM |
| **Total** | **239** | **2** |

Commands:

```sh
cargo check --all-targets
cargo test --bin flimru --lib --test batch_commands --test batch_report_ui \
  --test session_resources -- --test-threads=1
cargo test --test animation_control -- --test-threads=1
cargo test --test terminal_flow -- --test-threads=1
cargo clippy --all-targets -- -D warnings
```

The animation suite required approved command-specific sandbox escalation to bind
its task-owned local fake VM-service sockets; the normal sandbox denied loopback
binding. It passed after escalation. No external service was contacted.

The final implementation self-review tightened bounded label allocation during
target capture, shortened the reload footer to fit the narrow menu, and blocked
underlying pointer Quit before render/after resize. Afterwards, all seven focused
App checks and eleven report UI tests were rerun successfully, along with strict
all-target Clippy, owned-file `rustfmt --check` and `git diff --check`.

The six behavioral regressions cover stored capabilities across an actual catalog
edit/apply; mixed eligible/ended/stopping/profile/test/custom runs and a real full
nonblocking input queue; stable-ID reordering/removal/replacement; active-only
`r`/`R` versus explicit batch actions and no reentrant retry; frozen Arc/query/
severity/manual-offset/tab preservation; and live/query native cursor, selection,
paste, pointer, pending-resize coverage, and global quit ownership. The new PTY
flow verifies exact one-byte counts per child, local `r`/`R`, report input
isolation, and both explicit hot commands. Animation restoration verifies the
selected factor survives `Session::send(b"R")` across isolate replacement and
extension-registration delay.

## Actual rendering evidence

```sh
FLIMRU_P5_APP_EVIDENCE_DIR=docs/productivity-features-evidence/p5/integration-renders \
  cargo test --bin flimru batch_hot_actual_app_render_evidence
```

Nine actual App/TestBackend scenes are retained as TXT/SVG/PNG: mixed eligibility
Actions, zero eligibility Actions, and delivery report over frozen review, each
at 34×12, 80×24, and 120×40. SVG cells were converted with
`rsvg-convert INPUT.svg -o OUTPUT.png`. Visual inspection included the narrow
mixed menu (both complete counts and hot-command footer), the wide zero menu,
and the medium delivery-over-review scene. Parent query/live cursors are covered
by behavioral TestBackend visibility assertions. Earlier standalone report
evidence remains available for long labels, failures, and scrolling.

## Review artifacts and resources

`build-integration.patch` contains only this integration's 13 changed code/test
files relative to the supplied P4 baseline, including the new App test source.
It excludes unrelated dirty work and generated render binaries.

Retained integration rendering evidence is **2,608,918 bytes** across **27 files**
for independent review/reproduction. Validation used the normal reusable project
`target` cache; no disposable DerivedData/cache/build directory was created.
All fixtures use RAII temporary directories, owned children and sockets that exit
before test completion. No task-owned background process or disposable directory
remains. The coordinator-owned P4 baseline was preserved.

Remaining: independent simplification/correctness/UI review and any resulting
fixes, then coordinator P6 cross-feature validation and fresh final review before
installation.
