#!/usr/bin/env python3
"""Exercise signed Sparkle installs and real GUI adoption in disposable bundles.

macOS only. Requires Python cryptography, an existing Developer ID identity,
Xcode command-line tools and a built chda binary. No production key or installed
app is used. Test applications and their shells are terminated after each case.
"""
import argparse
import base64
import functools
import http.server
import json
import os
from pathlib import Path
import plistlib
import pty
import select
import shutil
import signal
import socket
import struct
import subprocess
import tempfile
import termios
import threading
import time
import xml.etree.ElementTree as ET

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

REPO = Path(__file__).resolve().parent.parent


def command(*args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, **kwargs)


def write_plist(app, **values):
    path = app / 'Contents/Info.plist'
    info = plistlib.loads(path.read_bytes())
    info.update(values)
    path.write_bytes(plistlib.dumps(info))


class Fixture:
    def __init__(self, binary, identity):
        self.root = Path(tempfile.mkdtemp(prefix='chda-up-', dir='/tmp'))
        self.identity = identity
        self.original = self.root / 'Original.app'
        self.target = self.root / 'installed/chda.app'
        self.backup = self.root / 'Recovery.app'
        self.feed = self.root / 'feed'
        self.feed.mkdir()
        self.results = []
        self.logs = (self.root / 'commands.log').open('w')
        key = Ed25519PrivateKey.generate()
        seed = self.root / 'seed'
        seed.write_bytes(base64.b64encode(key.private_bytes(
            serialization.Encoding.Raw, serialization.PrivateFormat.Raw,
            serialization.NoEncryption())))
        seed.chmod(0o600)
        public = base64.b64encode(key.public_key().public_bytes(
            serialization.Encoding.Raw, serialization.PublicFormat.Raw)).decode()
        self.env = dict(os.environ, CHDA_SIGN_IDENTITY=identity,
                        CHDA_SPARKLE_PUBLIC_KEY=public,
                        CHDA_SPARKLE_KEY_FILE=str(seed))
        class Quiet(http.server.SimpleHTTPRequestHandler):
            def log_message(self, *args):
                pass
        self.server = http.server.ThreadingHTTPServer(
            ('127.0.0.1', 0), functools.partial(Quiet, directory=str(self.feed)))
        self.url = f'http://127.0.0.1:{self.server.server_port}/'
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        (self.original / 'Contents/MacOS').mkdir(parents=True)
        shutil.copy2(binary, self.original / 'Contents/MacOS/chda')
        (self.original / 'Contents/Info.plist').write_bytes(
            (REPO / 'resources/Info.plist').read_bytes().replace(b'__VERSION__', b'0.1.19'))
        write_plist(self.original, CFBundleIdentifier=f'com.magicsih.chda.qa.{self.root.name}')
        self.run(REPO / 'scripts/package-macos-updater.sh', self.original, env=self.env)
        write_plist(self.original, SUFeedURL=self.url + 'appcast.xml')
        self.sign(self.original)
        new = self.feed / 'chda.app'
        self.run('ditto', self.original, new)
        write_plist(new, CFBundleVersion='0.1.20', CFBundleShortVersionString='0.1.20')
        self.sign(new)
        self.run('ditto', '-c', '-k', '--keepParent', new,
                 self.feed / 'chda-0.1.20-macos-universal.zip')
        self.run('python3', REPO / 'scripts/generate-appcast.py', '0.1.20', self.feed, env=self.env)
        self.appcast = (self.feed / 'appcast.xml').read_text().replace(
            'https://github.com/magicsih/chda/releases/download/v0.1.20/', self.url)
        self.reset()

    def run(self, *args, **kwargs):
        command(*args, stdout=self.logs, stderr=self.logs, cwd=REPO, **kwargs)

    def sign(self, app):
        self.run('codesign', '--force', '--options', 'runtime', '--timestamp',
                 '--entitlements', REPO / 'resources/entitlements.plist',
                 '--sign', self.identity, app)
        self.run('codesign', '--verify', '--deep', '--strict', app)

    def reset(self):
        self.target.parent.mkdir(exist_ok=True)
        for app in (self.target, self.backup):
            if app.exists():
                shutil.rmtree(app)
            self.run('ditto', self.original, app)
        (self.feed / 'appcast.xml').write_text(self.appcast)

    def environment(self, job):
        home = job / 'home'
        (home / '.config/chda').mkdir(parents=True)
        (home / '.config/chda/config.toml').write_text('update-check = false\n')
        return dict(os.environ, HOME=str(home), ZDOTDIR=str(home),
                    XDG_CONFIG_HOME=str(home / '.config'), XDG_DATA_HOME=str(job / 'data'))

    def job(self, name):
        job = self.root / name
        job.mkdir(mode=0o700)
        (job / 'progress.jsonl').write_text('')
        return job

    @staticmethod
    def progress(job):
        events = []
        for line in (job / 'progress.jsonl').read_text().splitlines():
            try:
                events.append(json.loads(line))
            except json.JSONDecodeError:
                pass
        return events[-1] if events else {}

    def passed(self, name, evidence):
        self.results.append(dict(case=name, evidence=evidence))
        (self.root / 'results.json').write_text(json.dumps(self.results, indent=2))
        print(f'{name}: PASS — {evidence}', flush=True)

    def installer(self, name):
        self.reset()
        job = self.job(name)
        env = self.environment(job)
        feed = ET.fromstring(self.appcast)
        enclosure = feed.find('./channel/item/enclosure')
        if name == 'bad-signature':
            enclosure.set('{http://www.andymatuschak.org/xml-namespaces/sparkle}edSignature',
                          base64.b64encode(bytes(64)).decode())
        elif name == 'network':
            enclosure.set('url', self.url + 'missing.zip')
        ET.ElementTree(feed).write(self.feed / 'appcast.xml', encoding='utf-8', xml_declaration=True)
        if name == 'cancel':
            (job / 'cancel').write_text('1')
        gui = subprocess.Popen([str(self.target / 'Contents/MacOS/chda')], env=env,
                               stdout=self.logs, stderr=self.logs) if name == 'install' else None
        if gui:
            time.sleep(1)
            assert gui.poll() is None, 'initial GUI exited before the installer test' 
        helper = subprocess.Popen([str(self.backup / 'Contents/Helpers/chda-updater'),
                                   str(self.target), str(job), '0.1.20', str(gui.pid if gui else os.getpid())],
                                  env=env, stdout=self.logs, stderr=self.logs)
        try:
            deadline = time.monotonic() + 90
            while time.monotonic() < deadline:
                state = self.progress(job)
                if state.get('phase') == 'ready' and name == 'install':
                    (job / 'install').write_text('1')
                    gui.terminate()
                    gui.wait(timeout=10)
                if state.get('phase') in ('installed', 'failed'):
                    break
                time.sleep(.1)
            expected = 'installed' if name == 'install' else 'failed'
            assert state.get('phase') == expected, state
            version = plistlib.loads((self.target / 'Contents/Info.plist').read_bytes())['CFBundleVersion']
            assert version == ('0.1.20' if name == 'install' else '0.1.19'), version
            self.passed(name, f'Sparkle {expected}; target version {version}')
        finally:
            for child in (helper, gui):
                if child is None:
                    continue
                if child.poll() is None:
                    child.terminate()
                child.wait(timeout=15)

    def adoption(self, name):
        import fcntl
        self.reset()
        job = self.job(name)
        env = self.environment(job)
        home = Path(env['HOME'])
        if name == 'new-app-failure':
            stub = job / 'fail.c'
            stub.write_text('int main(void) { return 1; }\n')
            self.run('xcrun', 'clang', stub, '-o', self.target / 'Contents/MacOS/chda')
            self.sign(self.target)
        (job / 'progress.jsonl').write_text(json.dumps(
            dict(phase='failed', message='Injected installation failure')
            if name == 'install-failure' else dict(phase='installed')) + '\n')
        fds, children, panes, snapshots = [], [], [], []
        broker = None
        try:
            for i in range(3):
                master, slave = pty.openpty()
                fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
                child = subprocess.Popen(['/bin/sh', '-c',
                    f'echo live; while read x; do echo "$x" >> "{job}/shell-{i}"; echo received:$x; done'],
                    stdin=slave, stdout=slave, stderr=slave, env=env, start_new_session=True)
                os.close(slave)
                children.append(child)
                fds.append(master)
                assert select.select([master], [], [], 5)[0]
                os.read(master, 65536)
                snapshot = ''.join(f'preserved-{i}-history-{n}\r\n' for n in range(40)).encode()
                snapshots.append(snapshot)
                panes.append(dict(pane=i+1, title=f'Pane {i}', agent_live=True,
                    agent=dict(agent='claude', status='Working', since=int(time.time()*1000), seen=False),
                    cols=80, rows=24, snapshot_len=len(snapshot)))
            def node(i):
                return dict(kind='pane', pane=i, cwd=str(home), agent='claude', session='retained')
            def window(root, x):
                return dict(bounds=dict(x=x, y=150, width=600, height=420),
                            tabs=[dict(title='Preserved', root=root, focused=0)], active=0)
            state = dict(session=dict(windows=[window(dict(kind='split', horizontal=True, ratio=.5,
                first=node(1), second=node(2)), 80), window(node(3), 710)], active_window=1),
                panes=panes, started_at=int(time.time()*1000), update=dict(directory=str(job),
                    target_exe=str(self.target / 'Contents/MacOS/chda'),
                    recovery_exe=str(self.backup / 'Contents/MacOS/chda')))
            data = json.dumps(state).encode() + b'\n' + b''.join(snapshots)
            ours, theirs = socket.socketpair()
            ours.settimeout(40)
            broker = subprocess.Popen([str(self.backup / 'Contents/MacOS/chda'),
                '--update-broker', str(theirs.fileno())], pass_fds=[*fds, theirs.fileno()],
                env=env, stdout=self.logs, stderr=self.logs)
            theirs.close()
            ours.sendall((f'chda-handoff 2 {len(data)}' + ''.join(
                f' {fd}:{child.pid}' for fd, child in zip(fds, children)) + '\n').encode() + data)
            assert ours.recv(100) == b'prepared\n'
            ours.sendall(b'commit\n')
            ours.close()
            for fd in fds:
                os.write(fd, b'during-handoff\n')
            assert broker.wait(timeout=45) == 0
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline and not all((job / f'shell-{i}').exists() for i in range(3)):
                time.sleep(.1)
            for i, child in enumerate(children):
                assert child.poll() is None, 'original shell terminated'
                assert 'during-handoff' in (job / f'shell-{i}').read_text()
            deadline = time.monotonic() + 10
            while True:
                client = socket.socket(socket.AF_UNIX)
                client.settimeout(10)
                client.connect(str(job / 'data/chda/hook.sock'))
                client.sendall(json.dumps(dict(request=dict(action='list_worktrees', cwd=str(home)))).encode()+b'\n')
                reply = json.loads(client.recv(65536))
                client.close()
                if reply['ok']:
                    break
                assert time.monotonic() < deadline, reply
                time.sleep(.1)
            (job / 'handoff.json').write_text(json.dumps(state, indent=2))
            self.passed(name, 'same 3 shell PIDs; 2 GUI windows and split; post-commit IPC responds')
        finally:
            if broker and broker.poll() is None:
                broker.terminate()
                broker.wait(timeout=10)
            # Only terminate GUI executables inside this private test directory.
            for row in subprocess.check_output(['ps', '-axo', 'pid,command'], text=True).splitlines()[1:]:
                pid, cmd = row.strip().split(None, 1)
                if cmd.startswith(str(self.root) + '/') and '/Contents/MacOS/chda --adopt ' in cmd:
                    try:
                        os.kill(int(pid), signal.SIGTERM)
                    except ProcessLookupError:
                        pass
            for fd in fds:
                os.close(fd)
            for child in children:
                if child.poll() is None:
                    child.terminate()
                child.wait(timeout=10)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--identity', required=True, help='Existing Developer ID signing identity')
    parser.add_argument('--binary', type=Path, default=REPO / 'target/debug/chda')
    args = parser.parse_args()
    fixture = Fixture(args.binary.resolve(), args.identity)
    print(f'Private evidence directory: {fixture.root}', flush=True)
    try:
        for case in ('install', 'bad-signature', 'network', 'cancel'):
            fixture.installer(case)
        for case in ('adopt', 'install-failure', 'new-app-failure'):
            fixture.adoption(case)
    finally:
        (fixture.root / 'seed').unlink(missing_ok=True)
        fixture.server.shutdown()
        fixture.logs.close()


if __name__ == '__main__':
    main()
