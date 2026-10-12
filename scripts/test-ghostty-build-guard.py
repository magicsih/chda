#!/usr/bin/env python3
"""Exercise Cargo's real source-change guard; requires the pinned Zig toolchain."""
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parent.parent


def check():
    return subprocess.run(
        ["cargo", "check", "--locked", "--quiet", "-p", "chda-term"],
        cwd=ROOT, capture_output=True, text=True,
    )


class BuildGuardTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        subprocess.run([sys.executable, str(ROOT / "scripts/prepare-ghostty.py"), "--check"], check=True)
        manifest = json.loads((ROOT / "resources/ghostty/source.json").read_text())
        cls.source = ROOT / ".ghostty-source" / manifest["source_sha256"][:16]

    def setUp(self):
        subprocess.run([sys.executable, str(ROOT / "scripts/prepare-ghostty.py")], check=True)
        result = check()
        self.assertEqual(result.returncode, 0, result.stderr)

    def assert_rejected(self):
        result = check()
        self.assertNotEqual(result.returncode, 0, "Cargo reused a build without validating changed source")
        self.assertIn("Ghostty source integrity mismatch", result.stderr)

    def test_root_file_addition_reruns_integrity_guard(self):
        extra = self.source / f"chda-integrity-fixture-{os.getpid()}.txt"
        with extra.open("xb") as output:
            output.write(b"source root addition regression\n")
        try:
            self.assert_rejected()
        finally:
            extra.unlink()

    def test_nested_file_change_reruns_integrity_guard(self):
        path = self.source / "src/terminal/PageList.zig"
        original = path.read_bytes()
        try:
            path.write_bytes(original + b"\n// chda source change regression fixture\n")
            self.assert_rejected()
        finally:
            path.write_bytes(original)

    def test_restored_source_is_blocked_until_native_cache_is_cleared(self):
        path = self.source / "src/terminal/PageList.zig"
        original = path.read_bytes()
        try:
            path.write_bytes(original + b"\n// unverified native build fixture\n")
            self.assert_rejected()
        finally:
            path.write_bytes(original)
        result = check()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("native cache is unverified", result.stderr)
        subprocess.run([sys.executable, str(ROOT / "scripts/prepare-ghostty.py")], check=True)
        self.assertEqual(check().returncode, 0)

    @classmethod
    def tearDownClass(cls):
        subprocess.run([sys.executable, str(ROOT / "scripts/prepare-ghostty.py")], check=True)
        result = check()
        if result.returncode != 0:
            raise AssertionError(result.stderr)


if __name__ == "__main__":
    unittest.main()
