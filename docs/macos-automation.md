# macOS Automation access

Commands and coding agents launched inside chda can send Apple Events to other
apps. macOS attributes these requests to the terminal. The signed macOS bundle
therefore declares the Apple Events entitlement and a usage description, so the
system can ask for permission instead of rejecting requests before showing a
prompt. For example, a coding agent may request a connection to Codex Computer Use.

Allow only the target apps you intend to automate. Review existing approvals in
System Settings > Privacy & Security > Automation > chda. This permission does
not replace the target app's Screen Recording or Accessibility requirements.
The hardened runtime remains enabled and the bundle does not add executable
memory or library-validation exceptions.

An already-running copy retains its original signing capabilities. Installing a
bundle with the updated declarations does not prove that existing terminal
processes have acquired them. Use the app's session-preserving update flow before
checking the same agent connection again; do not terminate active shells to
reset permissions.

The bundle-contract tests check the two declarations and retained hardening.
They do not establish an operating-system permission grant or a successful
Computer Use session. Verify the actual system prompt and app connection on the
installed candidate separately.
