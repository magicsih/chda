//! The repository / worktree / session tree with git and agent badges.

use std::path::PathBuf;

use chda_core::{AgentStatus, CheckState, PrState, RepoEntry, Sidebar, WorktreeEntry};
use gpui::{
    AnyElement, App, Context, ElementId, EventEmitter, FocusHandle, Focusable, Hsla, MouseButton,
    MouseDownEvent, Pixels, Point, Render, Window, div, prelude::*,
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
    /// Resume an agent session in its worktree.
    ResumeSession {
        worktree: PathBuf,
        agent: String,
        session: String,
    },
    AddRepo,
    /// Open a URL (a pull request badge was clicked).
    OpenUrl(String),
    /// Focus an open tab from the activity list.
    FocusTab(chda_core::TabId),
    /// The status dot of a worktree was clicked: go to its agent's pane.
    JumpToAgent(PathBuf),
}

pub struct SidebarView {
    pub model: Sidebar,
    expanded: Vec<PathBuf>,
    /// Highlighted worktree (e.g. after a notification for a closed pane).
    selected: Option<PathBuf>,
    focus_handle: FocusHandle,
    fg: Hsla,
    bg: Hsla,
}

impl EventEmitter<SidebarEvent> for SidebarView {}

pub fn status_color(status: AgentStatus) -> Hsla {
    match status {
        AgentStatus::Idle => gpui::rgb(0x6c7086).into(),
        AgentStatus::Working => gpui::rgb(0x89b4fa).into(),
        AgentStatus::WaitingInput => gpui::rgb(0xfab387).into(),
        AgentStatus::Review => gpui::rgb(0xa6e3a1).into(),
    }
}

impl SidebarView {
    pub fn new(fg: Hsla, bg: Hsla, cx: &mut Context<Self>) -> Self {
        Self {
            model: Sidebar::new(),
            expanded: Vec::new(),
            selected: None,
            focus_handle: cx.focus_handle(),
            fg,
            bg,
        }
    }

    /// Highlight a worktree and make sure its repository is expanded.
    pub fn select(&mut self, worktree: &std::path::Path, cx: &mut Context<Self>) {
        let found = self
            .model
            .worktree_for_path(worktree)
            .map(|(r, w)| (r.path.clone(), w.path.clone()));
        if let Some((repo, path)) = found {
            if let Some(r) = self.model.repo_mut(&repo) {
                r.collapsed = false;
            }
            self.selected = Some(path);
            cx.notify();
        }
    }

    /// New colors after a config reload.
    pub fn set_colors(&mut self, fg: Hsla, bg: Hsla, cx: &mut Context<Self>) {
        self.fg = fg;
        self.bg = bg;
        cx.notify();
    }

    fn toggle_expanded(&mut self, path: &PathBuf, cx: &mut Context<Self>) {
        if let Some(i) = self.expanded.iter().position(|p| p == path) {
            self.expanded.remove(i);
        } else {
            self.expanded.push(path.clone());
        }
        cx.notify();
    }

    fn render_repo(&self, repo: &RepoEntry, cx: &mut Context<Self>) -> AnyElement {
        let fg = self.fg;
        let path = repo.path.clone();
        let collapsed = repo.collapsed;
        let header = div()
            .id(ElementId::Name(
                format!("repo:{}", repo.path.display()).into(),
            ))
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
            .child(
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
            );
        let mut col = div().flex().flex_col().child(header);
        if let Some(err) = &repo.error {
            col = col.child(
                div()
                    .px_4()
                    .text_xs()
                    .text_color(gpui::rgb(0xf38ba8))
                    .child(err.clone()),
            );
        }
        if !collapsed {
            for wt in &repo.worktrees {
                col = col.child(self.render_worktree(wt, cx));
            }
        }
        col.into_any_element()
    }

