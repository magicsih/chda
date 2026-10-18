#!/usr/bin/env python3
"""Isolated candidate startup/input/PNG stage. Full-round checks remain pending."""
import argparse
import hashlib
import json
import os
import platform
import signal
import subprocess
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
    parser.add_argument("--round", type=int, choices=[1, 2, 3], required=True)
    parser.add_argument("--evidence", required=True)
    parser.add_argument("--preparation-probe", action="store_true")
    args = parser.parse_args()
    if platform.system() != "Linux" or (platform.machine() != "x86_64" and not args.preparation_probe):
        raise SystemExit("Product execution requires native Linux x86_64")
    root = Path(args.evidence).resolve()
    if root.exists() and any(root.iterdir()):
        raise SystemExit("Evidence directory must be new or empty")
    root.mkdir(parents=True, exist_ok=True)
    launcher = Path(args.launcher).resolve()
    artifact = Path(args.artifact).resolve()
    digest = sha256_file(artifact)
    home = root / "home"
    runtime = root / "runtime"
    repo = root / "repo with spaces 世界"
    for directory in [home, runtime, repo, root / "config/chda", root / "data", root / "cache"]:
        directory.mkdir(parents=True, exist_ok=True)
    runtime.chmod(0o700)
    (root / "config/chda/config.toml").write_text(
        "repos = [" + json.dumps(str(repo), ensure_ascii=False) + "]\n"
        "notifications = true\nrestore-session = true\n")
    (home / ".bashrc").write_text("PS1='CHDA_VALIDATION> '\n")
    (home / ".bash_profile").write_text('. "$HOME/.bashrc"\n')
    env = dict(os.environ)
    env.update(HOME=str(home), XDG_CONFIG_HOME=str(root / "config"),
               XDG_DATA_HOME=str(root / "data"), XDG_CACHE_HOME=str(root / "cache"),
               XDG_RUNTIME_DIR=str(runtime), XDG_CURRENT_DESKTOP="labwc" if args.backend == "wayland" else "Openbox",
               SHELL="/bin/bash", LANG="C.UTF-8", CHDA_NATIVE_ISOLATED="1",
               CHDA_CANDIDATE_SHA=args.candidate_sha, CHDA_ARTIFACT_SHA256=digest,
               APPIMAGE_EXTRACT_AND_RUN="1")
    for name in ["DISPLAY", "WAYLAND_DISPLAY", "SWAYSOCK", "DBUS_SESSION_BUS_ADDRESS"]:
        env.pop(name, None)
    children = []
    handles = []
    steps = []
    result = {"scope": "ARM/tool preparation only" if args.preparation_probe else "product candidate startup only",
              "candidateSha": args.candidate_sha, "artifactSha256": digest,
              "backend": args.backend, "round": args.round, "architecture": platform.machine(),
              "launcher": str(launcher), "artifact": str(artifact), "steps": steps,
              "fullRoundStatus": "not-run"}

    def start(command, name, *, stdout=None, cwd=None):
        log = (root / (name + ".log")).open("w")
        handles.append(log)
        child = subprocess.Popen(command, cwd=cwd, env=env,
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
        if args.backend == "x11":
            xserver = start(["Xvfb", "-displayfd", "1", "-screen", "0", "1440x1000x24", "-nolisten", "tcp", "-noreset"], "xvfb", stdout=subprocess.PIPE)
            number = xserver.stdout.readline().strip()
            env["DISPLAY"] = ":" + number
            wait_until(lambda: command(["xdpyinfo"], check=False).returncode == 0, "Xvfb")
            start(["openbox"], "openbox")
        else:
            env.update(WLR_BACKENDS="headless", WLR_RENDERER="pixman", WLR_LIBINPUT_NO_DEVICES="1")
            config = root / "labwc.xml"
            config.write_text('<?xml version="1.0"?><labwc_config><core><decoration>client</decoration></core><focus><followMouse>no</followMouse></focus></labwc_config>')
            start(["labwc", "-c", str(config), "-d"], "labwc")
            wait_until(lambda: any(p.is_socket() for p in runtime.glob("wayland-*")), "Wayland socket")
            env["WAYLAND_DISPLAY"] = next(p.name for p in runtime.glob("wayland-*") if p.is_socket())
        steps.append({"step": "isolated compositor", "status": "passed"})
        provider = Path(__file__).with_name("notification_server.py")
        start(["python3", str(provider), "--log", str(root / "notifications.jsonl")], "notification-provider")
        app = start([str(launcher)], "candidate", cwd=repo)
        window = None
        if args.backend == "x11":
            windows = []
            def observed():
                response = command(["xdotool", "search", "--onlyvisible", "--class", "com.magicsih.chda|CHDA_TOOL_FIXTURE"], check=False)
                windows[:] = response.stdout.splitlines()
                return bool(windows)
            wait_until(observed, "actual candidate X11 window")
            window = windows[-1]
            command(["xdotool", "windowactivate", "--sync", window])
        else:
            wait_until(lambda: bool(command(["wlrctl", "toplevel", "list"], check=False).stdout.strip()), "actual candidate Wayland window")
        if app.poll() is not None:
            raise RuntimeError("Candidate exited before input")
        steps.append({"step": "actual launcher and native window", "status": "passed", "window": window})
        subprocess.run(["vulkaninfo", "--summary"], env=env, stdout=(root / "vulkan.txt").open("w"), stderr=subprocess.STDOUT, check=False)
        # Driver subprocesses must receive only this explicitly isolated session.
        options = SimpleNamespace(backend=args.backend, compositor="labwc", window=window,
                                  evidence=str(root), artifact=str(artifact), screen_width=1440,
                                  screen_height=1000, preparation_probe=args.preparation_probe)
        previous_env = dict(os.environ)
        try:
            os.environ.clear()
            os.environ.update(env)
            driver = Driver(options)
            time.sleep(1)
            driver.screenshot("01-startup")
            marker = driver.marker("startup-input.txt", "printf CHDA_STARTUP_INPUT_OK")
            if marker != "CHDA_STARTUP_INPUT_OK":
                raise AssertionError("Unexpected startup marker content")
            driver.screenshot("02-after-input")
        finally:
            os.environ.clear()
            os.environ.update(previous_env)
        steps.append({"step": "actual shell input marker", "status": "passed"})
        steps.append({"step": "native PNG capture", "status": "passed", "visualReview": "pending independent inspection"})
        result["startupStatus"] = "passed"
    except Exception as error:
        result["startupStatus"] = "failed"
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


if __name__ == "__main__":
    main()
