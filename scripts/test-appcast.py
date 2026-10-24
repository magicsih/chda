#!/usr/bin/env python3
"""Regression checks for release appcast metadata; signatures use Sparkle's tool."""
import base64
import importlib.util
from pathlib import Path
import sys
sys.dont_write_bytecode = True
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('appcast', Path(__file__).with_name('generate-appcast.py'))
appcast = importlib.util.module_from_spec(spec)
spec.loader.exec_module(appcast)


class AppcastTests(unittest.TestCase):
    def test_release_identity_size_and_signature_are_required(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            archive = directory / 'chda-1.2.3-macos-universal.zip'
            archive.write_bytes(b'fixture')
            signature = base64.b64encode(bytes(64)).decode()
            xml = (f'<rss xmlns:sparkle="{appcast.SPARKLE}"><channel><item>'
                   '<sparkle:version>1.2.3</sparkle:version>'
                   '<enclosure url="https://github.com/magicsih/chda/releases/download/v1.2.3/'
                   f'{archive.name}" length="7" sparkle:edSignature="{signature}"/>'
                   '</item></channel></rss>')
            path = directory / 'appcast.xml'
            path.write_text(xml)
            self.assertEqual(appcast.validate(path, '1.2.3', archive), signature)
            for bad in (xml.replace('length="7"', 'length="9"'),
                        xml.replace('/v1.2.3/', '/v1.2.4/'),
                        xml.replace('<sparkle:version>1.2.3', '<sparkle:version>1.2.4'),
                        xml.replace(signature, '')):
                path.write_text(bad)
                with self.assertRaises(ValueError):
                    appcast.validate(path, '1.2.3', archive)


if __name__ == '__main__':
    unittest.main()
