# Development standards

Run `make setup-tools` once to install the pinned dependency audit tool. Rust's
`rustfmt` and `clippy` components must also be installed. Run `make check` before
finishing a change: it checks formatting, all-target Clippy warnings, Rust and
packaging tests, and the current RustSec advisory database. Dependency warnings fail the check;
investigate them rather than adding a blanket exception. Builds and tests use
`Cargo.lock` without changing dependency resolution.

Use Rust 2024 and rustfmt's standard formatting. Keep modules focused on their
existing responsibilities. Prefer explicit ownership, small functions, and
`Result` with useful context for fallible work. Production `unwrap`, `expect`,
explicit panics, unfinished placeholders, debug macros, and direct process exit
are denied by Clippy. Tests may fail fast while creating fixtures.

Every unsafe block needs a nearby `SAFETY:` comment explaining its actual
preconditions: descriptor/handle lifetime, pointer validity, ownership, and any
initialization assumptions. Keep these blocks small. A comment is a review aid,
not proof that the call is correct.

Workers must have an owner, cancellation, and a shutdown path. Use bounded
channels, capture sizes, history, and I/O deadlines. Never detach a Unix worker
or bypass `Drop`; signal only process groups created and owned by this app.
Treat Windows behavior as unqualified until its worker and process cleanup are
validated. Keep slow work and unbounded loops off the UI thread, and respect the
frame and input/output budgets.

Preserve argument boundaries when launching tools. Do not interpolate project
values into shell code, expose dart defines in status text, or overwrite corrupt
state. Animation status must reflect an acknowledged VM Service value.

For changed behavior, run the relevant existing regression coverage, then the
full check. Add tests for meaningful failure and boundary cases. UI/process
changes should use the real PTY tests, including terminal loss, repeated runs,
child cleanup, and mouse/resize behavior. Do not treat a passing linter as proof
of runtime safety: review the change and fix findings before calling it done.

The optional real Dart VM test requires `FLUTTER_TUI_TEST_DART`; it uses an
isolated fixture and does not qualify a physical-device run of a Flutter app.
All checks run locally; GitHub Actions is disabled for this repository.

Release preparation also uses Python 3.9+ (standard library only). See
[the release guide](docs/releasing.md); preparation does not publish anything.
