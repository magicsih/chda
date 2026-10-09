#!/usr/bin/env python3
"""macOS release evidence and immutable draft-to-public promotion.

Evidence describes actual, separately reviewed native runs. Unit-test fixtures
never supply release approval. Only bounded, allowlisted metadata is published.
"""
import argparse
import base64
from datetime import datetime, timedelta, timezone
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET

REPO = 'magicsih/chda'
CASES = frozenset(('PATH-164 NAV-149 PR-150 TREE-151 REMOVE-152 QUOTA-153 '
                   'CLOSE-154 NOTIFY-155 COPY-158 POLICY-159 RESTART-160 '
                   'CHILD-161 REVIEW-162 PREP-163').split())
FEATURES = frozenset(('Terminal', 'Tabs and splits', 'App updates', 'Title bar',
                      'Notifications', 'Sidebar', 'Worktrees', 'Git history',
                      'Diff review', 'Agents', 'Status bar', 'Palette', 'Config'))
ARTIFACT_CHECKS = frozenset(('signed-launch', 'session-preserving-update',
                            'gui-ipc', 'shell-pid-preservation'))
EXCEPTION_CHECKS = ARTIFACT_CHECKS | frozenset((
    'three-native-review-rounds', 'before-build-native-e2e', 'before-deploy-native-e2e'))
EXCEPTION_PATH = 'docs/releases/v0.1.21-native-input-exception.json'
SPARKLE = 'http://www.andymatuschak.org/xml-namespaces/sparkle'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def fields(value, expected):
    require(isinstance(value, dict) and set(value) == set(expected),
            'Missing or unsupported metadata fields')


def digest(value, length=64):
    require(isinstance(value, str) and re.fullmatch('[0-9a-f]{%d}' % length, value),
            'Invalid digest')


def instant(value):
    require(isinstance(value, str) and re.fullmatch(r'\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z', value),
            'UTC timestamps are required')
    return datetime.strptime(value, '%Y-%m-%dT%H:%M:%SZ').replace(tzinfo=timezone.utc)


def stamp(now):
    return now.strftime('%Y-%m-%dT%H:%M:%SZ')


def unique_items(values, expected):
    require(isinstance(values, list) and all(isinstance(v, str) for v in values)
            and len(values) == len(set(values)) and set(values) == set(expected),
            'Incomplete, duplicate or unsupported coverage')


def read_json_text(text):
    require(isinstance(text, str) and len(text.encode('utf-8')) <= 65536,
            'Evidence exceeds the 64 KiB limit')

    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, 'Duplicate JSON field')
            result[key] = value
        return result

    try:
        return json.loads(text, object_pairs_hook=pairs,
                          parse_constant=lambda _: require(False, 'Non-finite JSON number'))
    except RecursionError:
        raise ValueError('JSON nesting exceeds the metadata limit') from None


def read_json(path):
    with Path(path).open('rb') as handle:
        data = handle.read(65537)
    require(len(data) <= 65536, 'Metadata exceeds the 64 KiB limit')
    return read_json_text(data.decode('utf-8'))


def write_json(path, value):
    Path(path).write_text(json.dumps(value, sort_keys=True, indent=2) + '\n', encoding='utf-8')


def hash_file(path):
    result = hashlib.sha256()
    with Path(path).open('rb') as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b''):
            result.update(block)
    return result.hexdigest()


def candidate(value, tag, sha, tree, config):
    fields(value, ('tag', 'sha', 'tree', 'config_sha256'))
    require(re.fullmatch(r'v(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)', tag),
            'A stable vMAJOR.MINOR.PATCH tag is required')
    digest(sha, 40)
    digest(tree, 40)
    digest(config)
    require(value == dict(tag=tag, sha=sha, tree=tree, config_sha256=config),
            'Evidence belongs to another candidate or configuration')


