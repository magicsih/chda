//! The repository / worktree / session tree with git and agent badges.

use std::path::PathBuf;

use crate::status_icon::status_icon;

use chda_core::{
    AgentStatus, CheckState, DiffSummary, GitBadges, PrInfo, PrState, RepoEntry, SessionEntry,
    Sidebar, WorktreeEntry, activity_age, relative_age,
};
use gpui::{
    AnyElement, App, ClickEvent, Context, ElementId, EventEmitter, FocusHandle, Focusable, Hsla,
    MouseButton, MouseDownEvent, Pixels, Point, Render, Window, div, prelude::*,
};

/// What the user asked for in the sidebar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarEvent {
    /// Focus a pane in this worktree, or open one.
    OpenWorktree(PathBuf),
    /// Show the context menu for a worktree at a window position.
    WorktreeMenu(PathBuf, Point<Pixels>),
    RepoMenu(PathBuf, Point<Pixels>),
    /// Ask for a branch name for a new worktree of this repo.
    NewWorktree(PathBuf),
    /// Resume agent sessions: one opens a tab, several open one tab with a
    /// pane each.
    ResumeSessions(Vec<SessionPick>),
    AddRepo,
    RefocusTerminal,
    /// Folders were dropped on the sidebar: add them as repositories.
    AddRepos(Vec<PathBuf>),
    /// Open a URL (a pull request badge was clicked).
    OpenUrl(String),
    /// Focus an open tab from the activity list.
    FocusTab(chda_core::TabId),
    FocusPane(chda_core::PaneId),
    ClosePane(chda_core::PaneId),
    ToggleActiveLabel,
    /// The status dot of a worktree was clicked: go to its agent's pane.
    JumpToAgent(PathBuf),
    /// Open a read-only tab with the worktree's diff against its base.
    OpenDiff(PathBuf),
    /// Go to a STARRED branch's worktree.
    OpenStarred {
        repo: PathBuf,
        branch: String,
    },
    /// The star of a STARRED row was clicked: take the branch out.
    Unstar {
        repo: PathBuf,
        branch: String,
    },
    /// A repository header was dropped on another repository: show `from`
    /// where `to` is.
    MoveRepo {
        from: PathBuf,
        to: PathBuf,
    },
}

/// A repository header being dragged to a new place in the list.
#[derive(Clone)]
pub(crate) struct RepoDrag {
    path: PathBuf,
    index: usize,
}

/// What follows the mouse while a repository header is dragged.
struct RepoDragPreview {
    name: String,
    fg: Hsla,
    bg: Hsla,
}

impl Render for RepoDragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_sm()
            .bg(self.bg)
            .border_1()
            .border_color(self.fg.opacity(0.3))
            .text_sm()
            .font_weight(gpui::FontWeight::BOLD)
            .text_color(self.fg)
            .shadow_md()
            .child(self.name.clone())
    }
}

/// Distance from the list's top or bottom edge that scrolls while dragging.
const DRAG_SCROLL_EDGE: f32 = 32.0;

/// A past agent session to resume, and the worktree it belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionPick {
    pub worktree: PathBuf,
    pub agent: String,
    pub session: String,
}

/// How an agent is labeled in session lists.
#[derive(Clone, Debug)]
pub struct AgentLabel {
    /// Agent id, e.g. `claude`.
    pub id: String,
    /// e.g. `CC`.
    pub short: String,
    /// e.g. `Claude Code`.
    pub name: String,
    pub color: Hsla,
}

/// Label colors, by adapter order.
const AGENT_COLORS: [u32; 5] = [0xcba6f7, 0x94e2d5, 0xf9e2af, 0xf5c2e7, 0x74c7ec];

impl AgentLabel {
    pub fn new(index: usize, id: &str, short: String, name: &str) -> Self {
        Self {
            id: id.to_owned(),
            short,
            name: name.to_owned(),
            color: gpui::rgb(AGENT_COLORS[index % AGENT_COLORS.len()]).into(),
        }
    }
}

/// Badge icons, from the bundled Symbols Nerd Font Mono. Each badge's
/// tooltip says what it means.
const ICON_MERGED: &str = "\u{f062d}"; // nf-md-source_merge
const ICON_MISSING: &str = "\u{f0dcc}"; // nf-md-folder_alert
const ICON_PANES: &str = "\u{f04e9}"; // nf-md-tab
const ICON_DELETING: &str = "\u{f0a7a}"; // nf-md-trash_can_outline
const ICON_BUSY: &str = "\u{eb19}"; // nf-cod-loading

/// A badge icon in the icon font.
fn icon_text(icon: &'static str) -> gpui::Div {
    div().font_family(crate::fonts::SYMBOLS_FAMILY).child(icon)
}

/// A session's usage for its tooltip: the token breakdown, the cost at API
/// list prices, and the plan limit when the agent reports one. Empty when
/// the transcript records no usage.
fn usage_lines(usage: &chda_core::agents::Usage) -> String {
    use chda_core::agents::compact_tokens as k;
    if usage.is_empty() {
        return String::new();
    }
    let s = usage.summed();
    let mut parts = vec![format!("{} in", k(s.input))];
    let writes = s.cache_write_5m + s.cache_write_1h;
    if writes > 0 {
        parts.push(format!("{} cache write", k(writes)));
    }
    if s.cache_read > 0 {
        parts.push(format!("{} cache read", k(s.cache_read)));
    }
    parts.push(format!("{} out", k(s.output)));
    let mut text = format!("\n{} tokens: {}", k(usage.total()), parts.join(", "));
    let models: Vec<&str> = usage.models.iter().map(|m| m.model.as_str()).collect();
    let (dollars, complete) = usage.cost();
    if dollars > 0.0 {
        let approx = if complete {
            "\u{2248}"
        } else {
            "at least \u{2248}"
        };
        text.push_str(&format!(
            "\n{approx} ${dollars:.2} at API list prices ({}). Subscriptions are not billed per token.",
            models.join(", ")
        ));
    }
    if let Some(limit) = usage.limit {
        let window = match limit.window_minutes {
            10_080 => "weekly".to_owned(),
            m if m % 1_440 == 0 => format!("{}-day", m / 1_440),
            m if m % 60 == 0 => format!("{}-hour", m / 60),
            m => format!("{m}-minute"),
        };
        text.push_str(&format!(
            "\nPlan limit: {}% of the {window} window used at the last turn.",
            limit.used_percent
        ));
    }
    text
}

/// Tooltip for an operation running on a worktree, e.g. "deleting".
fn busy_tooltip(busy: &str) -> String {
    let mut chars = busy.chars();
    let first = chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_default();
    format!("{first}{} this worktree\u{2026}", chars.as_str())
}

