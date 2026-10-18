#!/usr/bin/env python3
"""Actual Fcitx5 input-method-v2 composition in an isolated labwc session."""
import json
import os
import re
import shlex
import signal
import subprocess
import time
from pathlib import Path


def _command(args, environment, check=True):
    return subprocess.run(args, env=environment, check=check, text=True,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10)


def _wait(check, description):
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        if check():
            return
        time.sleep(.1)
    raise RuntimeError("Timed out: " + description)


def prepare_labwc(start, environment, evidence):
    """Run after labwc, before launching the candidate with DISPLAY removed.

    Only Fcitx's engine/XKB discovery receives this labwc's private Xwayland
    display. The application must connect natively through WAYLAND_DISPLAY.
    start must track and terminate the daemon's process group on cleanup.
    """
    root = Path(evidence)
    if (environment.get("CHDA_NATIVE_ISOLATED") != "1" or
            not environment.get("WAYLAND_DISPLAY") or
            environment.get("XDG_CURRENT_DESKTOP") != "labwc"):
        raise RuntimeError("Fcitx preparation requires the isolated labwc session")
    for key in ["GTK_IM_MODULE", "QT_IM_MODULE", "XMODIFIERS"]:
        environment.pop(key, None)
    environment["XDG_SESSION_TYPE"] = "wayland"
    protocols = _command(["wayland-info"], environment).stdout
    (root / "ime-wayland-protocols.txt").write_text(protocols)
    for name in ["zwp_input_method_manager_v2", "zwp_text_input_manager_v3"]:
        if name not in protocols:
            raise RuntimeError("Missing required compositor protocol: " + name)
    xwayland = []

    def private_display():
        text = (root / "labwc.log").read_text()
        found = re.search(r"xwayland is running on display (:[0-9]+)", text)
        if found:
            xwayland[:] = [found.group(1)]
        return bool(xwayland)

    _wait(private_display, "private labwc Xwayland engine-discovery display")
    environment["DISPLAY"] = xwayland[0]
    config = Path(environment["XDG_CONFIG_HOME"]) / "fcitx5"
    config.mkdir(parents=True, exist_ok=True)
    for name in ["profile", "config"]:
        if (config / name).exists():
            raise RuntimeError("Refusing to overwrite existing Fcitx settings")
    (config / "profile").write_text(
        "[Groups/0]\nName=Default\nDefault Layout=us\nDefaultIM=hangul\n"
        "[Groups/0/Items/0]\nName=keyboard-us\nLayout=\n"
        "[Groups/0/Items/1]\nName=hangul\nLayout=\n[GroupOrder]\n0=Default\n")
    (config / "config").write_text(
        "[Hotkey/TriggerKeys]\n0=Control+space\n[Behavior]\nActiveByDefault=False\n")
    child = start(["fcitx5", "-D", "-r", "--verbose", "key_trace=5"],
                  "fcitx5", process_env=environment)

    def owns_bus_name():
        if child.poll() is not None:
            raise RuntimeError("Fcitx5 exited before candidate launch")
        # fcitx5-remote returns state 0 until an app input context exists.
        return "boolean true" in _command([
            "dbus-send", "--session", "--print-reply", "--dest=org.freedesktop.DBus",
            "/org/freedesktop/DBus", "org.freedesktop.DBus.NameHasOwner",
            "string:org.fcitx.Fcitx5"], environment).stdout

    _wait(owns_bus_name, "Fcitx5 session-bus owner")
    (root / "ime-wayland-environment.json").write_text(json.dumps({
        "transport": "Fcitx5 input-method-v2 to native Wayland text-input-v3",
        "engine": "hangul", "compositor": "labwc", "unicodePaste": False,
        "applicationEnvironment": "DISPLAY must be removed before app launch",
        "engineDiscoveryEnvironment": "private compositor Xwayland display",
        "scope": "isolated session; actual candidate execution not implied"
    }, indent=2) + "\n")
    return child


def exercise(driver):
    """Compose with one virtual keyboard, preserving keymap during the grab."""
    if driver.input_backend != "wayland" or driver.options.compositor != "labwc":
        raise RuntimeError("This helper requires native labwc Wayland input")
    target = driver.evidence / "ime-wayland-committed.txt"
    if target.exists():
        raise RuntimeError("Refusing to reuse IME commit marker")
    driver.key("ctrl+c")
    # An app input context must exist and be focused before selecting Hangul.
    _command(["fcitx5-remote", "-s", "hangul"], os.environ)
    selected = _command(["fcitx5-remote", "-n"], os.environ).stdout.strip()
    if selected != "hangul":
        raise RuntimeError("Focused candidate did not select the Hangul engine: " + selected)
    _command(["fcitx5-remote", "-c"], os.environ)
    path = shlex.quote(str(target))
    keys = ["wtype", "-s", "500", "-d", "30", "printf '%s' '",
            "-M", "ctrl", "-k", "space", "-m", "ctrl", "r", "-s", "1800",
            "k", "-s", "1800", "skek", "-s", "1800",
            "-M", "ctrl", "-k", "space", "-m", "ctrl",
            "' | tee " + path, "-k", "Return", "-s", "300"]
    frames = []
    log = (driver.evidence / "ime-wayland-keyboard.log").open("w")
    typing = subprocess.Popen(keys, env=os.environ, stdout=log, stderr=log,
                              start_new_session=True)
    try:
        driver.record("actual-persistent-keyboard-ime", engine="hangul",
                      asciiKeys="r,k,s,k,e,k", unicodePaste=False,
                      transport="input-method-v2/text-input-v3")
        time.sleep(1.1)
        frames.append(driver.screenshot("ime-wayland-01-preedit-consonant"))
        time.sleep(1.4)
        frames.append(driver.screenshot("ime-wayland-02-preedit-syllable"))
        time.sleep(1.5)
        frames.append(driver.screenshot("ime-wayland-03-preedit-final"))
        if typing.wait(timeout=15) != 0:
            raise RuntimeError("Persistent IME virtual keyboard failed")
        _wait(lambda: target.exists() and target.stat().st_size, "actual IME commit file")
        actual = target.read_text()
        frames.append(driver.screenshot("ime-wayland-04-committed"))
        result = {"keys": ["Ctrl+Space", "r", "k", "s", "k", "e", "k", "Ctrl+Space"],
                  "transport": "actual Fcitx5 Hangul input-method-v2/text-input-v3 key composition",
                  "expected": "가나다", "actual": actual,
                  "commitStatus": "passed" if actual == "가나다" else "failed",
                  "preeditVisualStatus": "pending independent PNG review; inspect pre-commit composition",
                  "screenshots": [str(p) for p in frames], "unicodePaste": False,
                  "scope": "ARM/tool preparation only" if driver.options.preparation_probe else "actual product candidate"}
        (driver.evidence / "ime-wayland-result.json").write_text(
            json.dumps(result, ensure_ascii=False, indent=2) + "\n")
        driver.record("actual-wayland-ime-composition", **result)
        if actual != "가나다":
            raise AssertionError("Actual Wayland IME key commit mismatch: " + repr(actual))
        return result
    finally:
        if typing.poll() is None:
            os.killpg(typing.pid, signal.SIGTERM)
            try:
                typing.wait(timeout=3)
            except subprocess.TimeoutExpired:
                os.killpg(typing.pid, signal.SIGKILL)
                typing.wait(timeout=3)
        log.close()