def validate_run(run, implementer, now, is_round=False):
    names = ('kind', 'result', 'started_at', 'completed_at', 'reviewer_sha256',
             'plan_sha256', 'evidence_sha256', 'cases', 'features', 'environment')
    if is_round:
        names += ('number', 'sha', 'tree', 'config_sha256')
    fields(run, names)
    require(run['kind'] == 'native-development-runtime' and run['result'] == 'passed',
            'A passed actual native development run is required')
    for name in ('reviewer_sha256', 'plan_sha256', 'evidence_sha256'):
        digest(run[name])
    require(run['reviewer_sha256'] != implementer, 'An independent reviewer is required')
    unique_items(run['cases'], CASES)
    unique_items(run['features'], FEATURES)
    env = run['environment']
    fields(env, ('os', 'os_version', 'architecture', 'claude_version', 'codex_version', 'isolated'))
    require(env['os'] == 'macos' and env['architecture'] in ('arm64', 'x86_64')
            and env['isolated'] is True, 'Isolated macOS execution is required')
    for name in ('os_version', 'claude_version', 'codex_version'):
        require(isinstance(env[name], str) and re.fullmatch(r'\d+(?:\.\d+){1,3}', env[name]),
                'Only numeric version metadata may be published')
    start, end = instant(run['started_at']), instant(run['completed_at'])
    require(start < end <= now, 'Invalid or future native run timestamps')
    return start, end


def validate_exception(data, record_text, tag, sha, tree, config, stage, now,
                       built_at, artifact_sha256, fresh):
    require(tag == 'v0.1.21' and record_text is not None,
            'This release has no recorded owner exception')
    record = read_json_text(record_text)
    fields(record, ('schema', 'tag', 'sha', 'tree', 'config_sha256', 'owner',
                    'owner_instruction', 'accepted_interpretation', 'accepted_at',
                    'blocked_evidence_sha256', 'independent_assessment_sha256',
                    'incomplete_checks'))
    require(type(record['schema']) is int and record['schema'] == 1
            and record['owner'] == 'magicsih'
            and record['owner_instruction'] == '이제 좀 공개해라 좀'
            and record['accepted_interpretation'] == 'publish-with-incomplete-native-validation',
            'Unsupported owner exception record')
    require((record['tag'], record['sha'], record['tree'], record['config_sha256'])
            == (tag, sha, tree, config), 'The owner exception identifies another candidate')
    for name in ('blocked_evidence_sha256', 'independent_assessment_sha256'):
        digest(record[name])
    unique_items(record['incomplete_checks'], EXCEPTION_CHECKS)
    names = ('schema', 'platform', 'stage', 'candidate', 'native_result',
             'incomplete_checks', 'exception_sha256', 'checked_at')
    if stage == 'before-deploy':
        names += ('artifact_sha256',)
    fields(data, names)
    require(type(data['schema']) is int and data['schema'] == 2
            and data['platform'] == 'macos' and data['stage'] == stage
            and data['native_result'] == 'blocked',
            'An exception must retain the incomplete native validation result')
    candidate(data['candidate'], tag, sha, tree, config)
    unique_items(data['incomplete_checks'], EXCEPTION_CHECKS)
    require(data['incomplete_checks'] == record['incomplete_checks'],
            'The exception cannot omit or change incomplete checks')
    digest(data['exception_sha256'])
    require(data['exception_sha256'] == hashlib.sha256(record_text.encode('utf-8')).hexdigest(),
            'Exception evidence differs from the main approval record')
    checked = instant(data['checked_at'])
    require(instant(record['accepted_at']) <= checked <= now,
            'Invalid or future exception readback timestamps')
    require(not fresh or now - checked <= timedelta(minutes=60),
            'Exception readback is older than 60 minutes; refresh before this action')
    if stage == 'before-deploy':
        digest(data['artifact_sha256'])
        require(artifact_sha256 is not None and data['artifact_sha256'] == artifact_sha256,
                'Exception readback must identify the exact immutable draft archive')
        require(built_at is not None and checked >= instant(built_at),
                'Deploy readback must occur after the signed draft build')
    return data


