# Testing

Three layers, all run by `cargo test --workspace` (CI: the macOS and Linux jobs).
Linux installs git, zsh and fish in addition to the GPUI development libraries;
the scenarios use real shells in temporary homes. Windows currently builds in CI.

| Layer | Where | What it covers |
|---|---|---|
| Logic | `#[test]` in each crate | Parsing, models, search and link detection, git and shell integration with real `git`, zsh, bash and fish |
| Scenarios | `crates/chda-ui/src/scenarios/` | A real `WorkspaceView` in GPUI's test platform, driven by key bindings, typed input and agent hook events, with real shells and git repositories in a temporary home |
| Measurements | `#[ignore]` tests | Numbers, not checks: run them by name with `--ignored --nocapture` |

## Rules

Linux package checks use the actual x86_64 AppImage and portable payload with
native X11 and Wayland windows, temporary HOME/XDG paths and software Vulkan.
They are separate from GPUI's headless scenarios. See
[`tests/linux-native/README.md`](../tests/linux-native/README.md) for the desktop
journey, exact-candidate three-round plan and independent visual/IME gates.

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
| Program notifications (OSC 9 and OSC 777): a history entry with the exact pane, an OS notification only out of view, a tab mark, a click that focuses the pane, one a second per pane, no duplicate for a live agent, the notifications setting, and plain truncated text without progress reports | `notifications::program_notifications_*`, `chda-term` `vt::tests::osc_9_and_777_ask_for_plain_truncated_notifications` | #250 |
| Gemini CLI, Copilot CLI and OpenCode report status like Claude Code | `agents::gemini_copilot_and_opencode_report_like_claude_code` | #20 |
| An agent creates a worktree, opens tabs and lists worktrees through `chda mcp` | `mcp::an_agent_creates_a_worktree_opens_tabs_and_lists_worktrees` | #22 |
| An agent lists its repository's sessions, reads a shell's screen without the line being typed and another agent's latest transcript answer, is refused another repository's pane, and waits for a turn to complete, a timeout or a closing pane without moving focus | `mcp::an_agent_lists_reads_and_waits_for_other_sessions_of_its_repository`, `chda-agents` `mcp::tests::*`, `control::tests::*`, `sharing::tests::recent_answers_keep_the_newest_of_the_exact_conversation` | #249 |
| MCP requests drained by another window list both windows, read the original window's shell, wait on its agent, preserve focus and observe hooks before subsequent requests in either direction | `mcp::requests_drained_by_another_window_can_coordinate_without_reentering_it` | |
| S3 Resuming: a worktree lists its agent sessions | `worktrees::worktree_lists_its_agent_sessions` | #41 |
| S3 Resuming several sessions side by side, newest message first | `worktrees::picked_sessions_resume_in_one_tab` | #30 |
| S4 Finishing: merge into main and clean up | `worktrees::merge_into_main_and_clean_up` | |
| Split agent rows keep their own branch labels and focus their exact pane; worktree navigation reveals clipped rows with minimal movement and repository context | `sidebar::split_agent_rows_keep_their_own_labels_and_follow_pane_focus`, `sidebar::navigation_reveals_only_clipped_rows_and_keeps_repository_context` | #149 |
| The repository name pinned at PROJECT's top acts like its header: a click collapses it and keeps the header in view, a right-click opens its menu and hover shows its path; the worktree row underneath gets no click, menu or tooltip, and the wheel still scrolls PROJECT | `sidebar::sticky_repository_header_acts_on_its_repository` | #149 |
| Sessions and PROJECT scroll independently with both overflowing, the 40% cap and a 180px sidebar; session clicks and tab switches (split-pane agents included) never scroll Sessions and reveal the branch only inside PROJECT, leaving a collapsed PROJECT collapsed; status changes, output and child updates keep rows and offsets; PROJECT navigation (row, Go to worktree, STARRED, notification fallback, collapsed section) leaves Sessions alone; closing a background terminal or idle pane keeps both offsets; a reveal scheduled while PROJECT is collapsed is dropped once the highlight moves; a row click keeps the pane's terminal scroll position; startup restore reveals the focused worktree only inside PROJECT; `sessions-collapsed` persists and migrates the earlier keys | `sessions::*`, `chda-core` `sessions::tests::*`, `chda-config` `legacy_session_collapse_keys_migrate` | #182 |
| Reports without a pane for a live agent's worktree, before or after its own events, keep its Sessions row, place, life and status; the worktree's attention still follows them | `sessions::reports_without_a_pane_after_the_live_agents_own_keep_its_row`, `sessions::reports_without_a_pane_before_the_live_agents_own_keep_its_row` | #205 |
| PROJECT groups repositories, folders and starred entries, with independent persisted collapse; plain terminal rows follow live agents and close the exact tab | `sidebar::project_is_a_peer_section_and_collapses_without_hiding_sessions`, `sidebar::plain_terminal_rows_follow_agents_and_close_the_exact_background_tab` | #151, #152, #154, #182 |
| Header buttons, `+ repo` and Alias / Branch, show their own tooltips and never the section toggle's, also when the pointer arrives from the toggle with its tooltip showing | `sidebar::header_buttons_show_their_own_tooltips` | #152, #182 |
| One PROJECT button collapses every repository while any is open and expands them all otherwise; mixed states, the hint after each click, the menu action, retained per-repository toggles, a separate section toggle, terminal focus and restart | `sidebar::one_button_collapses_or_expands_every_repository` | #242 |
| Status icons keep their hues and at least 3:1 contrast on the sidebar, selected and waiting rows, the tab bar and the active tab in light and dark themes | `sidebar_view::tests::status_icons_stand_out_three_to_one_in_light_and_dark_themes` | #182 |
| Session rows stay in tab order, activity ages and saved alias/branch toggle | `sidebar::session_rows_stay_in_place_and_labels_toggle_persistently`, `sidebar::activity_ages_refresh_without_output_or_session_writes` | #100 |
| Repository drag reordering: down/up, collapsed groups, no click on drag, drops outside or on itself, external folder drop afterwards, refresh and restart; dragging near PROJECT's edges scrolls only PROJECT | `reorder::dragging_repository_headers_reorders_and_persists`, `reorder::dragging_near_project_edges_scrolls_only_project` | #130, #182 |
| STARRED branches: menu starring without duplicates, same branch name in two repositories, navigation and tab reuse, one-click unstar, missing worktree, restart | `starred::starred_branches_navigate_unstar_in_one_click_and_persist` | #129 |
| Background panes: agent title spinners and output redraw neither the workspace nor the sidebar; status changes still do | `redraw::background_title_spinners_and_output_do_not_redraw_the_window` | #134 |
| Live idle agents: pane focus, mixed-state splits, targeted process cleanup, exit and restored-history exclusion | `idle_agents::*`, `agents::directly_typed_codex_tracks_each_turn_and_exits` | #112 |
| S5 Plain terminal: tabs, splits, focus | `workspace::new_tab_split_and_close` | |
| Font size shortcuts | `workspace::font_size_shortcuts_apply_to_every_pane` | #8 |
| Sharing selection: actual VT selection/context action, untouched clipboard until confirmation, Unicode and multiline editing, a long last line wrapping in the preview from its first character at 1200×800 and 640×600 without adding newlines to the copy, clicks on wrapped rows, cancel/typing focus, exact conversation revalidation | `sharing::*`, `text_input::tests::clicks_land_on_soft_wrapped_rows_without_changing_the_text`, `chda-agents` `sharing::tests::*`, `chda-term` selection tests | #158 |
| Unquoted full paths with spaces/parentheses/Unicode, existing shorter prefixes, relative paths and soft wraps: identical command-hover, command-click and actual path menu targets; deletion revalidation | `paths::bare_paths_share_complete_hover_click_and_menu_targets_across_soft_wraps`, `chda-term` `links::tests::bare_*` (16-check bound, hard newlines and adjacent paths) | #212 |
| Real Git diff keeps UTF-8 names in stat/patch, original pager/base and unchanged repository config | `chda-core` `refresh::diff_tests::diff_pager_prints_utf8_names_in_stat_and_patch_without_changing_config` | #210 |
| Scrollback search | `search::search_finds_steps_and_closes` | #6 |
| Cached prompt background after theme change; readable colors and opt-out | `terminal_element::tests::theme_switch_keeps_cached_prompt_background_readable`, `terminal_element::tests::contrast_adjustment_preserves_readable_colors_and_can_be_disabled` | |
| Minimum contrast reload preserves pane and unfinished input | `config::minimum_contrast_changes_apply_without_restarting_the_pane` | |
| Live config reload, broken values kept out | `config::ghostty_and_chda_config_changes_apply_live` | #9 |
| Ghostty's `config.ghostty` file name, live | `config::config_ghostty_wins_over_the_legacy_name_live` | #47 |
| Session restore | `restore::tabs_splits_and_directories_come_back` | #10 |
| Native full screen never replaces the saved windowed size, and leaving it records again | `restore::full_screen_keeps_the_windowed_size_for_the_next_launch` | #241 |
| `[[commands]]` run through a shortcut in a new tab, through the palette in a split and in a floating terminal that closes on success, a tab command that closes on success and keeps a failure's exit status, a floating failure that keeps its exit status until Escape, passes Escape to a running program and stops it on a second cmd-w; broken entries and a built-in shortcut clash are reported once, the rest run, and a live fix needs no restart | `commands::*`, `chda-config` `tests::broken_commands_are_reported_and_the_rest_load` | #251 |
| Dragging a tab reorders the tab bar and restores that order; a running pane moves through the tab menu to another tab, to a new tab and to another window with the same shell process and its scrollback, and an emptied tab closes | `moves::*`, `chda-core` `workspace::tests::tabs_reorder_and_panes_move_with_their_state` | #248 |
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
| Linux foreground cwd without OSC 7, byte-preserving paths and deleted directories | `chda-pty` `foreground_cwd_*` | #186 |
| Linux desktop-entry filtering, TCP/UDP and IPv6 socket sampling, notification activation targeting the exact pane | `platform::linux::tests::*`, `platform::resources::linux::tests::*`, `platform::tests::linux_notification_clicks_preserve_the_exact_pane` | #187, #197, #198 |
| cmd-hover and cmd-click on a URL | `input::cmd_hover_underlines_a_url_and_cmd_click_opens_it` | #7 |
| Mermaid output: cmd-click and the palette open the offline viewer | `diagrams::mermaid_output_opens_in_the_offline_viewer` | #35 |
| Right-click a folder in `ls -la`: open a tab there, `cd` there | `paths::ls_folder_opens_a_terminal_tab_and_cds_there` | #34 |
| Right-click `src/main.rs:12:5`: reveal in Finder, copy relative path | `paths::compiler_error_path_reveals_the_file` | #34 |
| Space-containing parenthesized paths, also after prose such as `(see …:5:2)`, Unicode and soft wraps use the same complete target and line/column for hover, cmd-click and path menus, never the prose or an existing shorter prefix | `paths::parenthesized_space_paths_hover_open_and_reveal_across_wraps`, `chda-term` `links::tests::parenthesized_*` | #164 |
| A quoted path with spaces is one link | `paths::quoted_path_with_spaces_is_one_link` | #34 |
| Drop files on a pane: quoted paths pasted, even right after typing | `paths::dropped_files_paste_as_quoted_paths` | #17 |
| Drop a folder on the sidebar: added as a repository | `paths::dropped_folder_on_the_sidebar_becomes_a_repository` | #17 |
| Add repository when the system cannot open the folder picker: one error, unchanged typed input, config and tabs; silent cancel; retry from the shortcut, palette and menu, several folders at once | `folders::add_repository_survives_an_unavailable_picker_and_retries`, `platform::tests::path_prompt_results_map_to_folder_picks`, `platform::macos::tests::*` | #181 |
| Palette arrow keys choose a later entry | `palette::arrow_keys_choose_a_later_entry` | |
| Theme picker: preview, restore on escape, save on enter | `palette::theme_picker_previews_restores_and_saves` | |
| A light and dark theme pair follows the window's appearance in every pane without a restart; the dark picker previews and saves only its half | `palette::a_theme_pair_follows_the_appearance_and_each_picker_keeps_the_other_half`, `chda-config` `tests::theme_pairs_follow_the_appearance_and_keep_the_other_half` | #247 |
| Menus dismiss existing hover hints, suppress new hints until closed, and refresh section toggle captions on the next hover | `menus::context_menu_removes_an_existing_tooltip_and_hover_recovers`, `menus::section_toggle_dismisses_old_caption_before_the_next_hover` | #211, #217 |
| Usage Esc/close and Star/Refresh/Copy link/Open link/Reveal return real keyboard input to the original pane; Open terminal keeps its destination | `menus::usage_escape_and_close_restore_the_original_terminal_input`, `menus::nonnavigation_menus_return_input_and_new_terminal_keeps_its_destination` | #219 |
| Launch sheet arrow policy selection and Tab/Shift-Tab group/Cancel/Start navigation; Enter/Space press and release never reach a newly started CLI | `menus::launch_permissions_and_buttons_are_operable_without_the_mouse`, `agent_launch::supported_launches_show_the_inherited_policy_before_starting`, `agent_launch::confirming_a_launch_sends_nothing_to_the_new_terminal`, `agent_launch::space_launch_sends_neither_the_press_nor_release_to_the_new_terminal` | #219 |
| Setup badge mouse clicks use its actual bounds and clamp within 640×600; rejected copy entries retain the file and concrete reason; tracked case aliases remain rejected | `preparation::setup_menu_uses_the_badge_bounds_and_stays_inside_a_small_window`, `chda-core` `preparation::tests::rejected_source_names_the_entry_and_the_specific_reason`, `preparation::tests::source_git_eligibility_matches_check_ignore_without_accepting_tracked_files` | #220 |
| Menu actions: Settings opens config.toml, agent items explain themselves | `menus::settings_writes_a_missing_config_file`, `menus::agent_items_explain_why_nothing_ran` | |
| Default Codex Legacy history appears once in the worktree session list and resumes its exact conversation with a minimal GUI PATH and a shell-configured install whose login shell takes longer than the launch wait | `menus::codex_legacy_sessions_are_listed_and_resume_with_a_slow_login_shell_path` | #226, #243 |
| A login shell slower than the two-second launch wait: the PATH is captured in the background, Codex usage appears without a restart, restored Codex conversations wait for it instead of opening a shell, and a missing CLI names the searched PATH or the failed capture | `quota::codex_usage_appears_when_the_login_shell_is_slower_than_the_launch_wait`, `restore::codex_conversations_wait_for_a_slow_login_shell_path_to_reopen`, `quota::missing_codex_names_the_paths_searched_and_a_failed_capture` | #243 |
| Codex Legacy events, Paginated items and response-only user input preserve repeated prompts, ignore duplicate model-input copies and eight recognized complete context wrappers, and re-index the old cache | `chda-agents` `codex::tests::legacy_user_events_keep_repeated_prompts_without_counting_response_copies`, `codex::tests::response_only_history_uses_user_input_and_ignores_other_roles`, `codex::tests::paginated_user_items_take_precedence_over_response_copies`, `codex::tests::response_fallback_excludes_complete_context_blocks_but_keeps_normal_prompts`, `session::tests::pre_legacy_cache_reparses_an_unchanged_codex_rollout` | #226 |
| Codex typed directly in a worktree terminal | `agents::directly_typed_codex_tracks_each_turn_and_exits` | |
| Provider details anchor above the left controls, represent only reported windows and update within a narrow window | `quota::details_open_above_both_left_controls_and_visualize_only_reported_windows` | #153 |
| Claude usage reports drained by the other window route to the exact pane, refresh the shared reading in both directions, and keep focus and both shells running without nested entity updates | `quota::claude_quota_routed_from_another_window_keeps_both_windows_running` | |
| Quota startup and restart wait for reports; empty Claude reports retain usage; multiple windows share one Codex query and retry after its original window closes | `quota::*` | |
| Claude usage follows the focused session's configuration folder: a session without a reading shows the folder's newest reading, folders never mix, empty states say what chda waits for, and the last reading per folder survives a restart marked previous run | `status_bar::tests::*`, `quota::a_restart_keeps_the_last_claude_reading_until_a_new_one` | #244 |
| Agent launch presets from the palette and the worktree menu | `menus::agent_presets_start_from_the_palette_and_the_sidebar_menu` | #16 |
| Explicit per-launch Claude/Codex permissions, preserved preset arguments, conflict cancellation and captured exact-session options after preset changes, resume and restore | `agent_launch::*`, `chda-agents` `launch::tests::*` | #159 |
| Confirming the launch sheet with Return (press and release) or Start sends no bytes to the launched CLI; headless only, so the native macOS key path still needs a native check | `agent_launch::confirming_a_launch_sends_nothing_to_the_new_terminal` | #159 |
| A new Codex conversation's first turn: the notify of the ephemeral title request, which has no saved transcript, neither ends the turn, records its identity nor announces it; Codex's title restores Working over a stale status, ✕ and cmd-w ask while the title reports work, and a conversation's notify before its Ready title still announces the turn once | `agents::codex_first_turn_helper_notify_keeps_working_and_close_confirms`, `chda-agents` `hook::tests::codex_notify_reports_only_saved_threads`, `children::tests::codex_notify_*` | #154, #159, #160, #161 |
| Managed exit state, exact same-pane restart, repeat clicks, missing CLI/directory/transcript, unknown conversation, previous run output and cold restore, live completion and surviving shells | `agent_restart::*`, `upgrade::tests::stopped_panes_keep_layout_identity_without_inherited_ptys`, `chda-term` `session::tests::every_exit_retains_scrollback_before_the_exit_event` | #160 |
| Provider-confirmed children, exact parent routing across windows, parent turn completion, late/duplicate events, a one-line summary on the parent row with the right-click child list, dedicated pane or details, a 180px sidebar, cold restart and private status payloads | `children::*`, `chda-agents` `children::tests::*`, `ipc::children_tests::*`, `chda-core` `handoff::tests::*` | #161, #246 |
| An Unknown report, a child missing from one poll or a failed Codex poll never replaces Turn complete or Ended, and a failed poll keeps Working too; the summary counts unknown and waiting children on their own instead of claiming 0 working | `children::codex_poll_failure_keeps_child_states`, `chda-agents` `children::tests::unknown_observation_keeps_turn_completion`, `children::tests::unknown_observation_keeps_ended`, `sidebar_view::tests::child_counts_never_claim_zero_working_and_keep_waiting_apart` | #161, #246 |
| Close buttons on single/inactive tabs; working/waiting splits, Cancel/Esc focus, real process shutdown and surviving tabs | `closing::tab_close_confirms_inactive_working_splits_and_preserves_cancelled_processes` | #108 |
| `exit` closes the tab and shows the nearest tab of the same branch, else no tab with the branch still selected; + opens a terminal there; cmd-w follows the same rule, an emptied window stays and cmd-w then closes it | `closing::exit_closes_the_tab_and_the_window_stays_on_its_branch` | |
| Stop and close from the pane shortcut or a tab's close button, and a pane whose process ends, clear the Working status a Claude Code (hooks) or Codex (titles) agent gave its worktree in PROJECT and Update branch; another working pane in the branch keeps it | `closing::stop_and_close_clears_the_worktree_working_status` | #154 |
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

