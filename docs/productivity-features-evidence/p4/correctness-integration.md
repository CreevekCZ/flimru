# P4 App integration correctness review

Independent read-only review of the six integration source/test paths, the
builder's scoped `integration.patch` against `p3-complete`, the separate
simplification patch, and both integration handoff reports. Backend/dialog
foundation reviews and their closed standalone fixes were retained; unchanged
backend matrices were not repeated. No product or existing test files were
edited by this reviewer.

## Findings

### P2: the first export cancellation notice resets a manually scrolled review

Location: `src/main.rs:1017`, interacting with `1098–1119`, `723–732`, and
`src/console_search.rs:614–619`.

Reproduced on the actual App state/draw/event methods at 80×24:

1. Capture roughly 90 retained rows, with the sole `needle` match in the first
   row. Apply that query, leave query editing, and render with no operation notice.
2. Press End and render, moving the manual review offset to `(76, 0)` while the
   current match remains in row zero.
3. Open export and render: the exact snapshot and `(76, 0)` offset are preserved.
4. Press Escape to cancel, then render: the offset becomes **`(0, 0)`**.

Cancellation calls `report_operation`, which introduces the first `review_notice`
row. The resulting one-row viewport change makes SearchView set `reveal` from the
current match again, overriding manual scroll. The same first-notice transition
is possible on export success/error. Preserve the user's viewport when adding an
operation outcome, while retaining deliberate match navigation and actual resize
behavior. Add a regression with the current match outside the manually chosen
viewport; the existing three-line export fixture cannot expose this.

The coordinator's original concern with an **already existing notice** was also
reproduced and **passes**: opening/rendering/cancelling export preserves the
bottom offset because both covered and uncovered viewports already reserve the
notice row (`src/main.rs:690–691`). That concern is not an open finding.

### P2: permitted long success paths lose their tail and byte count in retained details

Location: `src/main.rs:1055`, success formatting at `1023–1026`, and the generic
operation cap at `1098–1109` (`MAX_OPERATION_CHARS = 4096`).

A successful export outcome combines an absolute path of up to 4096 bytes with a
message prefix and the byte-count suffix, then passes it through a **4096-character
total-message cap**. A near-limit ASCII path therefore cannot survive in full;
both the path tail and byte count are replaced by `… [shortened]`. Operation
details cannot recover them because `last_operation` stores the shortened copy.

An independent actual-App formatter/retention probe used a 4095-byte absolute
path consisting of 31 components of 128 ASCII bytes and a 95-byte filename, all
within Linux component/path bounds, with `SavedExport.bytes = 123`. The retained
message contained 4093 characters and lacked the full path and `123 bytes`.
This boundary probe supplied a successful result directly; it does **not** claim
that such a filesystem path was created on macOS. Near-4096-byte paths are relevant
to the supported Unix/Linux path budget even though macOS has tighter filesystem
limits.

Retain the complete permitted path and count using a sufficient bounded outcome
budget or a structured retained result. Screen clipping can remain independent.
Add a success-result boundary regression with realistic component lengths.

### P2: final shutdown prints an unsanitized canonical destination to the terminal

Location: `src/main.rs:1073–1075` and the post-restoration print at `639–640`.

`shutdown_export` sends a sanitized copy to `report_operation`, but returns the
**original** `export_result_message` string. `run` prints that original string
after restoring the terminal. A clean edited parent symlink can resolve to a
canonical parent whose filename contains control bytes, so validating the input
destination does not make the final saved path safe to print verbatim.

Reproduced with a real task-owned export: create `safe` as a symlink to a directory
named `unsafe\x1b[31m-directory`, submit the clean `safe/out.txt` path, wait for
publication without consuming the worker completion, then call `shutdown_export`.
It returned a successful 162-byte outcome containing the raw **ESC `[31m`**
sequence from the canonical path. That exact string is the subsequent `eprintln!`
argument, allowing filesystem data to be interpreted as terminal instructions.

Apply consistent safe path/message presentation to the returned shutdown outcome
as well as normal status/details. Preserve truthful success and byte count when
publication wins. Add a real-filesystem regression through a clean symlink whose
resolved parent contains controls; the ordinary typed-path rejection does not
cover this path.

## Validation and cleared areas

- `cargo test --locked --bin flimru log_export -- --nocapture`: **6 passed**,
  97 filtered out; 0.07 seconds.
- `cargo test --locked --test terminal_flow console_export_real_binary -- --nocapture`:
  **1 passed**, 29 filtered out; 2.96 seconds.
- Independent App probes compiled read-only copies of the actual main module
  against the normal library artifacts, with absolute module paths and extra
  probe tests only in a task-owned temporary directory. The initial four probes
  produced **2 passes / 2 failures**: existing-notice viewport and live-draining
  lifecycle passed; first-notice cancellation and the result-size boundary failed.
  A second pair confirmed the size failure with realistic Linux-sized components
  and reproduced the raw-control shutdown issue using an actual export.
