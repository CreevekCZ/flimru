# P5 helper correctness foundation

Independent read-only review of `src/batch_commands.rs`,
`tests/batch_commands.rs`, the approved workflow and separate simplification
report. Consulted the actual launch planner and installed Flutter source to
verify launch-option semantics. No implementation or existing test was edited.

## Finding

### P2: Exclude launch options that remove interactive hot-command support

Location: `src/batch_commands.rs:94-98`.

`HotCapabilities::from_launch` returns `Supported` for debug app configurations
containing any of `--machine`, `--machine=true`, `--no-resident`,
`--resident=false`, `--trace-startup`, or `--trace-startup=true` in either `args`
or `toolArgs`. `LaunchConfig::flutter_args` accepts and forwards these options
(`src/config.rs:178-220`), so this is reachable with ordinary launch.json entries,
not just an invalid manually constructed configuration.

These options do not support the helper's ordinary `r` / `R` terminal protocol:

- Machine mode starts Flutter's daemon instead of installing `TerminalHandler`.
  The daemon consumes newline-delimited JSON messages. Raw `r` / `R` does not
  request a hot operation and may remain buffered with subsequent input.
- Nonresident mode deliberately does not install `TerminalHandler`, including
  during the interval when the child is still live while building/launching.
- Startup tracing forces Flutter's hot mode off even when the app build is debug
  and no `--no-hot` option was supplied.

Verified directly in the installed SDK at
`/Users/jankoznarek/fvm/default/packages/flutter_tools/lib/src/`:
`commands/run.dart:46-52` defines startup tracing,
`:495-506` defines hot/resident options, `:696-702` computes hot/resident state,
`:852-895` selects the machine daemon, and `:939-957` installs terminal input
only when resident. `commands/daemon.dart:198-203` selects stdio daemon streams;
`daemon.dart:135-147` parses JSON-framed messages rather than terminal commands.

Extend the launch-time exclusion logic for these option forms in both argument
sources, using bounded fixed exclusion descriptions. Preserve conservative
handling of conflicting enabling/disabling options. Add tests showing these
runs are excluded and receive no dispatch, while normal interactive debug runs
remain supported. No daemon protocol implementation or process replanning is
needed for this feature.

## Independent validation

- `cargo test --locked --test batch_commands`: **11 passed, 0 failed**.
- An external Rust harness linked the actual public library and reproduced all
  **12 false-eligibility cases** (six option spellings, each in `args` and
  `toolArgs`). For each case it checked successful argument planning and option
  forwarding, observed `Supported`, and confirmed dispatch called the sender once
  with stable ID `900` and byte `r`, reporting one requested input.

The existing helper otherwise records capabilities from launch data rather than
labels/current files; excludes tests/profile/release/hot-disabled/custom runners;
dispatches once per unique supplied stable ID; skips ended/stopping/unsupported
snapshots; continues after failures; and uses bounded, control-free device details
with fixed safe outcome descriptions. Counts and wording correctly distinguish
requested/queued input from Flutter completion. The callback API accepts stable
identities and does not require any respawning, launch planning, or mutable vector
position. Actual stable-ID allocation and caller re-resolution remain integration
responsibilities.

The harness used the normal reusable `target/` artifacts. Its task-owned temporary
source/binary directory occupied 968 KiB and was removed by an EXIT trap after the
child exited. No background process or disposable artifact remains owned by this
review.

## Remaining gate

The finding requires a separate fix and re-review. Foundation review does not
approve the phase. After main/UI integration, review launch-time capability
capture and runner classification, stable unique ID allocation and re-resolution,
eligible counts/explanations, dispatch through existing `Session::send`, exactly
one request per target, active-tab/review preservation, ordinary single-session
`r/R`, animation restart, modal routing, and rendered/PTY behavior.

## Recap

One P2 finding: unsupported launch options are currently reported eligible.
Existing 11 tests pass; 12 independent API cases reproduce the gap. No full-phase
approval; separate fix/re-review and later integration review are required.

## Independent foundation re-review after separate fix

Reviewed the updated helper, all 14 tests, and `fixes-foundation.md`. The P2
finding above is **closed**. The earlier finding and validation sections describe
the original implementation; this section records the current foundation verdict.

`MachineMode`, `NonResident`, and `StartupTracing` are fixed typed exclusions with
safe descriptions. All six reported incompatible option spellings are detected
in either argument source before `Supported` can be returned. An incompatible
occurrence remains excluding regardless of option order or source. Explicit safe
boolean values alone preserve normal debug eligibility. The correction does not
change dispatch, stable-ID resolution expectations, launch planning, process
ownership, or single-session input behavior.

The new regression matrix exercises successful argument forwarding for all 12
reported launch cases and confirms no sender invocation for both `r` and `R`
(24 dispatch checks). It also covers implicit/explicit debug with safe boolean
values, both argument sources, and reversal ordering within/across sources.
Independently reran `cargo test --locked --test batch_commands`: **14 passed,
0 failed**. No new concrete helper correctness issue was identified.

The foundation correctness gate is clear. Full P5 approval remains open pending
the actual main/UI integration, `Session::send` and animation behavior, and
rendered/PTY review identified above. This re-review used the existing reusable
cache and created no disposable artifacts or background processes.

Recap: the original P2 finding is closed; 14 focused tests pass; no remaining
foundation finding. Later integration and full-phase gates remain required.