## Terminal engine integrity (#224)

Run `python3 scripts/test-prepare-ghostty.py` for synthetic preparation guards
and `python3 scripts/prepare-ghostty.py --check` for the real prepared source.
With the pinned Zig on PATH, `python3 scripts/test-ghostty-build-guard.py`
checks that actual Cargo rejects a new root file and a changed nested source,
then restores the exact bytes and verifies a clean build.
The `vt::tests::scrolled_resize_keeps_debug_integrity` subprocess regression
covers row growth, preserved cursors and reflow without losing the test suite
to Zig's abort. `growing_rows_preserves_a_viewport_still_in_history` checks the
history anchor. The checked patch also carries direct PageList regressions. The source check
runs these alongside the upstream recycled-row, trimmed-row and viewport-pin
regressions and process-directory Kitty restrictions; it keeps Debug integrity
enabled throughout:

```sh
python3 scripts/prepare-ghostty.py --test
```

Keep Debug integrity enabled. Before the patch, the newly active viewport
cases abort with `ViewportPinInsufficientRows` during `grow()`. Native font and
window resizing, copied hard line breaks and Kitty temporary-file images still
require the final macOS candidate check.


## 0.1.23 activity and process lifecycle

`scenarios::lifecycle` uses real PTYs under isolated homes to check quiet theme/resize activity, echo-free committed input, an exact hook directory over a stale shell directory, missing confirmed directories without fallback resume, every close path with working/waiting/unknown children and Cancel-default keyboard controls, a background shell that exits with an error code closing its pane without moving focus, and a failed managed agent's stopped pane closing without a hidden child confirmation. Existing managed-agent restart, live-shell completion, split close, Sessions focus and update/handoff scenarios remain in the workspace suite.

