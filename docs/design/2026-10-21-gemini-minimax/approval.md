# Approval

- The user selected option 1 on 2026-10-21.
- Approved design: version 1.0 at `f1945a4`, with its test plan.
- Scope: Gemini default menu visibility; MiniMax Code managed launch, pane
  status, input waiting, completion notifications, presets and default action.
- MiniMax historical conversations, exact resume and usage remain deferred.
- Endpoint: implementation, validation and a reviewed Ready PR. No release.
- Implementation detail confirmed during source inspection: MiniMax filters
  custom hook environment variables. The plugin therefore reads its private
  pane context from a directory on the managed launch's inherited PATH, then
  restores only chda's helper and pane routing environment for the hook process.
  The context contains routing metadata only and is absent outside that launch.

## Expanded release endpoint

On 2026-10-21 the user explicitly requested continuing through the next public
version release. This supersedes the earlier Ready-PR-only endpoint. The next
minor version is 1.3.0 under the repository version policy; release-plz owns its
version and changelog changes.

The release scope includes the existing macOS and Linux distribution channels,
full CI regression, the repository's native release evidence, signed artifacts,
publication and public download verification. The unavailable MiniMax advisory
review remains recorded as unavailable. Previous version-specific publication
exceptions do not apply to 1.3.0, and unrun native checks must not be marked passed.
