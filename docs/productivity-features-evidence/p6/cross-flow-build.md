# P6 cross-feature real-PTY flow preparation

Prepared one coherent final workflow test in
`tests/support/productivity_workflow_pty.rs`. It uses `super::*` to reuse the
existing private `Terminal`, `multi_project`, and child-reaping helpers. It does
not import App internals or add dependencies.

The coordinator must register the module at the bottom of
`tests/terminal_flow.rs` after P5 is released:

```rust
#[path = "support/productivity_workflow_pty.rs"]
mod productivity_workflow_pty;
```

## Flow and evidence

The real binary launches two owned mock PTY children. Alpha's frozen console is
searched for `ORIGINAL` and filtered to Errors, visibly hiding its original info
line. An incomplete launch-file save raises the automatic configuration-error
indicator; a valid replacement clears it while the same review stays open. The
replacement includes a future runnable configuration and an unavailable
`preLaunchTask` entry.

An explicit single-session hot reload gives Alpha later live output. Export uses
Ctrl-U plus bracketed paste to edit the destination to the fixture's absolute
path. Its text body must equal the three original console lines and final newline,
including the hidden info line and excluding all
later shortcut output. Device/configuration metadata and absence of ANSI escapes
are also checked.

Reload-all and restart-all are invoked through Actions. Both reports must show
two requested/queued, zero skipped/failed, and unconfirmed Flutter completion.
The fixture transcript checks one command per target, accounting separately for
Alpha's earlier single-session reload. Dismissing each report must restore the
same Alpha snapshot, query, Errors filter, and result. Leaving review shows
Alpha's later output. Browsing the accepted new catalog opens the unavailable
entry's explanation without launching. Original PID files and one startup marker
per device establish that neither session respawned. Normal quit uses the existing
terminal-restoration and all-children-reaped assertions.

All polling has a six-second deadline and short cooperative polling intervals;
there are no unbounded waits or arbitrary long sleeps. The existing `Terminal`
RAII cleanup owns failed-test shutdown, and `TempDir` owns fixture/export files.

## Validation status and handoff

The source is intentionally **unregistered and unrun** until the coordinator
finishes P5 and releases the P6 gate. Source formatting/parsing can be checked
without registering it. `rustfmt --edition 2024 --check
tests/support/productivity_workflow_pty.rs` passed after the source review. No
runtime pass or whole-workflow acceptance is claimed.

Public labels aligned with the coordinator's current contracts:

- P3: `Console severity: Errors`, `Resume live console` and the existing snapshot
  source/query/filter presentation.
- P4: `Export retained console log`, `Destination (no overwrite)`, and the ready
  `Enter Export; Esc Cancel` hint. Success is checked through the exported file
  and dismissal of the destination prompt, without pinning saved-status wording.
- P5: `Reload all eligible (2)` / `Restart all eligible (2)`, delivery-report
  titles, queued count and completion caveat.

After registration, the intended focused command is:

```sh
cargo test --test terminal_flow productivity_workflow_pty -- --nocapture
```

Use the normal shared `target` cache. This preparation created no disposable
build cache, subprocess, or temporary artifact outside the owned test fixture
that will be created during execution.

## Recap

One unregistered cross-feature test is ready for P6 label alignment and execution.
It connects automatic reload, frozen severity review, full-snapshot export and
explicit batch dispatch with real PTY/file/process evidence. Runtime validation
and any fixes remain pending the P5 gate.