`chda-core::restore::tests` verifies directory precedence, no fallback from missing confirmed directories, conversation-specific cwd preservation, and legacy saved JSON. `Event::Frame { has_output }` counts only PTY output; `Event::Input` reports only nonempty input successfully written to the PTY. Focus reports and terminal presentation changes do not count as activity. The shared `PaneRun` retains the saved JSON `run` and `state` shape.


### Stable sidebar activation and split identification

`scenarios::sidebar_interactions` dispatches actual mouse down/up events for PROJECT and STARRED one-click activation, double- and triple-click deduplication, right-click dismissal and Open terminal, existing-pane reuse and agent-default launch gating. Background hook delivery keeps the row under the pointer and a captured menu target unchanged. Two synthetic agents run in real PTYs at 18pt and 640×600; #1/#2 bounds are checked at 280px and 180px. Shell prompt redraw is excluded from this synthetic agent fixture by running a process before the hooks. Native visual acceptance remains separate. Core tests cover main/name/path order and detached rows; color tests preserve hues and enforce 3:1 for marks and 4.5:1 for diff text across normal/selected/waiting/hovered light and dark surfaces. Existing PR palette tests remain unchanged.


## 0.1.23 selection-scoped sharing

`chda-agents::sharing::tests` covers real CLI/VT partial code and nested-list captures, Unicode and partial line boundaries, duplicate/overlapping source ranges, malformed or incomplete exact transcripts and unconfirmed versions. Only uniquely selected source ranges are copied; hidden fences and other answer text are excluded. `scenarios::sharing` retains actual PTY selection, preview scope, ordinary Copy, clipboard-before-confirm, edited text, cancellation and update/navigation regressions. Headless scenarios do not replace the final native macOS and unsent composer verification.

