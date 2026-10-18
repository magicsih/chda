#!/usr/bin/env python3
"""Additional actual candidate flows, separate from tool preparation results."""
import http.server
import json
import os
import shlex
import subprocess
import threading
import time
import uuid
from pathlib import Path

from native_driver import run
from native_visual import phrase_center, sentinel_bounds
from native_journey import wait


class Acceptance:
    def __init__(self, journey):
        self.journey=journey
        self.driver=journey.driver
        self.root=journey.root
        self.repo=journey.repo

    def cwd_checks(self):
        child_cwd=self.repo/"foreground cwd spaces 世界"
        child_cwd.mkdir()
        ready=self.root/"cwd-child-ready.txt"
        child=self.root/"cwd-child.py"
        child.write_text("import os,time\nfrom pathlib import Path\nos.chdir("+repr(str(child_cwd))+")\n"
                         "Path("+repr(str(ready))+").write_text(os.getcwd())\ntime.sleep(45)\n")
        original=self.driver.marker("cwd-original-pane.txt",'printf "%s" "$CHDA_PANE_ID"').strip()
        parent=self.driver.marker("cwd-parent-before.txt","pwd -P").strip()
        prompt=self.driver.marker("cwd-no-prompt-command.txt","printf 'PROMPT_COMMAND=%s' \"${PROMPT_COMMAND-}\"")
        if prompt!="PROMPT_COMMAND=":
            raise AssertionError("No-OSC7 fixture has an unexpected PROMPT_COMMAND")
        self.driver.command("python3 "+shlex.quote(str(child)))
        wait(ready.exists,"actual foreground child changed cwd")
        time.sleep(1.5)  # product CWD_POLL is one second
        self.driver.key("ctrl+shift+t")
        inherited=self.driver.marker("cwd-child-inherited.txt","pwd -P").strip()
        if Path(inherited)!=child_cwd.resolve():
            raise AssertionError("New tab did not inherit foreground process cwd without OSC7: "+inherited)
        self.driver.screenshot("cwd-foreground-child-inherited")
        self.driver.key("ctrl+shift+w")
        self.driver.key("ctrl+c")
        self.driver.marker("cwd-child-interrupted.txt","printf CWD_CHILD_INTERRUPTED")
        time.sleep(1.5)
        self.driver.key("ctrl+shift+t")
        after=self.driver.marker("cwd-parent-inherited.txt","pwd -P").strip()
        if after!=parent:
            raise AssertionError("New tab did not recover parent bash cwd after child interruption")
        self.driver.key("ctrl+shift+w")
        restored=self.driver.marker("cwd-restored-pane.txt",'printf "%s" "$CHDA_PANE_ID"').strip()
        if restored!=original:
            raise AssertionError("Cwd scenario did not return to its actual source pane")
        return {"originalPane":original,"parentCwd":parent,"childCwd":str(child_cwd),
                "newTabCwd":inherited,"afterInterruptNewTabCwd":after,
                "OSC7":"not emitted by the private bash fixture"}

    def worktree_checks(self):
        source = self.driver.marker("worktree-source-pane.txt", 'printf "%s" "$CHDA_PANE_ID"').strip()
        branch="native-"+self.journey.args.backend+"-round-"+str(self.journey.args.round)
        self.driver.key("ctrl+alt+n")
        time.sleep(.4)
        self.driver.screenshot("worktree-create-sheet")
        self.driver.text(branch)
        image=self.driver.screenshot("worktree-branch-entered")
        self.driver.click(*phrase_center(image,"Create worktree"))
        def created():
            output=run(["git","-C",str(self.repo),"worktree","list","--porcelain"])
            (self.root/"git-worktrees.txt").write_text(output)
            path=None
            for line in output.splitlines():
                if line.startswith("worktree "):path=line[len("worktree "):]
                if line=="branch refs/heads/"+branch:return path
            return None
        actual=Path(wait(created,"actual UI-created git worktree",timeout=30)).resolve()
        head=run(["git","-C",str(actual),"symbolic-ref","--short","HEAD"]).strip()
        if head!=branch:raise AssertionError("Created worktree branch mismatch")
        # Background creation and the actual shell must both complete.
        wait(lambda:any(Path(row["worktree"]).resolve()==actual for row in
                        self.journey.sessions("ui-created-worktree-ready") if row.get("worktree")),
             "UI-created worktree pane is registered",timeout=20)
        time.sleep(.3)
        cwd=self.driver.marker("worktree-shell-cwd.txt","pwd -P").strip()
        if Path(cwd)!=actual:raise AssertionError("Actual worktree terminal cwd mismatch: "+cwd)
        self.journey.mcp("list_worktrees",suffix="ui-created-worktree")
        self.journey.sessions("ui-created-worktree-sessions")
        self.driver.screenshot("worktree-created-sidebar")
        self.driver.key("ctrl+Page_Up")
        focused = self.driver.marker("worktree-returned-pane.txt", 'printf "%s" "$CHDA_PANE_ID"').strip()
        if focused != source:
            raise AssertionError("Tab navigation did not return from the new worktree to its source")
        return {"branch":branch,"path":str(actual),"actualShellCwd":cwd,
                "trigger":"actual CtrlAltN sheet and rendered Create worktree pointer click"}

    def hooks_checks(self,phase):
        pane=self.driver.marker("hooks-"+phase+"-pane.txt",'printf "%s" "$CHDA_PANE_ID"').strip()
        script=self.root/("installed-hook-"+phase+".py")
        settings=self.root/"data/chda/hooks/claude-settings.json"
        if not settings.is_file():raise AssertionError("Actual app did not install Claude hook settings")
        installed = json.loads(settings.read_text())["hooks"]
        commands = {event: installed[event][0]["hooks"][0]["command"] for event in
                    ["SessionStart", "UserPromptSubmit", "PermissionRequest", "SessionEnd"]}
        if phase == "before-restart":
            self.frozen_hook_commands = commands
            (self.root / "frozen-hook-commands.json").write_text(json.dumps(commands, indent=2)+"\n")
            appdir = self.driver.marker("original-appdir.txt", 'printf "APPDIR=%s\\n" "${APPDIR-}"').strip()
            if not appdir.startswith("APPDIR="):
                raise AssertionError("Unexpected AppImage environment marker")
            self.original_appdir = appdir[len("APPDIR="):]
            if self.original_appdir:
                Path(self.original_appdir).resolve().relative_to(self.journey.context["runtimeTmpdirs"][-1].resolve())
        else:
            if commands != self.frozen_hook_commands:
                raise AssertionError("Installed helper commands changed across cold package restart")
            commands = self.frozen_hook_commands
            if self.original_appdir and Path(self.original_appdir).exists():
                raise AssertionError("Old AppImage runtime path still exists during durable helper validation")
        # This script runs INSIDE the real terminal, so the hook inherits the
        # actual CHDA_PANE_ID and cwd. Never substitute the launcher for its
        # installed hook command, including after AppImage relaunch.
        script.write_text("""import json,os,shlex,subprocess,sys
from pathlib import Path
event=sys.argv[1]
command=FROZEN_COMMANDS[event]
program=Path(shlex.split(command)[0])
if not program.is_file() or not os.access(program,os.X_OK):
    raise RuntimeError('Installed hook executable is unavailable: '+str(program))
payload={'hook_event_name':event,'session_id':'native-hook-'+PHASE+'-'+os.environ['CHDA_PANE_ID'],'cwd':os.getcwd()}
result=subprocess.run(['/bin/sh','-c',command],input=json.dumps(payload),text=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=15)
Path(OUTPUT_PREFIX+event+'.json').write_text(json.dumps({'event':event,'phase':PHASE,'pane':os.environ['CHDA_PANE_ID'],'cwd':os.getcwd(),'command':command,'returncode':result.returncode,'stdout':result.stdout,'stderr':result.stderr},indent=2)+'\\n')
if result.returncode:raise SystemExit(result.returncode)
""".replace("FROZEN_COMMANDS",repr(commands)).replace("PHASE",repr(phase)).replace("OUTPUT_PREFIX",repr(str(self.root/("hooks-"+phase+"-")))))
        compile(script.read_text(),str(script),"exec")
        evidence=[]
        for event,expected in [("SessionStart",None),("UserPromptSubmit","working"),
                               ("PermissionRequest","waiting_for_input"),("SessionEnd","ended")]:
            before_notifications = len(self.journey.delivered())
            output=self.root/("hooks-"+phase+"-"+event+".json")
            self.driver.command("python3 "+shlex.quote(str(script))+" "+event)
            wait(output.exists,"actual installed hook execution "+event,timeout=20)
            result=json.loads(output.read_text())
            if result["returncode"] or result["pane"]!=pane:
                raise AssertionError("Installed hook fixture failed or targeted another pane")
            if expected:
                def observed():
                    sessions=self.journey.sessions("hooks-"+phase+"-"+expected)
                    target=next((row for row in sessions if row["pane"]==int(pane)),None)
                    if target and (target.get("status")==expected or expected=="ended" and target.get("kind")=="terminal"):
                        return target
                    return None
                result["observedStatus"]=wait(observed,"exact installed-hook pane status "+expected)
            if event == "PermissionRequest" and phase == "before-restart":
                wait(lambda: len(self.journey.delivered()) > before_notifications,
                     "actual notification from the installed hook command")
            evidence.append(result)
        self.driver.screenshot("hooks-"+phase+"-complete")
        self.journey.mcp("read_output",{"pane":int(pane),"lines":20},suffix="hooks-"+phase+"-read-output")
        return {"phase":phase,"pane":pane,"events":evidence,
                "scope":"actual installed hook CLI input fixture; not a real Claude provider session"}

    def link_checks(self):
        events=[]
        nonce=uuid.uuid4().hex
        expected_path="/CHDA_LINK_"+nonce
        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                events.append(self.path)
                self.send_response(200);self.end_headers();self.wfile.write(b"CHDA_LINK_OK")
            def log_message(self,*args):pass
        server=http.server.ThreadingHTTPServer(("127.0.0.1",0),Handler)
        thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
        url="http://127.0.0.1:"+str(server.server_port)+expected_path
        handler_output=self.root/"uri-handler.json"
        handler=self.root/"uri-handler.py"
        handler.write_text("import json,sys,urllib.request\nfrom pathlib import Path\n"
                           "expected="+repr(url)+"\nactual=sys.argv[1]\n"
                           "if actual!=expected:raise SystemExit('Unexpected URI')\n"
                           "reply=urllib.request.urlopen(actual,timeout=5).read().decode()\n"
                           "Path("+repr(str(handler_output))+").write_text(json.dumps({'uri':actual,'reply':reply,'scope':'isolated native URI handler fixture'}))\n")
        applications=self.root/"data/applications";applications.mkdir(exist_ok=True)
        desktop=applications/"chda-validation-uri.desktop"
        # xdg-open generic keeps quotes literal in Exec arguments. The CI
        # /evidence/session path is a safe single unquoted argument.
        if any(c not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_./-" for c in str(handler)):
            raise RuntimeError("URI fixture requires a safe /evidence/session path")
        desktop.write_text("[Desktop Entry]\nType=Application\nName=chda native URI fixture\nExec=/usr/bin/python3 "+str(handler)+" %u\nNoDisplay=true\nMimeType=x-scheme-handler/http;\n")
        mimeapps=self.root/"config/mimeapps.list"
        if mimeapps.exists():raise RuntimeError("Refusing to overwrite an existing URI preference in the test session")
        mimeapps.write_text("[Default Applications]\nx-scheme-handler/http=chda-validation-uri.desktop\n")
        self.driver.record("isolated-native-uri-handler-registered",desktop=str(desktop),url=url)
        try:
            query=run(["xdg-mime","query","default","x-scheme-handler/http"]).strip()
            if query!="chda-validation-uri.desktop":raise AssertionError("Private native URI handler not selected")
            self.driver.command("printf '\\033[2J\\033[H\\033]8;;"+url+"\\033\\\\CHDALINKSENTINEL\\033]8;;\\033\\\\\\n'")
            image=self.driver.screenshot("link-before-control-click")
            coordinates=sentinel_bounds(image,"CHDALINKSENTINEL")
            x=(coordinates[0]+coordinates[2])//2;y=coordinates[1]
            if self.driver.input_backend!="x11":
                raise RuntimeError("Link modifier helper currently requires actual XTest input; native Wayland via Mutter parent is supported")
            run(["xdotool","keydown","ctrl"])
            try:
                run(["xdotool","mousemove","--sync",str(x),str(y)])
                self.driver.screenshot("link-control-hover")
                self.driver.click(x,y)
            finally:run(["xdotool","keyup","ctrl"])
            wait(handler_output.exists,"native terminal Ctrl+click URI handler",timeout=15)
            output=json.loads(handler_output.read_text())
            if output["uri"]!=url or output["reply"]!="CHDA_LINK_OK" or expected_path not in events:
                raise AssertionError("Actual Ctrl+click did not deliver the exact local URI")
            (self.root/"link-http-requests.json").write_text(json.dumps(events,indent=2)+"\n")
            self.driver.screenshot("link-after-control-click")
            return {"handler":output,"requests":events,"at":[x,y],"trigger":"actual terminal OSC8 link and Ctrl+pointer click"}
        finally:
            server.shutdown();server.server_close()
