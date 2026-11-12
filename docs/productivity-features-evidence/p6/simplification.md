# P6 simplification

Reviewed the scoped P6 delta against `p5-complete`, the current source against
`p6-built`, and the integration fix report/patch. The global quit guards,
visible-header Quit exception, App regressions and cross-feature PTY assertions
are already narrow and readable. No Rust changes or additional abstraction were
warranted; existing modal semantics and all P1–P5 work are preserved.

Corrected the README controls table: moved its displaced Ctrl+q row back into
the table and stated that global exit confirmation remains available from any
view. Checked the documented Actions labels against `src/ui.rs`; no further
wording changes were needed.

Validation in this pass: inspected the corrected table and current quit hit
testing; `git diff --check` passed. Source and PTY test remain identical to the
post-fix `p6-built` snapshot. Reviewed existing evidence for four new App
regressions, the exact cross-feature PTY test, strict all-target Clippy and
format checks; those passing checks were not repeated for this prose-only edit.

No scratch directory, background process, build output, commit or installation
was created by this pass. Source is stable for the coordinator's final checks
and fresh independent review.
