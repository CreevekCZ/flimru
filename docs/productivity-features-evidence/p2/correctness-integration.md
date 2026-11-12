# P2 integration correctness and performance review

Verdict: **hold the P2 correctness gate** for the queued-input finding below.
No implementation edits were made by this reviewer. The earlier worker-foundation
review and closed watchdog finding remain valid.

## Finding

**P2 — reload completion re-enables Run while an earlier Enter is still queued.**

Location: `src/main.rs:895–900` (pending guard cleared on completion), connected
to `start_batch` at `src/main.rs:723–727` and the event loop at
`src/main.rs:460,528–575`.

The pending guard prevents a rapid `r` + Enter only when both events are processed
before the next completed reload is applied. The event loop deliberately limits
input processing to 128 events or five milliseconds, then calls `tick` before
reading the remaining input. A completed reload clears the guard even though an
Enter from the original input burst remains queued. Once availability has filled,
that Enter starts a child without a new Run action after reload completion.

This was reproduced against the current real binary using the existing PTY
fixture, without changing launcher source. After reaching the ready configuration
picker and rewriting the selected unique-name entry's arguments, send this in
**one** `Terminal::send` call:

```rust
let mut bytes = b"r".to_vec();
for _ in 0..256 {
    bytes.extend_from_slice(b"\x1bOP"); // F1: intentionally inert launcher input
}
bytes.extend_from_slice(b"\r");
tui.send(&bytes);
```

No further input was sent for two seconds. Observed: `session.log` was created
and the screen displayed `SESSION READY` with one running session. The isolated
regression assertion that no child had started failed. F1 events do not navigate,
select, dismiss modals or request Run; their only role is to cross the event-loop
budget while keeping the originally queued Enter pending. Scheduling delays can
also split otherwise short bursts at the five-millisecond budget boundary.

Keep launch arming separate from worker completion so input already queued across
that completion cannot become a fresh post-reload Run. Preserve the bounded input
loop and add a real-binary regression crossing its frame/event budget; the current
`tests/terminal_flow.rs:682–700` only exercises adjacent `r\r` in one short batch.
Cover both successful and failed manual completion, since failure also releases
the guard while retaining a runnable last-good catalog.

## Reviewed behavior without additional findings

Read the approved P2 contract, builder report/patch, simplification report and
actual production `src/main.rs`, `src/ui.rs` and `src/config_reload.rs`, including
the surrounding event loop, availability context/results, launch preflight and
worker shutdown. Compared the integration production delta with the coordinator's
`p1-complete` snapshot; settled P1 parser/availability/source/warning policy remains
intact. Disjoint, unwired console-search/export feature internals were excluded.

- Startup and `--check` use the same 4 MiB bounded regular-file reader and fixed
  safe parser diagnostics as background reload. The resolved selected path,
  including explicit relative `--launch`, is passed to the owned worker.
- All manual routes converge on one request method; completed manual/automatic
  work converges on one apply method. Worker and App generation guards reject
  stale results. A newer automatic completion can supersede an unread manual
  completion without leaving pending state stuck.
- Read/UTF-8/JSONC/schema failures preserve the accepted Arc catalog and a separate
  persistent error. Automatic failures do not replace unrelated status. Matching
  errors suppress duplicate redraw; automatic recovery only replaces status if
  the exact prior reload failure is still displayed.
- Initial semantic duplicates retain catalog/cache/selection state. Recovery
  refreshes metadata even when the parsed catalog matches the last-good version.
  Manual unchanged successes replace the Arc and refresh availability. Valid
  empty and rejected-only catalogs are accepted.
- Selection captures deferred logical source before replacement. Unique names
  must be unique on both sides; duplicate names require a unique matching
  normalized entry/configuration/diagnostics/warnings fingerprint in both
  catalogs. Removing an identical sibling remains ambiguous. Missing/ambiguous
  selection stays nonactionable through merge and partial availability fills.
- Apply preserves current page, active console, frozen text selection, picker
  filter/edit mode, open modal state, marked targets, favorites, unrelated status
  and existing run objects. Merge recomputes catalog-pinned devices while keeping
  established target metadata policy. No edit restarts existing sessions or
  changes their launch-time command.
- Presentation runs through the settled P1 asynchronous availability worker,
  including generation invalidation and same-generation synchronous overrides.

## Independent validation

- `cargo test --locked --bin flimru --lib --test availability --test terminal_flow`:
  **passed**, 57 library + 82 binary/App/render + 3 availability + 24 PTY tests;
  two intentional ignores (subprocess-only helper and optional real Dart test).
