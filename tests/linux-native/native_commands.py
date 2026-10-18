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
from native_visual import bright_text_ocr, ocr_words, phrase_matches, prompt_region, tooltip_region


REPOSITORY = "https://github.com/magicsih/chda"
DESCRIPTION = "Terminal with a git worktree sidebar and a live status board for coding agents."


def _plain(text):
    return re.sub(r"[^a-z0-9]", "", text.lower())


def _ocr(path, expected):
    words = ocr_words(path)
    Path(path).with_suffix(".ocr.json").write_text(json.dumps(words, indent=2) + "\n")
    text = " ".join(word["text"] for word in words if word["text"].strip())
    normalized = _plain(text)
    processing = "original PNG"
    if any(_plain(value) not in normalized for value in expected):
        # Keep dim shortcut text as well as the brighter command labels.
        words, contrast_path = bright_text_ocr(path, 120)
        text = " ".join(word["text"] for word in words if word["text"].strip())
        normalized = _plain(text)
        processing = str(contrast_path)
    Path(path).with_suffix(".text-check.json").write_text(json.dumps({
        "source": str(path), "processing": processing, "expected": expected,
        "observed": text, "passed": all(_plain(value) in normalized for value in expected),
    }, indent=2) + "\n")
    for value in expected:
        if _plain(value) not in normalized:
            raise AssertionError("Actual rendered text missing " + repr(value) + ": " + text)
    return text


def _about(path, version):
    region = prompt_region(path)
    if re.search(r"\bchda\s+" + re.escape(version) + r"(?=\s|$)", region["text"]) is None:
        raise AssertionError("Actual prompt version does not match the packaged version: " + region["text"])
    if re.findall(r"https?://\S+", region["text"]) != [REPOSITORY]:
        raise AssertionError("Actual prompt repository URI differs: " + region["text"])
    expected = [DESCRIPTION, "MIT license"]
    normalized = _plain(region["text"])
    for value in expected:
        if _plain(value) not in normalized:
            raise AssertionError("Actual prompt text missing " + repr(value) + ": " + region["text"])
    region["controls"] = {}
    for label in ["Close", "Open repository"]:
        matches = phrase_matches(region["words"], label, minimum_confidence=70)
        if len(matches) != 1:
            raise AssertionError("Expected one actual prompt control " + repr(label))
        at = matches[0]["at"]
        left, top, right, bottom = region["bounds"]
        if not (left <= at[0] < right and top <= at[1] < bottom):
            raise AssertionError("Prompt control is outside the detected panel")
        region["controls"][label] = matches[0]
    return region


def _commands(journey, label, width, height, keyboard=False):
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
    journey.driver.move_pointer(x, y)
    time.sleep(.9)
    hover = journey.driver.screenshot(label + "-commands-hover")
    try:
        _ocr(hover, ["Commands", "ctrl-shift-p"])
    except AssertionError:
        # Single-line segmentation avoids reading the tooltip as part of the
        # terminal behind it. Keep both original failed OCR and region evidence.
        tooltip = tooltip_region(hover, [x, y])
        if (re.search(r"\bCommands\b", tooltip["text"]) is None
                or re.search(r"\bctrl-shift-p\b", tooltip["text"]) is None):
            raise AssertionError("Actual Commands tooltip differs: " + tooltip["text"])
    if keyboard:
        journey.driver.key("ctrl+shift+p")
    else:
        journey.driver.click(x, y)
    opened = journey.driver.screenshot(label + "-commands-open")
    _ocr(opened, ["New tab ctrl-shift-t"])
    if width == 640:
        # Rename tab is the eleventh standard action, beyond this narrow
        # viewport. Actual key input must scroll it into view and keep it
        # visible when the open palette grows and shrinks.
        for _ in range(10):
            journey.driver.key("Down")
        scrolled = journey.driver.screenshot(label + "-commands-scrolled")
        _ocr(scrolled, ["Rename tab"])
        journey.setup_geometry(960, 640, label + "-palette-grown")
        _ocr(journey.driver.screenshot(label + "-selection-after-grow"), ["Rename tab"])
        journey.setup_geometry(width, height, label + "-palette-shrunk")
        _ocr(journey.driver.screenshot(label + "-selection-after-shrink"), ["Rename tab"])
        for _ in range(10):
            journey.driver.key("Up")
        _ocr(journey.driver.screenshot(label + "-commands-returned-top"), ["New tab"])
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
        region = _about(about, version)
        journey.driver.click(*region["controls"]["Close"]["at"])
        journey.driver.marker(label + "-after-close.txt", "printf ABOUT_CLOSE_OK")
        journey.driver.screenshot(label + "-after-close")
        result["dialogs"].append({"size": [width, height], "screenshot": str(about), "prompt": region})
    output, mimeapps, previous = _register_https(journey)
    try:
        about = _commands(journey, "commands-open-repository", 960, 640, keyboard=True)
        region = _about(about, version)
        journey.driver.click(*region["controls"]["Open repository"]["at"])
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
