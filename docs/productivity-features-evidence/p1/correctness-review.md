# P1 independent correctness/performance review

Result: **changes required**. Review is read-only apart from this report. Reviewed
the actual P1 source against `/private/tmp/flimru-productivity-p4tfofri/baseline`,
the approved workflow, builder evidence, and simplifier evidence. Unrelated
pre-existing animation/selection work and independently staged P3–P5 helpers were
excluded except where relevant to P1 integration.

## Findings

### [P1] Preserve source identity across device merges instead of selecting the first duplicate name

Location: `src/main.rs:755–758` (selection capture at `709–712`; discovery calls
merge at `909`).

The new source catalog permits a rejected entry and a runnable entry to have the
same display name, but merge restores selection using the first matching name.
With `[{"name":"Duplicate"},{"name":"Duplicate","type":"unsupported"}]`,
show unavailable, select source entry 1, and complete any successful device
refresh. Selection becomes source entry 0. Enter can now start the runnable entry
although the user selected the rejected entry for inspection. This can also
change the contents of an already open explanation. The existing duplicate-name
problem among parsed configurations does not cover this new transition from a
selected rejected row to an actionable row.

An independent test compiled against the current `App` and its actual modules
selected source 1 and called merge without replacing the catalog:

```text
selected source before merge=1, after merge=0
assertion failed: Device refresh must preserve selected rejected source entry
left: 0
right: 1
```

Preserve `source_index` for a device-only merge of the same accepted catalog.
Catalog replacement should use the separately specified unique-name/content
policy and clear ambiguous actionable selection. Add a regression containing
both runnable and rejected duplicate-name entries; cover discovery completion
while the rejected row and its explanation are selected, then Enter.

### [P2] Do not launch on Enter when the displayed unavailable row promises an explanation

Location: `src/main.rs:582–584`, `629–638`; the displayed behavior is selected by
cached assessment in `src/ui.rs:1372–1379` and `481–493`.

The cache intentionally keeps a blocked preview until merge or target change.
If an external fix makes that row runnable, Enter performs fresh preflight and
immediately starts it, despite the row showing blocked, Run being absent, and
the footer saying “Enter explains.” For example, select a row whose cwd does not
exist with unavailable shown, then create cwd without changing the catalog or
target and press Enter. Fresh preflight is necessary for launch enforcement, but
must not turn a displayed explanation action into child startup.

Independent reproduction used `/bin/echo` as the owned fixture executable and
asserted that the cached assessment was blocked and Run was absent before the
directory was created:

```text
after Enter on displayed blocked row: diagnostics_open=false runs=1 page=Console
assertion failed: Enter on a blocked row promises an explanation, not child startup
```

Route Enter on a presented unavailable row to explanation. It may refresh the
assessment and readiness, but require another explicit Run after presenting the
new ready state. Keep the existing fresh preflight when Run is actually invoked.
Add a blocked→ready cwd/helper recovery regression without intervening merge.

### [P2] Bound or move the first assessment fill off the UI thread

Location: `src/main.rs:799–807`, `824–830`, and `705–710`; filesystem work is in
`src/availability.rs:41` and `113–137` through `FlutterTool::plan`.

The cache avoids repeated planner IO on ordinary cached reads, but its first
fill still synchronously assesses every catalog entry on the UI thread.
`assess` checks cwd and the planner checks it again, in addition to applicable
helper checks and constructing then discarding full command/environment plans.
Every discovery merge unconditionally clears the cache, including unchanged
device snapshots. The future 4 MiB reload cap does not meaningfully bound entry
count or these filesystem operations. During this work the main loop cannot
drain live PTYs or handle input.

An independent current-App timing probe, using existing debug dependencies, one
target, an existing local cwd, no helper adapter and a catalog of only name
fields, measured:

| Entries | JSON bytes | merge / first cache fill | cached count | cached 80×24 draw |
| ---: | ---: | ---: | ---: | ---: |
| 1,000 | 28,910 | 7.75 ms | 0.32 ms | 4.99 ms |
| 10,000 | 298,910 | 56.36 ms | 2.44 ms | 19.93 ms |
| 100,000 | 3,088,910 | 578.79 ms | 26.06 ms | 201.46 ms |

The same probe was then compiled from actual current main/library sources using
`rustc -O -Clto=thin` and existing optimized release dependencies:

| Entries | JSON bytes | optimized merge / first fill | optimized cached count | optimized cached 80×24 draw |
| ---: | ---: | ---: | ---: | ---: |
| 1,000 | 28,910 | 5.03 ms | 0.023 ms | 0.70 ms |
| 10,000 | 298,910 | 46.55 ms | 0.255 ms | 5.17 ms |
| 100,000 | 3,088,910 | 491.79 ms | 5.68 ms | 63.49 ms |

