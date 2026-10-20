#!/usr/bin/env python3
"""Prepare the one pinned, integrity-checked Ghostty source used by every build."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
RESOURCES = ROOT / "resources/ghostty"
CACHES = {".zig-cache", "zig-out", "zig-pkg"}
GHOSTTY_GIT_URL = "https://github.com/ghostty-org/ghostty.git"
GITHUB_GIT_BASE = "https://github.com/"
GIT_TIMEOUT_SECONDS = 300
ZON_URL_HASH = re.compile(r'\.url = "([^"]+)",\s*\.hash = "([^"]+)"')
GITHUB_ARCHIVE = re.compile(r"https://github\.com/([\w.-]+)/([\w.-]+)/archive/([0-9a-f]{40})\.tar\.gz")


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
        raise ValueError(f"Ghostty source integrity mismatch; existing source was preserved. Move {source} aside, then rerun preparation")


def download_archive(manifest, destination):
    with urllib.request.urlopen(manifest["archive_url"], timeout=45) as response, destination.open("wb") as output:
        size = 0
        while block := response.read(1024 * 1024):
            size += len(block)
            if size > 100 * 1024 * 1024:
                raise ValueError("Ghostty archive exceeds the size limit")
            output.write(block)
    if hashlib.sha256(destination.read_bytes()).hexdigest() != manifest["archive_sha256"]:
        raise ValueError("Ghostty archive integrity mismatch")


def run_command(arguments, *, cwd=None, env=None):
    try:
        result = subprocess.run(arguments, cwd=cwd, env=env, capture_output=True, text=True, timeout=GIT_TIMEOUT_SECONDS)
    except subprocess.TimeoutExpired as error:
        raise ValueError(f"{arguments[0]} timed out") from error
    if result.returncode != 0:
        raise ValueError((result.stderr.strip().splitlines() or ["no error output"])[-1])
    return result.stdout.strip()


def git_archive(url, commit, prefix, repository, destination):
    # Git fetches the pinned commit by object ID and checks that it resolves to it.
    # Its tar is not codeload's gzip, so archive_sha256 does not apply; the caller
    # still gates each extracted tree with its pinned digest.
    git = ["git", "--git-dir", str(repository), "-c", "core.autocrlf=false", "-c", "core.eol=lf"]
    env = {**os.environ, "GIT_TERMINAL_PROMPT": "0"}
    run_command(["git", "init", "--bare", "--quiet", str(repository)], env=env)
    run_command([*git, "fetch", "--depth", "1", "--no-tags", url, commit], env=env)
    if run_command([*git, "rev-parse", "--verify", f"{commit}^{{commit}}"], env=env) != commit:
        raise ValueError(f"git resolved a different commit from {url}")
    run_command([*git, "archive", "--format=tar", f"--prefix={prefix}/", "-o", str(destination), commit], env=env)
    return destination


def fetch_git_archive(manifest, workdir):
    commit = manifest["ghostty_commit"]
    return git_archive(GHOSTTY_GIT_URL, commit, f"ghostty-{commit}", workdir / "ghostty.git", workdir / "ghostty-git.tar")


def zig_github_packages(source):
    # Packages that zig build would download from a GitHub archive. The manifests are
    # the verified root build.zig.zon and the pinned translate-c package's manifest.
    packages = []
    for manifest in [source / "build.zig.zon", *sorted((source / "zig-pkg").glob("*/build.zig.zon"))]:
        text = manifest.read_text()
        entries = ZON_URL_HASH.findall(text)
        if len(entries) != text.count(".url ="):
            raise ValueError(f"Unsupported package entry in {manifest.parent.name}")
        for url, digest in entries:
            archive = GITHUB_ARCHIVE.fullmatch(url)
            if archive:
                packages.append((*archive.groups(), digest))
    return packages


def fetch_zig_github_packages(source):
    # Some sessions refuse codeload.github.com, so git fetches each pinned commit instead.
    # zig fetch then checks the package hash from the manifest, as prepare_dependency does.
    with tempfile.TemporaryDirectory(prefix="ghostty-packages-", dir=source.parent) as temporary:
        workdir = Path(temporary)
        for owner, repo, commit, digest in zig_github_packages(source):
            if (source / "zig-pkg" / digest).is_dir():
                continue
            name = f"{repo}-{commit}"
            archive = git_archive(f"{GITHUB_GIT_BASE}{owner}/{repo}.git", commit, name, workdir / f"{name}.git", workdir / f"{name}.tar")
            if run_command(["zig", "fetch", str(archive)], cwd=source) != digest:
                raise ValueError(f"Pinned Zig package hash mismatch for {repo}")


def prepare(manifest, patches, source, archive=None, *, check=False):
    patch = patches / manifest["patch"]
    if hashlib.sha256(patch.read_bytes()).hexdigest() != manifest["patch_sha256"]:
        raise ValueError("Ghostty patch integrity mismatch")
    if source.exists() or source.is_symlink():
        verify_source(manifest, source)
        marker = source.with_suffix(".unverified")
        if marker.exists():
            if check:
                raise ValueError("Ghostty native cache is unverified; run python3 scripts/prepare-ghostty.py before Cargo")
            # A failed Cargo guard can race the dependency's native build.
            # Exact source restoration alone must not make that artifact fresh.
            subprocess.run(["cargo", "clean", "-p", "libghostty-vt-sys"], cwd=ROOT, check=True)
            marker.unlink()
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
            try:
                download_archive(manifest, archive)
            except OSError as error:
                # codeload.github.com refuses some sessions even for public repositories.
                # Git fetches the same pinned commit; the tree digest below still gates it.
                try:
                    archive = fetch_git_archive(manifest, Path(temporary))
                except (OSError, ValueError) as git_error:
                    raise ValueError(f"Ghostty archive download failed ({error}); git fetch failed ({git_error})") from git_error
        elif hashlib.sha256(archive.read_bytes()).hexdigest() != manifest["archive_sha256"]:
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
        subprocess.run(["git", "-c", "core.autocrlf=false", "apply", "--check", str(patch.resolve())], cwd=staging, env=env, check=True)
        subprocess.run(["git", "-c", "core.autocrlf=false", "apply", str(patch.resolve())], cwd=staging, env=env, check=True)
        verify_source(manifest, staging)
        if source.exists() or source.is_symlink():
            raise ValueError("Ghostty source appeared during preparation; existing source was preserved")
        staging.rename(source)
    prepare(manifest, patches, source, check=False)


def prepare_dependency(metadata, resources, source, *, check=False):
    archive = resources / metadata['archive']
    if hashlib.sha256(archive.read_bytes()).hexdigest() != metadata['archive_sha256']:
        raise ValueError('Pinned translate-c archive integrity mismatch')
    # Keep the upstream .zon URL/hash and source digest unchanged. Zig checks
    # the package's canonical content hash before populating its own cache.
    if not check:
        result = subprocess.run(['zig', 'fetch', str(archive.resolve())], cwd=source,
                                check=True, capture_output=True, text=True)
        if result.stdout.strip() != metadata['zig_hash']:
            raise ValueError('Pinned translate-c Zig package hash mismatch')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Verify prepared source without downloading or changing it")
    parser.add_argument("--test", action="store_true", help="Run the pinned Debug terminal integrity regressions after verification")
    parser.add_argument("--archive", type=Path, help="Use an already downloaded pinned source archive")
    args = parser.parse_args()
    manifest = json.loads((RESOURCES / "source.json").read_text())
    try:
        source = ROOT / ".ghostty-source" / manifest["source_sha256"][:16]
        prepare(manifest, RESOURCES, source, args.archive, check=args.check)
        dependency = json.loads((RESOURCES / 'translate-c.json').read_text())
        prepare_dependency(dependency, RESOURCES, source, check=args.check)
        if not args.check:
            fetch_zig_github_packages(source)
        if args.test:
            subprocess.run(["zig", "build", "test-lib-vt", "-Demit-lib-vt=true", "-Doptimize=Debug", "-Dtest-filter=PageList chda", "-Dtest-filter=chda temporary image", "-Dtest-filter=PageList eraseRow recycled", "-Dtest-filter=PageList eraseRowBounded recycled", "-Dtest-filter=PageList resize trimmed rows", "-Dtest-filter=PageList resize grow cols with unwrap fixes viewport pin", "--summary", "failures"], cwd=source, check=True)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Ghostty preparation failed: {error}\n")
    print("Pinned Ghostty source and patch verified")


if __name__ == "__main__":
    main()
