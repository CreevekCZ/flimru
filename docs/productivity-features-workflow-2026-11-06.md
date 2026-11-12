# Flimru productivity features: approval brief and execution workflow

Status: **complete, independently reviewed, fixed, and installed**.
Final outcome: [execution recap](productivity-features-evidence/FINAL.md).
Final integration review must use a fresh `gpt-6-astra` subagent, as requested by the user.
Prepared: 2026-11-06. Repository baseline inspected: `main` at `c644bf0`, with
existing uncommitted work, including the configuration-reload shortcut and
animation-control recovery. This document authorizes no implementation by itself.

## Scope to approve

Implement these five features in the existing Rust/Ratatui launcher:

1. Console search, including next/previous matches and recognized-severity filters.
2. Automatic reload of the selected launch configuration file.
3. Explanations for configurations that cannot run on the selected target(s).
4. Explicit hot reload / hot restart actions for all eligible running sessions.
5. Export of retained console logs as plain UTF-8 text.

After implementation and all review gates, build and install the update at the
existing system command location, currently `/Users/jankoznarek/.local/bin/flimru`,
as requested earlier in this chat. Verify the actual destination again then.
Preserve running instances; the new executable takes effect on the next launch.
Publishing releases, tags, tap updates, unrelated refactors, and changing Xcode
selection are outside this workflow.

## Product decisions

### Search and log review

- Offer **Search console** through Actions and a dedicated unused Control shortcut;
  validate terminal delivery before settling its label. Keep `Ctrl+f` for switching
  consoles and keep ordinary console typing, including `/`, available to Flutter
  outside review mode.
- Search the selected live, ended, or saved console. Entering review captures one
  bounded, frozen snapshot of its retained terminal history. Label it as a snapshot;
  live PTYs, including hidden sessions, continue draining normally. Provide explicit
  **Refresh snapshot** and **Resume live** actions. Search does not continuously
  re-index incoming output in this version.
- Use literal, case-insensitive text matching, highlighted results, a current/total
  match indicator, and next/previous navigation. No regex language in this scope.
  Query input is limited to 512 characters, consistent with existing picker filters.
- Add **All**, **Errors**, **Warnings**, and **Errors + warnings** filters. Reuse the
  existing prefix/severity classification independently of ANSI color. Search and
  filter operate on the same logical lines. Unclassified stacktrace continuation
  lines remain unclassified; this version does not infer complete error records.
- Join soft-wrapped rows for matching while retaining row/cell mappings for display;
  preserve explicit blank lines, wide characters, and combining characters. Unicode
  case conversion must not turn byte offsets into incorrect cell highlights.
- Review/query navigation and paste never reach Flutter. Escape first dismisses
  query editing, then leaves review; it must not stop the app as a side effect.
  Existing modal/focus/selection behavior needs an explicit precedence table and tests.
- A live session finishing during review must not dismiss its snapshot. Use stable
  session identity, including transfer to the single saved-console path. Explicit
  switch/close clears or transfers review deliberately; eviction or vector-index
  reuse cannot associate it with another console. Resize preserves snapshot content.

### Automatic configuration reload

- Watch the resolved path already selected by the launcher, including `--launch`.
  Prefer one owned polling worker with no new file-watcher dependency: check every
  500 ms and apply a changed version after two matching observations, typically
  about 0.5–1 second after the final write. Detect atomic replace, deletion/recreation,
  same-size edits, and content changes despite unchanged modification timestamps.
- Perform bounded file reading/parsing off the UI thread. Start with a documented
  4 MiB input limit and a clear diagnostic for larger files. Use generation IDs and
  coalesced requests/results; stale background results cannot overwrite newer manual
  reloads. Avoid repeated notifications or redraws for identical content/errors.
- Manual `r` on the configuration page, `c`, toolbar/menu reload, and automatic
  reload use the same parse/apply policy. Manual reload requests an immediate read
  without the automatic debounce.
- A whole-file read/JSONC/schema error retains the last good catalog and shows a
  persistent configuration error indicator. Recover automatically when corrected.
  A syntactically valid catalog can contain unavailable entries: accept the new
  catalog, show those reasons, and retain its valid siblings. A valid empty array
  is also an accepted update, not a reason to silently keep old entries.
