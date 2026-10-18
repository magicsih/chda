#!/usr/bin/env python3
"""Exercise the installed helper against the actual packaged executable."""
import concurrent.futures
import json
import os
import selectors
import shlex
import signal
import subprocess
import time

import mcp_fixture
from native_journey import wait


def exercise(journey):
    acceptance = journey.acceptance
    helper = shlex.split(acceptance.frozen_hook_commands["SessionStart"])[0]
    root = journey.root / "helper-lifecycle"
    root.mkdir(mode=0o700)
    temporary = root / "temporary"
    temporary.mkdir(mode=0o700)
    env = dict(os.environ, TMPDIR=str(temporary))
    # Saved external MCP commands must work without the GUI launch environment
    # or a FUSE device. The installed AppImage helper chooses its own run mode.
    for name in ("APPIMAGE_EXTRACT_AND_RUN", "APPIMAGE", "APPDIR", "TARGET_APPIMAGE"):
        env.pop(name, None)

    def version():
        child = subprocess.run([helper, "--version"], cwd=journey.repo, env=env,
                               stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, text=True, timeout=30, check=True)
        if not child.stdout.startswith("chda "):
            raise AssertionError("Installed helper did not return the product version")
        return {"stdout": child.stdout, "stderr": child.stderr}

    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as workers:
        versions = [workers.submit(version) for _ in range(3)]
        mcp = workers.submit(mcp_fixture.call, helper, journey.repo, "list_sessions", {},
                             environment=env)
        versions = [future.result() for future in versions]
        reply = mcp.result()
    if len({item["stdout"] for item in versions}) != 1:
        raise AssertionError("Concurrent helpers returned different versions")
    if list(temporary.iterdir()):
        raise AssertionError("Completed helpers left temporary extraction directories")
    # The GUI must survive those overlapping extract-and-run calls.
    responsive = journey.driver.marker("helper-parallel-responsive.txt", "printf PARALLEL_HELPERS_OK")
    if responsive != "PARALLEL_HELPERS_OK":
        raise AssertionError("Concurrent helpers interrupted the actual terminal")

    request = reply["request"]
    child = subprocess.Popen([helper, "mcp"], cwd=journey.repo, env=env,
                             stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, text=True, start_new_session=True)
    descendants = {}

    def processes():
        from pathlib import Path
        rows = {}
        for entry in Path("/proc").iterdir():
            if not entry.name.isdigit():
                continue
            try:
                fields = (entry / "stat").read_text().rsplit(")", 1)[1].split()
                rows[int(entry.name)] = (int(fields[1]), fields[0], fields[19])
            except (OSError, ValueError, IndexError):
                continue
        return rows

    def remember_descendants():
        rows = processes()
        parents = {child.pid}
        while True:
            found = {pid for pid, (parent, _, _) in rows.items() if parent in parents}
            if found <= parents:
                break
            parents.update(found)
        for pid in parents:
            if pid in rows:
                descendants[pid] = rows[pid][2]

    def alive():
        return [pid for pid, (_, state, born) in processes().items()
                if descendants.get(pid) == born and state != "Z"]

    try:
        child.stdin.write(json.dumps(request) + "\n")
        child.stdin.flush()
        selector = selectors.DefaultSelector()
        try:
            descriptor = child.stdout.fileno()
            os.set_blocking(descriptor, False)
            selector.register(descriptor, selectors.EVENT_READ)
            deadline = time.monotonic() + 30
            line = bytearray()
            while b"\n" not in line:
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not selector.select(timeout=remaining):
                    raise AssertionError("Streaming helper MCP did not answer before stdin EOF")
                chunk = os.read(descriptor, 65536)
                if not chunk:
                    raise AssertionError("Streaming helper MCP closed stdout before a JSON line")
                line.extend(chunk)
                if len(line) > 1024 * 1024:
                    raise AssertionError("Streaming helper MCP exceeded the fixture response limit")
            streamed = json.loads(line.split(b"\n", 1)[0])
        finally:
            selector.close()
        mcp_fixture.validate_reply(streamed, request)
        remember_descendants()
        child.terminate()  # Deliberately signal the launcher PID, not its group.
        status = child.wait(timeout=10)
        if status not in (-signal.SIGTERM, 128 + signal.SIGTERM):
            raise AssertionError("Direct helper termination returned unexpected status: " + str(status))
        wait(lambda: not alive(), "actual helper descendants stop after direct TERM", timeout=10)
        wait(lambda: not list(temporary.iterdir()), "actual helper extraction cleanup after TERM", timeout=10)
    finally:
        if child.poll() is None:
            child.terminate()
            try:
                child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=5)
        for pid in alive():
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        for stream in (child.stdin, child.stdout, child.stderr):
            stream.close()

    result = {"helper": helper, "versions": versions, "parallelMcp": reply,
              "streamedReplyBeforeEof": streamed, "directTermStatus": status,
              "observedProcessStartTimes": descendants, "remainingProcesses": alive(),
              "remainingTemporaryEntries": [str(path) for path in temporary.iterdir()],
              "scope": "actual installed packaged helper, isolated protocol input; no provider CLI"}
    (root / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    return result
