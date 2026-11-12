# P4 backend correctness foundation

Independent read-only review of `src/log_export.rs`, `tests/log_export.rs`, the
shared snapshot contract, the approved workflow, and the separate simplification
report. No implementation or existing test files were edited by this reviewer.

## Findings

No concrete backend correctness finding was identified. This covers the backend
foundation only; it does **not** approve P4 or its later main/UI integration.

The request owns an immutable `Arc<ConsoleSnapshot>` and a bounded sanitized
metadata header. Export always writes the entire snapshot text without accepting
search/filter state. The checked 16 MiB budget includes the header, and request
preparation performs no filesystem writes. Relative destinations resolve against
the supplied absolute workspace; only its immediate `flimru-logs` directory is
created automatically on submission. Edited destinations otherwise require an
existing parent.

The worker owns one job and retains its one completion until polling consumes it.
Drop cancels and joins the owned thread. Writing uses a private same-directory
temporary file, chunk cancellation checks, flush/sync, and atomic hard-link
publication that refuses an existing destination entry. A successful publication
reports success even if cancellation arrives afterward. Temporary ownership is
retained through success/error paths. Filesystems must support ordinary hard
links; unsupported publication produces an error instead of an unsafe fallback.
Errors retain operation, safe kind, and numeric OS code without arbitrary OS
payloads. The metadata API accepts labels and outcome, with no environment or
launch-argument fields.

## Independent validation

- `cargo test --locked --test log_export`: **14 passed, 0 failed**; test runtime
  0.85 seconds.
- An external, task-owned Rust test harness exercised the exported library API:
  **6 passed, 0 failed** across its normal and file-size-limited runs. The checks
  covered 20 pairs of concurrent exports to the same destination (exactly one
  winner per pair, complete contents, no partials), dangling-symlink and directory
  collisions, an edited symlink parent with a canonical absolute saved path and
  0600 permissions, completed-but-unconsumed job ownership and cancellation after
  publication, and frozen original-width Unicode history after resize followed by
  additional live parser output.
- The final harness check ran in an isolated child process with `SIGXFSZ` ignored
  and `RLIMIT_FSIZE=1024`, producing an actual partial write failure on a roughly
  200 KB export. The backend returned its safe `write export` error, published no
  destination, and removed the owned temporary file.

The harness reused normal project library artifacts, and its fixtures used RAII
temporary directories. Its source/binary directory occupied 2,083,780 bytes before
cleanup and was removed after the child processes exited. No additional build
cache, background process, or disposable artifact remains owned by this review.

## Remaining gate

Review the actual main/UI integration after it lands: capture-at-invocation versus
review-snapshot reuse, source/metadata identity, prompt input and cancellation,
one-job UI enforcement, notifications and safe path rendering, live PTY draining,
modal routing, and shutdown. Rendered/PTY evidence is also required by the phase
workflow. Backend validation alone cannot establish those behaviors.

## Recap

Backend foundation review: no findings; 14 focused tests and six independent API
checks passed, including an actual partial-write failure. Phase approval remains
pending integration and UI review.
