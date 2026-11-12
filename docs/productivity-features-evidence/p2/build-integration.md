# P2 automatic configuration reload integration

Status: **implemented and ready for separate simplification/review**. This builder
report does not approve the phase gate. Integration baseline:
`/private/tmp/flimru-productivity-p4tfofri/p1-complete`. The scoped delta against
that baseline is retained in `build-integration.patch`; unrelated dirty work and
all settled P1 behavior were preserved.

## Behavior and implementation

The runtime App owns an optional `ConfigReloader`; ordinary App test fixtures leave
it absent until a test deliberately requests reload. The one reviewed worker still
polls selected-file contents every 500 ms, debounces automatic changes with two
matching observations, coalesces results, rejects stale generations, caps input at
4 MiB, and wakes/joins on shutdown. Its production scheduling and subprocess
watchdog tests were preserved. The only helper-module addition is `load_bounded`,
which reuses the existing bounded reader and safe parser for launcher startup and
`--check`. No second parse or metadata-only size guard was added. The existing
`LaunchFile::parse`/`load` APIs remain compatible; the interactive launcher has no
remaining use of the old unbounded `load` path.

Picker `r`, `c`, toolbar and Actions reload use `request_config_reload`. Tick
consumes at most one coalesced completed result through `apply_config_reload`.
An App-side generation floor also rejects stale injected/queued results. A newer
automatic version can supersede an unread manual completion without leaving the
manual-pending guard stuck.

Explicit manual requests immediately disable Run until matching or newer work
completes, including failure. Run enforcement is at `start_batch`, with matching
Actions/toolbar/footer presentation. A rapid `r` + Enter burst starts no process;
completion never schedules a future Run. Automatic debounce continues to allow
the currently accepted catalog.

Whole-file read/JSONC/schema errors retain the accepted Arc catalog and show a
persistent `Config error` marker before the workspace name, including with an
active picker query or console. Diagnostics explain the safe error and last-good
policy. Identical automatic observations/errors do not publish/redraw repeatedly.
Automatic failures do not overwrite unrelated status. Manual completion reports
success/failure; automatic recovery clears an old reload-owned failure status
only if that exact message is still displayed.

A semantic-duplicate initial automatic result leaves the Arc, cached availability,
selection and UI unchanged. Duplicate-content recovery clears the error and
refreshes previews. Every manual success, including unchanged content, installs
the fresh Arc and refreshes availability so external cwd/helper-file repairs are
observed. A valid empty or rejected-only catalog is accepted.

Before replacement, App captures the logical source from deferred `desired_source`
or its visible selection. Unique names in both catalogs preserve identity across
content edits/reordering. Duplicate names require a single matching normalized
configuration/diagnostics/warnings fingerprint in **both** catalogs. Removing one
of two identical siblings is ambiguous and clears selection. Applying the new Arc
uses `set_config_source_selection(mapped_or_none)` and `merge`; cleared selection
remains non-actionable through asynchronous results until deliberate navigation or
selection. Pending selected sources are retained without highlighting a ready
sibling. No new global catalog ID was introduced.

Apply preserves page, active console, text selection, query/edit state, open
modals, marked devices, favorites, status unrelated to reload, and existing runs.
Merge recomputes new pinned/synthetic devices. No running process, launch-time
command, run label, or retained console is replaced by a file edit.

## Validation

Final exact commands:

- `cargo test --locked --bin flimru --lib --test availability --test terminal_flow`:
  **57 library + 82 App/worker/render + 3 availability + 24 real-binary PTY passed**;
  **2 intentionally ignored** entries total (the subprocess watchdog helper and
  optional real-Dart prerequisite). `build-focused-tests.log` retains output.
- `cargo clippy --locked --all-targets -- -D warnings`: **passed**,
  `build-clippy.log`; no lint suppression was added.
- `rustfmt --check --edition 2024 --config skip_children=true src/main.rs src/ui.rs src/config_reload.rs tests/terminal_flow.rs`:
  **passed**. Whole-project formatting was avoided during the independent P3
  writer's work. `git diff --check`: **passed**.
- `FLIMRU_P2_EVIDENCE_DIR=docs/productivity-features-evidence/p2 cargo test --locked --bin flimru configuration_reload_render_evidence -- --nocapture`:
  **passed**, `build-render.log`. Actual TestBackend cells produce 24 TXT/SVG
  artifacts; `/opt/homebrew/bin/rsvg-convert` produced their 24 PNG counterparts.
