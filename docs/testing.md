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
| S2 Watching agents: status per pane, ACTIVE list, Dock badge, notification, jump, notification click | `agents::hook_events_drive_status_badge_jump_and_notification_click` | #12, #13, #25 |
| Gemini CLI, Copilot CLI and OpenCode report status like Claude Code | `agents::gemini_copilot_and_opencode_report_like_claude_code` | #20 |
| An agent creates a worktree, opens tabs and lists worktrees through `chda mcp` | `mcp::an_agent_creates_a_worktree_opens_tabs_and_lists_worktrees` | #22 |
| S3 Resuming: a worktree lists its agent sessions | `worktrees::worktree_lists_its_agent_sessions` | #41 |
| S3 Resuming several sessions side by side, newest message first | `worktrees::picked_sessions_resume_in_one_tab` | #30 |
| S4 Finishing: merge into main and clean up | `worktrees::merge_into_main_and_clean_up` | |
| ACTIVE rows stay in tab order, activity ages and saved alias/branch toggle | `sidebar::active_tabs_stay_in_place_and_labels_toggle_persistently`, `sidebar::activity_ages_refresh_without_output_or_session_writes` | #100 |
| Live idle agents: pane focus, mixed-state splits, targeted process cleanup, exit and restored-history exclusion | `idle_agents::*`, `agents::directly_typed_codex_tracks_each_turn_and_exits` | #112 |
| S5 Plain terminal: tabs, splits, focus | `workspace::new_tab_split_and_close` | |
| Font size shortcuts | `workspace::font_size_shortcuts_apply_to_every_pane` | #8 |
| Scrollback search | `search::search_finds_steps_and_closes` | #6 |
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
| A quoted path with spaces is one link | `paths::quoted_path_with_spaces_is_one_link` | #34 |
| Drop files on a pane: quoted paths pasted, even right after typing | `paths::dropped_files_paste_as_quoted_paths` | #17 |
| Drop a folder on the sidebar: added as a repository | `paths::dropped_folder_on_the_sidebar_becomes_a_repository` | #17 |
| Palette arrow keys choose a later entry | `palette::arrow_keys_choose_a_later_entry` | |
| Theme picker: preview, restore on escape, save on enter | `palette::theme_picker_previews_restores_and_saves` | |
| Menu actions: Settings opens config.toml, agent items explain themselves | `menus::settings_writes_a_missing_config_file`, `menus::agent_items_explain_why_nothing_ran` | |
| Codex session resume with a minimal GUI PATH and a shell-configured install | `menus::codex_sessions_resume_with_the_login_shell_path` | |
| Codex typed directly in a worktree terminal | `agents::directly_typed_codex_tracks_each_turn_and_exits` | |
| Agent launch presets from the palette and the worktree menu | `menus::agent_presets_start_from_the_palette_and_the_sidebar_menu` | #16 |
| Close buttons on single/inactive tabs; working/waiting splits, Cancel/Esc focus, real process shutdown and surviving tabs | `closing::tab_close_confirms_inactive_working_splits_and_preserves_cancelled_processes` | #108 |
| Pane-only shortcut, completed/idle/shell immediate close, graph close, stable target ids and final restore cleanup | `closing::shortcut_closes_only_the_focused_pane_and_review_does_not_confirm`, `closing::immediate_tab_close_covers_shell_review_idle_and_graphs`, `closing::confirmed_close_never_targets_a_replacement_tab`, `closing::last_graph_shortcut_removes_restore_data` | #108 |

Not covered by scenarios: anything visual (colors, layout, rendering), since
the test platform draws no pixels; launching real agents (a stand-in
`claude` script takes their place); macOS notifications and the Dock themselves.

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
# Parse and frame throughput
CHDA_BENCH_FILE=/path/to/big.log cargo test --release -p chda-term bench -- --ignored --nocapture
```

## macOS issue release regressions

navigation::* covers recent-pane reuse, exact sidebar identity, manual
collapse under background output, Unicode clipboard insertion, previous
working activity and multi-window IPC/navigation/persistence. The wrapped
path scenario activates every row of a three-row path whose prefix exists.
A macOS integration test binds a real localhost listener and verifies its
owning process, listener removal and unknown initial CPU. Quota parsing tests
cover missing windows, multiple limits and account/session scope.

These tests do not replace native popup, narrow/light-theme, Pages layout
and real CLI resume verification. Record final release evidence separately.
