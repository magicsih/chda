// chda's MiniMax status bridge. Native hooks inherit PATH but not CHDA_*.
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

try {
  const directory = (process.env.PATH || "").split(path.delimiter).find((dir) =>
    /^chda-minimax-pane-[0-9]+$/.test(path.basename(dir)) &&
    fs.existsSync(path.join(dir, "context.json")));
  if (!directory) process.exit(0);
  const context = JSON.parse(fs.readFileSync(path.join(directory, "context.json"), "utf8"));
  if (context.version !== 1 || !Number.isSafeInteger(context.pane) || context.pane <= 0 ||
      !path.isAbsolute(context.hook_bin) || !path.isAbsolute(context.data_dir)) process.exit(0);
  const input = JSON.parse(fs.readFileSync(0, "utf8"));
  if (input.agent_id || input.parent_session_id || input.parentSessionId ||
      typeof input.session_id !== "string" || !input.session_id ||
      typeof input.cwd !== "string" || !input.cwd) process.exit(0);
  const kind = input.hook_event_name;
  if (!["SessionStart", "UserPromptSubmit", "PermissionRequest", "PostToolUse", "Stop", "SessionEnd"].includes(kind)) process.exit(0);
  let root;
  try { root = JSON.parse(fs.readFileSync(path.join(directory, "state.json"), "utf8")).session_id; } catch {}
  if (kind === "SessionStart") {
    if (root && root !== input.session_id && !["clear", "resume"].includes(input.source)) process.exit(0);
    // Runtime emits root admission hooks separately from child tool events.
    root = input.session_id;
    const temporary = path.join(directory, `.state-${process.pid}`);
    fs.writeFileSync(temporary, JSON.stringify({ session_id: root }), { mode: 0o600, flag: "wx" });
    fs.renameSync(temporary, path.join(directory, "state.json"));
  }
  if (root !== input.session_id) process.exit(0);
  // Discard the prompt, tool input and transcript path before invoking chda.
  const payload = JSON.stringify({ hook_event_name: kind, session_id: root, cwd: input.cwd });
  spawnSync(context.hook_bin, ["hook", "minimax"], {
    input: payload, stdio: ["pipe", "ignore", "ignore"], timeout: 8000,
    env: { ...process.env, CHDA_PANE_ID: String(context.pane), CHDA_HOOK_DATA_DIR: context.data_dir },
  });
} catch {
  // Status reporting must never decide permissions or interrupt a turn.
}
process.stdout.write("{}\n");
