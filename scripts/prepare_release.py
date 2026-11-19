#!/usr/bin/env python3
"""Build local release assets; never upload, tag, push, or change a repository."""

import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[1]
TARGET = "aarch64-apple-darwin"
DEPLOYMENT_TARGET = "11.0"
REPOSITORY = "https://github.com/CreevekCZ/flimru"
ARCHIVE_DOCS = (
    "LICENSE", "README.md", "docs/usage.md", "docs/agents.md", "examples/launch.json",
    "docs/screenshots/device-picker.png", "docs/screenshots/live-console.png",
)


def run(*args, cwd=ROOT, env=None):
    return subprocess.check_output(args, cwd=cwd, env=env, text=True).strip()


def archive_bytes(files, epoch):
    """Stable ordering and metadata; archive only explicitly selected files."""
    buffer = io.BytesIO()
    with gzip.GzipFile(fileobj=buffer, mode="wb", filename="", mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as archive:
            for name, source in sorted(files.items()):
                member = tarfile.TarInfo(name)
                data = source.read_bytes()
                member.size = len(data)
                member.mode = 0o755 if name == "flimru" else 0o644
                member.mtime = epoch
                archive.addfile(member, io.BytesIO(data))
    return buffer.getvalue()


def validate_version(version):
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
        raise ValueError("Release version must be a stable major.minor.patch version")


def formula(version, checksum):
    validate_version(version)
    if not re.fullmatch(r"[0-9a-f]{64}", checksum):
        raise ValueError("A real SHA-256 checksum is required")
    template = (ROOT / "packaging/homebrew/flimru.rb.in").read_text()
    return template.replace("@VERSION@", version).replace("@SHA256@", checksum)


def input_digest(root):
    paths = [root / name for name in (
        "Cargo.toml", "Cargo.lock", *ARCHIVE_DOCS, "packaging/release-notes.md",
        "packaging/homebrew/flimru.rb.in", "scripts/prepare_release.py",
    )]
    paths.extend((root / "src").rglob("*.rs"))
    digest = hashlib.sha256()
    for path in sorted(paths):
        digest.update(str(path.relative_to(root)).encode() + b"\0")
        digest.update(hashlib.sha256(path.read_bytes()).digest())
    return digest.hexdigest()


def verify_binary(binary, version, env):
    if run(str(binary), "--version", env=env) != f"flimru {version}":
        raise ValueError("Built binary version does not match Cargo.toml")
    if run("/usr/bin/lipo", "-archs", str(binary)) != "arm64":
        raise ValueError("Release binary is not Apple Silicon arm64")
    linkage = run("/usr/bin/otool", "-L", str(binary))
    for line in linkage.splitlines()[1:]:
        library = line.strip().split(" (", 1)[0]
        if not library.startswith(("/usr/lib/", "/System/Library/")):
            raise ValueError(f"Non-system runtime dependency: {library}")
    build = run("/usr/bin/vtool", "-show-build", str(binary))
    if not re.search(r"\bminos\s+11\.0(?:\.0)?\s", build):
        raise ValueError("Release deployment target is not macOS 11.0")
    subprocess.run(["/usr/bin/codesign", "--verify", "--strict", str(binary)], check=True)
    # Validate an actual launch file without executing Flutter or entering raw mode.
    with tempfile.TemporaryDirectory(prefix="flimru-release-check-") as temporary:
        project = Path(temporary)
        (project / ".vscode").mkdir()
        (project / ".vscode/launch.json").write_text(
            '{ // JSONC is supported\n"configurations": ['
            '{"name":"Release smoke test","type":"dart","request":"launch",'
            '"program":"lib/main.dart",},],}\n'
        )
        flutter = project / "flutter"
        flutter.write_text("#!/bin/sh\nexit 99\n")
        flutter.chmod(0o755)
        output = run(str(binary), str(project), "--check", "--flutter", str(flutter), env=env)
        if "1 launch configurations" not in output or "Release smoke test" not in output:
            raise ValueError("Packaged binary did not load the launch configuration")
    return linkage, build


def prepare(output=None, allow_dirty=False):
    if platform.system() != "Darwin" or platform.machine() != "arm64":
        raise ValueError("This release target must be built and tested on an Apple Silicon Mac")
    selected_xcode = run("/usr/bin/xcode-select", "-p")
    configured_xcode = os.environ.get("DEVELOPER_DIR")
    if configured_xcode and Path(configured_xcode).resolve() != Path(selected_xcode).resolve():
        raise ValueError("DEVELOPER_DIR differs from xcode-select; use the selected Xcode")
    dirty = bool(run("git", "status", "--porcelain", "--untracked-files=all"))
    if dirty and not allow_dirty:
        raise ValueError("Working tree is dirty; commit changes or use --allow-dirty for a local draft")
    metadata = json.loads(run("cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"))
    package = next(p for p in metadata["packages"] if Path(p["manifest_path"]) == ROOT / "Cargo.toml")
    version = package["version"]
    validate_version(version)
    destination = Path(output).resolve() if output else ROOT / "dist" / f"v{version}"
    if destination.exists():
        raise ValueError(f"Refusing to replace existing release assets: {destination}")
    revision = run("git", "rev-parse", "HEAD")
    epoch = int(run("git", "show", "-s", "--format=%ct", "HEAD"))
    source_digest = input_digest(ROOT)
    env = os.environ.copy()
    env.pop("CARGO_ENCODED_RUSTFLAGS", None)
    env.update(DEVELOPER_DIR=selected_xcode, MACOSX_DEPLOYMENT_TARGET=DEPLOYMENT_TARGET,
               RUSTFLAGS="-C target-cpu=generic")
    subprocess.run(["cargo", "build", "--locked", "--release", "--target", TARGET,
                    "--bin", "flimru"], cwd=ROOT, env=env, check=True)
    binary = Path(metadata["target_directory"]) / TARGET / "release/flimru"
    linkage, build = verify_binary(binary, version, env)
    if run("git", "rev-parse", "HEAD") != revision or input_digest(ROOT) != source_digest:
        raise ValueError("Release inputs changed during the build; prepare again from a stable checkout")
    if not allow_dirty and run("git", "status", "--porcelain", "--untracked-files=all"):
        raise ValueError("Working tree changed during the build; prepare again from a clean checkout")
    destination.parent.mkdir(parents=True, exist_ok=True)
    # Promote a complete directory only after build, smoke test and packaging succeed.
    with tempfile.TemporaryDirectory(prefix=".flimru-release-", dir=destination.parent) as temporary:
        stage = Path(temporary) / "assets"
        stage.mkdir()
        archive_name = f"flimru-v{version}-darwin-arm64.tar.gz"
        payload = archive_bytes({"flimru": binary, **{name: ROOT / name for name in ARCHIVE_DOCS}}, epoch)
        checksum = hashlib.sha256(payload).hexdigest()
        (stage / archive_name).write_bytes(payload)
        (stage / "SHA256SUMS").write_text(f"{checksum}  {archive_name}\n")
        (stage / "flimru.rb").write_text(formula(version, checksum))
        (stage / "release-notes.md").write_text((ROOT / "packaging/release-notes.md").read_text())
        (stage / "build-info.json").write_text(json.dumps({
            "version": version, "tag": f"v{version}", "target": TARGET,
            "source_revision": revision, "source_dirty": dirty, "source_inputs_sha256": source_digest,
            "status": "local draft" if dirty else "local candidate; not published",
            "archive": archive_name, "sha256": checksum,
            "download_url": f"{REPOSITORY}/releases/download/v{version}/{archive_name}",
            "macos_deployment_target": DEPLOYMENT_TARGET,
            "tested_macos": platform.mac_ver()[0], "selected_xcode": selected_xcode,
            "rustc": run("rustc", "--version"), "cargo": run("cargo", "--version"),
            "linkage": linkage, "binary_build_version": build,
        }, indent=2) + "\n")
        shutil.move(str(stage), destination)
    print(f"Prepared {'DRAFT ' if dirty else ''}release assets: {destination}")
    print(f"SHA-256: {checksum}\nNothing was uploaded, tagged, committed, or pushed.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, help="New output directory (default: dist/vVERSION)")
    parser.add_argument("--allow-dirty", action="store_true", help="Prepare a local draft from uncommitted work")
    options = parser.parse_args()
    try:
        prepare(options.output, options.allow_dirty)
    except (OSError, ValueError, subprocess.CalledProcessError, StopIteration) as error:
        parser.exit(1, f"Release preparation failed: {error}\n")


if __name__ == "__main__":
    main()