pub struct SidebarView {
    pub model: Sidebar,
    pub active_label: chda_config::ActiveLabel,
    /// The first session index has finished.
    pub sessions_loaded: bool,
    /// Labels and names of the agents chda knows.
    pub agents: Vec<AgentLabel>,
    /// Sessions cmd-clicked to resume together, in click order.
    pub picked: Vec<SessionPick>,
    expanded: Vec<PathBuf>,
    /// Highlighted worktree (e.g. after a notification for a closed pane).
    pub(crate) selected: Option<PathBuf>,
    pub(crate) active_tab: Option<chda_core::TabId>,
    /// Frame of the "working" spinner, advanced by the workspace.
    pub(crate) spin: u32,
    pub(crate) scroll: gpui::ScrollHandle,
    reveal: Option<PathBuf>,
    pending_navigation: bool,
    focus_handle: FocusHandle,
    fg: Hsla,
    bg: Hsla,
    /// Folders dragged over the window from another app.
    dragged: crate::external_drop::Dragged,
}

impl EventEmitter<SidebarEvent> for SidebarView {}

pub(crate) fn no_agent_color() -> Hsla {
    gpui::rgb(0x6c7086).into()
}

/// Background of a row whose agent waits for input.
fn waiting_tint() -> Hsla {
    status_color(AgentStatus::WaitingInput).opacity(0.16)
}

pub fn status_color(status: AgentStatus) -> Hsla {
    match status {
        AgentStatus::Idle => gpui::rgb(0xf9e2af).into(),
        AgentStatus::Working => gpui::rgb(0x89b4fa).into(),
        AgentStatus::WaitingInput => gpui::rgb(0xfab387).into(),
        AgentStatus::Review => gpui::rgb(0xa6e3a1).into(),
    }
}

impl SidebarView {
    pub fn new(fg: Hsla, bg: Hsla, agents: Vec<AgentLabel>, cx: &mut Context<Self>) -> Self {
        Self {
            model: Sidebar::new(),
            active_label: Default::default(),
            sessions_loaded: false,
            agents,
            picked: Vec::new(),
            expanded: Vec::new(),
            selected: None,
            active_tab: None,
            spin: 0,
            scroll: gpui::ScrollHandle::new(),
            reveal: None,
            pending_navigation: false,
            focus_handle: cx.focus_handle(),
            fg,
            bg,
            dragged: Default::default(),
        }
    }

    /// Navigation reveals its target once; background refreshes only update identity.
    pub fn select(&mut self, worktree: &std::path::Path, cx: &mut Context<Self>) {
        self.select_context(self.active_tab, Some(worktree), true, cx);
    }

    pub(crate) fn select_context(
        &mut self,
        tab: Option<chda_core::TabId>,
        cwd: Option<&std::path::Path>,
        navigation: bool,
        cx: &mut Context<Self>,
    ) {
        let found = cwd
            .and_then(|cwd| self.model.worktree_for_path(cwd))
            .map(|(r, w)| (r.path.clone(), w.path.clone()));
        self.active_tab = tab;
        if navigation {
            self.pending_navigation = true;
        }
        self.selected = found.as_ref().map(|(_, path)| path.clone());
        if let Some((repo, path)) = found
            && self.pending_navigation
        {
            if let Some(r) = self.model.repo_mut(&repo) {
                r.collapsed = false;
            }
            self.reveal = Some(path);
            self.pending_navigation = false;
        }
        cx.notify();
    }

    fn collapse_all(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        for repo in &mut self.model.repos {
            repo.collapsed = collapsed;
        }
        self.reveal = None;
        self.pending_navigation = false;
        cx.emit(SidebarEvent::RefocusTerminal);
        cx.notify();
    }

    /// New colors after a config reload.
    pub fn set_colors(&mut self, fg: Hsla, bg: Hsla, cx: &mut Context<Self>) {
        self.fg = fg;
        self.bg = bg;
        cx.notify();
    }

    /// Scroll the list when a dragged repository header nears its top or
    /// bottom edge, so a target outside the visible part can be reached.
    fn scroll_while_dragging(&mut self, e: &gpui::DragMoveEvent<RepoDrag>, cx: &mut Context<Self>) {
        let y = e.event.position.y;
        let edge = gpui::px(DRAG_SCROLL_EDGE);
        let step = if y < e.bounds.top() + edge {
            gpui::px(12.0)
        } else if y > e.bounds.bottom() - edge {
            gpui::px(-12.0)
        } else {
            return;
        };
        let mut offset = self.scroll.offset();
        let max = self.scroll.max_offset().y;
        offset.y = (offset.y + step).clamp(-max, gpui::px(0.0));
        if offset != self.scroll.offset() {
            self.scroll.set_offset(offset);
            cx.notify();
        }
    }

    /// Show or hide a worktree's past sessions.
    pub(crate) fn toggle_expanded(&mut self, path: &PathBuf, cx: &mut Context<Self>) {
        if let Some(i) = self.expanded.iter().position(|p| p == path) {
            self.expanded.remove(i);
        } else {
            self.expanded.push(path.clone());
        }
        cx.notify();
    }

