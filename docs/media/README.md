# Product media

The 1.0.25 media refresh uses the actual macOS development app and a disposable Atlas API repository. Agent text and status events are explicitly labeled samples; they contain no private project, prompt or conversation. The API test output comes from the demonstration repository, not from chda's release test suite.

- `hero.png` and `sessions.png`: dark and light native workspaces, split terminals, child counts and worktree notes.
- `commands.png`: a configured API test command in a floating terminal.
- `git-graph.jpg`: real commit and branch history from the demo repository.
- `demo.gif`: a 19-second guided sequence of native app captures, with captions outside the app image.
- `diagram.png`, `markdown.png`, `markdown-flow.png`: output from the current product's offline diagram and Markdown renderers, captured in the browser. The document overview intentionally shows the beginning of the sequence diagram; the paired flow image shows the entire diagram.

Native originals were captured at 1440 × 880. Workspace exports remove the top 34 pixels containing the OS window-sharing controls, then scale to 1200 × 705. The hero and GIF add a separate 45-pixel caption band. The Git tree image crops unused lower space and keeps its established 1920 × 990 export size. Pages also uses sidebar and palette close-ups and a social preview derived from the same originals.

Marketing media and demo events are separate from native release acceptance evidence. Regenerate feature sections and the catalog screenshot copies with `python3 scripts/sync-product-docs.py`.
