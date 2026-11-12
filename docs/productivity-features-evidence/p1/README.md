# P1 builder evidence

Implemented structured catalog entries, shared availability/preflight, and reachable
configuration explanations. Existing `LaunchFile::parse` and `configurations`
remain; `entries` maps source positions to parsed configuration indexes or safe
structured diagnostics. Unknown fields retain the existing warning/ignore policy;
known unsupported execution fields remain blocked.

Presentation uses one App-owned cached assessment view per catalog and target
(id/platform/label) / project-adapter signature. It retains reason metadata rather
than command arguments/environment. Catalog/device merge invalidates the view.
Run always performs a fresh all-target preflight and consumes those plans only
when every target passes. If an external directory/helper change invalidates a
cached ready row, Run reveals that same blocked row and its current explanation.

Controls: device picker `d` opens Configuration diagnostics without a target;
configuration picker `u` shows/hides unavailable entries and `i` explains the
selected row. Blocked rows have no Run action. Enter reveals their explanation.
The compact explanation scrolls with arrows/wheel; Esc/Enter closes it. Modal
keyboard, paste and pointer routing cannot start a child or send Flutter input.
Empty, rejected-only, text-filter-empty, and target-excluded states are distinct.

## Reproduce actual render evidence

```sh
FLIMRU_P1_EVIDENCE_DIR=docs/productivity-features-evidence/p1 \
  cargo test --bin flimru configuration_diagnostics_render_evidence -- --nocapture
python3 - <<'PY'
import pathlib, subprocess
for image in pathlib.Path('docs/productivity-features-evidence/p1').glob('*.svg'):
    subprocess.run(['rsvg-convert', '-o', str(image.with_suffix('.png')), str(image)], check=True)
PY
```

The test draws the real UI through Ratatui TestBackend at 34×12, 80×24 and
120×40. TXT records the cell grid; SVG/PNG renders its positions, colors and bold
modifiers. Nine named states include long/Unicode names, rejected-only/empty,
exclusion with unavailable hidden/shown, no filter matches, no devices,
no-target explanation, and blocked explanation before/after scrolling. The test
asserts no fixture environment/define/rejected values appear, that blocked Run
is absent, and that the explanation title/dismissal hint fit at every size.

## Checks at handoff

- Owned Rust files formatted with rustfmt (2024 edition, skip_children=true).
- `cargo clippy --all-targets -- -D warnings`: passed; `clippy.log`.
- `cargo test --locked --all-targets`: 168 passed, 3 ignored; `rust-tests.log`.
  This includes the independently authored polling/snapshot/batch helper tests
  present in the shared checkout. Mock VM-service tests needed local TCP binding
  outside the sandbox; the first sandboxed run was blocked at bind(2), then the
  complete permitted run passed.
- New availability tests exercise source order and duplicate display names,
  malformed/rejected fields, preserved unknown-field compatibility, safe variable
  diagnostics, no-target cwd/argument/task validation, target-specific pin/adapter
  reasons, tester and multi-target restrictions, and exact planned target args.
- App tests exercise cached presentation versus fresh launch preflight, blocked
  keyboard/pointer launches, modal paste/input isolation, and no-device access.
- Real-binary PTY tests exercise initial rejected-only and empty catalogs, blocked
  keyboard/mouse Run attempts, no discovered targets, 34×12 resize, and pasted
  input while an explanation is open. Existing multi-session, stop/reap, reload,
  clipboard, pointer, color and idle-render tests passed with updated count labels.

Self-review fixes included preserving warning compatibility for unknown fields,
removing secret-bearing values from rejection messages, replacing misleading
blocked-row Run hints, and retaining the exact selected source row when a fresh
preflight invalidates a cached preview. Independent simplify/correctness/UI gates
remain the coordinator's next steps. No real Flutter/device qualification, release
build, packaging/audit, or installation was performed by this builder.

Fixtures use RAII temporary directories. Normal project target cache was reused;
no disposable build/cache directory or background process is retained. This named
render/test evidence directory is retained intentionally (about 6 MiB).