    fn render_repo(&self, index: usize, repo: &RepoEntry, cx: &mut Context<Self>) -> AnyElement {
        let fg = self.fg;
        let path = repo.path.clone();
        let collapsed = repo.collapsed;
        let drag = RepoDrag {
            path: path.clone(),
            index,
        };
        let header = div()
            .id(ElementId::Name(
                format!("repo:{}", repo.path.display()).into(),
            ))
            .debug_selector(move || format!("repo-{index}"))
            .on_drag(drag, {
                let (name, fg, bg) = (repo.name.clone(), fg, self.bg);
                move |_, _, _, cx| {
                    crate::external_drop::start_internal(cx);
                    cx.new(|_| RepoDragPreview {
                        name: name.clone(),
                        fg,
                        bg: crate::workspace_view::blend(bg, fg, 0.12),
                    })
                }
            })
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .cursor_pointer()
            .hover(|s| s.bg(fg.opacity(0.08)))
            .on_click({
                let path = path.clone();
                cx.listener(move |this, _, _, cx| {
                    if let Some(r) = this.model.repo_mut(&path) {
                        r.collapsed = !r.collapsed;
                    }
                    cx.notify();
                })
            })
            .on_mouse_down(MouseButton::Right, {
                let path = path.clone();
                cx.listener(move |_, e: &MouseDownEvent, _, cx| {
                    cx.emit(SidebarEvent::RepoMenu(path.clone(), e.position));
                })
            })
            .child(div().w_3().text_color(fg.opacity(0.6)).child(if collapsed {
                "\u{25b8}"
            } else {
                "\u{25be}"
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child(repo.name.clone()),
            )
            .when(!repo.folder, |d| {
                d.child(
                    div()
                        .id(ElementId::Name(
                            format!("repo-add:{}", repo.path.display()).into(),
                        ))
                        .px_1()
                        .rounded_sm()
                        .text_color(fg.opacity(0.7))
                        .hover(|s| s.bg(fg.opacity(0.15)))
                        .on_click({
                            let path = path.clone();
                            cx.listener(move |_, _, _, cx| {
                                cx.stop_propagation();
                                cx.emit(SidebarEvent::NewWorktree(path.clone()));
                            })
                        })
                        .child("+"),
                )
            });
        // Dropping a header on this group moves that repository here; the
        // line shows which side it lands on.
        let marker = status_color(AgentStatus::Working);
        let mut col = div()
            .flex()
            .flex_col()
            .drag_over::<RepoDrag>(move |style, drag, _, _| match drag.index.cmp(&index) {
                std::cmp::Ordering::Greater => style.border_t_2().border_color(marker),
                std::cmp::Ordering::Less => style.border_b_2().border_color(marker),
                std::cmp::Ordering::Equal => style,
            })
            .on_drop(cx.listener({
                let path = path.clone();
                move |_, drag: &RepoDrag, _, cx| {
                    if drag.path != path {
                        cx.emit(SidebarEvent::MoveRepo {
                            from: drag.path.clone(),
                            to: path.clone(),
                        });
                    }
                }
            }))
            .child(header);
        if let Some(err) = &repo.error {
            col = col.child(
                div()
                    .px_4()
                    .text_xs()
                    .text_color(gpui::rgb(0xf38ba8))
                    .child(err.clone()),
            );
        }
        if let Some(hint) = &repo.pr_hint {
            col = col.child(
                div()
                    .px_4()
                    .text_xs()
                    .text_color(fg.opacity(0.5))
                    .child(hint.clone()),
            );
        }
        if !collapsed {
            for wt in &repo.worktrees {
                col = col.child(self.render_worktree(wt, repo.folder, cx));
            }
        }
        col.into_any_element()
    }

    /// A worktree's status icon and tooltip: `None` (gray) when no agent
    /// runs there, `Idle` (yellow) for a live idle agent.
    fn status_dot(&self, wt: &WorktreeEntry) -> (Option<AgentStatus>, String) {
        let status = wt.status();
        let live_idle = self.model.idle_agents.iter().any(|i| {
            i.cwd
                .as_ref()
                .and_then(|cwd| self.model.worktree_for_path(cwd))
                .is_some_and(|(_, w)| w.path == wt.path)
        });
        match status {
            AgentStatus::Idle if live_idle => (
                Some(status),
                "A live agent is idle here, ready for another task. Click to go there.".into(),
            ),
            AgentStatus::Idle => (None, status_tooltip(wt, &self.agents)),
            _ => (Some(status), status_tooltip(wt, &self.agents)),
        }
    }

    /// One STARRED row: the star (click to unstar), status dot, alias or
    /// branch name and repository. A branch without a worktree stays listed,
    /// dimmed, and does not navigate.
    fn render_starred(
        &self,
        index: usize,
        starred: &chda_config::StarredBranch,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fg = self.fg;
        let found = self.model.branch_worktree(&starred.repo, &starred.branch);
        let repo_name = found.map(|(r, _)| r.name.clone()).unwrap_or_else(|| {
            starred
                .repo
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| starred.repo.display().to_string())
        });
        let worktree = found.map(|(_, w)| w).filter(|w| !w.missing);
        let label = worktree
            .and_then(|w| match self.active_label {
                chda_config::ActiveLabel::Alias => w.note_title(),
                chda_config::ActiveLabel::Branch => None,
            })
            .unwrap_or(&starred.branch)
            .to_owned();
        let (dot, mut tip) = match worktree {
            Some(w) => self.status_dot(w),
            None => (None, "No worktree has this branch checked out.".to_owned()),
        };
        tip = format!("{}\n{repo_name} \u{2022} {}\n{tip}", label, starred.branch);
        let (repo, branch) = (starred.repo.clone(), starred.branch.clone());
        div()
            .id(ElementId::Name(format!("starred:{index}").into()))
            .debug_selector(move || format!("starred-{index}"))
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .px_2()
            .py_0p5()
            .cursor_pointer()
            .when(worktree.is_none(), |d| d.opacity(0.55))
            .when(dot == Some(AgentStatus::WaitingInput), |d| {
                d.bg(waiting_tint())
            })
            .when(
                worktree.is_some_and(|w| self.selected.as_ref() == Some(&w.path)),
                |d| d.bg(fg.opacity(0.14)),
            )
            .hover(|s| s.bg(fg.opacity(0.08)))
            .tooltip(crate::tooltip::text(tip))
            .on_click({
                let (repo, branch) = (repo.clone(), branch.clone());
                cx.listener(move |_, _, _, cx| {
                    cx.emit(SidebarEvent::OpenStarred {
                        repo: repo.clone(),
                        branch: branch.clone(),
                    })
                })
            })
            .child(
                div()
                    .id(ElementId::Name(format!("unstar:{index}").into()))
                    .debug_selector(move || format!("unstar-{index}"))
                    .flex_shrink_0()
                    .px_0p5()
                    .rounded_sm()
                    .text_color(gpui::rgb(0xf9e2af))
                    .hover(|s| s.bg(fg.opacity(0.15)))
                    .tooltip(crate::tooltip::text("Unstar branch"))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.stop_propagation();
                        cx.emit(SidebarEvent::Unstar {
                            repo: repo.clone(),
                            branch: branch.clone(),
                        });
                    }))
                    .child("\u{2605}"),
            )
            .child(status_icon(dot, self.spin))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(label),
            )
            .child(
                div()
                    .text_xs()
                    .flex_shrink_0()
                    .max_w(gpui::px(70.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(fg.opacity(0.5))
                    .child(repo_name),
            )
            .into_any_element()
    }

    /// A worktree's row; `folder` for the one row of a plain folder.
    fn render_worktree(
        &self,
        wt: &WorktreeEntry,
        folder: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fg = self.fg;
        let path = wt.path.clone();
        let expanded = self.expanded.contains(&wt.path);
        let (dot, status_tip) = self.status_dot(wt);
        let name = match &wt.branch {
            _ if folder => "(no git)".to_owned(),
            Some(branch) => branch.clone(),
            None => "(detached)".to_owned(),
        };
        let b = wt.badges;
        let badge_id = |kind: &str| ElementId::Name(format!("{kind}:{}", wt.path.display()).into());
        let mut badges: Vec<AnyElement> = Vec::new();
        if b.conflicted > 0 {
            badges.push(
                div()
                    .id(badge_id("conflicts"))
                    .text_color(gpui::rgb(0xf38ba8))
                    .tooltip(crate::tooltip::text(conflicts_tooltip(b.conflicted)))
                    .child(format!("!{}", b.conflicted))
                    .into_any_element(),
            );
        }
        if b.dirty_count() > b.conflicted {
            badges.push(
                div()
                    .id(badge_id("dirty"))
                    .text_color(gpui::rgb(0xf9e2af))
                    .tooltip(crate::tooltip::text(dirty_tooltip(&b)))
                    .child(format!("\u{25cf}{}", b.dirty_count() - b.conflicted))
                    .into_any_element(),
            );
        }
        if let (Some(a), Some(bh)) = (b.ahead, b.behind) {
            if a > 0 {
                badges.push(
                    div()
                        .id(badge_id("ahead"))
                        .tooltip(crate::tooltip::text(format!(
                            "{} not pushed.",
                            plural(a, "commit", "commits")
                        )))
                        .child(format!("\u{2191}{a}"))
                        .into_any_element(),
                );
            }
            if bh > 0 {
                badges.push(
                    div()
                        .id(badge_id("behind"))
                        .tooltip(crate::tooltip::text(format!(
                            "{} to pull.",
                            plural(bh, "commit", "commits")
                        )))
                        .child(format!("\u{2193}{bh}"))
                        .into_any_element(),
                );
            }
        }
        if let Some(diff) = wt.diff.as_ref().filter(|d| d.files > 0) {
            let path = wt.path.clone();
            badges.push(
                div()
                    .id(badge_id("diff"))
                    .flex()
                    .flex_row()
                    .gap_0p5()
                    .cursor_pointer()
                    .tooltip(crate::tooltip::text(diff_tooltip(diff)))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.stop_propagation();
                        cx.emit(SidebarEvent::OpenDiff(path.clone()));
                    }))
                    .child(
                        div()
                            .text_color(gpui::rgb(0xa6e3a1))
                            .child(format!("+{}", diff.added)),
                    )
                    .child(
                        div()
                            .text_color(gpui::rgb(0xf38ba8))
                            .child(format!("\u{2212}{}", diff.removed)),
                    )
                    .into_any_element(),
            );
        }
        if let Some(pr) = &wt.pr {
            let (icon, color): (&str, Hsla) = match (pr.state, pr.checks) {
                (PrState::Merged, _) => ("\u{2713}", gpui::rgb(0xcba6f7).into()),
                (PrState::Closed, _) => ("\u{2715}", fg.opacity(0.5)),
                (PrState::Open, CheckState::Success) => ("\u{25cf}", gpui::rgb(0xa6e3a1).into()),
                (PrState::Open, CheckState::Failure) => ("\u{25cf}", gpui::rgb(0xf38ba8).into()),
                (PrState::Open, CheckState::Pending) => ("\u{25cb}", gpui::rgb(0xf9e2af).into()),
                (PrState::Open, CheckState::None) => ("", fg.opacity(0.7)),
            };
            let url = pr.url.clone();
            badges.push(
                div()
                    .id(badge_id("pr"))
                    .text_color(color)
                    .cursor_pointer()
                    .tooltip(crate::tooltip::text(pr_tooltip(pr)))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.stop_propagation();
                        cx.emit(SidebarEvent::OpenUrl(url.clone()));
                    }))
                    .child(format!("{}{}", pr_reference(pr).1, icon))
                    .into_any_element(),
            );
        }
        if let Some(busy) = &wt.busy {
            let icon = if busy == "deleting" {
                ICON_DELETING
            } else {
                ICON_BUSY
            };
            badges.push(
                div()
                    .id(badge_id("busy"))
                    .text_color(fg.opacity(0.6))
                    .tooltip(crate::tooltip::text(busy_tooltip(busy)))
                    .child(icon_text(icon))
                    .into_any_element(),
            );
        }
        if wt.missing {
            badges.push(
                div()
                    .id(badge_id("missing"))
                    .text_color(gpui::rgb(0xf38ba8))
                    .tooltip(crate::tooltip::text(
                        "Folder missing: the worktree's folder was deleted outside git. Right-click to prune it.",
                    ))
                    .child(icon_text(ICON_MISSING))
                    .into_any_element(),
            );
        }
        if wt.is_merged() && !wt.is_main {
            badges.push(
                div()
                    .id(badge_id("merged"))
                    .text_color(gpui::rgb(0xa6e3a1))
                    .tooltip(crate::tooltip::text(
                        "Merged: the branch's commits are in the default branch.",
                    ))
                    .child(icon_text(ICON_MERGED))
                    .into_any_element(),
            );
        }
        if !wt.panes.is_empty() {
            let n = wt.panes.len();
            badges.push(
                div()
                    .id(badge_id("panes"))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_0p5()
                    .text_color(fg.opacity(0.6))
                    .tooltip(crate::tooltip::text(format!(
                        "{} in this worktree.",
                        plural(n, "open pane", "open panes")
                    )))
                    .child(icon_text(ICON_PANES))
                    .child(n.to_string())
                    .into_any_element(),
            );
        }
        let row = div()
            .id(ElementId::Name(format!("wt:{}", wt.path.display()).into()))
            .debug_selector({
                let p = wt.path.clone();
                move || format!("wt:{}", p.display())
            })
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .pl_4()
            .pr_2()
            .py_0p5()
            .cursor_pointer()
            .when(wt.missing || wt.busy.is_some(), |d| d.opacity(0.55))
            .when(dot == Some(AgentStatus::WaitingInput), |d| {
                d.bg(waiting_tint())
            })
            .when(self.selected.as_ref() == Some(&wt.path), |d| {
                d.bg(fg.opacity(0.14))
            })
            .hover(|s| s.bg(fg.opacity(0.08)))
            .on_click({
                let path = path.clone();
                cx.listener(move |this, _, _, cx| {
                    this.selected = Some(path.clone());
                    cx.emit(SidebarEvent::OpenWorktree(path.clone()))
                })
            })
            .on_mouse_down(MouseButton::Right, {
                let path = path.clone();
                cx.listener(move |_, e: &MouseDownEvent, _, cx| {
                    cx.emit(SidebarEvent::WorktreeMenu(path.clone(), e.position));
                })
            })
            .child(
                div()
                    .id(ElementId::Name(
                        format!("wt-exp:{}", wt.path.display()).into(),
                    ))
                    .w_3()
                    .text_color(fg.opacity(0.5))
                    .on_click({
                        let path = path.clone();
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_expanded(&path, cx);
                        })
                    })
                    .child(if wt.sessions.is_empty() {
                        " "
                    } else if expanded {
                        "\u{25be}"
                    } else {
                        "\u{25b8}"
                    }),
            )
            .child(
                div()
                    .id(ElementId::Name(
                        format!("wt-dot:{}", wt.path.display()).into(),
                    ))
                    .flex_shrink_0()
                    .tooltip(crate::tooltip::text(status_tip))
                    .on_click({
                        let path = path.clone();
                        cx.listener(move |_, _, _, cx| {
                            cx.stop_propagation();
                            cx.emit(SidebarEvent::JumpToAgent(path.clone()));
                        })
                    })
                    .child(status_icon(dot, self.spin)),
            )
            .child(match wt.note_title() {
                // With a note, the task is the label and the branch sits
                // small underneath.
                Some(title) => div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(title.to_owned()),
                    )
                    .child(
                        div()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_xs()
                            .text_color(fg.opacity(0.5))
                            .child(name.clone()),
                    ),
                None => div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(name.clone()),
            })
            .children(badges.into_iter().map(|b| div().text_xs().child(b)))
            .when_some(wt.note.clone(), |d, note| {
                d.tooltip(crate::tooltip::text(format!("{note}\n\n{name}")))
            });
        let reveal = self.reveal.as_ref() == Some(&wt.path);
        let row = if reveal {
            let scroll = self.scroll.clone();
            let path = wt.path.clone();
            let this = cx.entity().downgrade();
            div()
                .child(row)
                .on_children_prepainted(move |bounds, window, _| {
                    let Some(target) = bounds.first().copied() else {
                        return;
                    };
                    let viewport = scroll.bounds();
                    let delta = if target.top() < viewport.top() {
                        viewport.top() - target.top()
                    } else if target.bottom() > viewport.bottom() {
                        viewport.bottom() - target.bottom()
                    } else {
                        gpui::px(0.0)
                    };
                    let scroll = scroll.clone();
                    let path = path.clone();
                    let this = this.clone();
                    window.on_next_frame(move |_, cx| {
                        let _ = this.update(cx, |s, cx| {
                            if s.reveal.as_ref() != Some(&path) {
                                return;
                            }
                            s.reveal = None;
                            if delta != gpui::px(0.0) {
                                let mut offset = scroll.offset();
                                offset.y += delta;
                                scroll.set_offset(offset);
                                cx.notify();
                            }
                        });
                    });
                })
                .into_any_element()
        } else {
            row.into_any_element()
        };
        let mut col = div().flex().flex_col().child(row);
        if let Some(err) = &wt.error {
            col = col.child(
                div()
                    .id(ElementId::Name(
                        format!("wt-err:{}", wt.path.display()).into(),
                    ))
                    .pl_8()
                    .pr_2()
                    .text_xs()
                    .text_color(gpui::rgb(0xf38ba8))
                    .cursor_pointer()
                    .on_click({
                        let path = path.clone();
                        cx.listener(move |this, _, _, cx| {
                            if let Some(w) = this.model.worktree_mut(&path) {
                                w.error = None;
                            }
                            cx.notify();
                        })
                    })
                    .child(err.clone()),
            );
        }
        if expanded {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            for session in wt.sessions.iter().take(8) {
                col = col.child(self.render_session(&path, session, now, cx));
            }
        }
        col.into_any_element()
    }
}

