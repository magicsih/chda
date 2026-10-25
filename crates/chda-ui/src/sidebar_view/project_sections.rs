//! One level of user-owned project groups. Registration stays in `repos`.
use super::*;
use crate::text_input::{TextInput, TextInputEvent};
use chda_config::ProjectSection;
use gpui::{Bounds, Subscription};
use std::cell::RefCell;

#[derive(Clone)]
pub(super) struct SectionDrag {
    id: String,
    index: usize,
}

pub(super) struct SectionEditor {
    input: gpui::Entity<TextInput>,
    _subscription: Subscription,
}

#[derive(Clone)]
pub(super) struct ProjectLayout {
    repo: Option<(PathBuf, bool)>,
    section: Option<String>,
}

pub(super) type DropTargets = Rc<RefCell<Vec<(Option<String>, Bounds<Pixels>)>>>;

impl SidebarView {
    pub(crate) fn sync_sections(
        &mut self,
        config: &chda_config::ChdaConfig,
        cx: &mut Context<Self>,
    ) {
        self.project_sections = config.project_sections.clone();
        self.repo_sections = config.repo_sections.clone();
        self.uncategorized_collapsed = config.uncategorized_collapsed;
        cx.notify();
    }

    pub(crate) fn repo_section(&self, repo: &Path) -> Option<&str> {
        let id = self.repo_sections.get(repo)?;
        self.project_sections
            .iter()
            .any(|s| s.id == *id)
            .then_some(id.as_str())
    }

    fn set_section_collapsed(
        &mut self,
        id: Option<String>,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = &id {
            if let Some(s) = self.project_sections.iter_mut().find(|s| s.id == *id) {
                s.collapsed = collapsed;
            }
        } else {
            self.uncategorized_collapsed = collapsed;
        }
        cx.emit(SidebarEvent::SectionCollapsed { id, collapsed });
        cx.notify();
    }

    pub(super) fn reveal_project_section(&mut self, repo: &Path, cx: &mut Context<Self>) {
        if self.project_sections.is_empty() {
            return;
        }
        let id = self.repo_section(repo).map(str::to_owned);
        let collapsed = match &id {
            Some(id) => self
                .project_sections
                .iter()
                .any(|s| s.id == *id && s.collapsed),
            None => self.uncategorized_collapsed,
        };
        if collapsed {
            self.set_section_collapsed(id, false, cx);
        }
    }

