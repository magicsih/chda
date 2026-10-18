#!/usr/bin/env python3
"""Full candidate journey; pixel review remains an independent final gate."""
import json
import os
import re
import shlex
import subprocess
import time
from pathlib import Path

from native_driver import run
from native_visual import background_bounds, ocr_words, phrase_center
import mcp_fixture


def wait(check, description, timeout=15):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = check()
        if value:
            return value
        time.sleep(.15)
    raise AssertionError("Timed out: " + description)


def session_locations(rows):
    """MCP worktree is the actual row cwd; repository is its display name."""
    locations = [{key: row[key] for key in ("repository", "worktree", "branch")} for row in rows]
    # Sorting retains duplicates: opening two shells in one cwd must survive.
    return sorted(locations, key=lambda row: json.dumps(row, sort_keys=True))


def saved_pane_locations(saved):
    """Compare saved pane cwd/repo, excluding new ids and runtime agent state."""
    locations = []

    def visit(node):
        if node["kind"] == "pane":
            locations.append({key: node.get(key) for key in ("cwd", "repo")})
        elif node["kind"] == "split":
            visit(node["first"])
            visit(node["second"])
        else:
            raise AssertionError("Unknown saved terminal node: " + str(node))

    for window in saved["windows"]:
        for tab in window["tabs"]:
            if "root" in tab:
                visit(tab["root"])
    if not locations:
        raise AssertionError("Saved session has no terminal panes")
    return sorted(locations, key=lambda row: json.dumps(row, sort_keys=True))


