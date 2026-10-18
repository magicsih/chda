#!/usr/bin/env python3
"""Actual Linux Commands button and About prompt; independent PNG review pending."""
import configparser
import io
import json
import os
import re
import time
from pathlib import Path

from native_driver import run
from native_journey import wait
from native_visual import ocr_words, phrase_center


REPOSITORY = "https://github.com/magicsih/chda"


def _plain(text):
    return re.sub(r"[^a-z0-9]", "", text.lower())


def _ocr(path, expected):
    words = ocr_words(path)
    Path(path).with_suffix(".ocr.json").write_text(json.dumps(words, indent=2) + "\n")
    text = " ".join(word["text"] for word in words if word["text"].strip())
    normalized = _plain(text)
    for value in expected:
        if _plain(value) not in normalized:
            raise AssertionError("Actual rendered text missing " + repr(value) + ": " + text)
    return text


def _commands(journey, label, width, height):
    geometry = journey.setup_geometry(width, height, label + "-geometry")
    frame = geometry["frame"]
    if journey.window_mode == "CSD":
        # Same actual titlebar as drawn controls: outer5, padding8, three32px
        # controls, 8px flex gap, and the Commands button's own current width.
        x = frame["x"] + frame["width"] - 131
        y = frame["y"] + 22
    else:
        x = geometry["x"] + geometry["width"] - 23
        y = geometry["y"] + 17
    journey.driver.record("current-render-derived-commands-coordinate", at=[x, y],
                          screenshot=geometry["screenshot"], decoration=journey.window_mode,
                          visualInspection="pending actual hover/open PNG review")
    if journey.driver.input_backend != "x11":
        raise RuntimeError("Commands hover helper requires XTest or native Mutter parent input")
    run(["xdotool", "mousemove", "--sync", str(x), str(y)])
    time.sleep(.9)
    hover = journey.driver.screenshot(label + "-commands-hover")
    _ocr(hover, ["Commands", "Ctrl", "Shift", "P"])
    journey.driver.click(x, y)
    opened = journey.driver.screenshot(label + "-commands-open")
    _ocr(opened, ["New tab", "Ctrl", "Shift", "T"])
    # Typing must reach the palette's actual focused input.
    journey.driver.text("About chda")
    filtered = journey.driver.screenshot(label + "-about-filtered")
    _ocr(filtered, ["About chda"])
    journey.driver.key("Return")
    time.sleep(.3)
    return journey.driver.screenshot(label + "-about-dialog")


def _register_https(journey):
    if os.environ.get("CHDA_NATIVE_ISOLATED") != "1":
        raise RuntimeError("HTTPS handler requires the explicitly isolated session")
    root = journey.root
    output = root / "about-repository-uri.json"
    handler = root / "about-repository-handler.py"
    if output.exists() or handler.exists():
        raise RuntimeError("Refusing to reuse About repository evidence")
    handler.write_text("import json,sys\nfrom pathlib import Path\n"
                       "expected=" + repr(REPOSITORY) + "\n"
                       "actual=sys.argv[1]\n"
                       "if actual!=expected:raise SystemExit('Unexpected repository URI')\n"
                       "Path(" + repr(str(output)) + ").write_text(json.dumps({'uri':actual,"
                       "'externalNetwork':False,'trigger':'native desktop URI dispatch'}))\n")
    applications = root / "data/applications"
    applications.mkdir(exist_ok=True)
    desktop = applications / "chda-validation-about.desktop"
    # xdg-open's generic desktop launcher splits arguments without removing
    # Desktop Entry quotes. CI uses /evidence/session; refuse unsafe fixture
    # paths and use one plain argument, which works in both generic and GIO.
    if re.fullmatch(r"[A-Za-z0-9_./-]+", str(handler)) is None:
        raise RuntimeError("URI handler fixture requires the CI's safe /evidence/session path")
    desktop.write_text("[Desktop Entry]\nType=Application\nName=chda About URI fixture\n"
                       "Exec=/usr/bin/python3 " + str(handler) + " %u\nNoDisplay=true\n"
                       "MimeType=x-scheme-handler/https;\n")
    mimeapps = root / "config/mimeapps.list"
    previous = mimeapps.read_bytes() if mimeapps.exists() else None
    config = configparser.ConfigParser(interpolation=None)
    config.optionxform = str
    if previous is not None:
        config.read_string(previous.decode())
    if not config.has_section("Default Applications"):
        config.add_section("Default Applications")
    config["Default Applications"]["x-scheme-handler/https"] = desktop.name
    buffer = io.StringIO()
    config.write(buffer, space_around_delimiters=False)
    mimeapps.write_text(buffer.getvalue())
    selected = run(["xdg-mime", "query", "default", "x-scheme-handler/https"]).strip()
    if selected != desktop.name:
        raise AssertionError("Actual isolated HTTPS desktop preference was not selected")
    return output, mimeapps, previous


def exercise(journey):
    """Use actual rendered controls at default/narrow sizes, then exact URI handoff."""
    version_output = run([journey.args.launcher, "--version"]).strip()
    version = re.search(r"\b[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?", version_output)
    if version is None:
        raise AssertionError("Packaged launcher version is unreadable: " + version_output)
    version = version.group()
    result = {"packagedVersion": version_output, "dialogs": [],
              "visualReview": "pending independent default/narrow Commands and About PNG inspection"}
    for width, height, label in [(960, 640, "commands-default"), (640, 420, "commands-narrow")]:
        about = _commands(journey, label, width, height)
        text = _ocr(about, ["chda " + version, "MIT license", REPOSITORY])
        journey.driver.click(*phrase_center(about, "Close"))
        journey.driver.marker(label + "-after-close.txt", "printf ABOUT_CLOSE_OK")
        journey.driver.screenshot(label + "-after-close")
        result["dialogs"].append({"size": [width, height], "screenshot": str(about), "ocr": text})
    output, mimeapps, previous = _register_https(journey)
    try:
        about = _commands(journey, "commands-open-repository", 960, 640)
        _ocr(about, ["chda " + version, "MIT license", REPOSITORY])
        journey.driver.click(*phrase_center(about, "Open repository"))
        wait(output.exists, "About native HTTPS handler exact repository URI")
        actual = json.loads(output.read_text())
        if actual.get("uri") != REPOSITORY or actual.get("externalNetwork") is not False:
            raise AssertionError("About repository button did not dispatch the exact expected URI")
        journey.driver.marker("about-after-repository.txt", "printf ABOUT_REPOSITORY_OK")
        journey.driver.screenshot("about-after-repository")
        result["repository"] = actual
        return result
    finally:
        if previous is None:
            mimeapps.unlink()
        else:
            mimeapps.write_bytes(previous)