`scripts/test-macos-release.py` requires all 16 ISSUE conditions in each of three review rounds and both separate E2Es starting with v0.1.23, while preserving historical v0.1.21/v0.1.22 exact coverage and rejecting their exceptions for v0.1.23. These are synthetic validator tests and never release evidence.

Linux package acceptance records supported startup separately from known incompatible graphics environments. The Ubuntu 20.04 run uses glibc 2.31 with official Mesa 21.2, but actual application startup failed because the Vulkan adapter limits were below the graphics engine's request. The separate glibc 2.31 environment uses QA-only Mesa 23.3.6; compatibility-floor rendered runtime acceptance is still pending. A mapped window and successful PTY input do not establish rendered startup. Debian 11 stock Mesa 20 failed to expose a compatible Vulkan/EGL X11 surface; its negative check must retain `productStartupStatus: failed` and `supportedEnvironment: false`. Neither failed environment satisfies a positive startup requirement. The Linux guide preserves these limits and the independently confirmed unpatched IBus 1.5.32 XIM server crash.

`chda-agents::ipc::executable::appimage::tests` reproduces an AppImage helper path
lost when its temporary runtime directory disappears. A private executable
launcher retains the original package only when APPDIR contains this exact
packaged chda executable and its desktop marker; portable executables ignore
another application's inherited AppImage environment. Each helper uses its own
temporary extraction directory, preserves stdin and exits with its child status.
Direct termination forwards to the helper process group before cleanup. Private
metadata stays non-executable. Native package acceptance separately executes frozen
pre-restart hook commands after removing the stopped runtime's isolated temp
directory and relaunching under a different TMPDIR.
