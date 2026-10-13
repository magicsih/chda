#!/usr/bin/env python3
"""Check that the release bundle cannot inherit its parent's application tag."""
from pathlib import Path
import os
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parent.parent
workflow = Path(sys.argv[1]) if len(sys.argv) > 1 else root / '.github/workflows/release.yml'
lines = [line.strip() for line in workflow.read_text().splitlines()
         if 'scripts/bundle-macos.sh ' in line]
assert len(lines) == 1, 'Expected one release bundle invocation'
with tempfile.TemporaryDirectory(prefix='chda-engine-tag-') as directory:
    workspace = Path(directory)
    source = workspace / '.ghostty-source' / 'verified'
    source.mkdir(parents=True)
    subprocess.run(['git', 'init', '-q', directory], check=True)
    subprocess.run(['git', '-C', directory, '-c', 'user.name=release-fixture',
                    '-c', 'user.email=release-fixture@example.invalid', 'commit',
                    '--allow-empty', '-qm', 'chore: synthetic application tag'], check=True)
    subprocess.run(['git', '-C', directory, 'tag', 'v0.1.23'], check=True)
    inherited = subprocess.run(['git', '-C', str(source), 'describe', '--exact-match', '--tags'],
                               check=True, capture_output=True, text=True)
    assert inherited.stdout.strip() == 'v0.1.23'
    scripts = workspace / 'scripts'
    scripts.mkdir()
    bundle = scripts / 'bundle-macos.sh'
    bundle.write_text('#!/bin/sh\nif git -C "$GITHUB_WORKSPACE/.ghostty-source/verified" '
                      'rev-parse --show-toplevel >/dev/null 2>&1; then exit 1; fi\n')
    bundle.chmod(0o700)
    result = subprocess.run(['bash', '-c', lines[0]], cwd=workspace,
                            env={'PATH': os.environ['PATH'], 'GITHUB_WORKSPACE': directory,
                                 'RELEASE_TAG': 'v0.1.23'})
    assert result.returncode == 0, 'Release bundle inherited the chda repository tag'
print('Release engine Git boundary passed')
