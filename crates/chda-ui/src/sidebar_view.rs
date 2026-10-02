//! The repository / worktree / session tree with git and agent badges.

use std::path::PathBuf;

use chda_core::{
    AgentStatus, CheckState, GitBadges, PrInfo, PrState, RepoEntry, Sidebar, WorktreeEntry,
};
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
    /// The first session index has finished.
    pub sessions_loaded: bool,
    expanded: Vec<PathBuf>,
    /// Highlighted worktree (e.g. after a notification for a closed pane).
    selected: Option<PathBuf>,
    focus_handle: FocusHandle,
    fg: Hsla,
    bg: Hsla,
    /// Agent id to display name ("claude" to "Claude Code"), for tooltips.
    agent_names: Vec<(String, String)>,
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
    pub fn new(
        fg: Hsla,
        bg: Hsla,
        agent_names: Vec<(String, String)>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            model: Sidebar::new(),
            sessions_loaded: false,
            expanded: Vec::new(),
            selected: None,
            focus_handle: cx.focus_handle(),
            fg,
            bg,
            agent_names,
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
                    .child(format!("#{}{}", pr.number, icon))
                    .into_any_element(),
            );
        }
        if wt.missing {
            badges.push(
                div()
                    .id(badge_id("missing"))
                    .text_color(gpui::rgb(0xf38ba8))
                    .tooltip(crate::tooltip::text(
                        "The worktree's folder was deleted outside git. Right-click to prune it.",
                    ))
                    .child("folder missing")
                    .into_any_element(),
            );
        }
        if wt.is_merged() && !wt.is_main {
            badges.push(
                div()
                    .id(badge_id("merged"))
                    .text_color(gpui::rgb(0xa6e3a1))
                    .tooltip(crate::tooltip::text(
                        "The branch's commits are in the default branch.",
                    ))
                    .child("merged")
                    .into_any_element(),
            );
        }
        if !wt.panes.is_empty() {
            let n = wt.panes.len();
            badges.push(
                div()
                    .id(badge_id("panes"))
                    .text_color(fg.opacity(0.6))
                    .tooltip(crate::tooltip::text(format!(
                        "{} in this worktree.",
                        plural(n, "open pane", "open panes")
                    )))
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
            .when(wt.missing, |d| d.opacity(0.55))
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
                    .tooltip(crate::tooltip::text(status_tooltip(wt, &self.agent_names)))
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

/// "1 commit" / "3 commits".
fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
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

/// `host/owner/repo#5` from a pull request URL such as
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
    Some(format!("{host}/{}#{number}", path[..cut].join("/")))
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
    let mut text = format!("PR #{} {state}. Click to open.", pr.number);
    if let Some(location) = pr_location(&pr.url) {
        text.push('\n');
        text.push_str(&location);
    }
    text
}

/// The status dot's tooltip: each busy agent, most urgent first.
fn status_tooltip(wt: &WorktreeEntry, names: &[(String, String)]) -> String {
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
            let name = names
                .iter()
                .find(|(i, _)| i == id)
                .map_or(id.as_str(), |(_, n)| n.as_str());
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
                        .child("1. Add a repository: \"+ repo\" or cmd-shift-o.")
                        .child("2. Click \"+\" next to it and name a branch to create a worktree.")
                        .child("3. Right-click the worktree to open a terminal or run Claude Code / Codex."),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[test]
    fn pr_location_reads_github_gitlab_and_gitea_urls() {
        assert_eq!(
            pr_location("https://github.com/magicsih/chda/pull/49").as_deref(),
            Some("github.com/magicsih/chda#49")
        );
        assert_eq!(
            pr_location("https://gitlab.com/group/sub/proj/-/merge_requests/12").as_deref(),
            Some("gitlab.com/group/sub/proj#12")
        );
        assert_eq!(
            pr_location("https://gitea.example.com/org/repo/pulls/3").as_deref(),
            Some("gitea.example.com/org/repo#3")
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
    }

    #[test]
    fn status_tooltip_names_busy_agents_most_urgent_first() {
        let names = vec![
            ("claude".to_owned(), "Claude Code".to_owned()),
            ("codex".to_owned(), "Codex".to_owned()),
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