def validate_evidence(data, tag, sha, tree, config, stage, now=None,
                      built_at=None, artifact_sha256=None, fresh=True, exception_record=None):
    now = now or datetime.now(timezone.utc)
    require(stage in ('before-build', 'before-deploy'), 'Unsupported native gate')
    if isinstance(data, dict) and data.get('schema') == 2:
        return validate_exception(data, exception_record, tag, sha, tree, config, stage,
                                  now, built_at, artifact_sha256, fresh)
    names = ('schema', 'platform', 'stage', 'candidate', 'implementer_sha256', 'rounds', 'run')
    if stage == 'before-deploy':
        names += ('signed_artifact',)
    fields(data, names)
    require(type(data['schema']) is int and data['schema'] == 1 and data['platform'] == 'macos'
            and data['stage'] == stage, 'Unsupported evidence schema or stage')
    candidate(data['candidate'], tag, sha, tree, config)
    digest(data['implementer_sha256'])
    rounds = data['rounds']
    require(isinstance(rounds, list) and len(rounds) == 3, 'Exactly three native review rounds are required')
    previous_end = None
    evidence_hashes = set()
    for number, review in enumerate(rounds, 1):
        start, end = validate_run(review, data['implementer_sha256'], now, is_round=True)
        require(type(review['number']) is int and review['number'] == number,
                'Review rounds must be ordered 1, 2, 3')
        digest(review['sha'], 40)
        require(review['tree'] == tree and review['config_sha256'] == config,
                'Review rounds must cover the identical final source tree and configuration')
        require(previous_end is None or previous_end <= start, 'Review rounds overlap or are reordered')
        require(review['evidence_sha256'] not in evidence_hashes, 'Review evidence was reused')
        evidence_hashes.add(review['evidence_sha256'])
        previous_end = end
    start, end = validate_run(data['run'], data['implementer_sha256'], now)
    require(previous_end <= start and data['run']['evidence_sha256'] not in evidence_hashes,
            'The final E2E must be a separate run after the three review rounds')
    # Freshness is checked at the runner immediately before the relevant action.
    require(not fresh or now - end <= timedelta(minutes=60),
            'Native evidence is older than 60 minutes; rerun E2E before this action')
    if stage == 'before-deploy':
        artifact = data['signed_artifact']
        fields(artifact, ('sha256', 'evidence_sha256', 'checks', 'result'))
        digest(artifact['sha256'])
        digest(artifact['evidence_sha256'])
        require(artifact['result'] == 'passed', 'Signed artifact verification did not pass')
        unique_items(artifact['checks'], ARTIFACT_CHECKS)
        require(artifact_sha256 is not None and artifact['sha256'] == artifact_sha256,
                'Native artifact checks must cover the exact draft archive')
        require(built_at is not None and start >= instant(built_at),
                'Before-deploy E2E must start after the signed draft build')
    return data


def archive_name(tag):
    return 'chda-%s-macos-universal.zip' % tag[1:]


def appcast(root, tag):
    path = root / 'appcast.xml'
    require(path.stat().st_size <= 1024 * 1024, 'Appcast is too large')
    items = ET.parse(path).getroot().findall('./channel/item')
    require(len(items) == 1 and items[0].findtext('{%s}version' % SPARKLE) == tag[1:],
            'Appcast must contain only the exact release version')
    enclosure = items[0].find('enclosure')
    archive = root / archive_name(tag)
    require(enclosure is not None
            and enclosure.get('url') == 'https://github.com/%s/releases/download/%s/%s' % (REPO, tag, archive.name)
            and enclosure.get('length') == str(archive.stat().st_size),
            'Appcast archive URL or size differs from the candidate')
    signature = enclosure.get('{%s}edSignature' % SPARKLE, '')
    require(len(base64.b64decode(signature, validate=True)) == 64,
            'Appcast has no valid Ed25519 signature representation')


