#!/usr/bin/env python3
"""Send a documented CLI-input fixture through the candidate's real MCP server."""
import argparse
import json
import os
import subprocess
from pathlib import Path


def call(launcher, cwd, tool, arguments, pane=None, environment=None):
    env = dict(environment if environment is not None else os.environ)
    if env.get("CHDA_NATIVE_ISOLATED") != "1":
        raise RuntimeError("MCP fixtures require the isolated native session")
    if pane is not None:
        env["CHDA_PANE_ID"] = str(pane)
    request = {"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
        "_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28",
                  "io.modelcontextprotocol/clientInfo": {"name": "chda-native-validation-fixture", "version": "1"}},
        "name": tool, "arguments": arguments}}
    child = subprocess.run([str(launcher), "mcp"], cwd=str(cwd), env=env,
                           input=json.dumps(request) + "\n", text=True,
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30, check=True)
    replies = [json.loads(line) for line in child.stdout.splitlines() if line.strip()]
    if not replies:
        raise RuntimeError("MCP fixture received no JSON-RPC reply: " + child.stderr)
    reply = replies[-1]
    if "error" in reply or reply.get("result", {}).get("isError"):
        raise RuntimeError("MCP fixture failed: " + json.dumps(reply))
    return {"scope": "actual candidate CLI input fixture, not a provider CLI session",
            "pane": pane, "request": request, "reply": reply}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--launcher", required=True)
    parser.add_argument("--cwd", required=True)
    parser.add_argument("--tool", required=True)
    parser.add_argument("--arguments", default="{}")
    parser.add_argument("--pane-file", help="Pane ID produced by actual terminal input")
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    pane = int(Path(args.pane_file).read_text().strip()) if args.pane_file else None
    result = call(args.launcher, args.cwd, args.tool, json.loads(args.arguments), pane)
    Path(args.output).write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")


if __name__ == "__main__":
    main()