- `cargo test --locked --bin flimru reload_large_catalog_apply -- --nocapture`:
  **passed**, `build-performance.log`.

Deterministic App regressions cover unique-name edits/reorder; duplicate content
reorder; removed/ambiguous entries on either side, including identical sibling
removal; deferred pending selection; explicitly cleared selection; stale results;
initial semantic duplicate suppression; persistent last-good errors/recovery;
unchanged manual metadata refresh; all manual request routes; manual pending
launch prevention; empty/rejected catalogs; modal/filter/marked/favorite state;
and an unchanged owned live session/text snapshot while output keeps draining.
The reviewed foundation separately covers observation debounce, timestamps,
same-size edits, generation races, mailbox bounds, size/UTF-8, FIFO and teardown.

Real-binary PTY regressions cover explicit relative `--launch`, atomic rename,
invalid intermediate saves, deletion/recreation, last-good recovery, safe errors,
manual keyboard/toolbar/menu actions, the `r` + Enter burst, and an existing child
PID/launch-count staying unchanged through invalid and corrected automatic edits.
A separate real-binary `--check` regression verifies the initial 4 MiB limit,
secret-safe invalid-source diagnostic, valid empty catalog and full file warnings.

### Apply cost with a live PTY

The 100,000-entry fixture remains under 4 MiB. With a real owned PTY emitting
heartbeats, the test measures applying a separately parsed semantic duplicate,
then a reversed catalog with changed arguments on the selected named entry. It
verifies the source moves from 99,999 to 0, the live child remains, and the next
tick drains heartbeat output.

| Debug operation | Observed duration |
| --- | ---: |
| Semantic duplicate apply, including incoming catalog release | 12.542 ms |
| Reordered/changed Arc, identity mapping and merge | 15.074 ms |
| Following heartbeat-draining tick | 0.344 ms |

These satisfy the existing 33 ms interactive frame target on this machine;
regression assertions use the established broader 100 ms loaded-debug ceiling.
They do not qualify arbitrary remote filesystem or hardware timing. Reading and
parsing are outside the interactive apply path.

## Rendered self-review and fixes

Eight states at 34×12, 80×24 and 120×40: error, error with filter, error details,
recovered, empty, ready replacement without selection, manual pending and deferred
selected pending. Actual PNGs opened: error/filter 34×12, error details 34×12,
manual pending 34×12, recovered 34×12, no-selection 80×24 and selected-pending
120×40. Marker remains visible, safe diagnostics scroll, ready replacement has no
highlight/Run, and a pending selected source has no ready-sibling fallback.

Self-review corrected a fixture-only nonexistent display workspace that had made
replacement rows blocked; the fixture now creates a real `Example` directory,
asserts ready/pending counts and regenerates all evidence. The first new PTY run
also corrected its Ctrl+b/Ctrl+d navigation typo. An additional self-review made
automatic recovery clear a still-displayed reload-owned failure message while
preserving unrelated statuses, with regression coverage. Final checks above pass.
Independent correctness and rendered UI review remain required.

## Ownership, limits and recap

Owned source: `src/main.rs`, `src/ui.rs`, the seven-line public bounded loader in
`src/config_reload.rs`, and related `tests/terminal_flow.rs` changes. Other helper,
selection, animation, session, run and P1 parser/availability modules were not
changed. No commit, release build, installation, selected-Xcode action or real
Flutter/device operation occurred.

All new filesystem/process fixtures are RAII-owned. Their children are reaped
before fixture directories are removed. No new disposable build directory or
background process is retained; Cargo reused the normal shared `target` cache,
now about 4.3 GiB, intentionally retained for the remaining workflow. P2 evidence
is about 5.4 MiB. The coordinator's task temporary tree was not altered.
Synchronous kernel filesystem calls can still delay orderly worker shutdown;
chunk cancellation, regular-file checking and Unix nonblocking FIFO opens retain
the reviewed foundation's scope. Windows/physical-device behavior is not qualified.

Recap: automatic and explicit reload now share a bounded background parse and safe
last-good apply policy, with unambiguous selection restoration and an explicit
manual Run guard. Focused tests, strict lint, stress measurements and rendered
evidence pass. The shared source is held for separate phase-stage agents.