class Journey:
    def __init__(self, driver, args, root, repo, context):
        self.driver, self.args, self.root, self.repo = driver, args, root, repo
        self.context = context
        self.steps = []
        self.calibration = None
        self.csd_maximized = False
        self.notification_log = root / "notifications.jsonl"
        self.window_mode = "CSD" if args.backend == "wayland" and args.compositor == "mutter" else "SSD"
        self.report = {"scope": "tool preparation only" if args.preparation_probe else "product candidate automation",
                       "candidateSha": args.candidate_sha, "artifactSha256": os.environ["CHDA_ARTIFACT_SHA256"],
                       "backend": args.backend, "compositor": args.compositor if args.backend == "wayland" else "Openbox",
                       "round": args.round, "decoration": self.window_mode,
                       "automationStatus": "running", "fullRoundStatus": "not-run",
                       "imeCompositionStatus": "not-run; Unicode paste is not IME coverage",
                       "visualReview": "pending independent review of actual PNGs", "steps": self.steps}

    def save(self):
        (self.root / "full-round-result.json").write_text(json.dumps(self.report, indent=2) + "\n")

    def step(self, name, action):
        item = {"step": name, "status": "running"}
        self.steps.append(item)
        self.save()
        try:
            value = action()
            item.update(status="passed", evidence=value)
            return value
        except Exception as error:
            item.update(status="failed", error=str(error))
            self.report.update(automationStatus="failed", fullRoundStatus="failed")
            self.save()
            raise
        finally:
            self.save()

    def mcp(self, name, arguments=None, pane=None, suffix=None, cwd=None):
        result = mcp_fixture.call(self.args.launcher, cwd or self.repo, name, arguments or {}, pane=pane)
        path = self.root / ("mcp-" + (suffix or name) + ".json")
        path.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
        return result["reply"]["result"].get("structuredContent", {})

    def sessions(self, suffix):
        return self.mcp("list_sessions", suffix=suffix).get("sessions", [])

    def x11_windows(self):
        response = subprocess.run(["xdotool", "search", "--onlyvisible", "--class", self.args.app_id],
                                  text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        return response.stdout.splitlines()

    def x11_geometry(self, window):
        # xdotool getwindowgeometry double-adds a reparented client's relative
        # offset on the tested Openbox/Xvfb pair. xwininfo reports actual root
        # coordinates; retain that raw readback as well.
        values = run(["xwininfo", "-id", window, "-stats"])
        fields = {"x":r"Absolute upper-left X:\s*(-?\d+)", "y":r"Absolute upper-left Y:\s*(-?\d+)",
                  "width":r"Width:\s*(\d+)", "height":r"Height:\s*(\d+)"}
        return {key:int(re.search(pattern, values).group(1)) for key,pattern in fields.items()}

    def observe(self, name, obscured=False):
        image = self.driver.screenshot(name)
        # An open palette can cover almost all terminal pixels. X11 exposes
        # actual native geometry directly. CSD still exposes its surrounding
        # 5px frame, enough to measure bounds without certifying body rendering.
        pixels = (None if obscured and self.args.backend == "x11" else
                  background_bounds(image, minimum_pixels=4000 if obscured else 20000))
        if self.args.backend == "x11":
            geometry = self.x11_geometry(self.driver.options.window)
            response = run(["xprop", "-id", self.driver.options.window, "_NET_FRAME_EXTENTS"])
            extents = [int(value) for value in re.findall(r"\d+", response.split("=", 1)[-1])]
            if len(extents) != 4:
                raise AssertionError("Missing actual X11 frame extents")
            left, right, top, bottom = extents
            geometry["frame"] = {"x": geometry["x"]-left, "y": geometry["y"]-top,
                                 "width": geometry["width"]+left+right,
                                 "height": geometry["height"]+top+bottom}
            geometry["extents"] = extents
        elif self.window_mode == "CSD":
            # Configured exact-color outer CSD inset surrounds the app. Its
            # one-pixel border is blended, so include that border in the bounds.
            if self.csd_maximized:
                geometry = self.x11_geometry(self.driver.options.window)
                if abs(pixels["x"]+pixels["width"]-geometry["x"]-geometry["width"]) > 5:
                    raise AssertionError("Maximized native window did not reach nested output right edge in actual PNG")
            else:
                geometry = {"x": pixels["x"]-1, "y": pixels["y"]-1,
                            "width": pixels["width"]+2, "height": pixels["height"]+2}
            geometry["frame"] = dict(geometry)
        else:
            if self.calibration is None:
                raise AssertionError("labwc geometry must first be calibrated by explicit setup bindings")
            offset = self.calibration
            geometry = {"x": pixels["x"]-offset["x"], "y": pixels["y"]-offset["y"],
                        "width": pixels["width"]-offset["width"], "height": pixels["height"]-offset["height"]}
            geometry["frame"] = {"x": geometry["x"]-1, "y": geometry["y"]-27,
                                 "width": geometry["width"]+2, "height": geometry["height"]+28}
        geometry.update(pixels=pixels, screenshot=str(image), decoration=self.window_mode,
                        terminalObscured=obscured)
        (self.root / (name + "-observed.json")).write_text(json.dumps(geometry, indent=2) + "\n")
        return geometry

    def setup_geometry(self, width, height, name, obscured=False):
        """Setup may use compositor commands; resize acceptance uses actual drags."""
        if self.args.backend == "x11":
            run(["xdotool", "windowsize", self.driver.options.window, str(width), str(height)])
            run(["xdotool", "windowmove", self.driver.options.window, "100", "120"])
        elif self.window_mode == "SSD":
            self.driver.key("super+F7" if width == 960 else "super+F8")
            time.sleep(.5)
            pixels = background_bounds(self.driver.screenshot(name + "-calibration"))
            # labwc MoveTo addresses the SSD frame. With the private theme's
            # 1px border and 26px title the client begins at (81,127).
            # The plain foot fixture rounds its client to whole cells.
            actual_width = pixels["width"] if self.args.preparation_probe else width
            actual_height = pixels["height"] if self.args.preparation_probe else height
            self.calibration = {"x": pixels["x"]-81, "y": pixels["y"]-127,
                                "width": pixels["width"]-actual_width, "height": pixels["height"]-actual_height}
        else:
            before = self.observe(name + "-before-setup", obscured=obscured)
            frame = before["frame"]
            self.driver.drag(frame["x"]+frame["width"]-3, frame["y"]+frame["height"]-3,
                             frame["x"]+width-3, frame["y"]+height-3)
        time.sleep(.4)
        geometry = self.observe(name, obscured=obscured)
        tolerance = 30 if self.args.preparation_probe else 8
        if abs(geometry["width"]-width) > tolerance or abs(geometry["height"]-height) > tolerance:
            raise AssertionError("Requested setup size not observed: " + str(geometry))
        self.driver.record("setup-window-size", requested=[width, height], geometry=geometry,
                           acceptance="setup only; actual edge checks are separate")
        return geometry

    def control(self, name, geometry):
        frame = geometry["frame"]
        right = frame["x"]+frame["width"]
        if self.window_mode == "CSD":
            # Current product title bar: 5px inset, 8px right padding,
            # 32px buttons and 35px title height. Save the current PNG and
            # derived coordinate, then require the actual state change.
            x = right-29-32*{"close": 0, "maximize": 1, "minimize": 2}[name]
            y = frame["y"]+22
        elif self.args.backend == "x11":
            height = geometry["extents"][2]
            size = height-4
            x = right-3-size//2-size*{"close": 0, "maximize": 1, "minimize": 2}[name]
            y = frame["y"]+height//2
        else:
            x = right-14-26*{"close": 0, "maximize": 1, "minimize": 2}[name]
            y = frame["y"]+14
        self.driver.record("current-render-derived-control-coordinate", control=name, at=[x,y],
                           screenshot=geometry["screenshot"], decoration=self.window_mode,
                           visualInspection="pending independent inspection")
        self.driver.click(x, y)
        time.sleep(.6)
        if self.window_mode == "CSD" and name == "maximize":
            self.csd_maximized = not self.csd_maximized

    def restore_minimized(self):
        if self.args.backend == "x11":
            run(["xdotool", "windowmap", self.driver.options.window])
            run(["xdotool", "windowactivate", "--sync", self.driver.options.window])
        elif self.args.compositor == "labwc":
            run(["wlrctl", "toplevel", "focus", "app_id:"+self.args.app_id])
        else:
            # Openbox parent configuration deliberately has no Alt+Tab grab,
            # so this reaches the nested Mutter seat and its native window.
            self.driver.key("alt+Tab")
        time.sleep(.6)

    def window_checks(self):
        for width, height, name in [(960,640,"window-default"), (640,420,"window-narrow")]:
            self.setup_geometry(width, height, name)
        geometry = self.setup_geometry(960,640,"window-controls-baseline")
        self.control("maximize", geometry)
        maximized = self.observe("window-maximized")
        if any(maximized[axis] <= geometry[axis]+100 for axis in ("width", "height")):
            raise AssertionError("Actual maximize button did not increase window width and height")
        self.control("maximize", maximized)
        restored = self.observe("window-restored")
        if any(abs(restored[axis]-geometry[axis]) > 8 for axis in ("width", "height")):
            raise AssertionError("Actual maximize/restore button did not restore size")
        self.control("minimize", restored)
        minimized = False
        for attempt in range(8):
            image = self.driver.screenshot("window-minimized-" + str(attempt))
            try:
                background_bounds(image)
            except AssertionError:
                minimized = True
                break
            time.sleep(.3)
        if not minimized:
            raise AssertionError("Actual minimize button left the candidate background visible after animation settled")
        self.restore_minimized()
        unminimized = self.observe("window-unminimized")
        if any(abs(unminimized[axis]-restored[axis]) > 8 for axis in ("width", "height")):
            raise AssertionError("Actual minimize/restore changed the window size")
        self.driver.marker("after-unminimize.txt", "printf UNMINIMIZE_OK")
        geometry = self.observe("window-before-move")
        frame = geometry["frame"]
        title_y = frame["y"]+22 if self.window_mode == "CSD" else frame["y"]+12
        self.driver.drag(frame["x"]+frame["width"]//2, title_y,
                         frame["x"]+frame["width"]//2+45, title_y+35)
        moved = self.observe("window-after-move")
        if moved["x"]-geometry["x"] < 20 or moved["y"]-geometry["y"] < 15:
            raise AssertionError("Actual title bar drag did not move the candidate right and down")
        for edge in ["top", "bottom", "left", "right", "top-left", "top-right", "bottom-left", "bottom-right"]:
            before = self.observe("resize-"+edge+"-before")
            box = before["frame"]
            inset = 2 if self.window_mode == "CSD" else 0
            x = box["x"]+inset if "left" in edge else box["x"]+box["width"]-inset-1 if "right" in edge else box["x"]+box["width"]//2
            y = box["y"]+inset if "top" in edge else box["y"]+box["height"]-inset-1 if "bottom" in edge else box["y"]+box["height"]//2
            dx = 22 if "left" in edge else -22 if "right" in edge else 0
            dy = 22 if "top" in edge else -22 if "bottom" in edge else 0
            self.driver.drag(x, y, x+dx, y+dy)
            after = self.observe("resize-"+edge+"-after")
            if dx and before["width"]-after["width"] < 12:
                raise AssertionError("Actual "+edge+" inward drag did not reduce width")
            if dy and before["height"]-after["height"] < 12:
                raise AssertionError("Actual "+edge+" inward drag did not reduce height")
            for leading, trailing, coordinate, size in [("left", "right", "x", "width"),
                                                        ("top", "bottom", "y", "height")]:
                old_start = before["frame"][coordinate]
                new_start = after["frame"][coordinate]
                old_end = old_start+before["frame"][size]
                new_end = new_start+after["frame"][size]
                if leading in edge:
                    if new_start-old_start < 12 or abs(new_end-old_end) > 8:
                        raise AssertionError("Actual "+edge+" resize did not move its leading edge inward with the opposite edge anchored")
                elif trailing in edge:
                    if old_end-new_end < 12 or abs(new_start-old_start) > 8:
                        raise AssertionError("Actual "+edge+" resize did not move its trailing edge inward with the opposite edge anchored")
            self.driver.record("actual-window-edge-resize-passed", edge=edge, before=before, after=after)
        self.setup_geometry(960,640,"window-after-resize")
        return {"decoration": self.window_mode, "resizeEdges": 8,
                "screenshots": "window-*.png and resize-*.png", "visualReview": "pending"}

    def folder_checks(self):
        selected = self.root / "home/picked repo with spaces 世界"
        selected.mkdir()
        run(["git", "init", str(selected)])
        run(["git", "-C", str(selected), "symbolic-ref", "HEAD", "refs/heads/main"])
        run(["git", "-C", str(selected), "-c", "user.name=Validation Fixture", "-c",
             "user.email=validation@example.invalid", "commit", "--allow-empty", "-m", "test: picker fixture"])
        self.driver.key("ctrl+shift+o")
        time.sleep(1)
        opened = self.driver.screenshot("folder-picker-cancel-before")
        words = " ".join(word["text"] for word in ocr_words(opened))
        if "Cancel" not in words:
            raise AssertionError("Actual folder picker Cancel control not visible")
        self.driver.click(*phrase_center(opened, "Cancel"))
        self.driver.marker("after-picker-cancel.txt", "printf PICKER_CANCEL_OK")
        self.driver.key("ctrl+shift+o")
        time.sleep(.7)
        self.driver.key("alt+Home")
        time.sleep(.8)
        image = self.driver.screenshot("folder-picker-home-list")
        self.driver.click(*phrase_center(image, "picked repo with spaces"))
        time.sleep(.3)
        image = self.driver.screenshot("folder-picker-selected-location")
        self.driver.click(*phrase_center(image, "Add repository"))
        config = self.root / "config/chda/config.toml"
        wait(lambda: str(selected) in config.read_text(), "selected repository persisted by real picker")
        self.driver.screenshot("folder-picker-added-sidebar")
        # Registering a repo does not promise that a shell is immediately opened.
        # Use the real candidate MCP open_tab to open it, then validate actual
        # focused terminal input and the sidebar's persisted repository.
        self.mcp("open_tab", {"path": str(selected)}, suffix="picker-open-terminal", cwd=self.repo)
        time.sleep(.6)
        cwd = self.driver.marker("picked-shell-cwd.txt", "pwd -P").strip()
        if Path(cwd) != selected.resolve():
            raise AssertionError("Selected folder terminal cwd mismatch: "+cwd)
        self.driver.key("ctrl+shift+o")
        time.sleep(.5)
        self.driver.screenshot("folder-picker-reopened")
        self.driver.key("Escape")
        self.driver.marker("after-picker-reopen.txt", "printf PICKER_REOPEN_OK")
        self.driver.key("ctrl+shift+w")
        self.selected_repo = selected
        # Closing a branch's final tab intentionally leaves that branch empty.
        # Explicitly prepare a terminal for the subsequent hook/notification
        # checks instead of assuming an unrelated branch is selected for us.
        self.mcp("open_tab", {"path": str(self.repo)}, suffix="after-picker-main-terminal")
        from native_visual import wait_for_rendered
        wait_for_rendered(self.driver, "CHDAVALIDATION", "after-picker-main-readiness")
        resumed = self.driver.marker("after-picker-main-cwd.txt", "pwd -P").strip()
        if Path(resumed) != self.repo.resolve():
            raise AssertionError("Next scenario terminal cwd differs from the prepared main repository")
        return {"selected": str(selected), "config": str(config), "cancelAndRetry": "passed",
                "terminal": "actual terminal cwd after candidate MCP open_tab fixture",
                "nextScenarioTerminalCwd": resumed}

    def delivered(self):
        if not self.notification_log.exists():
            return []
        return [json.loads(line) for line in self.notification_log.read_text().splitlines()
                if line.strip() and json.loads(line).get("event") == "delivered"]

    def status(self, pane, value, suffix):
        return self.mcp("report_status", {"agent": "claude", "status": value}, pane=int(pane), suffix=suffix)

    def notify(self, pane, suffix):
        before = len(self.delivered())
        self.status(pane, "working", suffix+"-working")
        self.status(pane, "waiting_for_input", suffix+"-waiting")
        wait(lambda: len(self.delivered()) > before, "actual Notify for "+suffix)
        notification = self.delivered()[-1]
        if "default" not in notification["actions"][::2]:
            raise AssertionError("Actual notification lacks default action")
        return notification

    def activate(self, notification):
        run(["dbus-send", "--session", "--print-reply", "--dest=org.freedesktop.Notifications",
             "/org/freedesktop/Notifications", "org.magicsih.ChdaValidation.Activate",
             "uint32:"+str(notification["id"]), "string:default"])
        time.sleep(.6)

    def close_secondary(self):
        # Minimize original first only after recording a notification target;
        # start the new window before minimizing via its actual control below.
        before_sessions = self.sessions("before-secondary")
        before_panes = {row["pane"] for row in before_sessions}
        original_xid = self.driver.options.window
        original_geometry = self.observe("original-before-secondary")
        self.driver.key("ctrl+shift+n")
        time.sleep(.8)
        if self.args.backend == "x11":
            candidates = [item for item in self.x11_windows() if item != original_xid]
            if len(candidates) != 1:
                raise AssertionError("Expected one secondary X11 window")
            self.driver.options.window = candidates[0]
        self.mcp("open_tab", {"path":str(self.repo)}, suffix="secondary-repo-terminal")
        time.sleep(.5)
        secondary_pane = self.driver.marker("pane-secondary-window.txt", 'printf "%s" "$CHDA_PANE_ID"').strip()
        if int(secondary_pane) in before_panes:
            raise AssertionError("New window did not get an independent pane")
        if self.window_mode == "CSD":
            # Native Mutter maximization is setup for an unambiguous new
            # topmost window's output bounds. Button maximization was already
            # independently exercised above with an actual pointer click.
            self.driver.key("alt+F10")
            self.csd_maximized = True
            time.sleep(.6)
        elif self.args.backend != "x11":
            # Both native windows initially share the centered 960x640 bounds.
            # Keep the original at that same size before CtrlShiftN so the PNG
            # bounds remain those of the actual topmost candidate window.
            self.driver.record("overlapping-secondary-geometry", expected=original_geometry,
                               prerequisite="original centered 960x640 before new window")
        secondary = self.observe("secondary-before-drawn-close")
        self.control("close", secondary)
        self.csd_maximized = False
        if self.args.backend == "x11":
            wait(lambda: self.driver.options.window not in self.x11_windows(), "drawn secondary close")
            self.driver.options.window = original_xid
            run(["xdotool", "windowactivate", "--sync", original_xid])
        remaining = wait(lambda: self.sessions("after-secondary-close") if
                         all(row["pane"] != int(secondary_pane) for row in self.sessions("after-secondary-check")) else None,
                         "secondary pane closed")
        return {"secondaryPane": secondary_pane, "remainingPanes": [row["pane"] for row in remaining]}

    def notification_checks(self):
        original = self.driver.marker("notification-pane-original.txt", 'printf "%s" "$CHDA_PANE_ID"').strip()
        self.driver.key("ctrl+shift+t")
        time.sleep(.5)
        second = self.driver.marker("notification-pane-second.txt", 'printf "%s" "$CHDA_PANE_ID"').strip()
        if second == original:
            raise AssertionError("Notification source panes are not distinct")
        first_notice = self.notify(original, "notification-first")
        second_notice = self.notify(second, "notification-second")
        self.activate(first_notice)
        focused = self.driver.marker("notification-click-original.txt", 'printf "%s" "$CHDA_PANE_ID"').strip()
        if focused != original:
            raise AssertionError("First notification did not select its exact pane")
        self.activate(second_notice)
        focused = self.driver.marker("notification-click-second.txt", 'printf "%s" "$CHDA_PANE_ID"').strip()
        if focused != second:
            raise AssertionError("Second notification did not select its exact pane")
        self.driver.screenshot("notification-exact-pane")
        # Match default new window size before creating the most recent window.
        self.setup_geometry(960,640,"notification-before-secondary")
        if self.args.backend == "x11":
            run(["xdotool", "windowmove", self.driver.options.window, "240", "180"])
        elif self.args.compositor == "labwc":
            self.driver.key("super+F9")
        else:
            # The product uses Bounds::centered(960x640) in a 1280x900 nested
            # output; move actual CSD title bar to that observed target.
            observed = self.observe("notification-center-before")
            frame = observed["frame"]
            values = self.x11_geometry(self.driver.options.window)
            target_x, target_y = values["x"]+160, values["y"]+130
            self.driver.drag(frame["x"]+frame["width"]//2,frame["y"]+22,
                             target_x+frame["width"]//2,target_y+22)
        closed = self.close_secondary()
        after_close_notice = self.notify(original, "notification-after-window-close")
        self.activate(after_close_notice)
        focused = self.driver.marker("notification-click-after-window-close.txt", 'printf "%s" "$CHDA_PANE_ID"').strip()
        if focused != original:
            raise AssertionError("Notification after newest window close lost its exact owner pane")
        self.status(original, "done", "notification-original-done")
        self.status(second, "done", "notification-second-done")
        provider = self.context["provider"]
        os.killpg(provider.pid, __import__("signal").SIGTERM)
        provider.wait(timeout=5)
        before = time.monotonic()
        self.status(original, "working", "without-service-working")
        self.status(original, "waiting_for_input", "without-service-waiting")
        value = self.driver.marker("notification-no-service-responsive.txt", "printf NO_SERVICE_OK")
        elapsed = time.monotonic()-before
        if value != "NO_SERVICE_OK" or elapsed > 12:
            raise AssertionError("Absent notification service blocked terminal input")
        self.status(original, "done", "without-service-done")
        self.status(original, "ended", "fixture-original-ended")
        self.status(second, "ended", "fixture-second-ended")
        return {"originalPane":original, "secondPane":second, "closedWindow":closed,
                "serviceAbsentElapsedSeconds":elapsed,
                "trigger": "actual packaged MCP CLI report_status input fixture, not a Claude CLI session"}

    def restart(self):
        before = self.sessions("restart-before")
        if not before:
            raise AssertionError("No terminal state exists before restart")
        saved_path = self.root / "data/chda/session.json"
        saved_before = json.loads(saved_path.read_text())
        (self.root / "restart-saved-before.json").write_text(json.dumps(saved_before, indent=2)+"\n")
        self.driver.key("ctrl+shift+q")
        wait(lambda: self.context["app"].poll() is not None, "actual Quit exits package", timeout=20)
        retired = self.context["retire_runtime"]()
        (self.root / "retired-runtime.json").write_text(json.dumps(retired, indent=2)+"\n")
        self.context["launch_again"]()
        if self.args.backend == "x11":
            self.driver.options.window = wait(lambda: self.x11_windows(), "restarted product X11 window")[-1]
            run(["xdotool", "windowactivate", "--sync", self.driver.options.window])
        from native_visual import wait_for_rendered
        wait_for_rendered(self.driver, "CHDAVALIDATION", "restart-readiness")
        after = self.sessions("restart-after")
        saved_after = json.loads(saved_path.read_text())
        (self.root / "restart-saved-after.json").write_text(json.dumps(saved_after, indent=2)+"\n")
        locations = {"mcpBefore": session_locations(before), "mcpAfter": session_locations(after),
                     "savedBefore": saved_pane_locations(saved_before),
                     "savedAfter": saved_pane_locations(saved_after),
                     "mcpWorktreeMeaning": "actual session row cwd; repository is its display name",
                     "excluded": "pane ids, timestamps, statuses, active tab/window and runtime agent fields"}
        (self.root / "restart-location-signatures.json").write_text(json.dumps(locations, indent=2)+"\n")
        if len(after) != len(before):
            raise AssertionError("Restart changed restored session count")
        if locations["mcpBefore"] != locations["mcpAfter"]:
            raise AssertionError("Restart changed restored repository, worktree cwd or branch multiset")
        if locations["savedBefore"] != locations["savedAfter"]:
            raise AssertionError("Restart changed saved pane cwd or repository path multiset")
        if str(self.selected_repo) not in (self.root / "config/chda/config.toml").read_text():
            raise AssertionError("Selected repository lost across restart")
        self.driver.marker("restart-shell-usable.txt", "printf RESTART_OK")
        self.driver.screenshot("restart-restored")
        hooks = self.step("installed hook command and fresh MCP after relaunch",
                          lambda: self.acceptance.hooks_checks("after-restart"))
        from native_helpers import exercise as helper_checks
        helpers = self.step("installed helper parallel CLI, streaming MCP and direct termination",
                            lambda: helper_checks(self))
        return {"beforeSessionCount":len(before),"afterSessionCount":len(after),
                "locationSignatures": locations,
                "selectedRepositoryPreserved":str(self.selected_repo), "hooksAfterRestart": hooks,
                "helpersAfterRestart": helpers}

    def execute(self):
        from native_acceptance import Acceptance
        self.acceptance = Acceptance(self)
        try:
            self.step("repository terminal fixture through actual packaged MCP", lambda:
                      self.mcp("open_tab", {"path":str(self.repo)}, suffix="initial-repo-terminal"))
            time.sleep(.5)
            if self.args.backend == "x11":
                from native_ime import exercise
                self.step("native IBus XIM preedit and commit", lambda: exercise(self.driver))
                self.report["imeCompositionStatus"] = "native IBus commit passed; independent preedit PNG review pending"
            self.step("terminal Ctrl+C, CtrlShiftT/W/V", self.driver.terminal_checks)
            self.step("actual selection and CtrlShiftC exact clipboard", self.driver.copy_selection)
            self.step("foreground child cwd without OSC7 and recovery after CtrlC", self.acceptance.cwd_checks)
            self.step("actual worktree creation sheet, branch and terminal", self.acceptance.worktree_checks)
            self.step("native terminal Ctrl+click URI dispatch", self.acceptance.link_checks)
            from native_commands import exercise as commands_about
            self.step("actual Commands button and About prompt at default/narrow sizes",
                      lambda: commands_about(self))
            self.step("default and narrow window, actual buttons, title move, eight resize areas", self.window_checks)
            self.step("real folder chooser cancel, select and retry", self.folder_checks)
            self.step("actual installed hook command and pane-specific MCP status",
                      lambda: self.acceptance.hooks_checks("before-restart"))
            self.step("notification exact pane, newest window close, absent service", self.notification_checks)
            self.step("normal quit, same HOME restart and restored state", self.restart)
            self.report.update(automationStatus="passed", fullRoundStatus="pending-IME-and-independent-visual-review")
        finally:
            self.save()
        return self.report