def make_manifest(root, before, now=None):
    root = Path(root)
    appcast(root, before['candidate']['tag'])
    write_json(root / 'before-build.json', before)
    names = (archive_name(before['candidate']['tag']), 'appcast.xml', 'before-build.json')
    assets = {name: {'size': (root / name).stat().st_size,
                     'sha256': hash_file(root / name)} for name in names}
    manifest = dict(schema=1, candidate=before['candidate'],
                    built_at=stamp(now or datetime.now(timezone.utc)), assets=assets)
    write_json(root / 'manifest.json', manifest)
    sums = ''.join('%s  %s\n' % (hash_file(root / name), name)
                   for name in sorted((*names, 'manifest.json')))
    (root / 'SHA256SUMS').write_text(sums, encoding='ascii')
    return manifest


def validate_assets(root, manifest, tag, sha, tree, config):
    fields(manifest, ('schema', 'candidate', 'built_at', 'assets'))
    require(type(manifest['schema']) is int and manifest['schema'] == 1, 'Unsupported manifest schema')
    candidate(manifest['candidate'], tag, sha, tree, config)
    instant(manifest['built_at'])
    names = {archive_name(tag), 'appcast.xml', 'before-build.json'}
    fields(manifest['assets'], names)
    for name, expected in manifest['assets'].items():
        fields(expected, ('sha256', 'size'))
        digest(expected['sha256'])
        path = root / name
        require(path.is_file() and not path.is_symlink()
                and type(expected['size']) is int and expected['size'] > 0
                and path.stat().st_size == expected['size'] and hash_file(path) == expected['sha256'],
                'Release asset hash or size mismatch')
    require(hash_file(root / 'manifest.json') == hashlib.sha256(
        (json.dumps(manifest, sort_keys=True, indent=2) + '\n').encode('utf-8')).hexdigest(),
        'Unexpected manifest encoding')
    expected_sums = ''.join('%s  %s\n' % (hash_file(root / name), name)
                            for name in sorted((*names, 'manifest.json')))
    require((root / 'SHA256SUMS').stat().st_size <= 4096, 'Checksum metadata exceeds its limit')
    require((root / 'SHA256SUMS').read_text(encoding='ascii') == expected_sums,
            'Release checksum file differs from the immutable assets')
    require({p.name for p in root.iterdir()} <= names | {'manifest.json', 'SHA256SUMS', 'before-deploy.json'},
            'Unexpected release asset')
    appcast(root, tag)


def command(*argv):
    return subprocess.run(argv, check=True, stdout=subprocess.PIPE, text=True).stdout.strip()


def gh_release(tag):
    # Authentication/network errors are errors, not an absent draft.
    releases = json.loads(command('gh', 'api', '--paginate', '--slurp', 'repos/%s/releases?per_page=100' % REPO))
    matches = [r for page in releases for r in page if r['tag_name'] == tag]
    require(len(matches) <= 1, 'Duplicate release tag')
    return matches[0] if matches else None


def check_source(tag, sha, config):
    digest(sha, 40)
    digest(config)
    require(re.fullmatch(r'v(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)', tag), 'Invalid stable tag')
    require(command('git', 'rev-parse', 'refs/tags/%s^{commit}' % tag) == sha, 'Tag does not identify the exact candidate SHA')
    command('git', 'merge-base', '--is-ancestor', sha, 'origin/main')
    tree = command('git', 'rev-parse', '%s^{tree}' % sha)
    toml = command('git', 'show', '%s:Cargo.toml' % sha)
    section = re.search(r'^\[workspace\.package\]\s*\n(.*?)(?=^\[|\Z)', toml, re.M | re.S)
    version = re.search(r'^version\s*=\s*"([^"]+)"\s*$', section[1], re.M) if section else None
    require(version is not None and 'v' + version[1] == tag, 'Workspace version differs from the release tag')
    features = json.loads(command('git', 'show', '%s:docs/product-features.json' % sha))
    unique_items([f['area'] for f in features['features']], FEATURES)
    return tree


