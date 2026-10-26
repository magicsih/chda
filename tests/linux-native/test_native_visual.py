#!/usr/bin/env python3
"""Readiness must still require marker pixels when full-frame OCR misses text."""
import hashlib
from pathlib import Path
import sys
import subprocess
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
from PIL import Image, ImageDraw
import native_visual
from native_driver import Driver


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


class CopySelectionReadinessTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.driver = object.__new__(Driver)
        self.driver.evidence = self.root
        for method in ("command", "drag", "key", "record", "clipboard_read"):
            setattr(self.driver, method, unittest.mock.Mock())
        self.driver.clipboard_read.return_value = "CHDACOPYSENTINEL"
        self.frames = []
        self.originals = {}

        def screenshot(name):
            path = self.root / (name + ".png")
            Image.new("RGB", (640, 480), native_visual.BACKGROUND).save(path)
            self.frames.append(path)
            self.originals[path] = path.read_bytes()
            return path

        self.driver.screenshot = screenshot

    def sentinel_word(self):
        return {"text": "CHDACOPYSENTINEL", "left": "120", "top": "100",
                "width": "140", "height": "16"}

    def test_waits_for_actual_output_after_delayed_command_echo(self):
        echo = [{"text": part} for part in ("printf", "CHDA", "COPY", "SENTINEL")]
        with patch.object(native_visual, "ocr_words", side_effect=[echo, [self.sentinel_word()]]), \
             patch.object(native_visual.time, "monotonic", side_effect=[0, 0, .1]), \
             patch("native_driver.time.sleep"):
            self.driver.copy_selection()
        command = self.driver.command.call_args.args[0]
        self.assertNotIn("CHDACOPYSENTINEL", command)
        self.assertEqual(subprocess.check_output(["/bin/sh", "-c", command]),
                         b"\x1b[2J\x1b[H    CHDACOPYSENTINEL\n")
        self.assertEqual(len(self.frames), 3)  # Two readiness frames, then the actual selection.
        self.assertNotEqual(self.frames[0], self.frames[1])
        for path in self.frames[:2]:
            self.assertEqual(path.read_bytes(), self.originals[path])
            self.assertTrue(path.with_suffix(".ocr.json").exists())
        self.driver.drag.assert_called_once_with(121, 108, 262, 108)
        self.assertEqual((self.root / "copied-selection.txt").read_text(), "CHDACOPYSENTINEL")

    def test_missing_or_ambiguous_output_times_out_without_selecting(self):
        with patch.object(native_visual, "ocr_words", side_effect=[[], [self.sentinel_word()] * 2]), \
             patch.object(native_visual.time, "monotonic", side_effect=[0, 0, 1, 11]), \
             patch("native_driver.time.sleep"):
            with self.assertRaisesRegex(AssertionError, "Timed out waiting for rendered sentinel"):
                self.driver.copy_selection()
        self.driver.drag.assert_not_called()
        self.driver.clipboard_read.assert_not_called()
        self.assertEqual(len(self.frames), 2)
        for path in self.frames:
            self.assertEqual(path.read_bytes(), self.originals[path])
            self.assertTrue(path.with_suffix(".ocr.json").exists())

    def test_rendered_output_never_waives_exact_clipboard_equality(self):
        self.driver.clipboard_read.return_value = "CHDACOPYSENTINEL extra"
        with patch.object(native_visual, "ocr_words", return_value=[self.sentinel_word()]), \
             patch.object(native_visual.time, "monotonic", side_effect=[0, 0]), \
             patch("native_driver.time.sleep"):
            with self.assertRaisesRegex(AssertionError, "Actual selection/clipboard differs"):
                self.driver.copy_selection()
        self.driver.drag.assert_called_once()


if __name__ == "__main__":
    unittest.main()
