#!/usr/bin/env python3
"""Isolated native candidate stages with explicit automation/visual evidence."""
import argparse
import hashlib
import json
import os
import platform
import signal
import shlex
import shutil
import subprocess
import tempfile
import time
from pathlib import Path
from types import SimpleNamespace

from native_driver import Driver, sha256_file


def wait_until(check, description, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if check():
            return
        time.sleep(.1)
    raise RuntimeError("Timed out: " + description)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--launcher", required=True, help="Actual AppImage or extracted portable launcher")
    parser.add_argument("--artifact", required=True, help="Final AppImage or archive for digest readback")
    parser.add_argument("--candidate-sha", required=True)
    parser.add_argument("--backend", choices=["x11", "wayland"], required=True)
    parser.add_argument("--compositor", choices=["labwc", "mutter"], default="labwc")
    parser.add_argument("--app-id", default="com.magicsih.chda")
    parser.add_argument("--round", type=int, choices=[1, 2, 3], required=True)
    parser.add_argument("--evidence", required=True)
    parser.add_argument("--preparation-probe", action="store_true")
    parser.add_argument("--stage", choices=["startup", "clipboard", "terminal", "windows", "full", "ime-wayland"], default="startup")
    args = parser.parse_args()
    if args.stage == "ime-wayland" and (args.backend != "wayland" or args.compositor != "labwc"):
        raise SystemExit("Native Wayland IME supplement requires labwc and actual Wayland input")
    if args.stage == "full" and args.backend == "wayland" and args.compositor != "mutter":
        raise SystemExit("Full Wayland journey uses Mutter for actual product CSD; labwc remains startup/clipboard/window SSD coverage")
    if platform.system() != "Linux" or (platform.machine() != "x86_64" and not args.preparation_probe):
        raise SystemExit("Product execution requires native Linux x86_64")
    root = Path(args.evidence).resolve()
    if root.exists() and any(root.iterdir()):
        raise SystemExit("Evidence directory must be new or empty")
    root.mkdir(parents=True, exist_ok=True)
    launcher = Path(args.launcher).resolve()
    artifact = Path(args.artifact).resolve()
    args.launcher, args.artifact = str(launcher), str(artifact)
    digest = sha256_file(artifact)
    home = root / "home"
    runtime = root / "runtime"
    repo = root / "repo with spaces 世界"
    for directory in [home, runtime, repo, root / "config/chda", root / "config/ghostty", root / "data", root / "cache"]:
        directory.mkdir(parents=True, exist_ok=True)
    runtime.chmod(0o700)
    (root / "config/chda/config.toml").write_text(
        "repos = [" + json.dumps(str(repo), ensure_ascii=False) + "]\n"
        'notifications = true\nrestore-session = true\ndefault-action = "terminal"\n')
    (home / ".bashrc").write_text("PS1='CHDA_VALIDATION> '\n")
    (home / ".bash_profile").write_text('. "$HOME/.bashrc"\n')
    (root / "config/ghostty/config").write_text(
        "font-family = DejaVu Sans Mono\nfont-size = 16\nbackground = #202429\nforeground = #f0f0f0\nshell-integration = none\n")
    env = dict(os.environ)
    env.update(HOME=str(home), XDG_CONFIG_HOME=str(root / "config"),
               XDG_DATA_HOME=str(root / "data"), XDG_CACHE_HOME=str(root / "cache"),
               XDG_RUNTIME_DIR=str(runtime), XDG_CURRENT_DESKTOP=("GNOME" if args.compositor == "mutter" else "labwc") if args.backend == "wayland" else "Openbox",
               SHELL="/bin/bash", LANG="C.UTF-8", CHDA_NATIVE_ISOLATED="1",
               CHDA_CANDIDATE_SHA=args.candidate_sha, CHDA_ARTIFACT_SHA256=digest,
               APPIMAGE_EXTRACT_AND_RUN="1")
    for name in ["DISPLAY", "WAYLAND_DISPLAY", "SWAYSOCK", "DBUS_SESSION_BUS_ADDRESS", "PROMPT_COMMAND", "BASH_ENV", "ENV"]:
        env.pop(name, None)
    children = []
    handles = []
    runtime_temporary_root = Path(tempfile.mkdtemp(prefix="chda-native-runtime-"))
    launch_temporary_paths = []
    steps = []
    result = {"scope": "ARM/tool preparation only" if args.preparation_probe else "product candidate startup only",
              "candidateSha": args.candidate_sha, "artifactSha256": digest,
              "backend": args.backend, "round": args.round, "architecture": platform.machine(),
              "requestedStage": args.stage,
              "launcher": str(launcher), "artifact": str(artifact), "steps": steps,
              "fullRoundStatus": "not-run"}

    def start(command, name, *, stdout=None, cwd=None, process_env=None):
        log = (root / (name + ".log")).open("w")
        handles.append(log)
        child = subprocess.Popen(command, cwd=cwd, env=process_env if process_env is not None else env,
                                 stdout=stdout if stdout is not None else log,
                                 stderr=log, text=True, start_new_session=True)
        children.append(child)
        return child

    def command(command, *, check=True):
        return subprocess.run(command, env=env, text=True, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, check=check)

    try:
        command(["git", "init", str(repo)])
        command(["git", "-C", str(repo), "symbolic-ref", "HEAD", "refs/heads/main"])
        command(["git", "-C", str(repo), "-c", "user.name=Validation Fixture",
                 "-c", "user.email=validation@example.invalid", "commit", "--allow-empty", "-m", "test: create isolated fixture"])
        bus = start(["dbus-daemon", "--session", "--nofork", "--print-address=1"], "bus", stdout=subprocess.PIPE)
        env["DBUS_SESSION_BUS_ADDRESS"] = bus.stdout.readline().strip()
        if not env["DBUS_SESSION_BUS_ADDRESS"]:
            raise RuntimeError("Isolated session bus did not start")
        steps.append({"step": "new HOME/XDG/repository/bus", "status": "passed"})
        parent_window = None
        if args.backend == "x11" or args.compositor == "mutter":
            xserver = start(["Xvfb", "-displayfd", "1", "-screen", "0", "1440x1000x24", "-nolisten", "tcp", "-noreset"], "xvfb", stdout=subprocess.PIPE)
            number = xserver.stdout.readline().strip()
            env["DISPLAY"] = ":" + number
            wait_until(lambda: command(["xdpyinfo"], check=False).returncode == 0, "Xvfb")
            openbox_config = root / "openbox.xml"
            # Avoid an X11 parent Alt+Tab/Alt+F4 grab in the nested session.
            openbox_config.write_text('<openbox_config xmlns="http://openbox.org/3.4/rc"><theme><name>Clearlooks</name><titleLayout>NLIMC</titleLayout></theme><focus><focusNew>yes</focusNew></focus><keyboard/><mouse>'
                '<context name="Titlebar"><mousebind button="Left" action="Drag"><action name="Move"/></mousebind></context>'
                '<context name="Maximize"><mousebind button="Left" action="Click"><action name="ToggleMaximize"/></mousebind></context>'
                '<context name="Iconify"><mousebind button="Left" action="Click"><action name="Iconify"/></mousebind></context>'
                '<context name="Close"><mousebind button="Left" action="Click"><action name="Close"/></mousebind></context>'
                '<context name="Left Right Top Bottom TLCorner TRCorner BLCorner BRCorner"><mousebind button="Left" action="Drag"><action name="Resize"/></mousebind></context>'
                '</mouse></openbox_config>')
            start(["openbox", "--config-file", str(openbox_config)], "openbox")
        if args.backend == "wayland" and args.compositor == "labwc":
            env.update(WLR_BACKENDS="headless", WLR_RENDERER="pixman", WLR_LIBINPUT_NO_DEVICES="1")
            config = root / "labwc.xml"
            config.write_text('<?xml version="1.0"?><labwc_config><core><decoration>client</decoration></core><focus><followMouse>no</followMouse></focus><keyboard>'
                '<keybind key="W-F7"><action name="ResizeTo" width="960" height="640"/><action name="MoveTo" x="80" y="100"/></keybind>'
                '<keybind key="W-F8"><action name="ResizeTo" width="640" height="420"/><action name="MoveTo" x="80" y="100"/></keybind>'
                '<keybind key="W-F9"><action name="MoveTo" x="160" y="130"/></keybind></keyboard></labwc_config>')
            theme = root / "config/labwc"
            theme.mkdir()
            (theme / "themerc-override").write_text("window.button.width: 26\nwindow.button.height: 26\nwindow.button.spacing: 0\nwindow.titlebar.padding.width: 0\nwindow.titlebar.padding.height: 0\nborder.width: 1\n")
            start(["labwc", "-c", str(config), "-d"], "labwc")
            wait_until(lambda: any(p.is_socket() for p in runtime.glob("wayland-*")), "Wayland socket")
            env["WAYLAND_DISPLAY"] = next(p.name for p in runtime.glob("wayland-*") if p.is_socket())
            if args.stage in ["windows", "full"]:
                command(["wlr-randr", "--output", "HEADLESS-1", "--custom-mode", "1440x1000@60Hz"])
                (root / "labwc-output.json").write_text(command(["wlr-randr", "--json"]).stdout)
        if args.backend == "wayland" and args.compositor == "mutter":
            env.update(LIBGL_ALWAYS_SOFTWARE="1", GALLIUM_DRIVER="llvmpipe", NO_AT_BRIDGE="1",
                       MUTTER_DEBUG_DUMMY_MODE_SPECS="1280x900", MUTTER_DEBUG_DUMMY_MONITOR_SCALES="1")
            mutter = start(["mutter", "--wayland", "--nested"], "mutter")
            wait_until(lambda: any(p.is_socket() for p in runtime.glob("wayland-*")), "nested Mutter Wayland socket")
            env["WAYLAND_DISPLAY"] = next(p.name for p in runtime.glob("wayland-*") if p.is_socket())
            parents = []
            def observed_parent():
                response = command(["xdotool", "search", "--onlyvisible", "--pid", str(mutter.pid)], check=False)
                parents[:] = response.stdout.splitlines()
                return bool(parents)
            wait_until(observed_parent, "Mutter nested X11 parent")
            parent_window = parents[-1]
            command(["xdotool", "windowsize", parent_window, "1280", "900"])
            command(["xdotool", "windowmove", parent_window, "50", "50"])
            command(["xdotool", "windowactivate", "--sync", parent_window])
            protocols = command(["wayland-info"]).stdout
            (root / "wayland-protocols.txt").write_text(protocols)
            if "zxdg_decoration_manager_v1" in protocols:
                raise RuntimeError("Mutter CSD session unexpectedly exposes decoration protocol")
            for line in command(["pgrep", "-a", "Xwayland"], check=False).stdout.splitlines():
                parts = shlex.split(line)
                if "-auth" in parts:
                    auth = parts[parts.index("-auth") + 1]
                    if str(runtime) in auth:
                        env["CHDA_CLIPBOARD_DISPLAY"] = next(part for part in parts if part.startswith(":"))
                        env["CHDA_CLIPBOARD_XAUTHORITY"] = auth
            result.update(inputBackend="x11-parent", captureBackend="x11-parent", applicationBackend="native-wayland",
                          decorationProtocol="absent", compositor="Mutter nested")
        steps.append({"step": "isolated compositor", "status": "passed"})
        if args.stage == "full" and args.backend == "x11":
            from native_ime import prepare_x11
            prepare_x11(start, env, root)
        provider = Path(__file__).with_name("notification_server.py")
        provider_process = start(["python3", str(provider), "--log", str(root / "notifications.jsonl")], "notification-provider")
        wait_until(lambda: (root / "notifications.jsonl").exists(), "notification provider")
        if args.stage in ["full"]:
            portal_config = root / "config/xdg-desktop-portal"
            portal_config.mkdir()
            (portal_config / "portals.conf").write_text("[preferred]\ndefault=gtk\norg.freedesktop.impl.portal.FileChooser=gtk\n")
            activation = ["dbus-update-activation-environment", "HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_RUNTIME_DIR", "XDG_CURRENT_DESKTOP"]
            activation.extend(name for name in ["DISPLAY", "WAYLAND_DISPLAY"] if name in env)
            command(activation)
            portal_env = dict(env, GDK_BACKEND="wayland" if args.backend == "wayland" else "x11")
            start(["/usr/libexec/xdg-desktop-portal-gtk"], "portal-gtk", process_env=portal_env)
            wait_until(lambda: "boolean true" in command(["dbus-send", "--session", "--print-reply",
                       "--dest=org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus.NameHasOwner",
                       "string:org.freedesktop.impl.portal.desktop.gtk"]).stdout, "explicit GTK portal backend")
            start(["/usr/libexec/xdg-desktop-portal"], "portal", process_env=portal_env)
        if args.stage == "ime-wayland":
            from native_ime_wayland import prepare_labwc
            prepare_labwc(start, env, root)
        app_env = dict(env)
        if args.backend == "wayland":
            app_env.pop("DISPLAY", None)
        trace = os.environ.get("CHDA_NATIVE_TRACE") == "1"
        if trace and args.backend == "wayland":
            app_env["WAYLAND_DEBUG"] = "client"
        def launch_candidate(name):
            temporary = runtime_temporary_root / name
            temporary.mkdir(mode=0o700)
            launch_temporary_paths.append(temporary)
            candidate_env = dict(app_env, TMPDIR=str(temporary))
            invocation = [str(launcher)]
            if trace:
                invocation = ["strace", "-f", "-s", "128", "-e",
                              "trace=read,write,readv,writev,ioctl,recvmsg,sendmsg",
                              "-o", str(root / (name + "-io.trace"))] + invocation
            return start(invocation, name, cwd=repo, process_env=candidate_env)
        app = launch_candidate("candidate")
        window = parent_window
        if args.backend == "x11":
            windows = []
            def observed():
                response = command(["xdotool", "search", "--onlyvisible"] + (["--name", "CHDA_TOOL_FIXTURE"] if args.preparation_probe else ["--class", args.app_id]), check=False)
                windows[:] = response.stdout.splitlines()
                return bool(windows)
            wait_until(observed, "actual candidate X11 window")
            window = windows[-1]
            command(["xdotool", "windowactivate", "--sync", window])
        elif args.compositor == "labwc":
            wait_until(lambda: bool(command(["wlrctl", "toplevel", "list"], check=False).stdout.strip()), "actual candidate Wayland window")
        if app.poll() is not None:
            raise RuntimeError("Candidate exited before input")
        graphics = subprocess.run(["vulkaninfo", "--summary"], env=env, text=True,
                                  stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        if graphics.returncode:
            graphics = subprocess.run(["vulkaninfo"], env=env, text=True,
                                      stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        (root / "vulkan.txt").write_text(graphics.stdout)
        # Driver subprocesses must receive only this explicitly isolated session.
        options = SimpleNamespace(backend=args.backend, compositor=args.compositor, window=window,
                                  input_backend="x11" if args.backend == "x11" or args.compositor == "mutter" else "wayland",
                                  capture_backend="x11" if args.backend == "x11" or args.compositor == "mutter" else "wayland",
                                  evidence=str(root), artifact=str(artifact), screen_width=1440,
                                  screen_height=1000, preparation_probe=args.preparation_probe)
        if args.backend == "wayland" and args.compositor == "labwc" and args.stage not in ["startup", "ime-wayland"]:
            build = root / "pointer-helper"
            build.mkdir()
            for name in ["build-drag.sh", "wayland-drag.c", "wlr-virtual-pointer-unstable-v1.xml"]:
                shutil.copy2(Path(__file__).with_name(name), build / name)
            command(["sh", str(build / "build-drag.sh")])
            options.drag_helper = str(build / "wayland-drag")
        previous_env = dict(os.environ)
        try:
            os.environ.clear()
            os.environ.update(env)
            driver = Driver(options)
            if not args.preparation_probe:
                # A native window and shell input can precede the first GPU
                # frame. Wait for the actual rendered prompt on every distro.
                from native_visual import ocr_words
                readiness_attempt = 0
                def rendered_prompt():
                    nonlocal readiness_attempt
                    readiness_attempt += 1
                    image = driver.screenshot(f"startup-readiness-{readiness_attempt:02d}")
                    letters = "".join(char for word in ocr_words(image) for char in word["text"] if char.isalpha()).upper()
                    return "CHDAVALIDATION" in letters
                wait_until(rendered_prompt, "actual candidate rendered shell prompt", timeout=40)
            else:
                time.sleep(1)
            steps.append({"step": "actual launcher and native window", "status": "passed",
                          "inputWindow": window,
                          "windowEvidence": "rendered candidate shell prompt" if not args.preparation_probe else "observed tool window"})
            driver.screenshot("01-startup")
            # The typed command must not contain the contiguous OCR target.
            marker = driver.marker("startup-input.txt", "printf 'CHDA_%s_INPUT_%s' STARTUP OK")
            if marker != "CHDA_STARTUP_INPUT_OK":
                raise AssertionError("Unexpected startup marker content")
            if not args.preparation_probe:
                output_attempt = 0
                def rendered_output():
                    nonlocal output_attempt
                    output_attempt += 1
                    image = driver.screenshot(f"startup-output-{output_attempt:02d}")
                    letters = "".join(char for word in ocr_words(image) for char in word["text"] if char.isalpha()).upper()
                    return "CHDASTARTUPINPUTOK" in letters
                wait_until(rendered_output, "actual rendered startup input output", timeout=10)
            driver.screenshot("02-after-input")
            result["startupStatus"] = "passed"
            result["automationStatus"] = "passed"
            if args.stage == "ime-wayland":
                from native_ime_wayland import exercise
                result["imeWayland"] = exercise(driver)
                result.update(applicationBackend="native-wayland", inputBackend="native-wayland",
                              captureBackend="native-wayland", compositor="labwc",
                              imeVisualStatus="pending independent preedit PNG review")
            if args.stage == "terminal":
                driver.terminal_checks()
            if args.stage in ["clipboard", "terminal"]:
                driver.copy_selection()
            if args.stage in ["windows", "full"]:
                from native_journey import Journey
                context = {"app": app, "provider": provider_process,
                           "runtimeTmpdirs": launch_temporary_paths}
                def launch_again():
                    context["app"] = launch_candidate("candidate-restarted")
                def retire_runtime():
                    # Only this already-stopped launch's private temp directory.
                    # Do not let an extract-and-run cache hide a stale helper path.
                    previous = launch_temporary_paths[-1]
                    if previous.exists():
                        shutil.rmtree(previous)
                    assert not previous.exists()
                    return {"path": str(previous), "existsAfterCleanup": previous.exists(),
                            "scope": "owned isolated stopped runtime directory"}
                context["launch_again"] = launch_again
                context["retire_runtime"] = retire_runtime
                journey = Journey(driver, args, root, repo, context)
                if args.stage == "full":
                    report = journey.execute()
                    result["automationStatus"] = report["automationStatus"]
                    result["fullRoundStatus"] = report["fullRoundStatus"]
                else:
                    journey.step("window controls tool stage", journey.window_checks)
                    journey.report["automationStatus"] = "passed"
                    journey.save()
        finally:
            os.environ.clear()
            os.environ.update(previous_env)
        steps.append({"step": "actual shell input marker", "status": "passed"})
        steps.append({"step": "native PNG capture", "status": "passed", "visualReview": "pending independent inspection"})
        result["startupStatus"] = "passed"
    except Exception as error:
        result.setdefault("startupStatus", "failed")
        result["automationStatus"] = "failed"
        result["error"] = str(error)
        raise
    finally:
        (root / "session-result.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
        for child in reversed(children):
            try:
                os.killpg(child.pid, signal.SIGTERM)
                child.wait(timeout=3)
            except (ProcessLookupError, subprocess.TimeoutExpired):
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
        for handle in handles:
            handle.close()
        shutil.rmtree(runtime_temporary_root)


if __name__ == "__main__":
    main()
