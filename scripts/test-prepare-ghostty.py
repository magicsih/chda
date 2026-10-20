#!/usr/bin/env python3
"""Source preparation guards use synthetic archives, never native QA evidence."""
import hashlib
import importlib.util
import io
import os
import json
from pathlib import Path
import tarfile
import tempfile
import subprocess
import sys
import unittest
import urllib.error
from unittest import mock

sys.dont_write_bytecode = True

spec = importlib.util.spec_from_file_location("prepare", Path(__file__).with_name("prepare-ghostty.py"))
prepare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prepare)


class PreparationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.archive = self.root / "source.tar.gz"
        with tarfile.open(self.archive, "w:gz") as archive:
            for name, data in [("ghostty-commit/build.zig", b"original\n"), ("ghostty-commit/src/vt.zig", b"terminal\n")]:
                entry = tarfile.TarInfo(name)
                entry.size = len(data)
                archive.addfile(entry, io.BytesIO(data))
        self.patch = self.root / "resize.patch"
        self.patch.write_bytes(b"--- a/build.zig\n+++ b/build.zig\n@@ -1 +1 @@\n-original\n+patched\n")
        expected = self.root / "expected"
        (expected / "src").mkdir(parents=True)
        (expected / "build.zig").write_bytes(b"patched\n")
        (expected / "src/vt.zig").write_bytes(b"terminal\n")
        self.manifest = {
            "archive_sha256": hashlib.sha256(self.archive.read_bytes()).hexdigest(),
            "archive_url": "https://invalid.example/source.tar.gz",
            "patch": "resize.patch",
            "patch_sha256": hashlib.sha256(self.patch.read_bytes()).hexdigest(),
            "source_sha256": prepare.source_digest(expected),
        }
        self.output = self.root / "output"

    def run_prepare(self, **kwargs):
        return prepare.prepare(self.manifest, self.root, self.output, self.archive, **kwargs)

    def test_prepares_verified_source_and_is_idempotent(self):
        self.run_prepare()
        self.assertEqual((self.output / "build.zig").read_bytes(), b"patched\n")
        self.run_prepare(check=True)
        self.run_prepare()

    def test_git_global_autocrlf_does_not_change_prepared_source_bytes(self):
        with mock.patch.dict(os.environ, {
            "GIT_CONFIG_COUNT": "1",
            "GIT_CONFIG_KEY_0": "core.autocrlf",
            "GIT_CONFIG_VALUE_0": "true",
        }):
            self.run_prepare()
        self.assertEqual((self.output / "build.zig").read_bytes(), b"patched\n")

    def test_windows_style_checkout_preserves_the_pinned_patch_bytes(self):
        repository = self.root / "checkout"
        repository.mkdir()
        subprocess.run(["git", "init", "-q", str(repository)], check=True)
        subprocess.run(["git", "config", "core.autocrlf", "true"], cwd=repository, check=True)
        manifest = json.loads((prepare.RESOURCES / "source.json").read_text())
        relative = Path("resources/ghostty") / manifest["patch"]
        patch = repository / relative
        patch.parent.mkdir(parents=True)
        original = (prepare.ROOT / relative).read_bytes()
        patch.write_bytes(original)
        attributes = prepare.ROOT / ".gitattributes"
        if attributes.exists():
            (repository / ".gitattributes").write_bytes(attributes.read_bytes())
        subprocess.run(["git", "add", "."], cwd=repository, check=True, capture_output=True)
        output = self.root / "windows-checkout"
        subprocess.run(["git", "checkout-index", "--all", f"--prefix={output}/"], cwd=repository, check=True)
        self.assertEqual((output / relative).read_bytes(), original,
                         "Git CRLF checkout must not change the integrity-checked patch")

    def test_restored_source_clears_native_cache_only_after_verification(self):
        self.run_prepare()
        marker = self.output.with_suffix(".unverified")
        marker.write_text("unverified build attempt")
        with self.assertRaisesRegex(ValueError, "native cache"):
            self.run_prepare(check=True)
        with mock.patch.object(prepare.subprocess, "run") as clean:
            self.run_prepare()
        clean.assert_called_once_with(["cargo", "clean", "-p", "libghostty-vt-sys"], cwd=prepare.ROOT, check=True)
        self.assertFalse(marker.exists())

    def test_mismatched_source_does_not_clear_native_cache_or_rebuild_marker(self):
        self.run_prepare()
        marker = self.output.with_suffix(".unverified")
        marker.write_text("unverified build attempt")
        (self.output / "build.zig").write_text("changed")
        with mock.patch.object(prepare.subprocess, "run") as clean:
            with self.assertRaisesRegex(ValueError, "source integrity"):
                self.run_prepare()
        clean.assert_not_called()
        self.assertTrue(marker.exists())

    def test_modified_archive_is_rejected_without_source(self):
        self.archive.write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "archive"):
            self.run_prepare()
        self.assertFalse(self.output.exists())

    def test_modified_patch_is_rejected_even_when_source_exists(self):
        self.run_prepare()
        self.patch.write_text("changed")
        with self.assertRaisesRegex(ValueError, "patch"):
            self.run_prepare()

    def test_modified_source_is_rejected_without_overwriting_it(self):
        self.run_prepare()
        (self.output / "src/vt.zig").write_bytes(b"user modification")
        with self.assertRaisesRegex(ValueError, "source"):
            self.run_prepare()
        self.assertEqual((self.output / "src/vt.zig").read_bytes(), b"user modification")

    def test_missing_source_check_does_not_download(self):
        with self.assertRaisesRegex(ValueError, "not prepared"):
            self.run_prepare(check=True)
        self.assertFalse(self.output.exists())

    def test_archive_traversal_is_rejected(self):
        with tarfile.open(self.archive, "w:gz") as archive:
            entry = tarfile.TarInfo("ghostty-commit/../../escaped")
            entry.size = 1
            archive.addfile(entry, io.BytesIO(b"x"))
        self.manifest["archive_sha256"] = hashlib.sha256(self.archive.read_bytes()).hexdigest()
        with self.assertRaises(ValueError):
            self.run_prepare()
        self.assertFalse((self.root / "escaped").exists())
        self.assertFalse(self.output.exists())

    def test_unexpected_file_is_rejected_but_build_caches_are_ignored(self):
        self.run_prepare()
        (self.output / ".zig-cache").mkdir()
        (self.output / ".zig-cache/temporary").write_text("build cache")
        self.run_prepare(check=True)
        (self.output / "unexpected.zig").write_text("changed")
        with self.assertRaisesRegex(ValueError, "source"):
            self.run_prepare(check=True)

    def make_ghostty_repository(self, files):
        repository = self.root / "ghostty-repository"
        return repository, self.make_repository(repository, files)

    def make_repository(self, repository, files):
        repository.mkdir(parents=True)
        for name, data in files.items():
            (repository / name).parent.mkdir(parents=True, exist_ok=True)
            (repository / name).write_bytes(data)
        subprocess.run(["git", "init", "-q", str(repository)], check=True)
        git = ["git", "-c", "user.name=chda", "-c", "user.email=chda@example.invalid", "-c", "commit.gpgsign=false"]
        subprocess.run([*git, "add", "."], cwd=repository, check=True)
        subprocess.run([*git, "commit", "-q", "-m", "source"], cwd=repository, check=True)
        subprocess.run(["git", "config", "uploadpack.allowAnySHA1InWant", "true"], cwd=repository, check=True)
        return subprocess.run(["git", "rev-parse", "HEAD"], cwd=repository, capture_output=True, text=True, check=True).stdout.strip()

    def refused_download(self):
        refused = urllib.error.HTTPError(self.manifest["archive_url"], 403, "Forbidden", None, None)
        return mock.patch("urllib.request.urlopen", side_effect=refused)

    def test_refused_archive_download_fetches_the_pinned_commit_with_git(self):
        repository, commit = self.make_ghostty_repository({"build.zig": b"original\n", "src/vt.zig": b"terminal\n"})
        self.manifest["ghostty_commit"] = commit
        with self.refused_download(), mock.patch.object(prepare, "GHOSTTY_GIT_URL", repository.as_uri()):
            prepare.prepare(self.manifest, self.root, self.output)
        self.assertEqual((self.output / "build.zig").read_bytes(), b"patched\n")
        self.assertEqual((self.output / "src/vt.zig").read_bytes(), b"terminal\n")

    def test_git_fetch_ignores_a_windows_autocrlf_config(self):
        repository, commit = self.make_ghostty_repository({"build.zig": b"original\n", "src/vt.zig": b"terminal\n"})
        self.manifest["ghostty_commit"] = commit
        autocrlf = {"GIT_CONFIG_COUNT": "1", "GIT_CONFIG_KEY_0": "core.autocrlf", "GIT_CONFIG_VALUE_0": "true"}
        with self.refused_download(), mock.patch.object(prepare, "GHOSTTY_GIT_URL", repository.as_uri()), mock.patch.dict(os.environ, autocrlf):
            prepare.prepare(self.manifest, self.root, self.output)
        self.assertEqual((self.output / "build.zig").read_bytes(), b"patched\n")

    def test_git_fetch_with_a_different_tree_is_rejected(self):
        repository, commit = self.make_ghostty_repository({"build.zig": b"original\n", "src/vt.zig": b"changed\n"})
        self.manifest["ghostty_commit"] = commit
        with self.refused_download(), mock.patch.object(prepare, "GHOSTTY_GIT_URL", repository.as_uri()):
            with self.assertRaisesRegex(ValueError, "source integrity"):
                prepare.prepare(self.manifest, self.root, self.output)
        self.assertFalse(self.output.exists())

    def test_downloaded_archive_is_preferred_over_git(self):
        with mock.patch("urllib.request.urlopen", return_value=io.BytesIO(self.archive.read_bytes())), \
                mock.patch.object(prepare, "fetch_git_archive") as fetch:
            prepare.prepare(self.manifest, self.root, self.output)
        fetch.assert_not_called()
        self.assertEqual((self.output / "build.zig").read_bytes(), b"patched\n")

    def test_downloaded_archive_with_wrong_digest_is_rejected_without_git(self):
        self.manifest["archive_sha256"] = "0" * 64
        with mock.patch("urllib.request.urlopen", return_value=io.BytesIO(self.archive.read_bytes())), \
                mock.patch.object(prepare, "fetch_git_archive") as fetch:
            with self.assertRaisesRegex(ValueError, "archive integrity"):
                prepare.prepare(self.manifest, self.root, self.output)
        fetch.assert_not_called()
        self.assertFalse(self.output.exists())

    def test_refused_download_and_failed_git_fetch_report_both(self):
        self.manifest["ghostty_commit"] = "0" * 40
        with self.refused_download(), mock.patch.object(prepare, "GHOSTTY_GIT_URL", (self.root / "missing.git").as_uri()):
            with self.assertRaisesRegex(ValueError, "download failed.*git fetch failed"):
                prepare.prepare(self.manifest, self.root, self.output)
        self.assertFalse(self.output.exists())

    def write_zig_source(self, url, digest):
        source = self.root / "zig-source"
        source.mkdir()
        (source / "build.zig.zon").write_text(
            ".{\n"
            "    .dependencies = .{\n"
            "        .lib = .{\n"
            f'            .url = "{url}",\n'
            f'            .hash = "{digest}",\n'
            "        },\n"
            "        .other = .{\n"
            '            .url = "https://deps.files.ghostty.org/other.tar.gz",\n'
            '            .hash = "other-0.0.0-AAAA",\n'
            "        },\n"
            "    },\n"
            "}\n")
        return source

    def fake_zig(self, printed_hash):
        # Stands in for zig fetch: it prints the package hash and stores the package in zig-pkg.
        bin_dir = self.root / "bin"
        bin_dir.mkdir(exist_ok=True)
        zig = bin_dir / "zig"
        zig.write_text(f"#!/bin/sh\nmkdir -p zig-pkg/{printed_hash}\necho {printed_hash}\n")
        zig.chmod(0o755)
        return mock.patch.dict(os.environ, {"PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}"})

    def test_zig_github_archive_is_fetched_from_its_pinned_commit(self):
        github = self.root / "github"
        commit = self.make_repository(github / "octo" / "lib.git", {"build.zig": b"lib\n"})
        source = self.write_zig_source(f"https://github.com/octo/lib/archive/{commit}.tar.gz", "lib-0.0.0-fake")
        with self.fake_zig("lib-0.0.0-fake"), mock.patch.object(prepare, "GITHUB_GIT_BASE", github.as_uri() + "/"):
            self.assertEqual(prepare.zig_github_packages(source), [("octo", "lib", commit, "lib-0.0.0-fake")])
            prepare.fetch_zig_github_packages(source)
        self.assertTrue((source / "zig-pkg/lib-0.0.0-fake").is_dir())

    def test_cached_zig_package_is_not_fetched_again(self):
        source = self.write_zig_source(f"https://github.com/octo/lib/archive/{'a' * 40}.tar.gz", "lib-0.0.0-fake")
        (source / "zig-pkg/lib-0.0.0-fake").mkdir(parents=True)
        with mock.patch.object(prepare, "git_archive") as archive:
            prepare.fetch_zig_github_packages(source)
        archive.assert_not_called()

    def test_zig_package_hash_mismatch_is_rejected(self):
        github = self.root / "github"
        commit = self.make_repository(github / "octo" / "lib.git", {"build.zig": b"lib\n"})
        source = self.write_zig_source(f"https://github.com/octo/lib/archive/{commit}.tar.gz", "lib-0.0.0-expected")
        with self.fake_zig("lib-0.0.0-fake"), mock.patch.object(prepare, "GITHUB_GIT_BASE", github.as_uri() + "/"):
            with self.assertRaisesRegex(ValueError, "hash mismatch"):
                prepare.fetch_zig_github_packages(source)

    def test_zig_manifest_entry_without_a_hash_is_rejected(self):
        source = self.write_zig_source(f"https://github.com/octo/lib/archive/{'a' * 40}.tar.gz", "lib-0.0.0-fake")
        (source / "build.zig.zon").write_text('.{ .dependencies = .{ .lib = .{ .url = "https://github.com/octo/lib/archive/x.tar.gz" } } }\n')
        with self.assertRaisesRegex(ValueError, "Unsupported package entry"):
            prepare.zig_github_packages(source)


if __name__ == "__main__":
    unittest.main()
