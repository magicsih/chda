//! The repository / worktree / session tree with git and agent badges.

use std::path::PathBuf;

use chda_core::{AgentStatus, RepoEntry, Sidebar, WorktreeEntry};
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
}

pub struct SidebarView {
    pub model: Sidebar,
    expanded: Vec<PathBuf>,
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
            focus_handle: cx.focus_handle(),
            fg,
            bg,
        }
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
        if !wt.panes.is_empty() {
            badges.push(
                div()
                    .text_color(fg.opacity(0.6))
                    .child(format!("\u{2b1a}{}", wt.panes.len()))
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
            .hover(|s| s.bg(fg.opacity(0.08)))
            .on_click({
                let path = path.clone();
                cx.listener(move |_, _, _, cx| cx.emit(SidebarEvent::OpenWorktree(path.clone())))
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
                    .size_2()
                    .rounded_full()
                    .bg(status_color(status))
                    .flex_shrink_0(),
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
            .children(repos)
            .when(empty, |d| {
                d.child(
                    div()
                        .px_2()
                        .py_2()
                        .text_xs()
                        .text_color(fg.opacity(0.5))
                        .child("No repositories. Press cmd-shift-o or \"+ repo\"."),
                )
            })
    }
}
