#!/usr/bin/env python3
"""Prepare the one pinned, integrity-checked Ghostty source used by every build."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
RESOURCES = ROOT / "resources/ghostty"
CACHES = {".zig-cache", "zig-out", "zig-pkg"}


def source_digest(source):
    digest = hashlib.sha256()
    entries = []
    for directory, dirs, files in os.walk(source, followlinks=False):
        parent = Path(directory)
        if parent == source:
            dirs[:] = [name for name in dirs if name not in CACHES]
        for name in list(dirs):
            path = parent / name
            if path.is_symlink():
                files.append(name)
                dirs.remove(name)
        entries.extend(parent / name for name in files)
    for path in sorted(entries, key=lambda p: p.relative_to(source).as_posix()):
        name = path.relative_to(source).as_posix().encode("utf-8")
        if path.is_symlink():
            kind, content = b"L", os.readlink(path).encode("utf-8")
        else:
            kind, content = b"F", path.read_bytes()
        digest.update(kind + name + b"\0" + hashlib.sha256(content).digest())
    return digest.hexdigest()


def verify_source(manifest, source):
    if source.is_symlink() or not source.is_dir():
        raise ValueError("Ghostty source is not prepared; run python3 scripts/prepare-ghostty.py")
    if source_digest(source) != manifest["source_sha256"]:
        raise ValueError("Ghostty source integrity mismatch; existing source was preserved")


def prepare(manifest, patches, source, archive=None, *, check=False):
    patch = patches / manifest["patch"]
    if hashlib.sha256(patch.read_bytes()).hexdigest() != manifest["patch_sha256"]:
        raise ValueError("Ghostty patch integrity mismatch")
    if source.exists() or source.is_symlink():
        verify_source(manifest, source)
        return
    if check:
        raise ValueError("Ghostty source is not prepared; run python3 scripts/prepare-ghostty.py")
    source.parent.mkdir(parents=True, exist_ok=True)
    # Staging avoids exposing a partial source or overwriting an existing one.
    with tempfile.TemporaryDirectory(prefix="ghostty-prepare-", dir=source.parent) as temporary:
        staging = Path(temporary) / "source"
        staging.mkdir()
        if archive is None:
            archive = Path(temporary) / "source.tar.gz"
            with urllib.request.urlopen(manifest["archive_url"], timeout=45) as response, archive.open("wb") as output:
                size = 0
                while block := response.read(1024 * 1024):
                    size += len(block)
                    if size > 100 * 1024 * 1024:
                        raise ValueError("Ghostty archive exceeds the size limit")
                    output.write(block)
        if hashlib.sha256(archive.read_bytes()).hexdigest() != manifest["archive_sha256"]:
            raise ValueError("Ghostty archive integrity mismatch")
        try:
            with tarfile.open(archive) as bundle:
                for entry in bundle:
                    parts = entry.name.split("/", 1)
                    if len(parts) == 1 or not parts[1]:
                        continue
                    expected_root = "ghostty-" + manifest.get("ghostty_commit", "commit")
                    if parts[0] != expected_root:
                        raise ValueError("Ghostty archive has an unexpected root")
                    entry.name = parts[1]
                    bundle.extract(entry, staging, filter="data")
        except tarfile.TarError as error:
            raise ValueError("Ghostty archive contains an unsafe entry") from error
        env = os.environ.copy()
        env["GIT_CEILING_DIRECTORIES"] = str(staging.parent)
        subprocess.run(["git", "apply", "--check", str(patch.resolve())], cwd=staging, env=env, check=True)
        subprocess.run(["git", "apply", str(patch.resolve())], cwd=staging, env=env, check=True)
        verify_source(manifest, staging)
        if source.exists() or source.is_symlink():
            raise ValueError("Ghostty source appeared during preparation; existing source was preserved")
        staging.rename(source)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Verify prepared source without downloading or changing it")
    parser.add_argument("--test", action="store_true", help="Run the pinned Debug resize and row-metadata regressions after verification")
    parser.add_argument("--archive", type=Path, help="Use an already downloaded pinned source archive")
    args = parser.parse_args()
    manifest = json.loads((RESOURCES / "source.json").read_text())
    try:
        source = ROOT / "target/ghostty-source" / manifest["source_sha256"][:16]
        prepare(manifest, RESOURCES, source, args.archive, check=args.check)
        if args.test:
            subprocess.run(["zig", "build", "test-lib-vt", "-Demit-lib-vt=true", "-Doptimize=Debug", "-Dtest-filter=PageList chda", "-Dtest-filter=PageList eraseRow recycled", "-Dtest-filter=PageList eraseRowBounded recycled", "-Dtest-filter=PageList resize trimmed rows", "-Dtest-filter=PageList resize grow cols with unwrap fixes viewport pin", "--summary", "failures"], cwd=source, check=True)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Ghostty preparation failed: {error}\n")
    print("Pinned Ghostty source and patch verified")


if __name__ == "__main__":
    main()