- Preserve the page, active console, marked devices, picker filter, and unrelated
  status. Capture selection before replacement. Preserve a unique named entry;
  disambiguate duplicate names using matching entry content where possible.
  If the selected entry is removed or ambiguous, clear the actionable selection
  and require a fresh explicit selection before Run. Never quietly substitute a
  different configuration under a queued Enter/click.
- Recompute config-pinned/synthetic devices without losing favorites or marked
  targets. New launches use the accepted catalog; existing processes keep their
  launch-time settings. File edits do not restart or reconfigure running sessions.

### Explain unavailable configurations

- Retain structured source entries and diagnostics for rejected configurations,
  rather than relying on free-form warning strings. Preserve source order and
  display identity even when a configuration cannot become a runnable LaunchConfig.
- Use one availability assessment for presentation and launch enforcement. Cover
  pinned-device mismatch, multi-target compatibility, test/device-less restrictions,
  current project-adapter restrictions, unsupported fields/request/type, unresolved
  variables, unsupported preLaunchTask, and deterministic argument/cwd validation.
  Report target-specific reasons for multi-device selection.
- Default to runnable entries, with counts and a discoverable **Show unavailable**
  toggle. Unavailable rows are selectable for explanation; their Run action is
  disabled, and Enter/mouse cannot spawn them. Offer a compact explanation overlay
  so reasons are reachable at 34×12, independently of the wide details pane.
- Distinguish an empty file, no text-filter matches, and target/capability exclusion.
  A structurally valid file with zero runnable configurations opens the diagnostic
  picker, including when all source entries are rejected. An initially malformed
  whole file can continue to report the existing startup error.
- Make **Configuration diagnostics** reachable from the device picker even with
  no discovered/selected device. Explain source/argument errors immediately and
  state when a target selection is needed to assess device compatibility.
- Show field/variable names and corrective guidance without env values, dart defines,
  raw rejected JSON, or expanded secret-bearing values. Keep compound/file warnings
  visible. This is not a general device-readiness/SDK doctor, and it does not add
  execution support for previously unsupported VS Code tasks or debugger features.

### Hot reload / restart all sessions

- Add **Hot reload all eligible sessions (N)** and **Hot restart all eligible
  sessions (N)** to Actions, available while runs exist. Keep ordinary `r`/`R`
  restricted to the displayed session. No implicit broadcast mode or new broadcast
  typing shortcut.
- Record command capabilities from each run's launch-time kind/mode/options;
  never infer support from the configuration label or the currently edited file.
  Conservatively target supported debug app runs. Skip ended/stopping sessions,
  ordinary tests, profile/release runs, disabled hot mode, and custom runners whose
  command support is unknown. Explain exclusions and show eligible counts.
- Snapshot eligible session identities when invoked and send one command per target
  using the established nonblocking input and animation-restart behavior. Continue
  after per-target failures. Preserve the active tab and any review snapshot.
- Report **requested/queued**, skipped, and failed counts, with per-device details.
  Sending a byte does not prove Flutter completed a reload. Never revive an ended
  run, spawn new children, or rebuild a command from a reloaded configuration.

### Export retained console logs

- Add **Export retained console log** to Actions for live, ended, and saved consoles.
  In review, export that exact frozen snapshot; otherwise capture at invocation.
  Export all retained content regardless of the active query/severity filter, and
  state that scope in the prompt.
- Use plain UTF-8 text reconstructed from terminal cells, with no ANSI/control
  sequences, joined soft wraps, preserved explicit line breaks, and no duplicated
  wide continuation cells. This is retained terminal history, not the original raw
  byte stream or already-evicted output.
- Show an editable destination before writing, defaulting to a safe generated
  filename under the workspace's `flimru-logs/` directory. Create directories only
  on explicit Export. Sanitize filename components, keep a unique name, and use
  create-new semantics: an existing destination is an error, never an overwrite.
- Include only a small header with capture time, device/config labels, and outcome
  where available; omit environment and launch arguments. Do not claim the log text
  is secret-redacted. No automatic upload or clipboard modification.
- Perform writing away from the UI loop. Report the absolute saved path and byte
  count, or a useful error. Rejected/failed writes must not leave a completed-looking
  partial file or mutate console offsets, query state, or running sessions.