def check_round_commits(data):
    if data['schema'] == 2:
        return  # The validated owner exception records zero qualified native reviews.
    for review in data['rounds']:
        digest(review['sha'], 40)
        found = subprocess.run(['git', 'cat-file', '-e', review['sha'] + '^{commit}'],
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if found.returncode:
            command('git', 'fetch', '--no-tags', 'origin', review['sha'])
        require(command('git', 'rev-parse', '%s^{tree}' % review['sha']) == review['tree'],
                'A review round does not identify its recorded source tree')


def workflow_context():
    require(os.environ.get('GITHUB_REPOSITORY') == REPO
            and os.environ.get('GITHUB_EVENT_NAME') == 'workflow_dispatch'
            and os.environ.get('GITHUB_REF') == 'refs/heads/main',
            'Release mutations require an explicit dispatch on this repository main')


def freeze_exception(tag):
    """Copy only the authoritative main record before candidate checkout."""
    path = Path(__file__).with_suffix('.exception.json')
    require(not path.exists(), 'A frozen exception record already exists')
    if tag != 'v0.1.21':
        return
    found = subprocess.run(['git', 'cat-file', '-e', 'HEAD:' + EXCEPTION_PATH],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if found.returncode:
        return
    text = command('git', 'show', 'HEAD:' + EXCEPTION_PATH) + '\n'
    # Preserve the canonical committed bytes, including exactly one final newline.
    record = read_json_text(text)
    require(text == json.dumps(record, sort_keys=True, indent=2, ensure_ascii=False) + '\n',
            'The committed exception record must use canonical JSON encoding')
    with path.open('x', encoding='utf-8') as handle:
        handle.write(text)
    path.chmod(0o600)


def frozen_exception():
    path = Path(__file__).with_suffix('.exception.json')
    if not path.exists():
        return None
    require(path.is_file() and not path.is_symlink(), 'Invalid frozen exception file')
    with path.open('rb') as handle:
        body = handle.read(65537)
    require(len(body) <= 65536, 'Exception record exceeds the metadata limit')
    return body.decode('utf-8')


def evidence_completed(data):
    return data['checked_at'] if data['schema'] == 2 else data['run']['completed_at']


def download(root, tag):
    command('gh', 'release', 'download', tag, '--repo', REPO, '--dir', str(root))


def latest(tag):
    current = json.loads(command('gh', 'api', 'repos/%s/releases/latest' % REPO))
    require(current['tag_name'] == tag and not current['draft'] and not current['prerelease'],
            'The requested version is not the latest public stable release')


def prevent_downgrade(tag):
    current = json.loads(command('gh', 'api', 'repos/%s/releases/latest' % REPO))
    previous = current['tag_name']
    require(re.fullmatch(r'v\d+\.\d+\.\d+', previous), 'Latest release has an unsupported version')
    require(tuple(map(int, tag[1:].split('.'))) > tuple(map(int, previous[1:].split('.'))),
            'Publishing would replace latest with an equal or older version')


def update_cask(root, tag):
    token = os.environ.get('RELEASE_TOKEN')
    require(bool(token), 'The existing Homebrew release token is required')
    subprocess.run(['bash', 'scripts/update-cask.sh', tag[1:], str(root / archive_name(tag))],
                   check=True, env={**os.environ, 'GH_TOKEN': token,
                                    'CHDA_TAP_REPO': 'magicsih/homebrew-tap'})


def native_archive(root, tag):
    # Extract outside the immutable download directory. Apple validates the
    # same bytes that the native signed-artifact run records.
    with tempfile.TemporaryDirectory() as folder:
        command('ditto', '-x', '-k', str(root / archive_name(tag)), folder)
        app = Path(folder) / 'chda.app'
        with (app / 'Contents/Info.plist').open('rb') as handle:
            info = plistlib.load(handle)
        require(info.get('CFBundleShortVersionString') == tag[1:]
                and info.get('CFBundleVersion') == tag[1:]
                and info.get('CFBundleIdentifier') == 'com.magicsih.chda',
                'Signed bundle identity or version differs from the release candidate')
        require(info.get('SUFeedURL') == 'https://github.com/magicsih/chda/releases/latest/download/appcast.xml'
                and info.get('SUEnableAutomaticChecks') is False
                and info.get('SUAutomaticallyUpdate') is False
                and info.get('SUSendProfileInfo') is False,
                'Signed bundle updater settings differ from the approved manual update flow')
        require(len(base64.b64decode(info.get('SUPublicEDKey', ''), validate=True)) == 32,
                'Signed bundle is missing its Sparkle public key')
        item = ET.parse(root / 'appcast.xml').getroot().find('./channel/item/enclosure')
        command('xcrun', 'swift', str(Path(__file__).with_name('verify-macos-update.swift')),
                str(root / archive_name(tag)), item.get('{%s}edSignature' % SPARKLE),
                info['SUPublicEDKey'])
        command('codesign', '--verify', '--deep', '--strict', str(app))
        # codesign metadata is written to stderr, separately from normal output.
        details = subprocess.run(['codesign', '-d', '--verbose=2', str(app)], check=True,
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True).stderr
        require('Authority=Developer ID Application:' in details and 'TeamIdentifier=' in details
                and 'Signature=adhoc' not in details, 'A Developer ID signature is required')
        command('xcrun', 'stapler', 'validate', str(app))
        command('spctl', '--assess', '--type', 'execute', str(app))
        architectures = set(command('lipo', '-archs', str(app / 'Contents/MacOS/chda')).split())
        require(architectures == {'arm64', 'x86_64'}, 'The release executable must be universal')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('select', 'check-build', 'draft', 'publish', 'cask'))
    args = parser.parse_args()
    workflow_context()
    tag, sha, config = (os.environ[name] for name in ('RELEASE_TAG', 'RELEASE_SHA', 'RELEASE_CONFIG_SHA256'))
    tree = check_source(tag, sha, config)
    if args.action == 'select':
        # This copy of the verifier came from main, before candidate checkout.
        # No candidate-controlled code runs before tag/ancestry/version validation.
        require(command('git', 'rev-parse', 'HEAD') == command('git', 'rev-parse', 'origin/main'),
                'The verifier and exception record must start at the checked-out main')
        freeze_exception(tag)
        command('git', 'switch', '--detach', sha)
        print('Selected exact main candidate ' + tag)
        return
    require(command('git', 'rev-parse', 'HEAD') == sha, 'The checkout must be the exact candidate')
    approval = frozen_exception()
    existing = gh_release(tag)
    if args.action in ('check-build', 'draft'):
        require(existing is None, 'A release already exists: verify/publish the same draft; do not rebuild')
        before = read_json_text(os.environ['NATIVE_EVIDENCE'])
        validate_evidence(before, tag, sha, tree, config, 'before-build',
                          fresh=args.action == 'check-build', exception_record=approval)
        check_round_commits(before)
        if args.action == 'check-build':
            validate_evidence(before, tag, sha, tree, config, 'before-build', exception_record=approval)
            print(('Recorded native exception' if before['schema'] == 2 else 'Native before-build evidence')
                  + ' validated for ' + tag)
            return
        root = Path('target/bundle')
        native_archive(root, tag)
        manifest = make_manifest(root, before)
        assets = [root / name for name in (*manifest['assets'], 'manifest.json', 'SHA256SUMS')]
        notes = command('bash', 'scripts/release-notes.sh', tag)
        require(bool(notes), 'The release-plz changelog section is empty')
        if before['schema'] == 2:
            notes = ('Native UI validation is incomplete in this release. The owner requested publication '
                     'after the input-tool blocker was reported. CI, Developer ID signing, notarization '
                     'and update-signature verification remain required. '
                     '[Recorded exception](https://github.com/%s/blob/main/%s).\n\n'
                     'The Codex shared-server warning remains unresolved '
                     '([#173](https://github.com/%s/issues/173)).\n\n' % (REPO, EXCEPTION_PATH, REPO)) + notes
        with tempfile.TemporaryDirectory() as folder:
            notes_path = Path(folder) / 'notes.md'
            notes_path.write_text(notes + '\n', encoding='utf-8')
            command('gh', 'release', 'create', tag, *map(str, assets), '--repo', REPO,
                    '--verify-tag', '--draft', '--target', sha, '--title', 'chda ' + tag,
                    '--notes-file', str(notes_path))
        created = gh_release(tag)
        require(created is not None and created['draft'], 'Draft release creation was not confirmed')
        print('Signed assets retained in a draft; public latest and Homebrew are unchanged')
        return
    require(existing is not None and not existing['prerelease'], 'The stable release/draft is missing')
    require(bool(os.environ.get('RELEASE_TOKEN')), 'The existing Homebrew release token is required')
    with tempfile.TemporaryDirectory() as folder:
        root = Path(folder)
        download(root, tag)
        manifest = read_json(root / 'manifest.json')
        validate_assets(root, manifest, tag, sha, tree, config)
        before = read_json(root / 'before-build.json')
        validate_evidence(before, tag, sha, tree, config, 'before-build', fresh=False, exception_record=approval)
        require(instant(evidence_completed(before)) <= instant(manifest['built_at']),
                'Build evidence was recorded after the artifact build')
        check_round_commits(before)
        if args.action == 'cask':
            require(not existing['draft'] and (root / 'before-deploy.json').is_file(),
                    'Homebrew repair requires an already verified public release')
            after = read_json(root / 'before-deploy.json')
        else:
            after = read_json_text(os.environ['NATIVE_EVIDENCE'])
        archive_sha = manifest['assets'][archive_name(tag)]['sha256']
        validate_evidence(after, tag, sha, tree, config, 'before-deploy',
                          artifact_sha256=archive_sha, built_at=manifest['built_at'],
                          fresh=bool(existing['draft']), exception_record=approval)
        require(after['schema'] == before['schema'], 'Build and deploy evidence formats differ')
        if before['schema'] == 2:
            require(after['exception_sha256'] == before['exception_sha256']
                    and instant(after['checked_at']) > instant(before['checked_at']),
                    'Exception stages must share the same approval and distinct readbacks')
        else:
            require(after['rounds'] == before['rounds'] and after['implementer_sha256'] == before['implementer_sha256']
                    and after['run']['evidence_sha256'] != before['run']['evidence_sha256'],
                    'Build and deploy evidence must share the same reviews and distinct E2E runs')
        check_round_commits(after)
        native_archive(root, tag)
        after_path = root / 'before-deploy.json'
        if after_path.exists():
            require(read_json(after_path) == after, 'Existing deploy evidence cannot be replaced')
        else:
            require(existing['draft'] and args.action == 'publish', 'Public release has no deploy evidence')
            write_json(after_path, after)
            command('gh', 'release', 'upload', tag, str(after_path), '--repo', REPO)
        if existing['draft']:
            # Recheck freshness after download and Apple verification, immediately
            # before making the already built draft public.
            prevent_downgrade(tag)
            validate_evidence(after, tag, sha, tree, config, 'before-deploy',
                              artifact_sha256=archive_sha, built_at=manifest['built_at'], exception_record=approval)
            command('gh', 'release', 'edit', tag, '--repo', REPO, '--draft=false', '--latest')
        latest(tag)
        update_cask(root, tag)
        latest(tag)
        print('Verified identical public assets and latest release; Homebrew is synchronized')


if __name__ == '__main__':
    try:
        main()
    except (ValueError, KeyError, OSError, ET.ParseError, subprocess.CalledProcessError) as error:
        # Never echo the evidence body, credentials, argv environment or payloads.
        print('Release gate failed: %s' % (str(error) if isinstance(error, ValueError) else type(error).__name__),
              file=sys.stderr)
        sys.exit(1)
