# Testing

Three layers, all run by `cargo test --workspace` (CI: the macOS job).

| Layer | Where | What it covers |
|---|---|---|
| Logic | `#[test]` in each crate | Parsing, models, search and link detection, git and shell integration with real `git`, zsh, bash and fish |
| Scenarios | `crates/chda-ui/src/scenarios/` | A real `WorkspaceView` in GPUI's test platform, driven by key bindings, typed input and agent hook events, with real shells and git repositories in a temporary home |
| Measurements | `#[ignore]` tests | Numbers, not checks: run them by name with `--ignored --nocapture` |

## Rules

- A bug fix comes with a test that fails without the fix: a logic test when
  the bug is in a crate's logic, a scenario when it is in how the window
  behaves.
- A feature's acceptance criteria become a scenario when they describe what
  the user does in the window.
- Scenarios use no OS keyboard or mouse and nothing outside their temporary
  home: `Environment` points the window at temporary config and data
  directories, and `RecordingSystem` records notifications, the Dock badge
  and beeps instead of performing them.
- Wait for conditions (`Harness::wait_for`), never for fixed times. When a
  command's text also appears in the typed command line, wait for the
  output row itself.

## Scenarios

| Scenario | Test | Issues |
|---|---|---|
| S1 New work: create a worktree from the sheet | `worktrees::create_then_delete_a_worktree_in_one_go` | |
| Preparation imports wait for confirmation; copy/setup/open order, failure/edit/retry keep existing edits, owned cancellation and explicit skip retain agent permissions, background completion preserves other input, MCP cannot bypass the gate | `preparation::*`, `chda-core` `preparation::tests::*` | #163 |
| S2 Watching agents: status per pane, Sessions list, Dock badge, notification, jump, notification click | `agents::hook_events_drive_status_badge_jump_and_notification_click` | #12, #13, #25 |
| Title-bar notification history: consecutive events, unread arrivals while open, Escape typing focus, exact source pane, hidden sidebar, narrow bounds, per-window isolation and cold restart | `notifications::*`, `chda-core` `notifications::tests::*`, `handoff::tests::handoff_round_trips_with_binary_snapshots` | #155 |
| Gemini CLI, Copilot CLI and OpenCode report status like Claude Code | `agents::gemini_copilot_and_opencode_report_like_claude_code` | #20 |
| An agent creates a worktree, opens tabs and lists worktrees through `chda mcp` | `mcp::an_agent_creates_a_worktree_opens_tabs_and_lists_worktrees` | #22 |
| S3 Resuming: a worktree lists its agent sessions | `worktrees::worktree_lists_its_agent_sessions` | #41 |
| S3 Resuming several sessions side by side, newest message first | `worktrees::picked_sessions_resume_in_one_tab` | #30 |
| S4 Finishing: merge into main and clean up | `worktrees::merge_into_main_and_clean_up` | |
| Split agent rows keep their own branch labels and focus their exact pane; worktree navigation reveals clipped rows with minimal movement and repository context | `sidebar::split_agent_rows_keep_their_own_labels_and_follow_pane_focus`, `sidebar::navigation_reveals_only_clipped_rows_and_keeps_repository_context` | #149 |
| Sessions and PROJECT scroll independently with both overflowing, the 40% cap and a 180px sidebar; session clicks and tab switches (split-pane agents included) never scroll Sessions and reveal the branch only inside PROJECT, leaving a collapsed PROJECT collapsed; status changes, output and child updates keep rows and offsets; PROJECT navigation (row, Go to worktree, STARRED, notification fallback, collapsed section) leaves Sessions alone; closing a background terminal or idle pane keeps both offsets; a reveal scheduled while PROJECT is collapsed is dropped once the highlight moves; a row click keeps the pane's terminal scroll position; startup restore reveals the focused worktree only inside PROJECT; `sessions-collapsed` persists and migrates the earlier keys | `sessions::*`, `chda-core` `sessions::tests::*`, `chda-config` `legacy_session_collapse_keys_migrate` | #182 |
| PROJECT groups repositories, folders and starred entries, with independent persisted collapse; plain terminal rows follow live agents and close the exact tab | `sidebar::project_is_a_peer_section_and_collapses_without_hiding_sessions`, `sidebar::plain_terminal_rows_follow_agents_and_close_the_exact_background_tab` | #151, #152, #154, #182 |
| Session rows stay in tab order, activity ages and saved alias/branch toggle | `sidebar::session_rows_stay_in_place_and_labels_toggle_persistently`, `sidebar::activity_ages_refresh_without_output_or_session_writes` | #100 |
| Repository drag reordering: down/up, collapsed groups, no click on drag, drops outside or on itself, external folder drop afterwards, refresh and restart; dragging near PROJECT's edges scrolls only PROJECT | `reorder::dragging_repository_headers_reorders_and_persists`, `reorder::dragging_near_project_edges_scrolls_only_project` | #130, #182 |
| STARRED branches: menu starring without duplicates, same branch name in two repositories, navigation and tab reuse, one-click unstar, missing worktree, restart | `starred::starred_branches_navigate_unstar_in_one_click_and_persist` | #129 |
| Background panes: agent title spinners and output redraw neither the workspace nor the sidebar; status changes still do | `redraw::background_title_spinners_and_output_do_not_redraw_the_window` | #134 |
| Live idle agents: pane focus, mixed-state splits, targeted process cleanup, exit and restored-history exclusion | `idle_agents::*`, `agents::directly_typed_codex_tracks_each_turn_and_exits` | #112 |
| S5 Plain terminal: tabs, splits, focus | `workspace::new_tab_split_and_close` | |
| Font size shortcuts | `workspace::font_size_shortcuts_apply_to_every_pane` | #8 |
| Sharing selection: actual VT selection/context action, untouched clipboard until confirmation, Unicode and multiline editing, cancel/typing focus, exact conversation revalidation | `sharing::*`, `chda-agents` `sharing::tests::*`, `chda-term` selection tests | #158 |
| Scrollback search | `search::search_finds_steps_and_closes` | #6 |
| Cached prompt background after theme change; readable colors and opt-out | `terminal_element::tests::theme_switch_keeps_cached_prompt_background_readable`, `terminal_element::tests::contrast_adjustment_preserves_readable_colors_and_can_be_disabled` | |
| Minimum contrast reload preserves pane and unfinished input | `config::minimum_contrast_changes_apply_without_restarting_the_pane` | |
| Live config reload, broken values kept out | `config::ghostty_and_chda_config_changes_apply_live` | #9 |
| Ghostty's `config.ghostty` file name, live | `config::config_ghostty_wins_over_the_legacy_name_live` | #47 |
| Session restore | `restore::tabs_splits_and_directories_come_back` | #10 |
| Session restore reopens agent conversations | `restore::agent_conversations_reopen_in_their_panes` | #31 |
| Deleting a worktree takes one confirmation, even locked, large or with a terminal open in it | `worktrees::create_then_delete_a_worktree_in_one_go` | #29 |
| PR badges per host, login hint for the other | `pull_requests::badges_per_host_and_a_login_hint_for_the_other` | #33 |
| Update branch from upstream: disabled reasons, fast-forward, diverged choice | `worktrees::update_branch_from_upstream_when_safe` | #28 |
| New worktree from a branch, random name, collisions | `worktrees::new_worktree_from_a_branch_with_a_random_name` | #27 |
| Branch notes: sheet, new worktree sheet, external change, palette | `worktrees::branch_notes_show_in_the_sidebar`, `chda` `tests/note.rs` | #36 |
| GitLab and Gitea badges through `glab` and `tea` | `pull_requests::gitlab_and_gitea_repositories_get_badges` | #21 |
| Read-only commit graph: repository menu, merge connections, refs, refresh, mixed restore, paging and retry | `git_graph::graph_menu_merges_refs_refresh_and_mixed_restore`, `git_graph::graph_pages_keep_connections_and_queries_can_retry` | #107 |
| Worktree change size and read-only diff tab | `diff::diff_summary_and_diff_tab` | #15 |
| Hovering session rows does not crash the window | `worktrees::hovering_session_rows_does_not_crash` | |
| Markdown preview from a right-clicked path and the palette | `paths::markdown_path_previews_in_the_browser` | |
| Title bar follows branch notes, external edits, split focus and custom names | `worktrees::title_bar_follows_branch_notes_and_split_focus` | #106 |
| Title bar opens the worktree root in the picked app, pick remembered | `worktrees::title_bar_opens_the_worktree_in_the_picked_app` | #81 |
| Missing worktree folders | `worktrees::missing_worktree_is_marked_and_pruned` | #42 |
| Esc reaches a working agent; an open palette consumes Esc first | `input::escape_reaches_a_working_agent_but_closes_the_palette_first` | |
| IME composition with a hidden cursor | `input::ime_composition_is_placed_even_when_the_app_hides_the_cursor` | #37 |
| Shells get a UTF-8 locale | `input::shells_get_the_locale` | #38 |
| cmd-hover and cmd-click on a URL | `input::cmd_hover_underlines_a_url_and_cmd_click_opens_it` | #7 |
| Mermaid output: cmd-click and the palette open the offline viewer | `diagrams::mermaid_output_opens_in_the_offline_viewer` | #35 |
| Right-click a folder in `ls -la`: open a tab there, `cd` there | `paths::ls_folder_opens_a_terminal_tab_and_cds_there` | #34 |
| Right-click `src/main.rs:12:5`: reveal in Finder, copy relative path | `paths::compiler_error_path_reveals_the_file` | #34 |
| Space-containing parenthesized paths, Unicode and soft wraps use the same complete target for hover, cmd-click and path menus | `paths::parenthesized_space_paths_hover_open_and_reveal_across_wraps` | #164 |
| A quoted path with spaces is one link | `paths::quoted_path_with_spaces_is_one_link` | #34 |
| Drop files on a pane: quoted paths pasted, even right after typing | `paths::dropped_files_paste_as_quoted_paths` | #17 |
| Drop a folder on the sidebar: added as a repository | `paths::dropped_folder_on_the_sidebar_becomes_a_repository` | #17 |
| Add repository when the system cannot open the folder picker: one error, unchanged typed input, config and tabs; silent cancel; retry from the shortcut, palette and menu, several folders at once | `folders::add_repository_survives_an_unavailable_picker_and_retries`, `platform::tests::path_prompt_results_map_to_folder_picks`, `platform::macos::tests::*` | #181 |
| Palette arrow keys choose a later entry | `palette::arrow_keys_choose_a_later_entry` | |
| Theme picker: preview, restore on escape, save on enter | `palette::theme_picker_previews_restores_and_saves` | |
| Menu actions: Settings opens config.toml, agent items explain themselves | `menus::settings_writes_a_missing_config_file`, `menus::agent_items_explain_why_nothing_ran` | |
| Codex session resume with a minimal GUI PATH and a shell-configured install | `menus::codex_sessions_resume_with_the_login_shell_path` | |
| Codex typed directly in a worktree terminal | `agents::directly_typed_codex_tracks_each_turn_and_exits` | |
| Provider details anchor above the left controls, represent only reported windows and update within a narrow window | `quota::details_open_above_both_left_controls_and_visualize_only_reported_windows` | #153 |
| Quota startup and restart wait for reports; empty Claude reports retain usage; multiple windows share one Codex query and retry after its original window closes | `quota::*` | |
| Agent launch presets from the palette and the worktree menu | `menus::agent_presets_start_from_the_palette_and_the_sidebar_menu` | #16 |
| Explicit per-launch Claude/Codex permissions, preserved preset arguments, conflict cancellation and captured exact-session options after preset changes, resume and restore | `agent_launch::*`, `chda-agents` `launch::tests::*` | #159 |
| Managed exit state, exact same-pane restart, repeat clicks, missing CLI/directory/transcript, unknown conversation, previous run output and cold restore, live completion and surviving shells | `agent_restart::*`, `upgrade::tests::stopped_panes_keep_layout_identity_without_inherited_ptys`, `chda-term` `session::tests::opted_in_exit_output_retains_scrollback_before_the_exit_event` | #160 |
| Provider-confirmed children, exact parent routing across windows, parent turn completion, late/duplicate events, disclosure, dedicated pane or details, cold restart and private status payloads | `children::*`, `chda-agents` `children::tests::*`, `ipc::children_tests::*`, `chda-core` `handoff::tests::*` | #161 |
| Close buttons on single/inactive tabs; working/waiting splits, Cancel/Esc focus, real process shutdown and surviving tabs | `closing::tab_close_confirms_inactive_working_splits_and_preserves_cancelled_processes` | #108 |
| Pane-only shortcut, completed/idle/shell immediate close, graph close, stable target ids and final restore cleanup | `closing::shortcut_closes_only_the_focused_pane_and_review_does_not_confirm`, `closing::immediate_tab_close_covers_shell_review_idle_and_graphs`, `closing::confirmed_close_never_targets_a_replacement_tab`, `closing::last_graph_shortcut_removes_restore_data` | #108 |

