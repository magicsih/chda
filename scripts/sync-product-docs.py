#!/usr/bin/env python3
"""Generate README and Pages features from docs/product-features.json."""

import argparse
import html
import json
from pathlib import Path
import re
import struct
import sys


ROOT = Path(__file__).resolve().parent.parent
BEGIN = "<!-- BEGIN GENERATED FEATURES -->"
END = "<!-- END GENERATED FEATURES -->"


def replace_section(text, content):
    if text.count(BEGIN) != 1 or text.count(END) != 1:
        raise ValueError("expected exactly one pair of generated feature markers")
    before, section = text.split(BEGIN)
    _, after = section.split(END)
    return before + BEGIN + "\n" + content + "\n" + END + after


def inline(text):
    return re.sub(r"`([^`]+)`", r"<code>\1</code>", html.escape(text))


def image_size(data):
    if data[:8] == b"\x89PNG\r\n\x1a\n" and data[12:16] == b"IHDR" and len(data) >= 24:
        return struct.unpack(">II", data[16:24])
    if data[:2] == b"\xff\xd8":
        offset = 2
        while offset + 4 <= len(data) and data[offset] == 0xFF:
            marker = data[offset + 1]
            if marker == 0xFF:
                offset += 1
                continue
            length = int.from_bytes(data[offset + 2:offset + 4], "big")
            if length < 2 or offset + 2 + length > len(data):
                break
            if marker in (0xC0, 0xC1, 0xC2) and length >= 7:
                height, width = struct.unpack(">HH", data[offset + 5:offset + 9])
                return width, height
            offset += length + 2
    raise ValueError("invalid or unsupported PNG/JPEG screenshot")


def generated_files(root):
    catalog = json.loads((root / "docs/product-features.json").read_text())
    features = catalog["features"]
    if not features or len({f["area"] for f in features}) != len(features):
        raise ValueError("feature areas must be nonempty and unique")
    for feature in features:
        for field in ("area", "details", "title", "summary"):
            value = feature[field]
            if not isinstance(value, str) or not value.strip() or "\n" in value:
                raise ValueError(f"invalid feature field: {field}")

    def cell(value):
        return value.replace("|", "&#124;")

    readme = ["| Area | What you get |", "|---|---|"]
    readme += [f'| {cell(f["area"])} | {cell(f["details"])} |' for f in features]
    site = ['    <div class="cols">']
    site += [
        f'      <div class="col"><h3>{html.escape(f["title"])}</h3>'
        f'<p>{inline(f["summary"])}</p></div>' for f in features
    ]
    site += ["    </div>", '    <h3 class="gallery-title">Git history and live sessions</h3>',
             '    <div class="shots">']
    files = {}
    for shot in catalog["screenshots"]:
        name = shot["file"]
        if Path(name).name != name or not name.endswith((".png", ".jpg")):
            raise ValueError("screenshots must be PNG/JPEG filenames in docs/media")
        data = (root / "docs/media" / name).read_bytes()
        width, height = image_size(data)
        if not width or not height:
            raise ValueError(f"empty screenshot: {name}")
        alt = html.escape(shot["alt"], quote=True)
        label = html.escape(shot["label"])
        readme += ["", '<p align="center">',
                   f'  <img src="docs/media/{name}" alt="{alt}" width="880">', "</p>"]
        escaped_name = html.escape(name, quote=True)
        site += [f'      <a href="{escaped_name}"><img src="{escaped_name}" loading="lazy" '
                 f'decoding="async" alt="{alt}" width="{width}" height="{height}">'
                 f'<span>{label}</span></a>']
        files[root / "docs/site" / name] = data
    site += ["    </div>"]
    for name, content in (("README.md", "\n".join(readme)),
                          ("docs/site/index.html", "\n".join(site))):
        path = root / name
        files[path] = replace_section(path.read_text(), content).encode()
    return files


def sync(root, check=False):
    # Validate and render everything before writing any output.
    files = generated_files(root)
    changed = [path for path, data in files.items()
               if not path.exists() or path.read_bytes() != data]
    for path in changed:
        print(f'{"Out of date" if check else "Updated"}: {path.relative_to(root)}')
        if not check:
            path.write_bytes(files[path])
    return bool(check and changed)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="fail on stale generated files")
    args = parser.parse_args()
    try:
        sys.exit(sync(ROOT, args.check))
    except (ValueError, KeyError, OSError) as error:
        print(f"Cannot synchronize product docs: {error}", file=sys.stderr)
        sys.exit(1)
