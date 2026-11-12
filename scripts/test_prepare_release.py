"""Packaging boundary checks; no compiler, upload, or real repository mutation."""

import hashlib
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import prepare_release as release


class ReleaseTests(unittest.TestCase):
    def test_archive_is_stable_and_contains_only_selected_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "flimru"
            binary.write_bytes(b"test executable")
            license_file = root / "LICENSE"
            license_file.write_text("MIT\n")
            (root / "secret.env").write_text("never package this")
            files = {"flimru": binary, "LICENSE": license_file}
            first = release.archive_bytes(files, 12345)
            binary.touch()
            second = release.archive_bytes(dict(reversed(list(files.items()))), 12345)
            self.assertEqual(hashlib.sha256(first).hexdigest(), hashlib.sha256(second).hexdigest())
            with tarfile.open(fileobj=io.BytesIO(first), mode="r:gz") as archive:
                self.assertEqual(archive.getnames(), ["LICENSE", "flimru"])
                self.assertEqual(archive.getmember("flimru").mode, 0o755)
                self.assertEqual(archive.getmember("LICENSE").mode, 0o644)
                self.assertEqual(archive.getmember("flimru").uid, 0)
                self.assertEqual(archive.getmember("flimru").mtime, 12345)
                self.assertEqual(archive.extractfile("flimru").read(), b"test executable")

    def test_formula_binds_version_and_actual_archive_checksum(self):
        checksum = hashlib.sha256(b"release archive").hexdigest()
        formula = release.formula("0.1.0", checksum)
        self.assertIn(f'sha256 "{checksum}"', formula)
        self.assertIn('/v0.1.0/flimru-v0.1.0-darwin-arm64.tar.gz"', formula)
        self.assertNotIn("@VERSION@", formula)
        self.assertNotIn("@SHA256@", formula)
        self.assertIn("depends_on arch: :arm64", formula)

    def test_invalid_versions_and_checksums_cannot_enter_ruby(self):
        for version, checksum in [("../0.1.0", "a" * 64), ('0.1.0"\n', "a" * 64),
                                  ("0.1.0", "PLACEHOLDER"), ("0.1.0", "g" * 64)]:
            with self.subTest(version=version, checksum=checksum):
                with self.assertRaises(ValueError):
                    release.formula(version, checksum)

    def test_release_input_digest_detects_code_and_documentation_changes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            names = ["Cargo.toml", "Cargo.lock", "LICENSE", "README.md", "docs/usage.md",
                     "packaging/release-notes.md", "packaging/homebrew/flimru.rb.in", "src/main.rs"]
            for name in names:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(name)
            initial = release.input_digest(root)
            (root / "src/main.rs").write_text("changed source")
            self.assertNotEqual(initial, release.input_digest(root))
            source_changed = release.input_digest(root)
            (root / "README.md").write_text("changed documentation")
            self.assertNotEqual(source_changed, release.input_digest(root))

    @patch.dict("os.environ", {}, clear=True)
    @patch.object(release.platform, "machine", return_value="arm64")
    @patch.object(release.platform, "system", return_value="Darwin")
    @patch.object(release, "run")
    @patch.object(release.subprocess, "run")
    def test_existing_release_assets_cannot_be_overwritten(self, build, command, _system, _machine):
        command.side_effect = ["/selected/Xcode.app/Contents/Developer", "", json.dumps({
            "packages": [{"manifest_path": str(release.ROOT / "Cargo.toml"), "version": "0.1.0"}],
        })]
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaisesRegex(ValueError, "Refusing to replace"):
                release.prepare(Path(temporary))
        build.assert_not_called()

    @patch.dict("os.environ", {}, clear=True)
    @patch.object(release.platform, "machine", return_value="arm64")
    @patch.object(release.platform, "system", return_value="Darwin")
    @patch.object(release, "run", side_effect=["/Applications/Xcode.app/Contents/Developer", " M README.md"])
    @patch.object(release.subprocess, "run")
    def test_dirty_checkout_is_refused_before_build(self, build, _run, _system, _machine):
        with self.assertRaisesRegex(ValueError, "Working tree is dirty"):
            release.prepare()
        build.assert_not_called()

    @patch.dict("os.environ", {"DEVELOPER_DIR": "/other/Xcode.app/Contents/Developer"})
    @patch.object(release.platform, "machine", return_value="arm64")
    @patch.object(release.platform, "system", return_value="Darwin")
    @patch.object(release, "run", return_value="/Applications/Xcode.app/Contents/Developer")
    @patch.object(release.subprocess, "run")
    def test_another_xcode_override_is_refused(self, build, _run, _system, _machine):
        with self.assertRaisesRegex(ValueError, "DEVELOPER_DIR differs"):
            release.prepare()
        build.assert_not_called()

    @patch.object(release.platform, "system", return_value="Linux")
    @patch.object(release.subprocess, "run")
    def test_unsupported_build_host_cannot_claim_a_macos_release(self, build, _system):
        with self.assertRaisesRegex(ValueError, "Apple Silicon Mac"):
            release.prepare()
        build.assert_not_called()


if __name__ == "__main__":
    unittest.main()