Not covered by scenarios: anything visual (colors, layout, rendering), since
the test platform draws no pixels; launching real agents (a stand-in
`claude` script takes their place); macOS notifications and the Dock themselves;
the macOS folder picker, whose success, cancellation and creation failure need
a native check before release.

### Native notification glyph regression

Use an isolated development app on macOS with its bundled fonts and private
XDG config/data directories. Before the correction, the title-bar notification
control rendered a boxed question mark instead of a bell, although clicking
it opened the notification history. The icon font contains the bell but has
no `m` glyph, so GPUI refuses it as a primary measurement font.

Check the same title bar with the sidebar visible and hidden, in dark and
light themes and at narrow widths: a recognizable bell must appear before
the app picker, retain the unread count, open the history and return typing
focus after Escape. Retain the original native images for the failing and
corrected builds. The headless notification scenarios do not verify this
glyph; the native check is required before release.

### Native folder picker check

Add repository opens chda's own AppKit folder panel (#181). In an isolated
development app, check a real multi-folder selection and a cancellation. For
the failure path, start a debug build with `CHDA_QA_FOLDER_PICKER=unavailable`:
Add repository must leave the window running with typed input, tabs and
config unchanged, record one error in the notification history, and open the
real panel again once the app runs without the variable. Release builds ignore
the variable. Replacing the executable of a running app produced only an
open-panel service warning on macOS 26.6.2 and still showed the panel, so it
does not reproduce the reported failure.

