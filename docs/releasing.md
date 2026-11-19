# Release Flimru

Flimru's source and releases are hosted at `CreevekCZ/flimru`. The Homebrew tap is
`CreevekCZ/homebrew-tap`, with its local checkout at `~/Developer/homebrew-tap`.
The tap keeps formulas at its root.

## Build and validate

On an Apple Silicon Mac with Rust, Python 3.9+, and the Xcode selected by
`xcode-select -p`, run:

```sh
make setup-tools
make check
make release
```

Build from a clean commit on `main`. If `dist/v0.1.0/` already exists, use
`python3 scripts/prepare_release.py --output "dist/v0.1.0-$(git rev-parse --short HEAD)"`.
The builder checks the binary's ARM64 architecture, system library linkage,
ad-hoc signature, and JSONC parsing. It records the source commit and tested
macOS version in `build-info.json`. The deployment target is macOS 11.0.
Developer ID signing, notarization, and runtime testing on older macOS versions
are separate from this build.

Check the archive with `shasum -a 256 -c SHA256SUMS` in the output directory.
It contains the CLI, MIT license, README with screenshots, guides, and an example launch file.

## GitHub draft

Push the exact build commit to `main`.
From the output directory, create the draft targeting the `source_revision`
recorded in `build-info.json`.

```sh
gh release create v0.1.0 flimru-v0.1.0-darwin-arm64.tar.gz SHA256SUMS \
  --repo CreevekCZ/flimru --draft --target COMMIT_SHA \
  --title 'Flimru v0.1.0' --notes-file release-notes.md
```

To refresh an existing draft, use `gh release edit v0.1.0 --draft --target COMMIT_SHA
--notes-file release-notes.md`, then `gh release upload v0.1.0
flimru-v0.1.0-darwin-arm64.tar.gz SHA256SUMS --clobber`. Verify the draft's target
and attached SHA-256. Build a new version to change a published archive.

## Homebrew

Copy the generated `flimru.rb` to the root of the local tap checkout. To test
before public distribution, use a temporary copy of the formula in a local
test tap. Use a `file://` URL for the archive, set `version "0.1.0"`, and
keep the generated checksum. The explicit version lets Homebrew resolve the
local filename correctly. Run `brew style --formula`, `brew install --skip-link`,
and `brew test --force` on the test formula. Verify `flimru --version` from its keg.

The formula's GitHub URL needs an anonymously downloadable asset for a public
install. Keep the formula local until an approved public binary URL exists.
If the binary is hosted separately from the private source, regenerate the
formula with that release URL. Verify the public download and its checksum,
then commit and push the tap formula and test `brew install CreevekCZ/tap/flimru`.
