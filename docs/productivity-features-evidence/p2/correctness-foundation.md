# P2 worker foundation correctness review

Scope: independent review of `src/config_reload.rs` after the foundation builder
and simplifier, against the automatic-reload contract in
`docs/productivity-features-workflow-2026-11-06.md`. This review does **not** approve
P2 integration or the whole phase. Main/UI apply behavior, persistent last-good
catalog/error handling, selection preservation and session isolation still need
their later integration review.

## Findings

No production correctness or performance findings in the reviewed foundation.

One nonblocking test robustness issue remains:

- **P3 — teardown timeout still performs an unconditional blocking join.**
  `src/config_reload.rs:483–492`: `assert_drop_finishes` receives with a two-second
  timeout, then calls `dropping.join()` before asserting that the timeout passed.
  If `ConfigReloader::drop` deadlocks, the test hangs indefinitely instead of
  reporting its intended bounded failure. Reproduction: make the worker's join
  target remain blocked; after the receive times out, execution stays at line
  490. Prefer a child-process watchdog for teardown/failure-path tests so failure
  can terminate the owned process and release fixtures safely. This affects the
  reliability of regression detection, not observed production shutdown behavior.

## Reasoning checked

- The single owned worker performs content comparison and parsing off the UI
  thread. Two matching automatic observations accept a candidate; accepted
  contents and equal read-error categories suppress subsequent automatic work.
  Manual requests bypass debounce and are coalesced into one pending boolean.
- A request increments the generation and clears the mailbox while holding the
  same mutex used by publication and polling. Work captures its generation before
  reading, checks it before parsing, and checks again when publishing. An older
  parse cannot publish after a manual request.
- Debounce state changes before the stale-generation guard at lines 224–239.
  This ordering does not strand an accepted version: invalidation comes from a
  manual request; its pending read bypasses debounce and replaces accepted state,
  even when its contents equal the invalidated automatic observation. The worker
  executes these reads sequentially.
- The mailbox stores one result. Further accepted automatic versions replace
  unread older results; a manual request invalidates unread prior work. UI callers
  must use `poll` and apply results on their own thread, as the API documents.
- On Unix, `O_NONBLOCK` prevents FIFO-open hangs before metadata checking. The
  opened descriptor, rather than a pre-open path check, must be a regular file.
  Reads are bounded to 4 MiB plus one detection byte, including file growth after
  metadata was sampled, and check shutdown between 64 KiB chunks.
- Drop sets shutdown, clears results, wakes the deadline wait, then joins the
  owned worker. Synchronous filesystem operations already blocked in the kernel
  remain noninterruptible; this limitation is documented in the API.
- Worker diagnostics use fixed enums/text. Parser errors are discarded rather
  than exposing source snippets, environment expansions or OS-error strings.

## Validation

- `cargo test --lib config_reload`: **passed**, 11 tests, 0 failures.
- An isolated harness compiled a temporary copy of the current module against a
  deliberately gated parser. It blocked an automatic parse, rewrote the file,
  requested manual reload, then released parsing. Only the current manual
  generation and new contents were delivered: **passed**.
- The same harness invoked the actual reader with deterministic cancellation on
  the first post-chunk check: **passed**. It also grew the file from two bytes to
  4 MiB + 1 after metadata sampling and verified `TooLarge`: **passed**.
- `git diff --check -- src/config_reload.rs`: **passed**. The helper is untracked
  in the shared worktree; no production source was changed by this review.

The first isolated-harness compile selected an old cached libc artifact with an
incompatible compiler version. Selecting the current artifact resolved that
harness-only setup issue; no cache cleanup or source modification was needed.

## Artifact ownership

Both task-owned harness temporary directories were removed in Python `finally`
blocks, including the failed compile. The successful harness occupied 626,699
bytes before removal. No background process remains. Cargo reused the shared
project `target` cache, currently 4.1 GiB; it is retained for ongoing workflow
builds. The coordinator's temporary directory and other agents' work were not
modified or removed.

## Recap

The worker foundation passed its focused tests and three targeted failure/race
checks. No production issue was found; the teardown-test watchdog deserves a
small follow-up. Final P2 integration review remains pending.

## Fix re-review

The P3 teardown-test finding is **closed** after reviewing the updated test module
and `fixes-foundation.md`. The parent now invokes the exact ignored child helper
in its own process and owns the fixture/log files. Normal completion reaps the
child; timeout and RAII error/unwind paths kill and wait before fixture removal.
The child marks drop start, preserving the separate two-second teardown bound.
Every real-worker test uses this isolation, including paths where an assertion or
early error would otherwise implicitly drop a worker in the parent.

The deterministic `stuck_drop` case supplies a join target that parks forever.
Its parent verifies the drop marker, timeout and unsuccessful process exit. This
directly exercises the failure that the former thread/channel helper could not
bound. No remaining concrete finding was identified in the fix.

Re-review validation: `cargo test --lib config_reload` **passed** (12 passed,
0 failed, 1 intentionally ignored child-helper entry; total 2.01 seconds),
including the stuck-drop timeout test. Scoped rustfmt and report diff checks
passed. No temporary directory or background process was created by this
re-review outside the RAII-owned test fixtures; Cargo reused the shared cache.

Re-review recap: foundation findings are closed. This result still does not
approve P2 main/UI integration or the whole phase.