## Shared implementation constraints

- Introduce only a narrow shared snapshot/text abstraction for search and export;
  reuse existing soft-wrap/Unicode behavior without broadly rewriting selection.
  Keep picker filtering separate from console review state.
- Bound memory and work: one active review snapshot, at most one pending export,
  and coalesced/versioned search jobs. Initial text budget: 16 MiB per snapshot or
  export; reject oversize operations visibly rather than silently truncating.
  Bound match storage at 10,000 positions and label capped results explicitly.
  Bound auxiliary cell/coordinate storage too; text bytes alone do not bound it.
  Implementation review established a separate 128 MiB pre-clone screen estimate
  (using maximum historical width) and a 4 MiB filtered-row index budget.
  Measure worst-case capture/search behavior before claiming responsiveness.
- Snapshot extraction/searching must not rescan full histories on every frame.
  Background jobs own their data, cancellation, result generation, and shutdown.
  Closing a target or exiting joins/cancels owned work; stale results are ignored.
- Export remains tied to the captured source even if the active tab changes. A
  background file operation cannot later use a mutable vector index as its target.
- Preserve the existing eight-session limit, bounded PTY queues/history, graceful
  stop/force-stop behavior, animation-control recovery, selection/copy, and idle
  rendering behavior. Secondary commands primarily belong in the Actions menu.

## Phases and dependencies

| Phase | Coherent deliverable | Depends on | Completion evidence |
| --- | --- | --- | --- |
| P1 | Structured configuration catalog, availability reasons, compact explanations and disabled launch enforcement | Baseline | Parser/availability tests; rejected-only/no-device diagnostics; blocked keyboard/mouse launches; rendered empty/unavailable states |
| P2 | Automatic configuration reload through the shared manual/automatic apply path | P1 | Deterministic debounce/version tests; atomic-save/delete/recreate PTY flow; last-good recovery; selection preservation and running-session isolation |
| P3 | Shared bounded console snapshot plus search, match navigation and severity review | Baseline; integrate after P2 | Unicode/wrap/mapping tests; input/paste isolation; live→ended review; flood/resize/lifecycle checks; rendered search states |
| P4 | Explicit export of retained console snapshots | P3 | Exact text/export scope; collision/error/size/partial-write tests; live output during export; rendered save prompt and outcomes |
| P5 | Explicit hot reload/restart all eligible sessions | Baseline; integrate after P4 | Capability/target tests; one request per target; failure isolation; mixed live/ended/stopping runs; animation restart regression |
| P6 | Whole-workflow integration, final independent review, documentation and system installation | P1–P5 | Cross-feature PTY scenarios; full project checks; reviewed release build; atomic installation and checksum/startup verification |

P1 and P2 share a catalog contract; P3 and P4 share the snapshot contract. Batch
commands do not depend on reading the current launch file. Integrate shared
`src/main.rs` / `src/ui.rs` edits sequentially. Parallel work is allowed only for
independent reads/reviews or explicitly disjoint helper modules with settled
contracts; checkpoint mode must not start a later phase before permission.

## Workflow inside every feature phase

The orchestrator owns acceptance decisions, shared contracts and integration.
Each phase uses distinct stage agents with fresh scoped context:

1. **Build/test:** a builder implements the deliverable and meaningful regression
   tests; report actual commands/results and any missing validation.
2. **Simplify:** a separate simplifier reviews the actual diff, reduces needless
   branching/duplication and naming complexity, and preserves behavior and budgets.
   Rerun affected checks. Do not expand into unrelated dirty work.
3. **Correctness/performance review:** an independent reviewer examines the final
   phase diff and evidence, including modal routing, stale identities/results,
   bounded work, worker teardown, filesystem failures and process ownership.
4. **Fix/re-review loop:** a separate fixer addresses findings and runs affected
   checks. The original reviewer verifies the fixes. Repeat until no known serious
   findings remain; all other findings must be fixed or explicitly disclosed for
   disposition. A reviewer never approves their own edits.