- The live-output probe continued draining **4,000 new lines** while the export
  dialog remained open, observed the ending source transfer to the saved console
  under the same stable ID, confirmed the export snapshot remained exactly
  `INVOCATION`, verified no pre-submit `flimru-logs` directory existed, then closed
  that saved source and confirmed its pending export was cancelled.
- Reviewed invocation metadata/time, exact filtered-review Arc reuse/full export,
  historical-width and alternate-grid preclone bounds, sequential capture/writer
  ownership, duplicate submission, retry/no-clobber, cancellation/publication
  truth, close-source hooks, normal early modal routing, covered selection/cursor,
  and the narrow `ExportWorker::shutdown` join/result logic. No additional concrete
  backend ownership or lifecycle finding was identified.

The initial direct-rustc probe needed explicit Cargo package-version metadata;
that harness-only compile setup was corrected before executing the probes. Its
failed temporary directory was removed automatically. The two executed harness
directories occupied 13,498,557 and 13,476,748 bytes before cleanup; both and their
fixtures/owned child processes were removed or reaped before this report. No
additional build cache, background process, or disposable artifact remains owned
by this review. The coordinator's baseline was read only and preserved.

## Recap / gate

Three P2 integration findings require a separate fixer and independent re-review.
Existing focused App tests and the real-binary flow pass; targeted independent
probes expose the uncovered viewport and outcome cases. The original existing-
notice viewport concern is cleared. Full P4 correctness approval is **pending**;
the coordinator also owns the separate rendered/UI gate and its findings.

## Independent re-review of the separate integration fixes

All three P2 correctness findings above are **closed**. The preceding findings
remain as review history; this verdict supersedes their open disposition. Read
`fixes-integration.md`, the actual source changes, and the added regressions.
This reviewer made no product/test edits and did not repeat the unchanged backend
matrix or the broad 209-test baseline.

Confirmed fixes:

- SearchView's render geometry handling now preserves manual scroll for a
  height-only change. First-notice cancellation and the already-cleared existing-
  notice case both preserve the prior offset, query, and identical frozen Arc.
  Explicit next-match navigation still reveals the match, and a width change
  still reveals the current match as intended. Existing clamping remains active.
- The bounded operation budget is now 8192 characters, sufficient for an entire
  permitted export destination plus prefix and the maximum `usize` byte count.
  Both 4095- and 4096-byte component-bounded synthetic success paths remain exact
  in retained outcome/status. The focused regression also checks scrollable
  Operation details and continued bounded shortening above the new total budget.
- `shutdown_export` now returns the safe retained outcome after
  `report_operation`, so the final terminal print receives the sanitized value.
  A real clean-symlink export into a parent containing ESC `[31m` retained
  truthful publication success, filename, and actual byte count with no controls
  in its returned message; status and `last_operation` equal that safe result.

Validation rerun by the original correctness reviewer:

- `cargo test --locked --bin flimru log_export -- --nocapture`: **10 passed**,
  zero failures; 0.08 seconds.
- `cargo test --locked --test console_search_ui`: **20 passed**, zero failures,
  one existing explicit-measurement ignore; 0.24 seconds.
- `cargo test --locked --test export_dialog_ui`: **16 passed**, zero failures;
  0.14 seconds.
- A fresh task-owned harness compiled a read-only copy of the actual main module
  and reran three independent checks: **3 passed**, zero failures. These cover
  both first/existing notice cancellation followed by explicit match navigation
  and width resize, exact outcomes at both path-size boundaries with
  `usize::MAX` bytes, and actual published canonical-control-path shutdown.
  The notice checks also verify unchanged query/snapshot identity and no
  pre-submit filesystem creation.

Reviewed the narrow modal-coverage accessor and early mouse-routing exception:
it admits only the visible Quit action outside the last rendered modal coverage,
while coverage before first draw fails closed and tiny fallback covers the full
screen. The focused App suite exercises confirmation/cancellation and immutable
dialog state. The original UI reviewer independently owns final visible-Quit and
rendered validation; this correctness review does not replace that gate.

No concrete residual correctness/performance finding was identified. The fresh
external harness occupied 13,563,684 bytes before cleanup; its directory,
fixtures, and owned workers were removed/reaped after execution. No additional
build cache, background process, or disposable artifact remains from this review.

Re-review recap / gate: **P4 independent correctness gate passes**. All three
correctness findings are closed, 46 focused tests and three independent checks
passed, and the one existing measurement ignore is unchanged. The coordinator
retains the separate UI review and full phase/integration decision.