/// "1 commit" / "3 commits".
fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

fn diff_tooltip(d: &DiffSummary) -> String {
    format!(
        "{} against {}: {} added, {} removed.\nClick to view the diff.",
        plural(d.files, "file changed", "files changed"),
        d.base,
        plural(d.added, "line", "lines"),
        d.removed
    )
}

fn conflicts_tooltip(n: usize) -> String {
    format!("{} with merge conflicts.", plural(n, "file", "files"))
}

fn dirty_tooltip(b: &GitBadges) -> String {
    format!(
        "{} (uncommitted or untracked).",
        plural(
            b.dirty_count() - b.conflicted,
            "changed file",
            "changed files"
        )
    )
}

/// `host/owner/repo` from a pull request URL such as
/// `https://github.example.com/owner/repo/pull/5`.
fn pr_location(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let mut parts = rest.split('/');
    let host = parts.next().filter(|h| !h.is_empty())?;
    let path: Vec<&str> = parts.collect();
    // GitHub `/pull/N`, GitLab `/-/merge_requests/N`, Gitea `/pulls/N`.
    let cut = path
        .iter()
        .position(|p| matches!(*p, "pull" | "pulls" | "-" | "merge_requests"))?;
    let number = path.last()?;
    if cut == 0 || number.parse::<u64>().is_err() {
        return None;
    }
    Some(format!("{host}/{}", path[..cut].join("/")))
}