Optimization therefore leaves approximately half a second of synchronous first
assessment work for this accepted-size fixture. These are one-run local timing
probes, not full release qualification or a claim of exact production latency.
They demonstrate hundreds of thousands of synchronous filesystem/planner
operations within the proposed accepted file size; slower filesystems can cost
more. The cached-render measurement gives total UI context, not a separate
finding about unrelated baseline list rendering.

Cache visible source indexes/counts for each catalog/target/query/show version
instead of rescanning the whole catalog several times per draw. Use
generation-bound background or bounded incremental work for first presentation,
and avoid clearing unchanged assessments on every device scan. If an entry/work
budget is chosen, reject it with an explicit diagnostic rather than silently
hiding or dropping normal entries. Preserve fresh atomic launch preflight. Add a stress case within the
accepted input budget and measure first apply, target change and unchanged scan
while live PTYs emit output; define the supported responsiveness bound.

## Verification and remaining observations

- Inspected the actual builder/simplifier test and strict-Clippy logs, including
  the recorded synchronization fix and failure evidence, rather than accepting
  the prose summaries alone.
- Independently ran `cargo test --locked --bin flimru --test availability`:
  **64 passed**.
- Independently ran `cargo test --locked --test terminal_flow`:
  **19 passed, 1 ignored** (real Dart prerequisite).
- `git diff --check`: passed.
- Independent temporary compile harnesses used current main/UI modules and the
  normal reusable dependency cache. Both behavioral findings reproduced with
  failing assertions. No repository source/test files were changed to run them.
  The optimized performance probe compiled actual current library/main sources
  into its own temporary directory and linked the normal release dependency
  cache. Its first direct-link attempt omitted Rust LTO and hit Apple ld's older
  LLVM bitcode reader; adding the project's thin-LTO setting succeeded. This
  required no Xcode selection change or dependency rebuild.
- Shared availability assessment performs no process execution. Actual startup
  consumes the plans from the same fresh all-target assessment, so a deterministic
  planning failure prevents the batch before any child starts. Rejected entries,
  no-target checks, modal keyboard/paste/mouse routing, tester constraints, and
  safe diagnostics are covered by source review and the passing focused suites.
  No additional actionable defect was found in those reviewed paths.
- Temporary harness directories and their executables were owned by Python
  `TemporaryDirectory` and removed on both success and failure. The fixture echo
  session was owned by `App`/`Session` and dropped/reaped after the failing test.
  No real Flutter/device process, project release build, installation, or Xcode
  GUI was used. The existing selected Xcode remained
  `/Applications/Xcode.app/Contents/Developer`; ordinary rustc linking used the
  current toolchain. The normal reusable target cache remains; only this small
  report is a newly retained review artifact.

Recap: three actionable findings, two independently reproduced behavior gaps
and one measured UI-thread workload issue. Existing focused tests pass but do
not cover these cases. A separate fixer and re-review are required before the P1
gate passes.

## First fix re-review

Result: **changes still required**. Reviewed `fixes.md`, actual worker/main/UI
source, regression tests, and recorded final test/performance/Clippy evidence.
The original three repros are addressed: unchanged discovery now restores exact
source identity; a presented blocked or checking row opens explanation before
any child; first assessment work is on an owned worker, counts/indexes are cached,
and list construction is limited to the viewport. The new 100,000-entry stress
test keeps live PTY draining within its recorded UI-operation budget. This does
not yet close the gate because two newly introduced integration cases reproduce.

### [P1] Preserve desired selection while a metadata-changing discovery refills availability

Location: `src/main.rs:810–815`, with context reset at `830–870` and incremental
result application at `928–929`.

Exact source preservation works for an unchanged target context. However, a
discovery update that changes the selected target's name or platform resets the
view. With unavailable hidden, `visible_entries()` is temporarily empty until
the worker results arrive. Merge immediately falls back to index 0 and discards
the desired source identity. Results later populate the list without restoring
the selected source. The next Enter can start the first runnable configuration
instead of the previously selected configuration. This occurs on normal
synthetic/favorite → discovered metadata updates, not only catalog replacement.

Independent current-App reproduction:

1. Install two runnable entries named `First` and `Selected second`, select one
   target, keep unavailable hidden, and settle the worker.
2. Select source entry 1.
3. Keep the discovered target ID unchanged, change its name, and call merge as
   discovery completion does.
4. Settle the worker and inspect selection.

```text
after label-changing discovery selected source=0
assertion failed: metadata discovery must preserve selected source
left: 0
right: 1
```

