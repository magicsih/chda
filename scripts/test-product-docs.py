#!/usr/bin/env python3
"""Check documentation synchronization without touching the checkout."""

import importlib.util
from contextlib import redirect_stdout
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location(
    "product_docs", Path(__file__).with_name("sync-product-docs.py")
)
docs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(docs)


class ProductDocsTests(unittest.TestCase):
    def setUp(self):
        self.enterContext(redirect_stdout(io.StringIO()))
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "docs/media").mkdir(parents=True)
        (self.root / "docs/site").mkdir()
        self.catalog = {
            "features": [{"area": "Git | history", "details": "Read `HEAD`",
                          "title": "A <tree>", "summary": "Use `HEAD` & tags"}],
            "screenshots": [{"file": "graph.png", "alt": 'A "commit" graph',
                             "label": "View full-size graph"}],
        }
        self.write_catalog()
        self.png = (docs.ROOT / "docs/media/hero.png").read_bytes()
        (self.root / "docs/media/graph.png").write_bytes(self.png)
        self.original = "Before\n" + docs.BEGIN + "\nstale\n" + docs.END + "\nAfter\n"
        for path in ("README.md", "docs/site/index.html"):
            (self.root / path).write_text(self.original)

    def write_catalog(self):
        (self.root / "docs/product-features.json").write_text(json.dumps(self.catalog))

    def test_check_detects_drift_without_writing_and_generation_is_idempotent(self):
        self.assertTrue(docs.sync(self.root, check=True))
        self.assertEqual((self.root / "README.md").read_text(), self.original)
        docs.sync(self.root)
        self.assertFalse(docs.sync(self.root, check=True))
        self.assertEqual((self.root / "docs/site/graph.png").read_bytes(), self.png)
        text = (self.root / "README.md").read_text()
        self.assertTrue(text.startswith("Before\n"))
        self.assertTrue(text.endswith("\nAfter\n"))
        self.assertIn("Git &#124; history", text)
        site = (self.root / "docs/site/index.html").read_text()
        self.assertIn("A &lt;tree&gt;", site)
        self.assertIn("<code>HEAD</code> &amp; tags", site)
        self.assertIn("A &quot;commit&quot; graph", site)
        self.assertIn('<a href="graph.png">', site)

    def test_new_feature_updates_both_documents(self):
        docs.sync(self.root)
        self.catalog["features"].append({"area": "New feature", "details": "New behavior",
                                         "title": "New feature", "summary": "New behavior"})
        self.write_catalog()
        self.assertTrue(docs.sync(self.root, check=True))
        docs.sync(self.root)
        for path in ("README.md", "docs/site/index.html"):
            self.assertIn("New behavior", (self.root / path).read_text())

    def test_changed_screenshot_is_detected(self):
        docs.sync(self.root)
        (self.root / "docs/site/graph.png").write_bytes(b"stale")
        self.assertTrue(docs.sync(self.root, check=True))
        docs.sync(self.root)
        self.assertEqual((self.root / "docs/site/graph.png").read_bytes(), self.png)

    def test_png_and_jpeg_dimensions(self):
        self.assertEqual(docs.image_size(self.png), (1200, 750))
        jpeg = (docs.ROOT / "docs/media/git-graph.jpg").read_bytes()
        self.assertEqual(docs.image_size(jpeg), (1920, 990))
        with self.assertRaises(ValueError):
            docs.image_size(jpeg[:10])

    def test_duplicate_or_missing_markers_do_not_partially_write(self):
        for text in ("no markers", self.original + docs.BEGIN):
            (self.root / "docs/site/index.html").write_text(text)
            with self.assertRaises(ValueError):
                docs.sync(self.root)
            self.assertEqual((self.root / "README.md").read_text(), self.original)
            self.assertFalse((self.root / "docs/site/graph.png").exists())

    def test_invalid_catalog_and_media_are_rejected_before_writing(self):
        self.catalog["features"].append(self.catalog["features"][0])
        self.write_catalog()
        with self.assertRaises(ValueError):
            docs.sync(self.root)
        self.catalog["features"].pop()
        self.catalog["screenshots"][0]["file"] = "../graph.png"
        self.write_catalog()
        with self.assertRaises(ValueError):
            docs.sync(self.root)
        self.catalog["screenshots"][0]["file"] = "graph.png"
        self.write_catalog()
        (self.root / "docs/media/graph.png").write_bytes(b"not a PNG")
        with self.assertRaises(ValueError):
            docs.sync(self.root)
        self.assertEqual((self.root / "README.md").read_text(), self.original)


if __name__ == "__main__":
    unittest.main()
