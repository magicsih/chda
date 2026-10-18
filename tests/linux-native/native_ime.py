#!/usr/bin/env python3
"""Actual IBus/XIM input; commit and preedit pixel evidence stay separate."""
import json
import os
import shlex
import subprocess
import time
from pathlib import Path


def prepare_x11(start, environment, evidence):
    """Call after Xvfb/Openbox, BEFORE launching the actual candidate.

    start has native_session.start's signature and tracks the daemon group.
    This modifies only the fresh isolated session environment/settings.
    """
    if environment.get("CHDA_NATIVE_ISOLATED") != "1" or not environment.get("DISPLAY"):
        raise RuntimeError("IBus preparation requires the isolated X11 session")
    environment["XMODIFIERS"] = "@im=ibus"
    # Set the engine's explicit start mode only in the disposable HOME/XDG.
    code = "from gi.repository import Gio; s=Gio.Settings.new('org.freedesktop.ibus.engine.hangul'); s.set_string('initial-input-mode','latin'); Gio.Settings.sync()"
    subprocess.run(["python3", "-c", code], env=environment, check=True, timeout=10,
                   stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    child = start(["ibus-daemon", "--panel=disable", "--emoji-extension=disable", "--xim", "--replace"],
                  "ibus-xim", process_env=environment)
    deadline = time.monotonic()+20
    while time.monotonic()<deadline:
        if child.poll() is not None:
            raise RuntimeError("IBus daemon exited before candidate launch")
        try:
            selected = subprocess.run(["ibus","engine","hangul"],env=environment,check=False,
                                      text=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=4)
            if selected.returncode == 0:
                current = subprocess.run(["ibus","engine"],env=environment,check=True,
                                         text=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=4).stdout.strip()
                if current == "hangul":
                    Path(evidence,"ime-engine.json").write_text(json.dumps({"engine":current,
                        "transport":"XIM", "XMODIFIERS":"@im=ibus", "initialMode":"latin",
                        "scope":"isolated development session"},indent=2)+"\n")
                    return child
        except subprocess.TimeoutExpired:
            pass
        time.sleep(.2)
    raise RuntimeError("IBus Hangul engine did not become ready")


def exercise(driver):
    """Use actual key events, without Unicode text injection or clipboard paste."""
    target = driver.evidence / "ime-committed.txt"
    if target.exists():
        raise RuntimeError("Refusing to reuse IME marker")
    driver.key("ctrl+c")
    driver.text("printf '%s' '")
    driver.key("shift+space")
    driver.text("r")
    consonant=driver.screenshot("ime-01-preedit-consonant")
    driver.text("k")
    syllable=driver.screenshot("ime-02-preedit-syllable")
    driver.text("skek")
    final=driver.screenshot("ime-03-preedit-final")
    driver.key("shift+space")
    driver.text("' | tee "+shlex.quote(str(target)))
    driver.key("Return")
    deadline=time.monotonic()+10
    while time.monotonic()<deadline and (not target.exists() or not target.stat().st_size):
        time.sleep(.1)
    actual=target.read_text() if target.exists() else None
    committed=driver.screenshot("ime-04-committed")
    result={"keys":["Shift+Space","r","k","s","k","e","k","Shift+Space"],
            "transport":"actual IBus Hangul/XIM key composition", "expected":"가나다", "actual":actual,
            "commitStatus":"passed" if actual=="가나다" else "failed",
            "preeditVisualStatus":"pending independent PNG review; must show composition before commit",
            "screenshots":[str(consonant),str(syllable),str(final),str(committed)],
            "scope":"preparation only" if driver.options.preparation_probe else "actual product candidate"}
    (driver.evidence/"ime-result.json").write_text(json.dumps(result,ensure_ascii=False,indent=2)+"\n")
    driver.record("actual-ime-composition",**result)
    if actual!="가나다":
        raise AssertionError("Actual IME key composition commit mismatch: "+repr(actual))
    return result