/// "PR" and `#5`, or "MR" and `!5` for a GitLab merge request.
fn pr_reference(pr: &PrInfo) -> (&'static str, String) {
    if pr.url.contains("/-/merge_requests/") {
        ("MR", format!("!{}", pr.number))
    } else {
        ("PR", format!("#{}", pr.number))
    }
}

/// What a pull request badge means, and where it points.
fn pr_tooltip(pr: &PrInfo) -> String {
    let state = match (pr.state, pr.checks) {
        (PrState::Merged, _) => "merged",
        (PrState::Closed, _) => "closed without merging",
        (PrState::Open, CheckState::Success) => "open, checks passed",
        (PrState::Open, CheckState::Failure) => "open, checks failed",
        (PrState::Open, CheckState::Pending) => "open, checks running",
        (PrState::Open, CheckState::None) => "open, no checks",
    };
    let (kind, reference) = pr_reference(pr);
    let mut text = format!("{kind} {reference} {state}. Click to open.");
    if let Some(location) = pr_location(&pr.url) {
        text.push('\n');
        text.push_str(&location);
        text.push_str(&reference);
    }
    text
}

/// The status dot's tooltip: each busy agent, most urgent first.
fn status_tooltip(wt: &WorktreeEntry, labels: &[AgentLabel]) -> String {
    let mut agents: Vec<(&String, AgentStatus)> = wt
        .agents
        .iter()
        .map(|(id, s)| (id, *s))
        .filter(|(_, s)| *s != AgentStatus::Idle)
        .collect();
    if agents.is_empty() {
        return "No agent is active here.".into();
    }
    agents.sort_by_key(|(_, s)| std::cmp::Reverse(s.urgency()));
    let lines: Vec<String> = agents
        .into_iter()
        .map(|(id, status)| {
            let name = labels
                .iter()
                .find(|a| a.id == *id)
                .map_or(id.as_str(), |a| a.name.as_str());
            match status {
                AgentStatus::Working => format!("{name} is working."),
                AgentStatus::WaitingInput => format!("{name} is waiting for input."),
                AgentStatus::Review => format!("{name} finished; not looked at yet."),
                AgentStatus::Idle => unreachable!(),
            }
        })
        .collect();
    format!("{}\nClick to go there.", lines.join("\n"))
}

