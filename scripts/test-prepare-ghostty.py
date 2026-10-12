#!/usr/bin/env python3
"""Source preparation guards use synthetic archives, never native QA evidence."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import sys
import unittest

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
        self.patch.write_text("--- a/build.zig\n+++ b/build.zig\n@@ -1 +1 @@\n-original\n+patched\n")
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