- `cargo test --locked --bin flimru reload_large_catalog_apply -- --nocapture`:
  **passed**. The 100,000-entry live-PTY case measured semantic duplicate
  apply/release at **22.773 ms**, changed/reordered apply/identity/merge at
  **27.207 ms**, and following heartbeat-draining tick at **0.457 ms**. These met
  the established frame target on this machine; they do not establish arbitrary
  filesystem/hardware latency qualification.
- Extra isolated real-binary PTY regression above: **failed**, demonstrating the
  queued-input issue despite the standard suite passing.

The extra harness was compiled from a temporary copy of the existing PTY test
module plus the focused regression, linking the current normal-cache artifacts
and executing `target/debug/flimru`. One initial compile needed the missing
`serde_json` linker input added. An initial 1,024-F1 payload hit the fixture's
nonblocking-write limit; the reproduced 256-F1 case fits in a single write. These
were harness setup issues, not additional launcher findings.

## Ownership and recap

All isolated harness directories were removed in `finally` blocks, including
failure paths. The successful reproduction called fixture `quit` before asserting
the failure; launcher/session children were reaped, and the fixture's RAII cleanup
also covers earlier harness failures. No background process or disposable build
directory is retained. Cargo reused the shared project `target` cache, about
4.4 GiB, retained for the ongoing workflow. Coordinator-owned data and unrelated
dirty work were preserved.

Recap: the standard P2 behavior suite and live apply measurements pass, but a
queued Enter crossing the bounded input loop can start a process after manual
reload completion. Correctness approval remains pending its fix and re-review;
rendered UI approval is a separate gate.

## Fix re-review and current verdict

**P2 correctness/performance gate: PASS.** The earlier queued-input finding is
**closed**. This verdict supersedes the initial hold above; independent rendered
UI approval and the coordinator's overall phase gate remain separate.

Read `fixes-integration.md` and inspected the actual final fence state, event-loop
idle acknowledgement, launch guard, UI action/footer/header conditions and new
App/PTY regressions. No remaining actionable correctness/performance finding was
identified.

The implementation now distinguishes generation-pending work from Run arming:

- A manual request arms the input fence before worker creation, so even a
  worker-start failure cannot release previously queued Run input.
- Matching/newer completion clears generation-pending work but sets the input
  fence for both success and failure. A newer automatic result superseding an
  unread manual result follows the same path; stale generations return before
  changing either state.
- The fence clears only at the existing event-loop branch where the positive
  `event::poll` returns false (`src/main.rs:540–546`). Neither the 128-event cap
  nor the five-millisecond work cap calls that acknowledgement. The idle branch
  breaks the input pass and requests redraw, so the next frame is drawn before
  later input can invoke Run.
- `start_batch` and UI Run presentation use the common guard. Other events
  continue through their normal handlers; there is no key flushing, queued
  implicit launch, added sleep or new confirmation. The guard-owned status is
  restored only if its exact message remains displayed, preserving unrelated
  statuses.

Independently inspected crossterm 0.29.0's selected `use-dev-tty` backend in the
local dependency source. Its positive-timeout event source checks parsed events
before polling the tty and retries interruptions. Retaining the existing
positive minimum timeout avoids the zero-timeout path that can skip that parsed
event check.

Re-review validation:

- `cargo test --locked --bin flimru --lib --test availability --test terminal_flow`:
  **passed**, 57 library + 83 binary/App/render + 3 availability + 26 PTY tests;
  **169 passed, 0 failed, 2 intentionally ignored**.
- This run re-executed the original single-write `r` + 256 inert F1 + Enter
  reproduction through the new event-budget regression. It now starts no child
  until a fresh post-ready Enter. The PTY tests also verified keyboard/toolbar
  requests, successful/failed reload, event-count/time-budget bursts, exactly one
  fresh launch, fresh arguments after success and last-good arguments after
  failure: all eight route/outcome/budget cases passed.
- The App regression passed idle-while-pending protection, stale-result
  preservation, newer automatic completion coalescing, blocked queued Run,
  navigation/filter editing/quit-cancel routing during the fence, outcome status
  restoration and re-arming without an implicit launch. Existing manual-route,
  live-session/state preservation, automatic recovery, P1 async-source/override
  semantics and 100,000-entry live-PTY apply performance regressions passed in
  the same focused suite.
- Scoped rustfmt and source diff checks: **passed**.

No new standalone harness or persistent process was needed for this re-review;
the original reproduction is now a maintained real-binary regression. Test
fixtures and child lifetimes use their existing RAII ownership, and Cargo reused
the shared normal project cache. No coordinator data or unrelated file was
removed or edited.

Re-review recap: all foundation and integration correctness findings are closed.
The full reviewed P2 correctness/performance contract passes; rendered UI and
overall phase approval remain with their designated reviewer/coordinator.