impl SidebarView {
    /// One past session: agent label, first prompt, message count and time
    /// since the last message; "Resume" on hover.
    fn render_session(
        &self,
        worktree: &std::path::Path,
        session: &SessionEntry,
        now: u64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fg = self.fg;
        let pick = SessionPick {
            worktree: worktree.to_path_buf(),
            agent: session.agent.clone(),
            session: session.id.clone(),
        };
        let picked = self.picked.contains(&pick);
        let label = self.agents.iter().find(|a| a.id == session.agent);
        let (short, name, color) = match label {
            Some(l) => (l.short.clone(), l.name.clone(), l.color),
            None => (
                session.agent.clone(),
                session.agent.clone(),
                fg.opacity(0.5),
            ),
        };
        let group: gpui::SharedString = format!("sess:{}", session.id).into();
        let messages = match session.message_count {
            1 => "1 message".to_owned(),
            n => format!("{n} messages"),
        };
        div()
            .id(ElementId::Name(group.clone()))
            .group(group.clone())
            .flex()
            .flex_row()
            .gap_1()
            .pl_8()
            .pr_2()
            .py_0p5()
            .text_xs()
            .text_color(fg.opacity(0.75))
            .cursor_pointer()
            .when(picked, |d| d.bg(fg.opacity(0.14)))
            .hover(|s| s.bg(fg.opacity(0.08)))
            .tooltip(crate::tooltip::text(format!(
                "{name}, {messages}.{}\nClick to resume in a new tab; \
                 cmd-click to pick several and resume them side by side.",
                usage_lines(&session.usage)
            )))
            .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                if e.modifiers().platform {
                    this.toggle_pick(pick.clone(), cx);
                } else {
                    this.picked.clear();
                    cx.emit(SidebarEvent::ResumeSessions(vec![pick.clone()]));
                    cx.notify();
                }
            }))
            .child(
                div()
                    .id(ElementId::Name(format!("sess-agent:{}", session.id).into()))
                    .flex_shrink_0()
                    .text_color(color)
                    .tooltip(crate::tooltip::text(name.clone()))
                    .child(short),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(session.snippet.clone()),
            )
            // Hover swaps the count and age for "Resume". Only visibility
            // changes: hover is decided separately in prepaint and paint, and
            // a child that appears between the two (display none to block)
            // is painted without being prepainted, which panics in GPUI.
            .child(
                div()
                    .relative()
                    .flex_shrink_0()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_1()
                            .text_color(fg.opacity(0.5))
                            .group_hover(group.clone(), |s| s.invisible())
                            .child(if session.usage.is_empty() {
                                session.message_count.to_string()
                            } else {
                                chda_core::agents::compact_tokens(session.usage.total())
                            })
                            .child(relative_age(now, session.last_active_at)),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .pl_1()
                            .invisible()
                            .text_color(fg)
                            // The hovered row's color, so a short age does
                            // not let the prompt show through.
                            .bg(crate::workspace_view::blend(self.bg, fg, 0.08))
                            .group_hover(group, |s| s.visible())
                            .child("Resume"),
                    ),
            )
            .into_any_element()
    }

    /// Add a session to the ones to resume together, or take it out.
    pub fn toggle_pick(&mut self, pick: SessionPick, cx: &mut Context<Self>) {
        if let Some(i) = self.picked.iter().position(|p| *p == pick) {
            self.picked.remove(i);
        } else {
            self.picked.push(pick);
        }
        cx.notify();
    }

    /// Resume the picked sessions together.
    pub fn resume_picked(&mut self, cx: &mut Context<Self>) {
        let picked = std::mem::take(&mut self.picked);
        if !picked.is_empty() {
            cx.emit(SidebarEvent::ResumeSessions(picked));
        }
        cx.notify();
    }
}