Persist the desired source identity across the pending context, then reconcile
it as results arrive. Keep the actionable selection empty if the selected source
cannot be retained; do not silently substitute the first ready row. Add name and
platform discovery changes with a non-first selected row and subsequent Enter.

### [P2] Fence queued previews against newer synchronous per-entry assessments

Location: `src/main.rs:928–929`, `874–880`; synchronous fresh result insertion is
at `643–648`. Worker `generation` only fences catalog/target contexts.

A same-generation worker result may have been computed before an external
filesystem change. After Enter performs a newer fresh assessment, the queued
preview can overwrite that result. A failed launch attempt can therefore open
Diagnostics with the correct cwd reason and then change to “Ready to run” as
soon as the next tick drains the old ready result. The reverse ordering can also
replace an explicitly refreshed ready state with an older blocked preview.
Fresh Run still preflights before spawning, so this is misleading presentation
and lost current explanation rather than an execution-validation bypass.

Independent current-App reproduction, with no fixture child started:

1. Create cwd for a single entry; show unavailable and request its worker context.
2. Allow its ready preview to queue without draining it (100 ms in this local
   fixture); the App assessment remains pending.
3. Remove cwd and invoke Enter/start_batch. Confirm the modal opens with the
   freshly cached `cwd` reason.
4. Call `advance_availability()` to apply the queued result.

```text
after queued preview applies: runnable=true reason_count=0
assertion failed: queued preview must not replace the fresher launch failure
```

Fence synchronous overrides per entry for the current context, or otherwise
invalidate/discard the older result before it can overwrite fresh preflight
metadata. A new catalog/target generation may reset that override deliberately.
Add both queued ready → fresh blocked and queued blocked → fresh ready cases.

### Re-review verification and lifecycle observations

- Independently reran `cargo test --locked --bin flimru --test availability
  --test terminal_flow`: **91 passed, 1 ignored**. Existing suite remains green;
  the two temporary independent regression assertions above fail.
- The worker owns immutable catalog/tool/target snapshots, one coalesced pending
  job, a two-chunk result channel, and generation checks before assessment/send.
  App applies only its current context generation, at most two chunks per tick.
  Stale catalog/source indexes cannot be accepted merely because an index is
  reused by the next catalog.
- Drop sets stop while holding the wait-predicate mutex, wakes the condition
  variable, and joins the owned thread. A full result channel checks stop/current
  at bounded retry intervals. An in-flight OS filesystem metadata call is not
  cancellable and can delay orderly join on a slow filesystem, as explicitly
  disclosed by the fixer. No additional worker ownership defect was found.
- Both repro harnesses compiled actual current main/worker/UI modules in an owned
  `TemporaryDirectory`, reused normal dependencies, started no Flutter/device
  child, and were removed after their failing assertions. No source/test edits
  were made. This appendix is the only review write.

Recap: the original fixes are effective, but pending-view selection and
same-generation result freshness each need a separate fix and re-review. The P1
correctness gate remains open.

## Second fix re-review

Result: **the two async behavior findings are closed; one remaining P1-phase
performance path requires a fix**. The deferred source anchor and explicit
non-actionable sentinel preserve the chosen source during a metadata-changing
fill. Deliberate picker transitions clear the anchor. Per-source synchronous
override fencing prevents same-generation previews from replacing fresh
preflight metadata, and context replacement resets those fences deliberately.
No further serious worker cancellation/generation/ownership issue was found.

Both independent async repros from the preceding appendix were rerun against the
actual current modules and now pass:

```text
metadata selection repro now retains source1
queued preview repro now retains fresh cwd failure
```

Reviewed the new deterministic partial-fill, both-direction override, explicit
picker-transition and empty-selection helper tests. Independently reran
`cargo test --locked --bin flimru --test availability --test terminal_flow`:
**95 passed, 1 ignored**. Inspected the actual strict-Clippy and performance logs;
`git diff --check` passes. These tests qualify the corrected paths but the
all-ready performance fixture does not exercise the rejected-catalog warning
path below.

### [P2] Bound diagnostics work for rejected catalogs instead of rewrapping every entry's warning per event

Location: `src/main.rs:1090`; `src/ui.rs:1866–1888`, `1902–1911`;
`src/main.rs:1208–1221`.

Each rejected source entry already has its selected-row structured diagnostic,
but also adds a legacy free-form warning to `launch.warnings`. The diagnostics
view appends and clones **all entries'** warnings, then wraps the entire list on
every draw and scroll clamp. A Down event invokes the layout clamp before and
after changing the offset. This synchronous UI work remains proportional to all
rejected entries even though the picker itself is now virtualized. It stalls
input and live PTY draining in the exact rejected-only diagnostic flow P1 adds.

