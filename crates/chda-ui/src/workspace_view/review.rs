use super::*;
use crate::diff_review::{DiffReviewView, ReviewCopyRequest, ReviewEvent, ReviewTarget};

impl WorkspaceView {
    pub(crate) fn open_review(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let context = self
            .sidebar
            .read(cx)
            .model
            .worktree_for_path(path)
            .and_then(|(r, w)| {
                Some((
                    r.path.clone(),
                    w.path.clone(),
                    w.diff.as_ref()?.base.clone(),
                ))
            });
        let Some((repo, worktree, base)) = context else {
            self.notify_error("A worktree diff against a base is required".into());
            return;
        };
        let id = self.ws.open_review(worktree.clone(), repo, base.clone());
        if let Some((view, _)) = self.reviews.get(&id) {
            view.update(cx, |v, cx| {
                if v.base != base {
                    v.base = base;
                    v.refresh(cx);
                }
            });
        } else {
            self.create_review_view(id, worktree, base, cx);
        }
        self.focus_active(window, cx);
        self.sync_panes(cx);
    }
    pub(super) fn create_review_view(
        &mut self,
        id: TabId,
        worktree: PathBuf,
        base: String,
        cx: &mut Context<Self>,
    ) {
        let repository = self
            .env
            .windows
            .borrow_mut()
            .reviews
            .get_or_insert_with(|| {
                Arc::new(chda_core::ReviewRepository::new(self.env.data_dir.clone()))
            })
            .clone();
        let env = self.env.clone();
        let bg = hsla(self.settings.colors.background.unwrap_or_default());
        let fg = hsla(self.settings.colors.foreground.unwrap_or_default());
        let view = cx.new(|cx| {
            let mut view = DiffReviewView::new(worktree, base, repository, env, bg, fg, cx);
            view.font_family = self.settings.font_family.clone();
            view.font_size = self.settings.font_size;
            view
        });
        let sub = cx.subscribe(&view, move |v, review, event, cx| {
            let weak = v.self_weak.clone();
            match event {
                ReviewEvent::Targets => {
                    let mut candidates: Vec<_> = v
                        .env
                        .windows
                        .borrow()
                        .entries
                        .iter()
                        .flat_map(|entry| {
                            entry
                                .sessions
                                .iter()
                                .filter_map(|(pane, (agent, session, live))| {
                                    if !live {
                                        return None;
                                    }
                                    Some(ReviewTarget {
                                        pane: *pane,
                                        cwd: entry.panes.get(pane)?.0.clone(),
                                        agent: agent.clone(),
                                        session: session.clone(),
                                        label: format!("Pane {} · choose conversation", pane.raw()),
                                    })
                                })
                        })
                        .collect();
                    for target in &mut candidates {
                        let label = |view: &WorkspaceView| {
                            view.ws
                                .tabs()
                                .iter()
                                .find(|t| t.panes().contains(&target.pane))
                                .map(|tab| {
                                    format!(
                                        "{} · pane {} · {}",
                                        view.ws.tab_title(tab),
                                        target.pane.raw(),
                                        match view
                                            .ws
                                            .pane(target.pane)
                                            .and_then(|p| p.agent.as_ref())
                                            .map(|a| a.status)
                                        {
                                            Some(AgentStatus::Idle) => "Idle",
                                            Some(AgentStatus::Working) => "Working",
                                            Some(AgentStatus::WaitingInput) => "Needs input",
                                            Some(AgentStatus::Review) => "Turn complete",
                                            None => "Unknown",
                                        }
                                    )
                                })
                        };
                        let entries = v.env.windows.borrow().entries.clone();
                        if let Some(entry) = entries
                            .iter()
                            .find(|e| e.sessions.contains_key(&target.pane))
                        {
                            target.label = if entry.view == v.self_weak {
                                label(v)
                            } else {
                                entry.view.upgrade().and_then(|other| label(other.read(cx)))
                            }
                            .unwrap_or_else(|| "Conversation status unknown".into());
                        }
                    }
                    cx.defer(move |cx| {
                        review.update(cx, |r, cx| r.prepare_batch(candidates, cx));
                    });
                }
                ReviewEvent::Changed => {
                    let entries = v.env.windows.borrow().entries.clone();
                    cx.defer(move |cx| {
                        for entry in entries {
                            let _ = entry.view.update(cx, |v, cx| {
                                for (r, _) in v.reviews.values() {
                                    r.update(cx, |r, cx| r.reload_notes(cx));
                                }
                            });
                        }
                    });
                }
                ReviewEvent::Copy(request) => {
                    let request = request.clone();
                    // The child is emitting this event while borrowed; defer all routing.
                    cx.defer(move |cx| {
                        let _ =
                            weak.update(cx, |v, cx| v.deliver_review_copy(id, review, request, cx));
                    });
                }
            }
        });
        self.reviews.insert(id, (view, sub));
    }
    fn deliver_review_copy(
        &mut self,
        id: TabId,
        review: Entity<DiffReviewView>,
        request: ReviewCopyRequest,
        cx: &mut Context<Self>,
    ) {
        let current = self.ws.active_tab().is_some_and(|tab| tab.id == id)
            && review.read(cx).repository.generation() == request.generation;
        let matches: Vec<_> = self
            .env
            .windows
            .borrow()
            .entries
            .iter()
            .filter(|entry| {
                entry
                    .sessions
                    .get(&request.target.pane)
                    .is_some_and(|(a, s, live)| {
                        *live && a == &request.target.agent && s == &request.target.session
                    })
                    && entry
                        .panes
                        .get(&request.target.pane)
                        .is_some_and(|(cwd, _)| cwd == &request.target.cwd)
            })
            .cloned()
            .collect();
        let accepted = current && matches.len() == 1;
        if accepted {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(request.text.clone()));
            let entry = matches[0].clone();
            let pane = request.target.pane;
            let target = request.target.clone();
            // Never paste, clear input or submit a command. Focusing only happens after copy.
            cx.defer(move |cx| {
                let _ = entry.window.update(cx, |_, window, cx| {
                    window.activate_window();
                    let _ = entry.view.update(cx, |v, cx| {
                        if v.share_identity(pane)
                            == Some((target.agent.clone(), target.session.clone()))
                            && v.ws.pane(pane).is_some_and(|p| {
                                p.agent_live && p.cwd.as_ref() == Some(&target.cwd)
                            })
                        {
                            v.on_sidebar_event(SidebarEvent::FocusPane(pane), window, cx);
                        }
                    });
                });
            });
        }
        review.update(cx, |v, cx| v.copy_result(request, accepted, cx));
    }
}