    pub(crate) fn edit_section(
        &mut self,
        id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = id
            .as_ref()
            .and_then(|id| self.project_sections.iter().find(|s| s.id == *id))
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let input = cx.new(|cx| {
            let mut input =
                TextInput::new("Section name", self.fg, blend(self.bg, self.fg, 0.08), cx);
            input.set_text(&name, cx);
            input
        });
        let subscription = cx.subscribe_in(&input, window, move |this, _, event, _, cx| {
            match event {
                TextInputEvent::Submit(name) if name.trim().is_empty() => return,
                TextInputEvent::Submit(name) => cx.emit(SidebarEvent::SaveSection {
                    id: id.clone(),
                    name: name.trim().into(),
                }),
                TextInputEvent::Cancel => {}
            }
            this.section_editor = None;
            cx.emit(SidebarEvent::RefocusTerminal);
            cx.notify();
        });
        window.focus(&input.read(cx).focus_handle(cx), cx);
        self.section_editor = Some(SectionEditor {
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    pub(super) fn render_section_editor(&self) -> Option<AnyElement> {
        let editor = self.section_editor.as_ref()?;
        Some(
            div()
                .debug_selector(|| "section-name-editor".into())
                .mx_2()
                .my_1()
                .flex()
                .flex_col()
                .gap_1()
                .text_xs()
                .child("Enter to save, Escape to cancel")
                .child(editor.input.clone())
                .into_any_element(),
        )
    }

    fn render_project_section(
        &self,
        section: Option<&ProjectSection>,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = section.map(|s| s.id.clone());
        let name = section
            .map_or("Uncategorized", |s| s.name.as_str())
            .to_owned();
        let collapsed = section.map_or(self.uncategorized_collapsed, |s| s.collapsed);
        let count = self
            .model
            .repos
            .iter()
            .filter(|r| self.repo_section(&r.path) == id.as_deref())
            .count();
        let marker = status_color(Some(AgentStatus::Working), self.fg, self.bg);
        div()
            .id(ElementId::Name(
                format!(
                    "project-section:{}",
                    id.as_deref().unwrap_or("uncategorized")
                )
                .into(),
            ))
            .debug_selector({
                let id = id.clone();
                move || format!("project-group-{}", id.as_deref().unwrap_or("uncategorized"))
            })
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .mt_1()
            .cursor_pointer()
            .text_xs()
            .text_color(self.fg.opacity(0.7))
            .hover(|s| s.bg(self.fg.opacity(0.08)))
            .tooltip(crate::tooltip::text(format!(
                "{} {name}. Drop projects or folders here. Right-click for section actions.",
                if collapsed { "Expand" } else { "Collapse" }
            )))
            .on_click({
                let id = id.clone();
                cx.listener(move |this, _, _, cx| {
                    this.set_section_collapsed(id.clone(), !collapsed, cx)
                })
            })
            .on_mouse_down(MouseButton::Right, {
                let id = id.clone();
                cx.listener(move |_, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    cx.emit(SidebarEvent::SectionMenu(id.clone(), e.position));
                })
            })
            .drag_over::<RepoDrag>(move |s, _, _, _| s.bg(marker.opacity(0.16)))
            .on_drop({
                let id = id.clone();
                cx.listener(move |_, drag: &RepoDrag, _, cx| {
                    cx.stop_propagation();
                    cx.emit(SidebarEvent::AssignRepoSection {
                        repo: drag.path.clone(),
                        section: id.clone(),
                    });
                })
            })
            .when_some(section, |d, section| {
                let drag = SectionDrag {
                    id: section.id.clone(),
                    index,
                };
                let (name, fg, bg) = (name.clone(), self.fg, self.bg);
                d.on_drag(drag, move |_, _, _, cx| {
                    crate::external_drop::start_internal(cx);
                    cx.new(|_| RepoDragPreview {
                        name: name.clone(),
                        fg,
                        bg,
                    })
                })
                .drag_over::<SectionDrag>(move |s, drag, _, _| match drag.index.cmp(&index) {
                    std::cmp::Ordering::Greater => s.border_t_2().border_color(marker),
                    std::cmp::Ordering::Less => s.border_b_2().border_color(marker),
                    std::cmp::Ordering::Equal => s,
                })
                .on_drop({
                    let id = section.id.clone();
                    cx.listener(move |_, drag: &SectionDrag, _, cx| {
                        cx.stop_propagation();
                        cx.emit(SidebarEvent::MoveSection {
                            from: drag.id.clone(),
                            to: id.clone(),
                        });
                    })
                })
            })
            .child(div().w_3().child(if collapsed { "▸" } else { "▾" }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(name),
            )
            .child(
                div()
                    .rounded_sm()
                    .px_1()
                    .bg(self.fg.opacity(0.08))
                    .child(count.to_string()),
            )
            .into_any_element()
    }

    pub(super) fn render_project_groups(&self, cx: &mut Context<Self>) -> gpui::Div {
        let mut rows = Vec::new();
        let mut layout = Vec::new();
        let mut append =
            |section: Option<&ProjectSection>, index: usize, grouped: bool| {
                let id = section.map(|s| s.id.clone());
                if grouped {
                    rows.push(self.render_project_section(section, index, cx));
                    layout.push(ProjectLayout {
                        repo: None,
                        section: id.clone(),
                    });
                }
                let collapsed =
                    grouped && section.map_or(self.uncategorized_collapsed, |s| s.collapsed);
                if !collapsed {
                    for (i, repo) in
                        self.model.repos.iter().enumerate().filter(|(_, r)| {
                            !grouped || self.repo_section(&r.path) == id.as_deref()
                        })
                    {
                        rows.push(self.render_repo(i, repo, cx));
                        layout.push(ProjectLayout {
                            repo: Some((repo.path.clone(), repo.collapsed)),
                            section: id.clone(),
                        });
                    }
                }
            };
        if self.project_sections.is_empty() {
            append(None, 0, false);
        } else {
            for (i, section) in self.project_sections.iter().enumerate() {
                append(Some(section), i, true);
            }
            append(None, self.project_sections.len(), true);
        }
        let scroll = self.project_scroll.clone();
        let current = self.sticky_repo.clone();
        let overhang = self.sticky_overhang.clone();
        let targets = self.section_drop_targets.clone();
        let this = cx.entity().downgrade();
        div()
            .flex()
            .flex_col()
            .children(rows)
            .on_children_prepainted(move |bounds, window, _| {
                *targets.borrow_mut() = layout
                    .iter()
                    .zip(&bounds)
                    .map(|(row, b)| (row.section.clone(), *b))
                    .collect();
                let viewport = scroll.bounds();
                let next = layout.iter().zip(&bounds).find_map(|(row, bounds)| {
                    let (path, collapsed) = row.repo.as_ref()?;
                    (!collapsed
                        && bounds.top() < viewport.top()
                        && bounds.bottom() > viewport.top() + gpui::px(REPO_CONTEXT_HEIGHT))
                    .then(|| {
                        overhang.set(viewport.top() - bounds.top());
                        path.clone()
                    })
                });
                if next != current {
                    let this = this.clone();
                    window.on_next_frame(move |_, cx| {
                        let _ = this.update(cx, |s, cx| {
                            if s.sticky_repo != next {
                                s.sticky_repo = next;
                                cx.notify();
                            }
                        });
                    });
                }
            })
    }

    pub(super) fn scroll_section_drag(
        &mut self,
        e: &gpui::DragMoveEvent<SectionDrag>,
        cx: &mut Context<Self>,
    ) {
        self.scroll_project_drag(e.event.position, e.bounds, cx);
    }
}
