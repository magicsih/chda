#!/usr/bin/env python3
"""Primitives for an explicitly isolated, native x86_64 chda candidate session.

This driver does not certify native visual acceptance. Screenshots and manual
checks remain pending until independently reviewed against the recorded digest.
"""
import argparse
import hashlib
import json
import os
import platform
import shlex
import subprocess
import sys
import time
from pathlib import Path


def run(args, *, text=None, capture=True, env=None):
    try:
        return subprocess.run(args, input=text, text=True, check=True,
                              stdout=subprocess.PIPE if capture else None,
                              stderr=subprocess.PIPE if capture else None, env=env, timeout=20).stdout
    except subprocess.CalledProcessError as error:
        raise RuntimeError(f"{args[0]} exited {error.returncode}: {error.stderr or ''}") from error


def sha256_file(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


class Driver:
    def __init__(self, options):
        if os.environ.get("CHDA_NATIVE_ISOLATED") != "1":
            raise RuntimeError("Set CHDA_NATIVE_ISOLATED=1 only inside the disposable test session")
        if platform.system() != "Linux" or (platform.machine() != "x86_64" and not options.preparation_probe):
            raise RuntimeError("Product native rounds require actual Linux x86_64")
        for field in ("CHDA_CANDIDATE_SHA", "CHDA_ARTIFACT_SHA256"):
            if not os.environ.get(field):
                raise RuntimeError(f"Missing {field}")
        artifact = Path(options.artifact).resolve()
        actual_digest = sha256_file(artifact)
        if actual_digest != os.environ["CHDA_ARTIFACT_SHA256"]:
            raise RuntimeError("The actual package digest differs from the approved candidate")
        self.options = options
        self.input_backend = getattr(options, "input_backend", None) or options.backend
        self.capture_backend = getattr(options, "capture_backend", None) or self.input_backend
        self.evidence = Path(options.evidence).resolve()
        self.evidence.mkdir(parents=True, exist_ok=True)
        self.steps = self.evidence / "driver-events.jsonl"
        if self.input_backend == "x11":
            if not os.environ.get("DISPLAY") or not options.window:
                raise RuntimeError("X11 requires the isolated DISPLAY and observed --window")
        if options.backend == "wayland" and not os.environ.get("WAYLAND_DISPLAY"):
            raise RuntimeError("Wayland requires the explicit session WAYLAND_DISPLAY")
        if options.backend == "wayland" and options.compositor == "sway" and not os.environ.get("SWAYSOCK"):
            raise RuntimeError("sway requires the explicit session SWAYSOCK")

    def record(self, action, **fields):
        with self.steps.open("a") as file:
            file.write(json.dumps({"time": time.time(), "backend": self.options.backend,
                                   "candidate": os.environ["CHDA_CANDIDATE_SHA"],
                                   "inputBackend": self.input_backend, "captureBackend": self.capture_backend,
                                   "artifact": os.environ["CHDA_ARTIFACT_SHA256"],
                                   "scope": "preparation-only" if self.options.preparation_probe else "product-candidate",
                                   "action": action, **fields}) + "\n")

    def key(self, key):
        if self.input_backend == "x11":
            # Preserve the compositor's current focus. Re-activating the main
            # window here steals focus from real portal dialogs/new windows
            # and would invalidate notification-routing observations.
            run(["xdotool", "key", "--clearmodifiers", key])
        else:
            # wtype's device needs to remain alive while its seat is initialized.
            parts = key.split("+")
            modifiers = {"ctrl": "ctrl", "shift": "shift", "alt": "alt", "super": "logo"}
            command = ["wtype", "-s", "100"]
            for modifier in parts[:-1]:
                command.extend(["-M", modifiers[modifier.lower()]])
            command.extend(["-k", parts[-1]])
            for modifier in reversed(parts[:-1]):
                command.extend(["-m", modifiers[modifier.lower()]])
            run(command)
        self.record("key", key=key)
        time.sleep(.15)

    def text(self, text):
        if self.input_backend == "x11":
            run(["xdotool", "type", "--clearmodifiers", "--delay", "5", "--", text])
        else:
            run(["wtype", "-s", "300", "-d", "5", "-"], text=text)
        self.record("typed-input", length=len(text))

    def command(self, text):
        self.text(text)
        self.key("Return")

    def marker(self, name, command):
        marker = self.evidence / name
        if marker.exists():
            raise RuntimeError(f"Refusing to reuse marker {marker}")
        self.command(command + " | tee " + shlex.quote(str(marker)))
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and (not marker.exists() or marker.stat().st_size == 0):
            time.sleep(.1)
        if not marker.exists() or marker.stat().st_size == 0:
            self.screenshot("failed-marker-" + marker.stem)
            raise AssertionError(f"Actual terminal input did not produce {name}")
        self.record("marker-observed", name=name, value=marker.read_text())
        return marker.read_text()

    def screenshot(self, name):
        path = self.evidence / (name + ".png")
        time.sleep(.35)
        if self.capture_backend == "x11":
            run(["scrot", str(path)])
            geometry = run(["xdotool", "getwindowgeometry", "--shell", self.options.window])
            focus = {}
            for label, query in [("active", "getactivewindow"), ("input", "getwindowfocus")]:
                observed = subprocess.run(["xdotool", query], text=True,
                                          stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                focus[label] = {"window": observed.stdout.strip(), "returncode": observed.returncode}
            (self.evidence / (name + "-x11-focus.json")).write_text(json.dumps(focus, indent=2) + "\n")
        else:
            run(["grim", str(path)])
            geometry = (run(["swaymsg", "-t", "get_tree"])
                        if self.options.compositor == "sway"
                        else run(["wlrctl", "toplevel", "list"]))
        # Read the real PNG dimensions rather than assuming a compositor's
        # headless default. Virtual-pointer absolute coordinates use these.
        with path.open("rb") as file:
            header = file.read(24)
        if header[:8] != b"\x89PNG\r\n\x1a\n":
            raise AssertionError("Capture did not produce a PNG")
        import struct
        self.options.screen_width, self.options.screen_height = struct.unpack(">II", header[16:24])
        (self.evidence / (name + "-geometry.txt")).write_text(geometry)
        self.record("screenshot-captured", path=str(path),
                    sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                    visualAcceptance="pending independent pixel inspection")
        return path

    def clipboard_write(self, text):
        clipboard_env = self.clipboard_environment()
        command = (["xclip", "-selection", "clipboard"] if clipboard_env is not None or self.options.backend == "x11"
                   else ["wl-copy", "--type", "text/plain;charset=utf-8"])
        # Clipboard tools may daemonize to own the selection. Capture is disabled
        # so inherited stdout pipes cannot hold this subprocess open.
        run(command, text=text, capture=False, env=clipboard_env)

    def clipboard_read(self):
        clipboard_env = self.clipboard_environment()
        return run(["xclip", "-selection", "clipboard", "-o", "-target", "UTF8_STRING"]
                   if clipboard_env is not None or self.options.backend == "x11"
                   else ["wl-paste", "--no-newline"], env=clipboard_env)

    def clipboard_environment(self):
        if os.environ.get("CHDA_CLIPBOARD_DISPLAY"):
            env = dict(os.environ)
            env["DISPLAY"] = env["CHDA_CLIPBOARD_DISPLAY"]
            env["XAUTHORITY"] = env["CHDA_CLIPBOARD_XAUTHORITY"]
            return env
        return None

    def move_pointer(self, x, y):
        """Send real XTest motion to the isolated X server, including Mutter's seat."""
        if self.input_backend != "x11":
            raise RuntimeError("XTest pointer movement requires the isolated X11 input backend")
        import ctypes
        x, y = round(x), round(y)
        x11 = ctypes.CDLL("libX11.so.6")
        xtst = ctypes.CDLL("libXtst.so.6")
        display_type = ctypes.c_void_p
        int_pointer = ctypes.POINTER(ctypes.c_int)
        x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
        x11.XOpenDisplay.restype = display_type
        x11.XDefaultScreen.argtypes = [display_type]
        x11.XDefaultScreen.restype = ctypes.c_int
        x11.XSync.argtypes = [display_type, ctypes.c_int]
        x11.XSync.restype = ctypes.c_int
        x11.XCloseDisplay.argtypes = [display_type]
        x11.XCloseDisplay.restype = ctypes.c_int
        xtst.XTestQueryExtension.argtypes = [display_type, int_pointer, int_pointer,
                                           int_pointer, int_pointer]
        xtst.XTestQueryExtension.restype = ctypes.c_int
        xtst.XTestFakeMotionEvent.argtypes = [display_type, ctypes.c_int, ctypes.c_int,
                                            ctypes.c_int, ctypes.c_ulong]
        xtst.XTestFakeMotionEvent.restype = ctypes.c_int
        display = x11.XOpenDisplay(os.environ["DISPLAY"].encode())
        if not display:
            raise RuntimeError("Could not open the isolated X11 display for pointer input")
        try:
            event, error, major, minor = (ctypes.c_int() for _ in range(4))
            if not xtst.XTestQueryExtension(display, ctypes.byref(event), ctypes.byref(error),
                                           ctypes.byref(major), ctypes.byref(minor)):
                raise RuntimeError("The isolated X11 server does not provide XTest")
            if not xtst.XTestFakeMotionEvent(display, x11.XDefaultScreen(display), x, y, 0):
                raise RuntimeError("XTest rejected the isolated pointer movement")
            x11.XSync(display, False)
        finally:
            if x11.XCloseDisplay(display) != 0:
                raise RuntimeError("Could not close the isolated pointer display")
        self.record("actual-pointer-move", at=[x, y], transport="XTestFakeMotionEvent + XSync")

    def drag(self, x1, y1, x2, y2):
        if self.input_backend == "x11":
            self.move_pointer(x1, y1)
            time.sleep(.15)
            run(["xdotool", "mousedown", "1"])
            time.sleep(.1)
            for index in range(1, 21):
                self.move_pointer(x1 + (x2-x1)*index/20, y1 + (y2-y1)*index/20)
                time.sleep(.02)
            time.sleep(.1)
            run(["xdotool", "mouseup", "1"])
        else:
            helper = Path(getattr(self.options, "drag_helper", None) or Path(__file__).with_name("wayland-drag"))
            run([str(helper), str(x1), str(y1), str(x2), str(y2),
                 str(self.options.screen_width), str(self.options.screen_height)])
        self.record("actual-pointer-drag", start=[x1, y1], end=[x2, y2])

    def click(self, x, y):
        if self.input_backend == "x11":
            self.move_pointer(x, y)
            time.sleep(.15)
            run(["xdotool", "click", "1"])
        else:
            helper = Path(getattr(self.options, "drag_helper", None) or Path(__file__).with_name("wayland-drag"))
            run([str(helper), str(x), str(y), str(x), str(y),
                 str(self.options.screen_width), str(self.options.screen_height)])
        self.record("actual-pointer-click", at=[x, y])

    def terminal_checks(self):
        original = self.marker("pane-original.txt", 'printf "%s" "$CHDA_PANE_ID"')
        if not original.strip():
            raise AssertionError("Candidate shell lacks CHDA_PANE_ID")
        self.key("ctrl+shift+t")
        time.sleep(1)
        new = self.marker("pane-new.txt", 'printf "%s" "$CHDA_PANE_ID"')
        if new == original:
            raise AssertionError("Ctrl+Shift+T did not create a new pane")
        self.key("ctrl+shift+w")
        restored = self.marker("pane-after-close.txt", 'printf "%s" "$CHDA_PANE_ID"')
        if restored != original:
            raise AssertionError("Ctrl+Shift+W did not restore the original pane")
        leak = self.evidence / "interrupt-leaked.txt"
        self.command("sleep 30; printf BAD > " + shlex.quote(str(leak)))
        time.sleep(.3)
        self.key("ctrl+c")
        self.marker("interrupt-ok.txt", "printf INTERRUPT_OK")
        if leak.exists():
            raise AssertionError("Ctrl+C did not interrupt the foreground command")
        paste = self.evidence / "paste-ok.txt"
        self.clipboard_write("printf PASTE_OK > " + shlex.quote(str(paste)))
        self.key("ctrl+shift+v")
        self.key("Return")
        deadline = time.monotonic() + 10
        while not paste.exists() and time.monotonic() < deadline:
            time.sleep(.1)
        if not paste.exists() or paste.read_text() != "PASTE_OK":
            raise AssertionError("Ctrl+Shift+V failed to execute the clipboard command")
        self.screenshot("terminal-controls")
        self.record("terminal-core-checks-passed")

    def copy_selection(self):
        from native_visual import wait_for_sentinel
        sentinel = "CHDACOPYSENTINEL"
        # Keep the target text inside the terminal, away from the sidebar's
        # resize hit area at the first column.
        # The complete marker appears only in output, never the typed command.
        # A queued input echo is not evidence that the output has been painted.
        self.command("printf '\\033[2J\\033[H    %s%s%s\\n' CHDA COPY SENTINEL")
        coordinates = wait_for_sentinel(self, sentinel, "copy-before-selection")
        self.drag(*coordinates)
        self.screenshot("copy-selection")
        self.key("ctrl+shift+c")
        copied = self.clipboard_read()
        (self.evidence / "copied-selection.txt").write_text(copied)
        if copied != sentinel:
            raise AssertionError("Actual selection/clipboard differs: " + repr(copied))
        self.record("actual-selection-copy-passed", sentinel=sentinel, coordinates=coordinates)
        self.key("Escape")
        # Readline binds ESC followed by the next command's first `p` to
        # Meta-p history search. Start the next fixture at a clean prompt.
        # The exact selection/clipboard assertion above remains mandatory.
        self.key("ctrl+c")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--backend", choices=["x11", "wayland"], required=True)
    parser.add_argument("--preparation-probe", action="store_true", help="Tool fixture only; never product acceptance")
    parser.add_argument("--window", help="Observed isolated X11 candidate window id")
    parser.add_argument("--compositor", choices=["sway", "labwc", "mutter"], default="labwc")
    parser.add_argument("--input-backend", choices=["x11", "wayland"])
    parser.add_argument("--capture-backend", choices=["x11", "wayland"])
    parser.add_argument("--evidence", required=True)
    parser.add_argument("--artifact", required=True, help="Actual final package file whose digest is verified")
    parser.add_argument("--screen-width", type=int, default=1440)
    parser.add_argument("--screen-height", type=int, default=1000)
    sub = parser.add_subparsers(dest="action", required=True)
    sub.add_parser("terminal-checks")
    shot = sub.add_parser("screenshot")
    shot.add_argument("name")
    text = sub.add_parser("text")
    text.add_argument("value")
    key = sub.add_parser("key")
    key.add_argument("value")
    drag = sub.add_parser("drag")
    drag.add_argument("coordinates", nargs=4, type=int)
    click = sub.add_parser("click")
    click.add_argument("coordinates", nargs=2, type=int)
    sub.add_parser("clipboard-read")
    args = parser.parse_args()
    driver = Driver(args)
    if args.action == "terminal-checks":
        driver.terminal_checks()
    elif args.action == "screenshot":
        driver.screenshot(args.name)
    elif args.action == "text":
        driver.text(args.value)
    elif args.action == "key":
        driver.key(args.value)
    elif args.action == "drag":
        driver.drag(*args.coordinates)
    elif args.action == "click":
        driver.click(*args.coordinates)
    elif args.action == "clipboard-read":
        print(driver.clipboard_read())


if __name__ == "__main__":
    main()
