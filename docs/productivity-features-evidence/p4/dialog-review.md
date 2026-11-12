# P4 isolated export-dialog review

The modal module performs no capture or filesystem work. The caller supplies the
original source label, bounded export metadata and optional immutable snapshot.
Those values remain tied to that source while other tabs continue running.

API: `ExportDialog::new(source, metadata, destination, snapshot)`, immutable
`metadata()` / `snapshot()` getters, `set_snapshot`, `set_error`, `render`, and
`handle_event`. `DialogAction::Submit(PathBuf)` changes the dialog to Saving
immediately; the caller constructs/submits the export request and reports any
validation/worker error through `set_error`. `Cancel` requires the caller to
cancel/join owned capture/export work and close the dialog. All remaining input
returns `Consumed`; the caller must not forward it to Flutter. Global quit/menu
policy precedes modal handling.

Six tests passed with `cargo test --test export_dialog_ui`. Strict focused Clippy
and scoped rustfmt checks passed. Tests cover explicit readiness, duplicate
submission, retryable destination errors, immutable shared source, UTF-8 cursor
editing, whole-paste control/size rejection, preparing/error/capture completion,
tiny terminals, tab focus, actual rendered mouse regions and background input
consumption. Review fixed the preparing/error/capture edge case and made oversized
paste rejection constant-time before inspecting its contents.

The fifteen TXT/SVG/PNG triplets are actual TestBackend renders at 34×12, 80×24
and 120×40 for ready, preparing, saving, error and long Unicode paths/source labels.
TXT/SVG generation is reproducible by setting `FLIMRU_P4_EVIDENCE_DIR` while
running the focused tests; PNGs use `rsvg-convert`. Visual inspection included
compact ready/error and wide long-path states. Long sources show an ellipsis;
horizontally scrolled destinations show a left marker and keep the cursor visible.
Compact errors may wrap/clamp to the two status rows; the full sanitized message
remains in modal state and is visible on wider terminals.

Evidence occupies approximately 1.9 MiB. Tests use reusable Cargo cache; no task
temporary directories, background processes or disposable deliverables remain.
Only the new modal module, its new integration test and this evidence directory
were changed during this portion. App integration and live PTY validation remain
with the coordinator.
