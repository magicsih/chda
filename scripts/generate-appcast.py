#!/usr/bin/env python3
"""Generate and validate an appcast with the pinned official Sparkle tool.

The signing seed stays in the file named by CHDA_SPARKLE_KEY_FILE. Never put
it in argv, logs or the repository. Missing/mismatching keys fail closed.
"""
import os
import plistlib
import zipfile
from pathlib import Path
import re
import subprocess
import sys
import xml.etree.ElementTree as ET

SPARKLE = 'http://www.andymatuschak.org/xml-namespaces/sparkle'


def validate(path, version, archive, expected_protocol=None):
    root = ET.parse(path).getroot()
    items = root.findall('./channel/item')
    matching = [i for i in items if i.findtext(f'{{{SPARKLE}}}version') == version]
    if len(matching) != 1:
        raise ValueError('Appcast must contain exactly one matching version')
    if expected_protocol is not None and matching[0].findtext('chdaSessionProtocol') != str(expected_protocol):
        raise ValueError('Appcast session protocol does not match the release archive')
    enclosure = matching[0].find('enclosure')
    expected = f'https://github.com/magicsih/chda/releases/download/v{version}/{archive.name}'
    if enclosure is None or enclosure.get('url') != expected:
        raise ValueError('Appcast download URL does not match the release archive')
    if int(enclosure.get('length', '0')) != archive.stat().st_size:
        raise ValueError('Appcast archive length mismatch')
    import base64
    if len(base64.b64decode(enclosure.get(f'{{{SPARKLE}}}edSignature', ''), validate=True)) != 64:
        raise ValueError('Appcast must contain an Ed25519 signature')
    return enclosure.get(f'{{{SPARKLE}}}edSignature')


def archive_protocol(archive):
    with zipfile.ZipFile(archive) as bundle:
        info = plistlib.loads(bundle.read('chda.app/Contents/Info.plist'))
        helper = bundle.getinfo('chda.app/Contents/MacOS/chda-session')
    protocol = info.get('CHDASessionProtocol')
    if type(protocol) is not int or protocol < 1 or not helper.external_attr >> 16 & 0o111:
        raise ValueError('Archive must contain an executable session service and its protocol')
    return protocol


def annotate(path, version, protocol):
    # Sparkle exposes foreign XML nodes through SUAppcastItem.propertiesDictionary.
    ET.register_namespace('sparkle', SPARKLE)
    tree = ET.parse(path)
    matching = [i for i in tree.findall('./channel/item')
                if i.findtext(f'{{{SPARKLE}}}version') == version]
    if len(matching) != 1:
        raise ValueError('Expected one appcast item before adding session protocol')
    item = matching[0]
    for old in item.findall('chdaSessionProtocol'):
        item.remove(old)
    ET.SubElement(item, 'chdaSessionProtocol').text = str(protocol)
    tree.write(path, encoding='utf-8', xml_declaration=True)


def main():
    version, directory = sys.argv[1:]
    if not re.fullmatch(r'\d+\.\d+\.\d+', version):
        raise SystemExit('A stable semantic version is required')
    directory = Path(directory).resolve()
    archive = directory / f'chda-{version}-macos-universal.zip'
    key = Path(os.environ['CHDA_SPARKLE_KEY_FILE']).resolve()
    if not key.is_file():
        raise SystemExit('Sparkle signing key file is missing')
    tool = Path(__file__).resolve().parent.parent / 'target/sparkle-2.10.0/bin'
    generate = [str(tool / 'generate_appcast'), '--ed-key-file', str(key),
                    '--download-url-prefix', f'https://github.com/magicsih/chda/releases/download/v{version}/',
                    '--maximum-deltas', '0', '--versions', version, str(directory)]
    protocol = archive_protocol(archive)
    subprocess.run(generate, check=True)
    annotate(directory / 'appcast.xml', version, protocol)
    # Editing the feed invalidates its optional whole-feed signature. Sparkle's
    # generator preserves custom nodes and signs the updated feed on this pass.
    subprocess.run(generate, check=True)
    signature = validate(directory / 'appcast.xml', version, archive, protocol)
    subprocess.run([str(tool / 'sign_update'), '--verify', '--ed-key-file', str(key), str(archive), signature], check=True)
    print('Validated appcast.xml and signed release archive')


if __name__ == '__main__':
    main()
