#!/usr/bin/env python3
"""Pinned dependency preparation checks; synthetic subprocesses are not native proof."""
import hashlib
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('prepare', Path(__file__).with_name('prepare-ghostty.py'))
prepare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prepare)

class DependencyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.archive = self.root / 'dependency.tar.gz'
        self.archive.write_bytes(b'synthetic pinned archive')
        self.metadata = dict(archive=self.archive.name, archive_sha256=hashlib.sha256(self.archive.read_bytes()).hexdigest(), zig_hash='exact-package-hash')
    def tearDown(self):
        self.temp.cleanup()
    def test_archive_tampering_stops_before_zig_and_check_is_read_only(self):
        with patch.object(prepare.subprocess, 'run') as run:
            prepare.prepare_dependency(self.metadata, self.root, self.root, check=True)
            run.assert_not_called()
            self.archive.write_bytes(b'tampered')
            for check in (False, True):
                with self.assertRaisesRegex(ValueError, 'archive integrity'):
                    prepare.prepare_dependency(self.metadata, self.root, self.root, check=check)
                run.assert_not_called()
    def test_exact_zig_hash_required_without_rewriting_upstream_source(self):
        with patch.object(prepare.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, 'exact-package-hash\n')) as run:
            prepare.prepare_dependency(self.metadata, self.root, self.root)
            self.assertEqual(run.call_args.args[0], ['zig', 'fetch', str(self.archive.resolve())])
            self.assertEqual(run.call_args.kwargs['cwd'], self.root)
        with patch.object(prepare.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, 'different-package-hash\n')):
            with self.assertRaisesRegex(ValueError, 'package hash'):
                prepare.prepare_dependency(self.metadata, self.root, self.root)
    def test_zig_fetch_failure_is_preserved(self):
        with patch.object(prepare.subprocess, 'run', side_effect=subprocess.CalledProcessError(1, ['zig','fetch'])):
            with self.assertRaises(subprocess.CalledProcessError):
                prepare.prepare_dependency(self.metadata, self.root, self.root)

if __name__ == '__main__':
    unittest.main()
