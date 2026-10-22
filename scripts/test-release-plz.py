#!/usr/bin/env python3
"""Check the release configuration with a real, offline new-member release.

Pass an existing release-plz executable with --binary. The application repository
is read only; the generated Cargo manifests and Git tags live in a private fixture.
Both successful and failed fixture logs are retained for inspection.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib


REPO = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--config', type=Path, default=REPO / 'release-plz.toml')
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    config_bytes = args.config.read_bytes()
    config = tomllib.loads(config_bytes.decode())
    root = Path(tempfile.mkdtemp(prefix='chda-release-new-member-'))
    root.chmod(0o700)
    home = root / 'home'
    home.mkdir()
    project = root / 'project'
    project.mkdir()
    cargo = shutil.which('cargo')
    if not cargo:
        raise RuntimeError('cargo is required')
    env = dict(
        PATH=str(Path(cargo).parent) + os.pathsep + os.defpath,
        HOME=str(home),
        CARGO_HOME=os.environ.get('CARGO_HOME', str(Path.home() / '.cargo')),
        RUSTUP_HOME=os.environ.get('RUSTUP_HOME', str(Path.home() / '.rustup')),
        CARGO_NET_OFFLINE='true',
        GIT_CONFIG_GLOBAL=os.devnull,
        GIT_CONFIG_NOSYSTEM='1',
        GIT_TERMINAL_PROMPT='0',
    )
    log = (root / 'commands.log').open('w')

    def run(*command):
        return subprocess.run([str(c) for c in command], cwd=project, env=env,
                              stdout=log, stderr=log, check=True)

    def workspace(members):
        (project / 'Cargo.toml').write_text(
            '[workspace]\nresolver = "2"\nmembers = ' + json.dumps(members) + '\n'
            '[workspace.package]\nversion = "1.0.0"\nedition = "2024"\npublish = false\n')

    def package(name):
        directory = project / name
        (directory / 'src').mkdir(parents=True)
        (directory / 'Cargo.toml').write_text(
            '[package]\nname = ' + json.dumps(name) + '\n'
            'version.workspace = true\nedition.workspace = true\npublish.workspace = true\n')
        (directory / 'src/lib.rs').write_text('pub fn ready() -> bool { true }\n')

    members = ['chda', 'chda-core', 'chda-session']
    try:
        shutil.copyfile(REPO / 'rust-toolchain.toml', project / 'rust-toolchain.toml')
        workspace(members[:2])
        for name in members[:2]:
            package(name)
        run('git', 'init', '-q', '-b', 'main')
        run('git', 'config', 'user.name', 'Release fixture')
        run('git', 'config', 'user.email', 'fixture@example.invalid')
        run('cargo', 'generate-lockfile', '--offline')
        run('git', 'add', '.')
        run('git', 'commit', '-qm', 'feat: add the initial workspace')
        run('git', 'tag', 'v1.0.0')

        workspace(members)
        package('chda-session')
        # Use the real workspace/package release settings, restricted to the
        # fixture's members. JSON scalars and arrays are valid TOML values.
        lines = ['[workspace]']
        lines += [f'{key} = {json.dumps(value)}' for key, value in config['workspace'].items()]
        for entry in config['package']:
            if entry['name'] not in members:
                continue
            entry = dict(entry)
            if 'changelog_include' in entry:
                entry['changelog_include'] = [n for n in entry['changelog_include'] if n in members]
            lines += ['', '[[package]]']
            lines += [f'{key} = {json.dumps(value)}' for key, value in entry.items()]
        (project / 'release-plz.toml').write_text('\n'.join(lines) + '\n')
        run('cargo', 'generate-lockfile', '--offline')
        run('git', 'add', '.')
        run('git', 'commit', '-qm', 'feat(session): add a new workspace member')
        run(binary, 'update', '--no-changelog', '--repo-url', 'https://github.com/magicsih/chda')
        manifest = tomllib.loads((project / 'Cargo.toml').read_text())
        assert manifest['workspace']['package']['version'] == '1.1.0', manifest
        for name in members:
            manifest = tomllib.loads((project / name / 'Cargo.toml').read_text())
            assert manifest['package']['version'] == {'workspace': True}, manifest
        run('git', 'add', '.')
        run('git', 'commit', '-qm', 'chore: release v1.1.0')
        for entry in config['package']:
            if entry['name'] not in members:
                continue
            if not entry.get('git_tag_enable', config['workspace'].get('git_tag_enable', True)):
                continue
            pattern = entry.get('git_tag_name', config['workspace'].get(
                'git_tag_name', '{{ package }}-v{{ version }}'))
            tag = pattern.replace('{{ version }}', '1.1.0').replace('{{ package }}', entry['name'])
            run('git', 'tag', tag)
        (project / 'chda-core/src/lib.rs').write_text('pub fn ready() -> bool { false }\n')
        run('git', 'add', '.')
        run('git', 'commit', '-qm', 'fix(core): correct the reported readiness')
        run(binary, 'update', '--no-changelog', '--repo-url', 'https://github.com/magicsih/chda')
        manifest = tomllib.loads((project / 'Cargo.toml').read_text())
        assert manifest['workspace']['package']['version'] == '1.1.1', manifest
        result = 'passed'
    except Exception:
        result = 'failed'
        raise
    finally:
        log.close()
        (root / 'result.json').write_text(json.dumps(dict(
            result=result,
            config_sha256=hashlib.sha256(config_bytes).hexdigest(),
            binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
            fixture=str(project),
        ), indent=2))
        print(f'New-member release {result}: {root}', flush=True)


if __name__ == '__main__':
    main()
