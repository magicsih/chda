#!/usr/bin/env python3
"""Readiness must still require marker pixels when full-frame OCR misses text."""
import hashlib
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
from PIL import Image, ImageDraw
import native_visual


class RenderedReadinessTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.frame = Path(self.temp.name) / "capture.png"
        image = Image.new("RGB", (640, 480), (0, 0, 0))
        ImageDraw.Draw(image).rectangle((100, 80, 599, 429), fill=native_visual.BACKGROUND)
        image.save(self.frame)
        self.original = self.frame.read_bytes()
        self.driver = unittest.mock.Mock()
        self.driver.screenshot.return_value = self.frame

    def test_reads_current_terminal_pixels_when_full_frame_segmentation_is_empty(self):
        def words(path, psm=11):
            return [{"text": "CHDA_VALIDATION>"}] if Path(path).stem.endswith(".terminal") else []
        with patch.object(native_visual, "ocr_words", side_effect=words), patch.object(native_visual.time, "monotonic", side_effect=[0, 0, 1]):
            self.assertEqual(native_visual.wait_for_rendered(self.driver, "CHDAVALIDATION", "startup", timeout=.5), self.frame)
        self.assertEqual(self.frame.read_bytes(), self.original)
        crop = self.frame.with_suffix(".terminal.png")
        with Image.open(crop) as image:
            self.assertEqual(image.size, (500, 350))
        import json
        proof = json.loads(self.frame.with_suffix(".terminal.json").read_text())
        self.assertEqual(proof["sourceSha256"], hashlib.sha256(self.original).hexdigest())

    def test_a_window_without_rendered_marker_still_times_out(self):
        with patch.object(native_visual, "ocr_words", return_value=[]), patch.object(native_visual.time, "monotonic", side_effect=[0, 0, 1]):
            with self.assertRaisesRegex(AssertionError, "Timed out waiting for rendered text"):
                native_visual.wait_for_rendered(self.driver, "CHDAVALIDATION", "startup", timeout=.5)
        self.assertEqual(self.frame.read_bytes(), self.original)

    def test_no_terminal_background_does_not_create_a_readiness_pass(self):
        Image.new("RGB", (640, 480), (0, 0, 0)).save(self.frame)
        with patch.object(native_visual, "ocr_words", return_value=[]), patch.object(native_visual.time, "monotonic", side_effect=[0, 0, 1]):
            with self.assertRaises(AssertionError):
                native_visual.wait_for_rendered(self.driver, "CHDAVALIDATION", "startup", timeout=.5)
        self.assertFalse(self.frame.with_suffix(".terminal.png").exists())


if __name__ == "__main__":
    unittest.main()