The directly typed Codex scenario uses a stand-in CLI emitting the exact
Codex 0.160.0 OSC titles, through a real zsh shell. It verifies argument
forwarding, startup idle, consecutive turns, background work, user input,
completion, notification deduplication, two Codex panes in one branch, title
clearing and return to the shell. Shell-level
tests cover bash 3.2, installed newer bash, zsh and fish, including user aliases
or functions and nonzero CLI exit codes. These do not execute a paid model
request or prove the layout emitted by every Codex version.

## Measurements

```sh
# Keystroke-to-echo latency through zsh (#40)
cargo test --release -p chda-term echo_latency -- --ignored --nocapture
# Session indexing over the real ~/.claude and ~/.codex (#41)
CHDA_REAL=1 cargo test --release -p chda-agents index_real_sessions -- --ignored --nocapture
# Window redraw cost with 16 busy background panes, macOS (#134); compare
# a release build with the previous release under similar host load
scripts/measure-background-panes.sh target/release/chda
# Parse and frame throughput
CHDA_BENCH_FILE=/path/to/big.log cargo test --release -p chda-term bench -- --ignored --nocapture
```

## macOS issue release regressions

navigation::* covers recent-pane reuse, exact sidebar identity, manual
collapse under background output, Unicode clipboard insertion, previous
working activity and multi-window IPC/navigation/persistence. The wrapped
path scenario activates every row of a three-row path whose prefix exists.
Terminal tests retain the full target after resizing and scrolling back,
and keep hard-newline file entries separate. Old hook events cannot mark a
newly restored pane as live.
A macOS integration test binds a real localhost listener and verifies its
owning process, listener removal and unknown initial CPU. Quota parsing tests
cover missing windows, multiple limits and account/session scope.
Quota regressions also cover signed-out/API-key accounts, request ordering,
helper-process cleanup and deadlines, account-safe transport failures,
empty reports retaining their real observation time, and prompt retry delays.
The quota scenarios use an isolated stand-in CLI and actual local IPC; they do
not make model requests or prove provider availability.

