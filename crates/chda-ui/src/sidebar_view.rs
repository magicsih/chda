//! The repository / worktree / session tree with git and agent badges.

use std::path::PathBuf;

use crate::status_icon::status_icon;
use crate::workspace_view::blend;

use chda_core::{
    AgentStatus, CheckState, DiffSummary, GitBadges, PaneId, PrInfo, PrState, RepoEntry,
    SessionEntry, SessionKey, SessionKind, SessionRow, Sidebar, TabId, WorktreeEntry, activity_age,
    relative_age,
};
use gpui::{
    AnyElement, App, ClickEvent, Context, ElementId, EventEmitter, FocusHandle, Focusable, Hsla,
    MouseButton, MouseDownEvent, Pixels, Point, Render, Window, div, prelude::*, relative,
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
    /// Restore typing focus after a repository disclosure changes.
    RefocusTerminal,
    /// Folders were dropped on the sidebar: add them as repositories.
    AddRepos(Vec<PathBuf>),
    /// Open a URL (a pull request badge was clicked).
    OpenUrl(String),
    /// A Sessions row for a tab was clicked.
    FocusTab(TabId),
    /// A Sessions row for an agent pane was clicked: focus that exact pane,
    /// leaving its terminal's scroll position alone.
    FocusPane(PaneId),
    ClosePane(PaneId),
    CloseTab(TabId),
    ChildDetails(chda_core::agents::ChildActivity),
    /// Switch Sessions labels between branch aliases and branch names.
    ToggleActiveLabel,
    ToggleSessions,
    ToggleProject,
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

/// Distance from PROJECT's top or bottom edge that scrolls while dragging.
const DRAG_SCROLL_EDGE: f32 = 32.0;
const REPO_CONTEXT_HEIGHT: f32 = 32.0;

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

/// Provider-confirmed children of one parent agent, shown under the
/// parent's Sessions row (its pane's row, else its tab's first row).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ChildGroup {
    pub pane: PaneId,
    pub tab: TabId,
    pub agent: String,
    pub session: String,
    pub label: String,
    pub children: Vec<chda_core::agents::ChildActivity>,
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

/// The active tab and its focused pane, if it has panes.
pub(crate) type Focus = (TabId, Option<PaneId>);

pub struct SidebarView {
    pub model: Sidebar,
    pub(crate) child_groups: Vec<ChildGroup>,
    pub(crate) child_collapsed: std::collections::HashSet<(String, String)>,
    pub active_label: chda_config::ActiveLabel,
    pub(crate) sessions_collapsed: bool,
    pub(crate) project_collapsed: bool,
    sticky_repo: Option<PathBuf>,
    /// The first session index has finished.
    pub sessions_loaded: bool,
    /// Labels and names of the agents chda knows.
    pub agents: Vec<AgentLabel>,
    /// Sessions cmd-clicked to resume together, in click order.
    pub picked: Vec<SessionPick>,
    expanded: Vec<PathBuf>,
    /// Highlighted PROJECT worktree: the focused pane's, or a notification's
    /// whose pane is gone.
    pub(crate) selected: Option<PathBuf>,
    /// The active tab and its focused pane; Sessions highlights their row.
    pub(crate) focus: Option<Focus>,
    /// Frame of the "working" spinner, advanced by the workspace.
    pub(crate) spin: u32,
    /// Sessions scrolls only when the user scrolls it.
    pub(crate) sessions_scroll: gpui::ScrollHandle,
    /// PROJECT, which navigation reveals worktrees in.
    pub(crate) project_scroll: gpui::ScrollHandle,
    /// The PROJECT row to bring into view once it is laid out; always the
    /// highlighted one.
    reveal: Option<PathBuf>,
    /// A navigation target whose worktree is not listed yet, such as a
    /// worktree just created.
    pending_reveal: Option<PathBuf>,
    /// The focused pane's directory at the last sync.
    focus_cwd: Option<PathBuf>,
    /// The focus when a notification's worktree took the highlight; it keeps
    /// it until the focus moves.
    held: Option<(Option<Focus>, Option<PathBuf>)>,
    focus_handle: FocusHandle,
    fg: Hsla,
    bg: Hsla,
    /// Folders dragged over the window from another app.
    dragged: crate::external_drop::Dragged,
}

impl EventEmitter<SidebarEvent> for SidebarView {}

/// Highlight of a selected row: the foreground at this opacity.
const SELECTED_ROW: f32 = 0.14;

/// Background of a row whose agent waits for input.
fn waiting_tint(fg: Hsla, bg: Hsla) -> Hsla {
    status_color(Some(AgentStatus::WaitingInput), fg, bg).opacity(0.16)
}

/// The hue of `status`, or gray without a live agent, before it is fitted
/// to a theme.
fn status_base(status: Option<AgentStatus>) -> Hsla {
    gpui::rgb(match status {
        None => 0x6c7086,
        Some(AgentStatus::Idle) => 0xf9e2af,
        Some(AgentStatus::Working) => 0x89b4fa,
        Some(AgentStatus::WaitingInput) => 0xfab387,
        Some(AgentStatus::Review) => 0xa6e3a1,
    })
    .into()
}

/// The icon color of `status`, or gray without a live agent, on `bg` in a
/// theme whose text is `fg`. A hue that stands out less than 3:1 from `bg`
/// or from a row selected on it (WCAG 2.2 SC 1.4.11 for state graphics)
/// darkens on a light background and lightens on a dark one until it does.
pub fn status_color(status: Option<AgentStatus>, fg: Hsla, bg: Hsla) -> Hsla {
    let mut color = status_base(status);
    let row = luminance(bg);
    let selected = luminance(blend(bg, fg, SELECTED_ROW));
    let step = if ratio(row, 0.0) >= ratio(row, 1.0) {
        -0.01
    } else {
        0.01
    };
    while (0.0..=1.0).contains(&(color.l + step)) {
        let l = luminance(color);
        if ratio(l, row).min(ratio(l, selected)) >= 3.0 {
            break;
        }
        color.l += step;
    }
    color
}

/// WCAG 2.2 relative luminance of an opaque color.
fn luminance(color: Hsla) -> f32 {
    let color = color.to_rgb();
    let linear = |v: f32| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}

/// WCAG 2.2 contrast ratio of two relative luminances, from 1 to 21.
fn ratio(a: f32, b: f32) -> f32 {
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn pr_badge(pr: &PrInfo, bg: Hsla) -> (&'static str, Hsla) {
    // Primer fgColor.open / done / closed, with the default light/dark modes.
    let light = bg.l > 0.5;
    let (icon, color) = match pr.state {
        PrState::Open => ("\u{25cf}", if light { 0x1a7f37 } else { 0x3fb950 }),
        PrState::Merged => ("\u{2713}", if light { 0x8250df } else { 0xab7df8 }),
        PrState::Closed => ("\u{2715}", if light { 0xd1242f } else { 0xf85149 }),
    };
    (icon, gpui::rgb(color).into())
}

impl SidebarView {
    fn render_children(&self, group: &ChildGroup, now: u64, cx: &mut Context<Self>) -> AnyElement {
        let key = (group.agent.clone(), group.session.clone());
        let collapsed = self.child_collapsed.contains(&key);
        let active = group
            .children
            .iter()
            .filter(|e| e.child.state.active())
            .count();
        let fg = self.fg;
        let pane = group.pane.raw();
        div().flex().flex_col().min_w_0()
            .child(div().id(ElementId::Name(format!("children-toggle-{pane}").into())).debug_selector(move || format!("children-toggle-{pane}"))
                .pl_4().pr_2().py_0p5().flex().gap_1().text_xs().text_color(fg.opacity(0.7)).cursor_pointer()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |s, _, _, cx| { cx.stop_propagation(); if !s.child_collapsed.remove(&key) { s.child_collapsed.insert(key.clone()); } cx.emit(SidebarEvent::RefocusTerminal); cx.notify(); }))
                .child(if collapsed { "▸" } else { "▾" })
                .child(format!("{} · {active} active · {} children", group.agent, group.children.len()))
                .tooltip(crate::tooltip::text(format!("{}\nExact parent: {}\nCounts reflect provider reports; usage is already included in the existing session total.", group.label, group.session))))
            .when(!collapsed, |d| d.children(group.children.iter().map(|entry| {
                let child = entry.clone();
                let id = entry.child.id.clone();
                let state = entry.child.state;
                let label = entry.child.label.clone().unwrap_or_else(|| entry.child.id.clone());
                let age = activity_age(now, entry.observed_at);
                div().id(ElementId::Name(format!("child-{pane}-{id}").into())).debug_selector(move || format!("child-{pane}-{id}"))
                    .pl_6().pr_2().py_0p5().flex().flex_col().min_w_0().text_xs().cursor_pointer().hover(|s| s.bg(fg.opacity(0.08)))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |_, _, _, cx| { cx.stop_propagation(); cx.emit(SidebarEvent::ChildDetails(child.clone())); }))
                    .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(fg).child(label))
                    .child(div().text_color(fg.opacity(0.65)).child(format!("{} · reported {age} ago", state.label())))
                    .into_any_element()
            })))
            .into_any_element()
    }

    /// A section header: a toggle with the chevron, `name` and `count` that
    /// collapses or expands the section, and `button` beside it. The button
    /// stays outside the toggle so hovering it shows only its own tooltip.
    fn section_header(
        &self,
        name: &'static str,
        collapsed: bool,
        count: usize,
        toggle: SidebarEvent,
        button: impl IntoElement,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let fg = self.fg;
        let selector = format!("{}-section-toggle", name.to_lowercase());
        div()
            .flex_none()
            .flex()
            .flex_row()
            .text_xs()
            .text_color(fg.opacity(0.6))
            .hover(|s| s.bg(fg.opacity(0.08)))
            .child(
                div()
                    .id(ElementId::Name(selector.clone().into()))
                    .debug_selector(move || selector)
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .items_center()
                    .pl_2()
                    .pt_2()
                    .pb_1()
                    .cursor_pointer()
                    .tooltip(crate::tooltip::text(if collapsed {
                        format!("Expand {name}")
                    } else {
                        format!("Collapse {name}")
                    }))
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(toggle.clone())))
                    .child(div().w_3().child(if collapsed { "▸" } else { "▾" }))
                    .child(div().flex_1().child(name))
                    .child(div().px_1().child(count.to_string())),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .pr_2()
                    .pt_2()
                    .pb_1()
                    .child(button),
            )
    }

    pub fn new(fg: Hsla, bg: Hsla, agents: Vec<AgentLabel>, cx: &mut Context<Self>) -> Self {
        Self {
            model: Sidebar::new(),
            child_groups: Vec::new(),
            child_collapsed: Default::default(),
            active_label: Default::default(),
            sessions_collapsed: false,
            project_collapsed: false,
            sticky_repo: None,
            sessions_loaded: false,
            agents,
            picked: Vec::new(),
            expanded: Vec::new(),
            selected: None,
            focus: None,
            spin: 0,
            sessions_scroll: gpui::ScrollHandle::new(),
            project_scroll: gpui::ScrollHandle::new(),
            reveal: None,
            pending_reveal: None,
            focus_cwd: None,
            held: None,
            focus_handle: cx.focus_handle(),
            fg,
            bg,
            dragged: Default::default(),
        }
    }

    /// Highlight a worktree and reveal it in PROJECT, e.g. for a
    /// notification whose pane is gone.
    pub fn select(&mut self, worktree: &std::path::Path, cx: &mut Context<Self>) {
        let held = (self.focus, self.focus_cwd.clone());
        self.select_context(self.focus, Some(worktree), true, cx);
        self.focus_cwd = held.1.clone();
        self.held = Some(held);
    }

    /// Follow the focus: Sessions highlights the focused row and PROJECT the
    /// worktree containing `cwd`. Only `navigation` (a focus change the user
    /// made) reveals that worktree, inside PROJECT; Sessions never scrolls.
    pub(crate) fn select_context(
        &mut self,
        focus: Option<Focus>,
        cwd: Option<&std::path::Path>,
        navigation: bool,
        cx: &mut Context<Self>,
    ) {
        let held = !navigation
            && self
                .held
                .as_ref()
                .is_some_and(|(f, c)| *f == focus && c.as_deref() == cwd);
        self.focus = focus;
        self.focus_cwd = cwd.map(std::path::Path::to_path_buf);
        if held {
            cx.notify();
            return;
        }
        self.held = None;
        if navigation {
            self.pending_reveal = cwd.map(std::path::Path::to_path_buf);
        } else if self.pending_reveal.as_deref() != cwd {
            // The focus moved on before the target was listed.
            self.pending_reveal = None;
        }
        let found = cwd
            .and_then(|cwd| self.model.worktree_for_path(cwd))
            .map(|(r, w)| (r.path.clone(), w.path.clone()));
        self.selected = found.as_ref().map(|(_, path)| path.clone());
        // A reveal not drawn yet (PROJECT collapsed, sidebar hidden) is for
        // the highlight it was scheduled with; once that moves on, expanding
        // PROJECT later must not scroll to the earlier worktree.
        if self.reveal.is_some() && self.reveal != self.selected {
            self.reveal = None;
        }
        if let Some((repo, path)) = found
            && self.pending_reveal.take().is_some()
        {
            if let Some(r) = self.model.repo_mut(&repo) {
                r.collapsed = false;
            }
            self.reveal = Some(path);
        }
        cx.notify();
    }

    /// The Sessions row holding the focus.
    pub(crate) fn focused_session(&self) -> Option<SessionKey> {
        let (tab, pane) = self.focus?;
        chda_core::focused_session(&self.model.sessions, tab, pane)
    }

    /// New colors after a config reload.
    pub fn set_colors(&mut self, fg: Hsla, bg: Hsla, cx: &mut Context<Self>) {
        self.fg = fg;
        self.bg = bg;
        cx.notify();
    }

    /// Scroll PROJECT when a dragged repository header nears its top or
    /// bottom edge, so a target outside the visible part can be reached.
    /// Sessions never scrolls for a drag.
    fn scroll_while_dragging(&mut self, e: &gpui::DragMoveEvent<RepoDrag>, cx: &mut Context<Self>) {
        let position = e.event.position;
        if position.x < e.bounds.left() || position.x > e.bounds.right() {
            return;
        }
        let edge = gpui::px(DRAG_SCROLL_EDGE);
        let step = if position.y < e.bounds.top() + edge {
            gpui::px(12.0)
        } else if position.y > e.bounds.bottom() - edge {
            gpui::px(-12.0)
        } else {
            return;
        };
        let scroll = &self.project_scroll;
        let mut offset = scroll.offset();
        let max = scroll.max_offset().y;
        offset.y = (offset.y + step).clamp(-max, gpui::px(0.0));
        if offset != scroll.offset() {
            scroll.set_offset(offset);
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
                        bg: blend(bg, fg, 0.12),
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
                    cx.emit(SidebarEvent::RefocusTerminal);
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
                    .whitespace_nowrap()
                    .text_ellipsis()
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
        let marker = status_color(Some(AgentStatus::Working), self.fg, self.bg);
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
        let live_idle = self.model.sessions.iter().any(|row| {
            row.live_idle()
                && row
                    .cwd
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
                d.bg(waiting_tint(self.fg, self.bg))
            })
            .when(
                worktree.is_some_and(|w| self.selected.as_ref() == Some(&w.path)),
                |d| d.bg(fg.opacity(SELECTED_ROW)),
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
            .child(status_icon(dot, self.spin, self.fg, self.bg))
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
            let (icon, color) = pr_badge(pr, self.bg);
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
                    .flex()
                    .gap_0p5()
                    .child(format!("{}{}", pr_reference(pr).1, icon))
                    .child(div().text_color(fg.opacity(0.8)).child(match pr.checks {
                        CheckState::Success => "✓",
                        CheckState::Failure => "!",
                        CheckState::Pending => "◌",
                        CheckState::None => "",
                    }))
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
                d.bg(waiting_tint(self.fg, self.bg))
            })
            .when(self.selected.as_ref() == Some(&wt.path), |d| {
                d.bg(fg.opacity(SELECTED_ROW))
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
                    .child(status_icon(dot, self.spin, self.fg, self.bg)),
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
            let scroll = self.project_scroll.clone();
            let path = wt.path.clone();
            let this = cx.entity().downgrade();
            div()
                .child(row)
                .on_children_prepainted(move |bounds, window, _| {
                    let Some(target) = bounds.first().copied() else {
                        return;
                    };
                    let viewport = scroll.bounds();
                    // Reserve the repository context strip at the upper edge.
                    // Fully visible rows retain their exact scroll offset.
                    let top = viewport.top() + gpui::px(REPO_CONTEXT_HEIGHT);
                    let delta = if target.top() < top {
                        top - target.top()
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
                            let mut offset = scroll.offset();
                            offset.y =
                                (offset.y + delta).clamp(-scroll.max_offset().y, gpui::px(0.0));
                            if offset != scroll.offset() {
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
            .when(picked, |d| d.bg(fg.opacity(SELECTED_ROW)))
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
                            .bg(blend(self.bg, fg, 0.08))
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

/// The id part shared by a Sessions row's element ids and debug selectors.
fn session_id(key: SessionKey) -> String {
    match key {
        SessionKey::Pane(pane) => format!("pane-{}", pane.raw()),
        SessionKey::Tab(tab) => format!("tab-{tab:?}"),
    }
}

impl SidebarView {
    /// One Sessions row: status icon, label, what it is, activity age and,
    /// for terminals and idle agents, a close button. Child agents follow it.
    fn render_session_row(
        &self,
        row: &SessionRow,
        focused: bool,
        children: Vec<&ChildGroup>,
        now: u64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fg = self.fg;
        let key = row.key;
        let id = session_id(key);
        let agent_name = row.agent.as_ref().map(|agent| {
            self.agents
                .iter()
                .find(|a| a.id == *agent)
                .map_or_else(|| agent.clone(), |a| a.name.clone())
        });
        let kind = match (row.kind, &agent_name) {
            (SessionKind::Agent, Some(name)) if row.shares_tab => {
                format!("{name} \u{b7} pane {}", row.pane_index)
            }
            (SessionKind::Agent, Some(name)) => name.clone(),
            (SessionKind::Agent, None) => "Agent".into(),
            (SessionKind::Terminal, _) => "Terminal".into(),
            (SessionKind::View(_), _) => "View".into(),
        };
        // Branch names read with their repository; an alias stands alone.
        let label = match (&row.repo, &row.branch) {
            (Some(repo), Some(branch)) if row.title == *branch => format!("{repo} / {branch}"),
            _ => row.title.clone(),
        };
        let place = match (row.kind, &agent_name) {
            (SessionKind::Agent, Some(name)) => {
                format!(
                    "{name} \u{b7} {} \u{b7} pane {}",
                    row.tab_title, row.pane_index
                )
            }
            _ => row.tab_title.clone(),
        };
        let tooltip = format!(
            "{label}\n{}\n{place}\n{}",
            row.location(),
            row.status_label()
        );
        let idle_for = (row.live_idle() && row.since > 0).then(|| activity_age(now, row.since));
        let close = match row.kind {
            SessionKind::Terminal => Some((
                format!("session-close-tab-{:?}", row.tab),
                "Close terminal tab",
                SidebarEvent::CloseTab(row.tab),
            )),
            SessionKind::Agent if row.live_idle() => match key {
                SessionKey::Pane(pane) => Some((
                    format!("session-close-pane-{}", pane.raw()),
                    "Close agent pane",
                    SidebarEvent::ClosePane(pane),
                )),
                SessionKey::Tab(_) => None,
            },
            _ => None,
        };
        let click = match key {
            SessionKey::Pane(pane) => SidebarEvent::FocusPane(pane),
            SessionKey::Tab(tab) => SidebarEvent::FocusTab(tab),
        };
        let line = div()
            .id(ElementId::Name(format!("session:{id}").into()))
            .debug_selector({
                let id = id.clone();
                move || format!("session-{id}")
            })
            .when(row.status == Some(AgentStatus::WaitingInput), |d| {
                d.bg(waiting_tint(self.fg, self.bg))
            })
            .when(focused, |d| d.bg(fg.opacity(SELECTED_ROW)))
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .px_2()
            .py_0p5()
            .cursor_pointer()
            .hover(|s| s.bg(fg.opacity(0.08)))
            .tooltip(crate::tooltip::text(tooltip))
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(click.clone())))
            .child(
                div()
                    .id(ElementId::Name(format!("session-status:{id}").into()))
                    .debug_selector({
                        let id = id.clone();
                        move || format!("session-status-{id}")
                    })
                    .flex_shrink_0()
                    .tooltip(crate::tooltip::text(row.status_label()))
                    .child(status_icon(row.status, self.spin, self.fg, self.bg)),
            )
            // The label keeps a readable width; what the row is gives way first.
            .child(
                div()
                    .debug_selector({
                        let id = id.clone();
                        move || format!("session-label-{id}")
                    })
                    .flex_1()
                    .min_w(gpui::px(48.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(label),
            )
            .child(
                div()
                    .text_xs()
                    .flex_shrink_1()
                    .min_w_0()
                    .max_w(gpui::px(96.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(fg.opacity(0.5))
                    .child(kind),
            )
            .child(
                div()
                    .id(ElementId::Name(format!("activity-age:{id}").into()))
                    .w(gpui::px(34.0))
                    .flex_shrink_0()
                    .text_right()
                    .text_xs()
                    .text_color(fg.opacity(0.5))
                    .tooltip(crate::tooltip::text({
                        let mut text = match row.previous_activity {
                            Some(at) => format!(
                                "Last output: {} ago\nPrevious work before restart: {} ago",
                                activity_age(now, row.last_activity),
                                activity_age(now, at)
                            ),
                            None => "Time since the last terminal output or screen update".into(),
                        };
                        if let Some(age) = &idle_for {
                            text.push_str(&format!("\nReady for another task for {age}"));
                        }
                        text
                    }))
                    .child(activity_age(now, row.last_activity)),
            )
            .when_some(close, |d, (selector, tip, event)| {
                d.child(
                    div()
                        .id(ElementId::Name(selector.clone().into()))
                        .debug_selector(move || selector.clone())
                        .flex_shrink_0()
                        .px_1()
                        .rounded_sm()
                        .text_color(fg.opacity(0.7))
                        .hover(|s| s.bg(fg.opacity(0.15)))
                        .tooltip(crate::tooltip::text(tip))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.stop_propagation();
                            cx.emit(event.clone());
                        }))
                        .child("\u{00d7}"),
                )
            });
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .child(line)
            .children(
                children
                    .into_iter()
                    .map(|g| self.render_children(g, now, cx)),
            )
            .into_any_element()
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
        let focused = self.focused_session();
        // A child group sits under its parent pane's row, else under the
        // first row of the parent's tab.
        let parents: Vec<(SessionKey, &ChildGroup)> = self
            .child_groups
            .iter()
            .filter_map(|g| {
                let rows = &self.model.sessions;
                let own = SessionKey::Pane(g.pane);
                let key = if rows.iter().any(|r| r.key == own) {
                    Some(own)
                } else {
                    rows.iter().find(|r| r.tab == g.tab).map(|r| r.key)
                };
                key.map(|key| (key, g))
            })
            .collect();
        let sessions: Vec<AnyElement> = if self.sessions_collapsed {
            Vec::new()
        } else {
            self.model
                .sessions
                .iter()
                .map(|row| {
                    let children = parents
                        .iter()
                        .filter(|(key, _)| *key == row.key)
                        .map(|(_, g)| *g)
                        .collect();
                    self.render_session_row(row, focused == Some(row.key), children, now, cx)
                })
                .collect()
        };
        let starred: Vec<AnyElement> = self
            .model
            .starred
            .iter()
            .enumerate()
            .map(|(i, s)| self.render_starred(i, s, cx))
            .collect();
        let label_toggle = div()
            .id("sessions-label-toggle")
            .debug_selector(|| "sessions-label-toggle".into())
            .px_1()
            .cursor_pointer()
            .hover(|s| s.bg(fg.opacity(0.08)))
            .tooltip(crate::tooltip::text(
                "Switch Sessions labels between branch aliases and branch names",
            ))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|_, _, _, cx| {
                cx.stop_propagation();
                cx.emit(SidebarEvent::ToggleActiveLabel)
            }))
            .child(match self.active_label {
                chda_config::ActiveLabel::Alias => "Alias",
                chda_config::ActiveLabel::Branch => "Branch",
            });
        let sessions_header = self.section_header(
            "Sessions",
            self.sessions_collapsed,
            self.model.sessions.len(),
            SidebarEvent::ToggleSessions,
            label_toggle,
            cx,
        );
        // Sessions takes what it needs up to 40% of the sidebar, or the
        // room PROJECT leaves when collapsed; it scrolls on its own.
        let sessions_list = div()
            .id("sessions-list")
            .debug_selector(|| "sessions-list".into())
            .flex()
            .flex_col()
            .min_h_0()
            .when(self.project_collapsed, |d| d.flex_shrink_1())
            .when(!self.project_collapsed, |d| {
                d.flex_none().max_h(relative(0.4))
            })
            .overflow_y_scroll()
            .track_scroll(&self.sessions_scroll)
            .children(sessions)
            .when(self.model.sessions.is_empty(), |d| {
                d.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(fg.opacity(0.5))
                        .child("No open sessions"),
                )
            });
        let add_repo = div()
            .id("add-repo")
            .debug_selector(|| "add-repo".into())
            .px_1()
            .rounded_sm()
            .cursor_pointer()
            .hover(|s| s.bg(fg.opacity(0.15)))
            .tooltip(crate::tooltip::text("Add a repository (cmd-shift-o)"))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|_, _, _, cx| {
                cx.stop_propagation();
                cx.emit(SidebarEvent::AddRepo);
            }))
            .child("+ repo");
        let project_header = self
            .section_header(
                "PROJECT",
                self.project_collapsed,
                self.model.repos.len(),
                SidebarEvent::ToggleProject,
                add_repo,
                cx,
            )
            .mt_1()
            .border_t_1()
            .border_color(fg.opacity(0.1));
        let paths: Vec<_> = self
            .model
            .repos
            .iter()
            .map(|r| (r.path.clone(), r.collapsed))
            .collect();
        let scroll = self.project_scroll.clone();
        let current = self.sticky_repo.clone();
        let this = cx.entity().downgrade();
        let repo_groups = div()
            .flex()
            .flex_col()
            .children(repos)
            .on_children_prepainted(move |bounds, window, _| {
                let viewport = scroll.bounds();
                let next = paths
                    .iter()
                    .zip(bounds)
                    .find_map(|((path, collapsed), bounds)| {
                        (!collapsed
                            && bounds.top() < viewport.top()
                            && bounds.bottom() > viewport.top() + gpui::px(REPO_CONTEXT_HEIGHT))
                        .then(|| path.clone())
                    });
                if next == current {
                    return;
                }
                let this = this.clone();
                window.on_next_frame(move |_, cx| {
                    let _ = this.update(cx, |s, cx| {
                        if s.sticky_repo != next {
                            s.sticky_repo = next;
                            cx.notify();
                        }
                    });
                });
            });
        let project = div().flex().flex_col().pl_2()
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
            .when(!self.picked.is_empty(), |d| {
                let count = self.picked.len();
                let accent = status_color(Some(AgentStatus::Working), fg, self.bg);
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
                                .hover(move |s| s.text_color(accent))
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
            .child(repo_groups)
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
        // PROJECT's own viewport: navigation reveals, drag auto-scroll and
        // the sticky repository name all work inside it.
        let sticky = self
            .sticky_repo
            .as_ref()
            .and_then(|path| self.model.repos.iter().find(|r| &r.path == path));
        let project_region = div()
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("project-list")
                    .debug_selector(|| "project-list".into())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.project_scroll)
                    .on_drag_move(
                        cx.listener(|this, e: &gpui::DragMoveEvent<RepoDrag>, _, cx| {
                            this.scroll_while_dragging(e, cx)
                        }),
                    )
                    .child(project),
            )
            .when_some(sticky, |d, repo| {
                d.child(
                    div()
                        .id("sticky-repo-context")
                        .debug_selector(|| "sticky-repo-context".into())
                        .absolute()
                        .top_0()
                        .left_0()
                        .w_full()
                        .h(gpui::px(REPO_CONTEXT_HEIGHT))
                        .flex()
                        .items_center()
                        .px_3()
                        .bg(self.bg)
                        .border_b_1()
                        .border_color(fg.opacity(0.15))
                        .text_sm()
                        .font_weight(gpui::FontWeight::BOLD)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .tooltip(crate::tooltip::text(
                            repo.path.to_string_lossy().into_owned(),
                        ))
                        .child(repo.name.clone()),
                )
            });
        div()
            .id("sidebar")
            .debug_selector(|| "sidebar".into())
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(self.bg)
            .text_color(fg)
            .text_sm()
            .on_drag_move(cx.listener(|this, e, _, cx| this.dragged.track(e, cx)))
            .child(sessions_header)
            .when(!self.sessions_collapsed, |d| d.child(sessions_list))
            .child(project_header)
            .when(!self.project_collapsed, |d| d.child(project_region))
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

    #[test]
    fn pr_colors_follow_lifecycle_in_both_themes_for_every_check_state() {
        for (bg, colors) in [
            (0xffffff, [0x1a7f37, 0x8250df, 0xd1242f]),
            (0x1e1e2e, [0x3fb950, 0xab7df8, 0xf85149]),
        ] {
            for (state, color) in [PrState::Open, PrState::Merged, PrState::Closed]
                .into_iter()
                .zip(colors)
            {
                for checks in [
                    CheckState::Success,
                    CheckState::Failure,
                    CheckState::Pending,
                    CheckState::None,
                ] {
                    let info = pr(state, checks, "https://github.com/org/repo/pull/1");
                    assert_eq!(
                        pr_badge(&info, gpui::rgb(bg).into()).1,
                        gpui::rgb(color).into(),
                        "{state:?}, {checks:?}"
                    );
                    assert!(pr_tooltip(&info).contains(match state {
                        PrState::Open => "open",
                        PrState::Merged => "merged",
                        PrState::Closed => "closed",
                    }));
                }
            }
        }
    }

    #[test]
    fn status_icons_stand_out_three_to_one_in_light_and_dark_themes() {
        let contrast = |a: Hsla, b: Hsla| ratio(luminance(a), luminance(b));
        // Terminal background and text of each theme.
        let themes = [
            ("GitHub Light Default", 0xffffff, 0x1f2328),
            ("Catppuccin Latte", 0xeff1f5, 0x4c4f69),
            ("GitHub Dark Default", 0x0d1117, 0xe6edf3),
            ("Catppuccin Mocha", 0x1e1e2e, 0xcdd6f4),
        ];
        let statuses = [
            None,
            Some(AgentStatus::Working),
            Some(AgentStatus::WaitingInput),
            Some(AgentStatus::Review),
            Some(AgentStatus::Idle),
        ];
        let hex = |c: Hsla| {
            let c = c.to_rgb();
            let byte = |v: f32| (v * 255.0).round() as u8;
            format!("#{:02x}{:02x}{:02x}", byte(c.r), byte(c.g), byte(c.b))
        };
        let mut low = Vec::new();
        for (theme, bg, fg) in themes {
            let (bg, fg): (Hsla, Hsla) = (gpui::rgb(bg).into(), gpui::rgb(fg).into());
            // The sidebar is the background blended 4% toward the text; the
            // tab bar 6%, with the active tab on the background itself.
            let sidebar = blend(bg, fg, 0.04);
            let bar = blend(bg, fg, 0.06);
            for status in statuses {
                let base = status_base(status);
                for (icon, color) in [
                    ("sidebar", status_color(status, fg, sidebar)),
                    ("tab", status_color(status, fg, bar)),
                ] {
                    assert!(
                        (color.h - base.h).abs() < 1e-3 && (color.s - base.s).abs() < 1e-3,
                        "{theme} {icon} {status:?} left its hue"
                    );
                }
                let color = status_color(status, fg, sidebar);
                let selected = blend(sidebar, fg, SELECTED_ROW);
                if contrast(base, sidebar).min(contrast(base, selected)) >= 3.0 {
                    assert_eq!(color, base, "{theme} {status:?} changed while legible");
                }
                let mut under = vec![
                    ("sidebar", color, sidebar),
                    ("selected row", color, selected),
                    ("tab bar", status_color(status, fg, bar), bar),
                    ("active tab", status_color(status, fg, bar), bg),
                ];
                if status == Some(AgentStatus::WaitingInput) {
                    let tint = waiting_tint(fg, sidebar);
                    let tinted = blend(sidebar, tint.opacity(1.0), tint.a);
                    under.push(("tinted row", color, tinted));
                }
                for (surface, color, under) in under {
                    let ratio = contrast(color, under);
                    if ratio < 3.0 {
                        low.push(format!(
                            "{theme}, {surface} {}: {status:?} {} {ratio:.2}:1",
                            hex(under),
                            hex(color)
                        ));
                    }
                }
            }
        }
        assert!(
            low.is_empty(),
            "below 3:1 (WCAG 2.2 SC 1.4.11):\n{}",
            low.join("\n")
        );
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
