#!/usr/bin/env python3
"""Generate and validate an appcast with the pinned official Sparkle tool.

The signing seed stays in the file named by CHDA_SPARKLE_KEY_FILE. Never put
it in argv, logs or the repository. Missing/mismatching keys fail closed.
"""
import os
from pathlib import Path
import re
import subprocess
import sys
import xml.etree.ElementTree as ET

SPARKLE = 'http://www.andymatuschak.org/xml-namespaces/sparkle'


def validate(path, version, archive):
    root = ET.parse(path).getroot()
    items = root.findall('./channel/item')
    matching = [i for i in items if i.findtext(f'{{{SPARKLE}}}version') == version]
    if len(matching) != 1:
        raise ValueError('Appcast must contain exactly one matching version')
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
    subprocess.run([str(tool / 'generate_appcast'), '--ed-key-file', str(key),
                    '--download-url-prefix', f'https://github.com/magicsih/chda/releases/download/v{version}/',
                    '--maximum-deltas', '0', '--versions', version, str(directory)], check=True)
    signature = validate(directory / 'appcast.xml', version, archive)
    subprocess.run([str(tool / 'sign_update'), '--verify', '--ed-key-file', str(key), str(archive), signature], check=True)
    print('Validated appcast.xml and signed release archive')


if __name__ == '__main__':
    main()