These tests do not replace native popup, narrow/light-theme, Pages layout
and real CLI resume verification. Record final release evidence separately.

## Session-preserving updates

Release workflow policy tests are `python3 scripts/test-macos-release.py`.
They exercise synthetic source/evidence/artifact and promotion failures, not
native UI, signing or public release proof. See [macOS release operations](releasing.md)
for the distinct native review, before-build, before-deploy and artifact gates.

Run `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`,
`python3 scripts/test-appcast.py` and `python3 scripts/sync-product-docs.py --check`.
The macOS CI job also compiles the universal native helper against the pinned
Sparkle framework; Linux and Windows continue to build without it.

The Rust scenarios exercise duplicate clicks across windows, shared progress,
waiting for active tasks while the original shell remains usable,
failed handoff returning to the original live shell, provisional successor
failure, rejection of unsupported inline-image state without killing the
shell, snapshot fidelity and concurrent event journal/fallback delivery.

For native acceptance on macOS, build `cargo build -p chda`, then run in a
Python environment with `cryptography` installed:

```sh
python scripts/test-macos-update.py --identity "Developer ID Application: Your Name (TEAMID)"
```

The script uses that existing signing identity, generates and deletes an
isolated test Ed25519 seed, and retains private logs/results under its printed
temporary directory. It never updates `/Applications/chda.app`, changes
production signing configuration or publishes a release. Its loopback feed
exists only inside the test bundles. The seven cases cover official Sparkle
installation, invalid signature, network error, cancellation, distinct broker
and GUI processes with three surviving shell PIDs/two windows/a split and
the original active window,
installation failure recovery and a signed replacement executable that exits
before preparation. Native success means the post-commit GUI also answers IPC.

This is separate from notarization and production release acceptance. Before
shipping, verify an administrator-owned app with the actual system approval
and authentication-cancellation dialogs. Native narrow-window reconnection
progress, recovered terminal text and error wrapping, and desktop/narrow Pages
were visually checked during implementation; repeat them for release artifacts.
Production Sparkle signing input must be registered before publishing the first
updater-enabled release. See [decision 0011](decisions/0011-session-preserving-updates.md).

## Private Diff review

`scenarios::diff_review` uses isolated repositories, real shells and native GPUI entities. It covers discovery beside the existing pager; old/new range entry; refresh preserving a draft; stale edits retaining their original anchor; restored virtual tabs without additional PTYs; failed saves and draft copying; fresh batch previews; changed diff and exact conversation rejection; manual clipboard handoff preserving unfinished terminal input. `chda-core::review::tests` and `chda-git::review_diff::tests` cover exact context attachment, private storage, optimistic window conflicts, path and hunk parsing, external-helper exclusion and handoff serialization. These tests do not count as pixel or actual macOS review rounds.