    fn render_worktree(&self, wt: &WorktreeEntry, cx: &mut Context<Self>) -> AnyElement {
        let fg = self.fg;
        let path = wt.path.clone();
        let expanded = self.expanded.contains(&wt.path);
        let status = wt.status();
        let name = wt.branch.clone().unwrap_or_else(|| "(detached)".into());
        let b = wt.badges;
        let mut badges: Vec<AnyElement> = Vec::new();
        if b.conflicted > 0 {
            badges.push(
                div()
                    .text_color(gpui::rgb(0xf38ba8))
                    .child(format!("!{}", b.conflicted))
                    .into_any_element(),
            );
        }
        if b.dirty_count() > 0 {
            badges.push(
                div()
                    .text_color(gpui::rgb(0xf9e2af))
                    .child(format!("\u{25cf}{}", b.dirty_count() - b.conflicted))
                    .into_any_element(),
            );
        }
        if let (Some(a), Some(bh)) = (b.ahead, b.behind) {
            if a > 0 {
                badges.push(div().child(format!("\u{2191}{a}")).into_any_element());
            }
            if bh > 0 {
                badges.push(div().child(format!("\u{2193}{bh}")).into_any_element());
            }
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
                    .id(ElementId::Name(format!("pr:{}", wt.path.display()).into()))
                    .text_color(color)
                    .cursor_pointer()
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.stop_propagation();
                        cx.emit(SidebarEvent::OpenUrl(url.clone()));
                    }))
                    .child(format!("#{}{}", pr.number, icon))
                    .into_any_element(),
            );
        }
        if wt.is_merged() && !wt.is_main {
            badges.push(
                div()
                    .text_color(gpui::rgb(0xa6e3a1))
                    .child("merged")
                    .into_any_element(),
            );
        }
        if !wt.panes.is_empty() {
            let n = wt.panes.len();
            badges.push(
                div()
                    .text_color(fg.opacity(0.6))
                    .child(if n == 1 {
                        "1 tab".to_owned()
                    } else {
                        format!("{n} tabs")
                    })
                    .into_any_element(),
            );
        }
        let row = div()
            .id(ElementId::Name(format!("wt:{}", wt.path.display()).into()))
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .pl_4()
            .pr_2()
            .py_0p5()
            .cursor_pointer()
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
                    .text_color(status_color(status))
                    .on_click({
                        let path = path.clone();
                        cx.listener(move |_, _, _, cx| {
                            cx.stop_propagation();
                            cx.emit(SidebarEvent::JumpToAgent(path.clone()));
                        })
                    })
                    .child("\u{25cf}"),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(name),
            )
            .children(badges.into_iter().map(|b| div().text_xs().child(b)));
        let mut col = div().flex().flex_col().child(row);
        if expanded {
            for s in wt.sessions.iter().take(8) {
                let (agent, id) = (s.agent.clone(), s.id.clone());
                let path = path.clone();
                col = col.child(
                    div()
                        .id(ElementId::Name(format!("sess:{id}").into()))
                        .flex()
                        .flex_row()
                        .gap_1()
                        .pl_8()
                        .pr_2()
                        .py_0p5()
                        .text_xs()
                        .text_color(fg.opacity(0.75))
                        .cursor_pointer()
                        .hover(|s| s.bg(fg.opacity(0.08)))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(SidebarEvent::ResumeSession {
                                worktree: path.clone(),
                                agent: agent.clone(),
                                session: id.clone(),
                            })
                        }))
                        .child(
                            div()
                                .text_color(fg.opacity(0.5))
                                .child(short_agent(&s.agent)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .child(s.snippet.clone()),
                        )
                        .child(
                            div()
                                .text_color(fg.opacity(0.5))
                                .child(format!("{}", s.message_count)),
                        ),
                );
            }
        }
        col.into_any_element()
    }
}

fn short_agent(agent: &str) -> &'static str {
    match agent {
        "claude" => "CC",
        "codex" => "CX",
        _ => "??",
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
            .map(|r| self.render_repo(r, cx))
            .collect();
        let empty = self.model.repos.is_empty();
        let active: Vec<AnyElement> = self
            .model
            .active_tabs
            .iter()
            .map(|t| {
                let id = t.tab;
                div()
                    .id(ElementId::Name(format!("active:{:?}", t.tab).into()))
                    .flex()
                    .flex_row()
                    .gap_1()
                    .px_2()
                    .py_0p5()
                    .cursor_pointer()
                    .hover(|s| s.bg(fg.opacity(0.08)))
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(SidebarEvent::FocusTab(id))))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(if t.status == AgentStatus::Idle {
                                gpui::transparent_black()
                            } else {
                                status_color(t.status)
                            })
                            .child("\u{25cf}"),
                    )
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
                            .text_color(fg.opacity(0.5))
                            .child(t.repo.clone().unwrap_or_else(|| "\u{2014}".into())),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .id("sidebar")
            .flex()
            .flex_col()
            .size_full()
            .bg(self.bg)
            .text_color(fg)
            .text_sm()
            .overflow_y_scroll()
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
            .when(!active.is_empty(), |d| {
                d.child(
                    div()
                        .px_2()
                        .pt_2()
                        .pb_1()
                        .text_xs()
                        .text_color(fg.opacity(0.6))
                        .child("ACTIVE"),
                )
                .children(active)
                .child(div().h(gpui::px(6.0)))
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
                        .child("1. Add a repository: \"+ repo\" or cmd-shift-o.")
                        .child("2. Click \"+\" next to it and name a branch to create a worktree.")
                        .child("3. Right-click the worktree to open a terminal or run Claude Code / Codex."),
                )
            })
    }
}