5. **Rendered TUI review:** a separate UI reviewer inspects actual rendered output
   and interactions at 34×12, 80×24 and 120×40, including resize, long/Unicode labels,
   focus, mouse hit regions and errors. Use TestBackend renders plus the real-binary
   PTY harness; retain reviewable render/screenshot evidence. Do not substitute
   code-only review for rendered validation. Separate fixes return to the relevant
   correctness/UI reviewers.
6. **Phase gate:** give a brief recap of behavior, checks, fixes and limitations.
   Advance only after the gates pass and the approved checkpoint policy permits it.

Default models: `gpt-6.1-sol`, high reasoning, for builders, simplifiers touching
state/contracts, reviewers and fixers. Use `gpt-6.1-terra`, medium, only for mechanical
check execution/log summaries or simple documentation boilerplate. Set models
explicitly and report any substitution; keep persistent settings unchanged.
New phases receive new stage agents. Limit concurrency to available slots and
give every parallel writer explicit non-overlapping ownership.

If a fix loop repeats without progress or exposes a material scope change, provide
the concrete blocker/revised brief for approval. Do not waive a failing gate or
call an unrun check a pass.

## Validation and final integration gate

Before implementation, record the dirty baseline and establish current check
results. Keep unrelated work intact; do not reset, stash, discard, or silently
commit it. Use a `codex/` execution branch when appropriate without excluding the
already-installed local changes. Planning adds only this document.

Each phase extends the relevant existing tests rather than duplicating equivalent
coverage. Deterministic watcher/search-controller tests use injected clocks/jobs;
filesystem/process/PTY scenarios use task-owned fixtures. Specifically cover:

- unavailable-only/empty catalogs and no-device diagnostic access, duplicate names,
  secret-safe reasons, filtered
  unavailable entries, multi-device restrictions and atomic batch prevalidation;
- invalid intermediate saves, same-content rewrites, stale manual/automatic races,
  removed/ambiguous selected configs, and unchanged running processes;
- active/hidden output floods during search, query paste, no matches, match wrapping,
  capped results, Unicode case/cell mapping, soft wraps, recognized severity versus
  unclassified continuation lines, resize and completion/close/replacement;
- exact full-snapshot export while filtered, safe filenames, collision, unwritable
  destination, cap rejection, cancellation and partial-file cleanup;
- eligible/unsupported/ended/stopping batch targets, one failing target among healthy
  ones, active-tab preservation, and existing `r/R`, animation and stop semantics.

After P1–P5, a **new final integration reviewer**, separate from all phase workers,
checks the combined diff and connected flows. Exercise: launch multiple targets,
auto-reload a changed/temporarily invalid catalog, inspect exclusions, search/filter
one console while others emit output, broadcast supported hot commands, complete
that console during review, export its exact snapshot, then close/relaunch/quit.
Route findings through a separate fixer and affected phase reviewers before final
integration sign-off.

Run `make check` (format, strict all-target Clippy, Rust tests, dependency audit and
packaging tests), the optional real Dart VM animation regression when available,
and `cargo build --locked --release`. Use the selected Xcode only if an Apple tool
is needed; no competing GUI instance. Automated/mock/Dart evidence does not qualify
physical iOS acceptance or Windows support.

Reuse the normal project build cache. Isolate disposable audit/tools/render results
when needed, track exact task-owned paths, join child processes, preserve useful
named evidence, and remove disposable output before finishing. Do not sweep shared
temporary directories. Report retained large artifacts and their purpose.

Only after final review/checks: atomically replace the verified existing installed
command, compare its checksum with the reviewed release build and smoke-test it.
Do not terminate a user-owned running launcher to make it use the update.

## Approval and checkpoint policy

Approval must include one execution mode:

- **All phases:** execute P1–P6 autonomously, including every simplify/review/fix/UI
  gate and final installation. Pause only for a material scope change or blocker.
- **Pause after each phase:** execute one phase, report its concrete review/check
  evidence, and wait for permission before beginning the next phase.

Approved in this chat: execute all phases autonomously, then obtain the final
independent Astra review, fix/recheck findings, and install the reviewed update.

## Recap

Five features, five reviewed implementation phases, then independent integration
review and system installation. Structured config diagnostics precede automatic
reload; search and export share a bounded snapshot contract. Existing sessions,
input isolation, dirty work and process/resource ownership remain explicit gates.
