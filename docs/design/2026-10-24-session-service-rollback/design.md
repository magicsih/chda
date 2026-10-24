# Session-service rollback

The owner approved returning to v1.3.0 and withdrawing v1.4.0, v1.5.0 and v1.5.1 on 2026-10-24 after terminal input froze and the 1.5.1 app could not start a shell. The independent macOS session service first shipped in 1.4.0.

## Scope and approval

Restore the v1.3.0 app-owned terminal implementation, key bindings, window lifecycle, bundle and updater behavior. Remove the session-service crate and its process and persistence integration. Keep the existing workspace version and generated release history; release-plz owns future version changes. Keep historical design and publication records explicitly marked as withdrawn. Reject withdrawn tags before every release stage, so previous approvals cannot republish them.

Public distribution uses the unchanged, previously signed v1.3.0 archive. The three withdrawn GitHub Releases retain their assets privately as drafts, and their Git tags remain in history. Latest downloads, the Sparkle feed and Homebrew return to v1.3.0. No replacement archive or new release is created. Existing session-service processes and profile files are left intact; v1.3.0 opens its own terminals and cannot reconnect to those runs. Restoring saved layout is distinct from keeping processes alive after Quit.

The owner declined backups of personal settings and terminal content. The temporary local backup created before that instruction was removed. Actual profile files were retained. Public release assets and metadata were separately copied to NAS and verified by direct readback of size and SHA-256.

## Validation

L1 development checks cover source equivalence to v1.3.0, Cargo metadata with no session-service dependency, formatting, workspace clippy, workspace tests, generated product documentation, appcast and release policy tests. The new release-policy regression must fail before the withdrawal guard. CI validates macOS and Linux tests and Windows builds. This is restoration of the existing interface, with no new app screen or redesign.

The installed signed 1.3.0 archive passed checksum, Developer ID, Gatekeeper notarization and universal architecture checks. Its shell process and rendered prompt were observed. The native control tool returned `noWindowsAvailable` for coordinate clicks. Using the actual new-terminal keyboard shortcut, pasting `printf` and pressing Enter then produced the expected `chda_rollback_shell_ready` output. The old update notice was gone after the confirmed release cache was refreshed and the app reopened. The test tab was closed after confirmation. This focused native check is separate from full native regression or signed-update acceptance.

## Retained evidence

The original signed archives and pre-withdrawal release metadata are retained at `smb://nas1dual/Data/Backup/agent-artifacts/chda/2026-10-24/068ef5fd9852d18d011e7a0cbc72304f84b2108b/rollback-d8c472624f`. The verification manifest records all 29 files and their SHA-256 values. No personal settings or terminal content were transmitted to NAS.