Independent fixture: a syntactically valid configurations array containing
10,000 or 100,000 `{"type":"unsupported"}` entries, selected target, unavailable
shown, worker settled, Diagnostics open, actual 80×24 TestBackend draw and one
Down key. The 100,000-entry input is only 2,300,020 bytes, below the planned 4 MiB
file limit. The 10,000-entry debug draw took **631.98 ms**. The same actual
main/library sources compiled with `rustc -O -Clto=thin` and existing optimized
release dependencies measured:

| Rejected entries | JSON bytes | Optimized modal draw | Optimized single Down key |
| ---: | ---: | ---: | ---: |
| 10,000 | 230,020 | 88.42 ms | 168.30 ms |
| 100,000 | 2,300,020 | 889.85 ms | 1,769.11 ms |

These are one-run local probes, not whole-release qualification, but the
optimized multi-second event cost clearly exceeds the stated 33 ms local UI
budget. No Flutter/device child was needed for this measurement.

Keep selected-entry reasons/warnings separate from global file/compound warnings,
or cache and virtualize the full warning representation with bounded layout work
off the UI event path. Preserve warning compatibility and access deliberately;
do not silently truncate diagnostics. Add a rejected-heavy stress regression
within the accepted file budget, including modal open, draw, Down/wheel, resize,
and live PTY draining. A ready-only catalog is insufficient to validate this
path.

All new probe code and optimized outputs lived in owned TemporaryDirectories,
which were removed after execution. Normal reusable dependency caches were
reused. No source/test file was edited; only this review appendix is retained.

Recap: the four behavior findings across the review loops are now closed. The
remaining synchronous rejected-catalog diagnostics workload keeps the P1
correctness/performance gate open pending a separate fix and re-review.

## Final warning-path re-review and gate verdict

**P1 correctness/performance gate: PASS. No remaining actionable finding in the
reviewed P1 scope.** This closes the preceding rejected-catalog diagnostics
workload finding. The separate rendered-UI gate remains the UI reviewer's
responsibility; this verdict does not qualify physical devices, a release build,
installation, or subsequent feature integration.

Reviewed the actual parser/main/UI changes and appended fixer evidence.
Warnings are now assigned structurally during parsing to their exact source
entry or to the whole file. The legacy aggregate preserves all warning strings
and ordering for `--check`. Selected diagnostics show that entry's reasons and
warnings plus file warnings; sibling warnings are excluded. No-selection/device
views use counts and file warnings without traversing the aggregate. Display
bounds are applied before wrapping with explicit shortening/omission notices;
the original model/CLI strings remain intact. The warning-scope change leaves
acceptance, availability enforcement, generation fences, and deferred selection
contracts intact.

Independent verification:

- Recompiled the original rejected-modal reproduction against actual current
  main/worker/UI modules in an owned temporary harness. Both fixture sizes pass
  the broad 100 ms check, preserve every source/aggregate warning, and expose the
  selected rejection reason:

  | Rejected entries | JSON bytes | Current debug draw | Current debug Down |
  | ---: | ---: | ---: | ---: |
  | 10,000 | 230,020 | 2.49 ms | 0.086 ms |
  | 100,000 | 2,300,020 | 2.24 ms | 0.174 ms |

- Independently reran the rejected-heavy live-PTY stress test. Its 100,000-entry
  fixture measured modal open **1.97 ms**, maximum draw **2.38 ms**, Down
  **0.290 ms**, wheel **0.253 ms**, tick including PTY drain **8.11 ms**, and
  resize/draw **3.55 ms**. These are local debug measurements, not hardware or
  network-filesystem guarantees.
- Independently reran `cargo test --locked --lib --bin flimru --test availability
  --test terminal_flow`: **154 passed, 2 marked ignored** (the parent-invoked
  watchdog child and real-Dart prerequisite). This includes structural duplicate
  warning ownership, aggregate ordering, selected/sibling/global isolation,
  extreme Unicode text, explicit presentation notices, compact reverse scroll,
  ready-heavy stress, both async regressions, and blocked/no-device PTY flows.
- Exercised the actual binary's `--check` with duplicate-name entry warnings,
  40 warnings on another entry, and compounds: all **43 warnings** printed,
  including the last warning beyond the UI display cap.
- Inspected the final strict-Clippy log; `git diff --check` passes.

The independent harness and CLI fixtures used owned TemporaryDirectories and
were removed. Test fixtures own and reap their children. No source/test edit,
real Flutter process, Xcode GUI, installation or project release build was
performed by this re-review; only this appendix is retained.

Recap: all findings from this P1 review are closed. The scoped correctness and
performance gate passes with the documented automated/local-runtime limits.
