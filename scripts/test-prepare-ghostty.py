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


if __name__ == "__main__":
    unittest.main()
