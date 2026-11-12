# Homebrew release preparation

Current status: **local preparation only**. No GitHub release, tag, tap update, or
repository visibility change has been published. The first package is version
**0.1.0 for Apple Silicon macOS**. Intel and Linux binaries are not prepared.

## Repositories

- Utility: `CreevekCZ/flimru` (currently private).
- Public tap: `CreevekCZ/homebrew-tap`.
- Maintained local tap checkout: `~/Documents/Development/homebrew-tap`.
- The tap keeps formulas at its root; `flimru.rb` belongs beside `afm-cli.rb` and
  `vash.rb`. Do not introduce a `Formula/` directory without migrating all formulas:
  Homebrew searches the first formula directory it finds.
- Homebrew's installed tap under `$(brew --repository CreevekCZ/tap)` is a separate
  checkout. Edit the maintained Development checkout, not the installed copy.

Public Homebrew installs need an anonymously downloadable, immutable release
archive. Before publication, choose whether to make Flimru public or host the
binary release in a separate public repository and update the formula URL.
A public tap cannot download an asset from the current private Flimru repository.
Do not put GitHub tokens in formulas or archive URLs.

## Local build

Use an Apple Silicon Mac, Python 3.9+, Rust/Cargo, and the Xcode selected by
`xcode-select -p`. `make check` also needs cargo-audit (`make setup-tools`).
GitHub Actions remains disabled; all checks and builds run locally.

```sh
make check
make release-draft
```

`release-draft` permits uncommitted preparation changes and marks the build as a
**local draft** in its metadata. After the release source is committed and the
checkout is clean, use `make release` for the final candidate. Both refuse to
replace an existing output directory; retain the draft elsewhere before rebuilding,
or choose a new directory explicitly:

```sh
python3 scripts/prepare_release.py --output /tmp/flimru-final-candidate
```

The builder uses `Cargo.lock`, the explicit `aarch64-apple-darwin` target, generic
ARM CPU code generation, and a macOS 11.0 deployment target. It checks the binary's
architecture, system-only dynamic library linkage, deployment target, and ad-hoc
code signature. This is not Developer ID signing or notarization. The metadata
records the actual macOS version tested; older macOS versions still need runtime
qualification before claiming they were tested.

A smoke test loads a JSONC launch file using a fake Flutter path without executing
Flutter. The archive includes only the executable, MIT license, README, and usage
guide. Deterministic archive metadata avoids timestamp-only checksum changes;
this does not claim reproducible Rust compilation across different toolchains.

Output in `dist/v0.1.0/`:

| File | Purpose |
| --- | --- |
| `flimru-v0.1.0-darwin-arm64.tar.gz` | Release download |
| `SHA256SUMS` | Exact archive SHA-256 |
| `flimru.rb` | Tap formula with versioned URL and real checksum |
| `release-notes.md` | Candidate GitHub release text |
| `build-info.json` | Source revision, dirty state, toolchain, host, binary checks |

`dist/` is ignored by Git. Keep generated archives out of commits. The maintained
formula template is `packaging/homebrew/flimru.rb.in`; generate it rather than
editing a checksum placeholder. Installation instructions belong in Flimru's
project `README.md`; leave the tap's documentation unchanged.

## Validate the candidate

```sh
cd dist/v0.1.0
shasum -a 256 -c SHA256SUMS
ruby -c flimru.rb
```

Extract the archive to a temporary directory and run its `flimru --version`,
`--help`, and `--check /path/to/flutter/project`. Check archive contents before
publication. The formula's test loads a commented/trailing-comma launch file and
checks the parsed configuration. It requires neither Flutter downloads nor a
simulator/device.

The local archive can be installed into an isolated test tap by replacing the URL
in a **temporary formula copy** with a `file://` URL while keeping its checksum.
Homebrew requires the formula to be inside a tap under its `Library/Taps` directory
even for linting. Run `brew style --formula`, `brew install --skip-link`, `brew test --force`,
and `brew uninstall` against that temporary tap formula;
do not commit the local URL or replace the existing Cargo-installed executable.
Normal online formula checks cannot succeed before the public asset exists.

## Publication handoff (separate, later action)

1. Review and commit the intended release source and preparation files. Rebuild
   from the clean revision and run the full local checks. Verify `source_dirty`
   is false and `source_revision` is the intended release commit.
2. Resolve public download hosting. Create tag `v0.1.0` at that exact source
   revision, then publish its release with the archive, `SHA256SUMS`, and reviewed
   release notes. Do not publish a dirty draft or replace a published version's
   archive with different bytes; bump the version instead.
3. Verify an unauthenticated download of the published URL and its checksum.
4. Copy the **final generated** `flimru.rb` to the Development tap root. Update the
   installation instructions in Flimru's project `README.md` after publication.
   Keep the tap's documentation and other formulas unchanged; do not introduce
   release workflows.
5. Test through Homebrew using the published URL, run `brew test` and
   `brew audit --strict`, review the tap diff, then commit and push that tap update.
6. Verify installation from a fresh tap checkout:
   `brew install CreevekCZ/tap/flimru`.

Flutter and FVM stay user-managed. Homebrew installs only `flimru`, not a Flutter
SDK. Existing `~/.local/bin/flimru` from Cargo may win in `PATH`; use `type -a flimru`
and `$(brew --prefix)/bin/flimru` to verify the Homebrew version. Migration from
Cargo is a separate choice; do not remove the current installation during release
preparation. `brew uninstall flimru` preserves saved favorites and settings.

## Homebrew references

- [Formula Cookbook](https://docs.brew.sh/Formula-Cookbook)
- [Maintaining a tap](https://docs.brew.sh/How-to-Create-and-Maintain-a-Tap)