impl Focusable for SidebarView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SidebarView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let fg = self.fg;
        let repos: Vec<AnyElement> = self
            .model
            .repos
            .iter()
            .enumerate()
            .map(|(i, r)| self.render_repo(i, r, cx))
            .collect();
        let empty = self.model.repos.is_empty();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let active: Vec<AnyElement> = self
            .model
            .active_tabs
            .iter()
            .map(|t| {
                let id = t.tab;
                div()
                    .id(ElementId::Name(format!("active:{:?}", t.tab).into()))
                    .debug_selector(move || format!("active-{id:?}"))
                    .when(t.status == AgentStatus::WaitingInput, |d| {
                        d.bg(waiting_tint())
                    })
                    .when(self.active_tab == Some(id), |d| d.bg(fg.opacity(0.14)))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_0p5()
                    .cursor_pointer()
                    .hover(|s| s.bg(fg.opacity(0.08)))
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(SidebarEvent::FocusTab(id))))
                    .when_some(t.branch.clone(), |d, branch| {
                        d.tooltip(crate::tooltip::text(branch))
                    })
                    .child(status_icon(
                        (t.status != AgentStatus::Idle || t.agent_live).then_some(t.status),
                        self.spin,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(t.title.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .flex_shrink_0()
                            .max_w(gpui::px(70.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(fg.opacity(0.5))
                            .child(t.repo.clone().unwrap_or_else(|| "\u{2014}".into())),
                    )
                    .child(
                        div()
                            .id(ElementId::Name(format!("activity-age:{:?}", t.tab).into()))
                            .w(gpui::px(38.0))
                            .flex_shrink_0()
                            .text_right()
                            .text_xs()
                            .text_color(fg.opacity(0.5))
                            .tooltip(crate::tooltip::text(match t.previous_activity {
                                Some(at) => format!(
                                    "Last output: {} ago\nPrevious work before restart: {} ago",
                                    activity_age(now, t.last_activity),
                                    activity_age(now, at)
                                ),
                                None => {
                                    "Time since the last terminal output or screen update".into()
                                }
                            }))
                            .child(activity_age(now, t.last_activity)),
                    )
                    .into_any_element()
            })
            .collect();
        let starred: Vec<AnyElement> = self
            .model
            .starred
            .iter()
            .enumerate()
            .map(|(i, s)| self.render_starred(i, s, cx))
            .collect();
        let idle: Vec<AnyElement> = self
            .model
            .idle_agents
            .iter()
            .map(|entry| {
                let pane = entry.pane;
                let name = self
                    .agents
                    .iter()
                    .find(|a| a.id == entry.agent)
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| entry.agent.clone());
                let context = format!("{} · pane {}", entry.tab, entry.pane_index);
                let tooltip = format!(
                    "{}\n{}\n{}",
                    name,
                    context,
                    entry
                        .cwd
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|| entry.location.clone())
                );
                div()
                    .id(ElementId::Name(format!("idle:{}", pane.raw()).into()))
                    .debug_selector(move || format!("idle-{}", pane.raw()))
                    .flex()
                    .flex_col()
                    .w_full()
                    .min_w_0()
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .hover(|s| s.bg(fg.opacity(0.08)))
                    .tooltip(crate::tooltip::text(tooltip))
                    .on_click(
                        cx.listener(move |_, _, _, cx| cx.emit(SidebarEvent::FocusPane(pane))),
                    )
                    .child(
                        div()
                            .flex()
                            .w_full()
                            .min_w_0()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .id(ElementId::Name(format!("idle-name:{}", pane.raw()).into()))
                                    .debug_selector(move || format!("idle-name-{}", pane.raw()))
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        div()
                                            .debug_selector(move || {
                                                format!("idle-dot-{}", pane.raw())
                                            })
                                            .flex_shrink_0()
                                            .child(status_icon(Some(AgentStatus::Idle), 0)),
                                    )
                                    .child(
                                        div()
                                            .debug_selector(move || {
                                                format!("idle-label-{}", pane.raw())
                                            })
                                            .min_w_0()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .child(name),
                                    ),
                            )
                            .child(
                                div()
                                    .id(ElementId::Name(format!("idle-age:{}", pane.raw()).into()))
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_color(fg.opacity(0.6))
                                    .tooltip(crate::tooltip::text(if entry.since == 0 {
                                        "Idle start time is unavailable"
                                    } else {
                                        "Time since this session became ready for another task"
                                    }))
                                    .child(if entry.since == 0 {
                                        "—".into()
                                    } else {
                                        format!("{} idle", activity_age(now, entry.since))
                                    }),
                            )
                            .child(
                                div()
                                    .id(ElementId::Name(
                                        format!("idle-close:{}", pane.raw()).into(),
                                    ))
                                    .debug_selector(move || format!("idle-close-{}", pane.raw()))
                                    .px_1()
                                    .rounded_sm()
                                    .hover(|s| s.bg(fg.opacity(0.15)))
                                    .tooltip(crate::tooltip::text("Close this pane"))
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.stop_propagation();
                                        cx.emit(SidebarEvent::ClosePane(pane));
                                    }))
                                    .child("×"),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .text_xs()
                            .text_color(fg.opacity(0.7))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(entry.location.clone()),
                    )
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .text_xs()
                            .text_color(fg.opacity(0.5))
                            .flex()
                            .gap_1()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(entry.tab.clone()),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .child(format!("pane {}", entry.pane_index)),
                            ),
                    )
                    .into_any_element()
            })
            .collect();
        let list = div()
            .id("sidebar")
            .flex()
            .flex_col()
            .size_full()
            .bg(self.bg)
            .text_color(fg)
            .text_sm()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .on_drag_move(cx.listener(|this, e: &gpui::DragMoveEvent<RepoDrag>, _, cx| {
                this.scroll_while_dragging(e, cx)
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(fg.opacity(0.6))
                    .child(div().flex_1().child("WORKTREES"))
                    .children([(false, "expand-all", "▾", "Expand all"), (true, "collapse-all", "▸", "Collapse all")].into_iter().map(|(collapsed, id, icon, tip)| {
                        div().id(id).debug_selector(move || id.into()).px_1().rounded_sm().cursor_pointer()
                            .tooltip(crate::tooltip::text(tip)).hover(|s| s.bg(fg.opacity(0.15)))
                            .on_click(cx.listener(move |this, _, _, cx| this.collapse_all(collapsed, cx))).child(icon)
                    }))
                    .child(
                        div()
                            .id("add-repo")
                            .px_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .hover(|s| s.bg(fg.opacity(0.15)))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SidebarEvent::AddRepo)))
                            .child("+ repo"),
                    ),
            )
            .when(!starred.is_empty(), |d| {
                d.child(
                    div()
                        .px_2()
                        .pt_2()
                        .pb_1()
                        .text_xs()
                        .text_color(fg.opacity(0.6))
                        .child("STARRED"),
                )
                .children(starred)
                .child(div().h(gpui::px(6.0)))
            })
            .when(!active.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .px_2()
                        .pt_2()
                        .pb_1()
                        .text_xs()
                        .text_color(fg.opacity(0.6))
                        .child(div().flex_1().child("ACTIVE"))
                        .child(
                            div()
                                .id("active-label-toggle")
                                .debug_selector(|| "active-label-toggle".into())
                                .px_1()
                                .cursor_pointer()
                                .hover(|s| s.bg(fg.opacity(0.08)))
                                .tooltip(crate::tooltip::text(
                                    "Switch ACTIVE labels between branch aliases and branch names",
                                ))
                                .on_click(cx.listener(|_, _, _, cx| {
                                    cx.emit(SidebarEvent::ToggleActiveLabel)
                                }))
                                .child(match self.active_label {
                                    chda_config::ActiveLabel::Alias => "Alias",
                                    chda_config::ActiveLabel::Branch => "Branch",
                                }),
                        ),
                )
                .children(active)
                .child(div().h(gpui::px(6.0)))
            })
            .when(!idle.is_empty(), |d| {
                d.child(div().px_2().pt_2().pb_1().text_xs()
                    .text_color(fg.opacity(0.6)).child("Idle agents"))
                    .children(idle)
            })
            .when(!self.picked.is_empty(), |d| {
                let count = self.picked.len();
                d.child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .items_center()
                        .mx_2()
                        .mb_1()
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .bg(fg.opacity(0.1))
                        .text_xs()
                        .child(
                            div()
                                .id("resume-picked")
                                .flex_1()
                                .cursor_pointer()
                                .hover(|s| s.text_color(status_color(AgentStatus::Working)))
                                .on_click(cx.listener(|this, _, _, cx| this.resume_picked(cx)))
                                .child(match count {
                                    1 => "Resume 1 session".to_owned(),
                                    n => format!("Resume {n} sessions side by side"),
                                }),
                        )
                        .child(
                            div()
                                .id("clear-picked")
                                .cursor_pointer()
                                .text_color(fg.opacity(0.6))
                                .hover(|s| s.text_color(fg))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.picked.clear();
                                    cx.notify();
                                }))
                                .child("Clear"),
                        ),
                )
            })
            .when(!self.sessions_loaded && !empty, |d| {
                d.child(
                    div()
                        .px_2()
                        .pb_1()
                        .text_xs()
                        .text_color(fg.opacity(0.5))
                        .child("Loading agent sessions\u{2026}"),
                )
            })
            .children(repos)
            .when(empty, |d| {
                d.child(
                    div()
                        .px_2()
                        .py_2()
                        .text_xs()
                        .text_color(fg.opacity(0.5))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child("Get started")
                        .child("1. Add a repository: \"+ repo\", cmd-shift-o, or drop its folder here.")
                        .child("2. Click \"+\" next to it and name a branch to create a worktree.")
                        .child("3. Right-click the worktree to open a terminal or run Claude Code / Codex."),
                )
            });
        div()
            .relative()
            .size_full()
            .on_drag_move(cx.listener(|this, e, _, cx| this.dragged.track(e, cx)))
            .child(list)
            .child(crate::external_drop::catcher(
                cx.entity(),
                |v: &mut Self| &mut v.dragged,
                |_, paths, _, cx| {
                    let folders: Vec<PathBuf> = paths.into_iter().filter(|p| p.is_dir()).collect();
                    if !folders.is_empty() {
                        cx.emit(SidebarEvent::AddRepos(folders));
                    }
                },
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_usage_tooltip() {
        use chda_core::agents::{ModelUsage, Usage};
        assert_eq!(usage_lines(&Usage::default()), "");
        let mut u = Usage::default();
        u.add(&ModelUsage {
            model: "claude-opus-5-5".into(),
            input: 30_000,
            cache_write_1h: 210_000,
            cache_read: 950_000,
            output: 20_000,
            ..Default::default()
        });
        assert_eq!(
            usage_lines(&u),
            "\n1.2M tokens: 30K in, 210K cache write, 950K cache read, 20K out\n\u{2248} $2.39 at API list prices (claude-opus-5-5). Subscriptions are not billed per token."
        );
        u.limit = Some(chda_core::agents::LimitUsage {
            used_percent: 34,
            window_minutes: 10_080,
        });
        assert!(
            usage_lines(&u)
                .ends_with("Plan limit: 34% of the weekly window used at the last turn.")
        );
    }

    #[test]
    fn busy_badges_explain_themselves() {
        assert_eq!(busy_tooltip("deleting"), "Deleting this worktree\u{2026}");
    }

    fn pr(state: PrState, checks: CheckState, url: &str) -> PrInfo {
        PrInfo {
            number: 5,
            url: url.into(),
            state,
            checks,
        }
    }

    #[test]
    fn pr_tooltips_name_the_state_and_the_repository() {
        let url = "https://github.example.com/org/repo/pull/5";
        let cases = [
            (
                PrState::Merged,
                CheckState::Success,
                "PR #5 merged. Click to open.",
            ),
            (
                PrState::Closed,
                CheckState::None,
                "PR #5 closed without merging. Click to open.",
            ),
            (
                PrState::Open,
                CheckState::Success,
                "PR #5 open, checks passed. Click to open.",
            ),
            (
                PrState::Open,
                CheckState::Failure,
                "PR #5 open, checks failed. Click to open.",
            ),
            (
                PrState::Open,
                CheckState::Pending,
                "PR #5 open, checks running. Click to open.",
            ),
            (
                PrState::Open,
                CheckState::None,
                "PR #5 open, no checks. Click to open.",
            ),
        ];
        for (state, checks, first) in cases {
            assert_eq!(
                pr_tooltip(&pr(state, checks, url)),
                format!("{first}\ngithub.example.com/org/repo#5")
            );
        }
        assert_eq!(
            pr_tooltip(&pr(PrState::Open, CheckState::None, "not a url")),
            "PR #5 open, no checks. Click to open."
        );
        assert_eq!(
            pr_tooltip(&pr(
                PrState::Merged,
                CheckState::None,
                "https://gitlab.com/group/proj/-/merge_requests/5"
            )),
            "MR !5 merged. Click to open.\ngitlab.com/group/proj!5"
        );
    }

    #[test]
    fn pr_location_reads_github_gitlab_and_gitea_urls() {
        assert_eq!(
            pr_location("https://github.com/magicsih/chda/pull/49").as_deref(),
            Some("github.com/magicsih/chda")
        );
        assert_eq!(
            pr_location("https://gitlab.com/group/sub/proj/-/merge_requests/12").as_deref(),
            Some("gitlab.com/group/sub/proj")
        );
        assert_eq!(
            pr_location("https://gitea.example.com/org/repo/pulls/3").as_deref(),
            Some("gitea.example.com/org/repo")
        );
        assert_eq!(pr_location("https://github.com/pull/x"), None);
    }

    #[test]
    fn git_badge_tooltips_count_correctly() {
        let b = GitBadges {
            changed: 2,
            untracked: 1,
            conflicted: 2,
            ..Default::default()
        };
        assert_eq!(
            dirty_tooltip(&b),
            "3 changed files (uncommitted or untracked)."
        );
        assert_eq!(conflicts_tooltip(2), "2 files with merge conflicts.");
        assert_eq!(conflicts_tooltip(1), "1 file with merge conflicts.");
        assert_eq!(plural(1, "commit", "commits"), "1 commit");
        let diff = DiffSummary {
            base: "origin/main".into(),
            merge_base: "abc".into(),
            files: 3,
            added: 120,
            removed: 1,
        };
        assert_eq!(
            diff_tooltip(&diff),
            "3 files changed against origin/main: 120 lines added, 1 removed.\nClick to view the diff."
        );
    }

    #[test]
    fn status_tooltip_names_busy_agents_most_urgent_first() {
        let names = vec![
            AgentLabel::new(0, "claude", "CC".into(), "Claude Code"),
            AgentLabel::new(1, "codex", "CX".into(), "Codex"),
        ];
        let mut wt = WorktreeEntry::default();
        assert_eq!(status_tooltip(&wt, &names), "No agent is active here.");
        wt.agents.insert("codex".into(), AgentStatus::Working);
        wt.agents.insert("claude".into(), AgentStatus::WaitingInput);
        assert_eq!(
            status_tooltip(&wt, &names),
            "Claude Code is waiting for input.\nCodex is working.\nClick to go there."
        );
    }
}
