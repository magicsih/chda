//! Native unified diff and private notes. Delivery is an explicit clipboard handoff.
use crate::{
    environment::Environment,
    text_input::{TextInput, TextInputEvent},
};
use chda_core::{
    DiffLineKind, PaneId, ReviewAnchor, ReviewCopy, ReviewDiff, ReviewNote, ReviewRepository,
    ReviewSide, ReviewSnapshot, read_review_diff, review_batch,
};
use gpui::{
    AnyElement, Context, Div, Entity, EventEmitter, Focusable, Hsla, Render, Stateful,
    Subscription, UniformListScrollHandle, Window, div, prelude::*, px, uniform_list,
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReviewTarget {
    pub pane: PaneId,
    pub cwd: PathBuf,
    pub agent: String,
    pub session: String,
    pub label: String,
}
#[derive(Clone)]
pub(crate) struct ReviewCopyRequest {
    pub target: ReviewTarget,
    pub text: String,
    pub generation: u64,
    pub ids: Vec<u64>,
}
pub(crate) enum ReviewEvent {
    Targets,
    Changed,
    Copy(ReviewCopyRequest),
}
pub(crate) struct NoteEditor {
    expected: Option<ReviewNote>,
    anchor: ReviewAnchor,
    pub(crate) input: Entity<TextInput>,
    _events: Subscription,
    saving: bool,
}
pub(crate) struct Batch {
    text: String,
    revision: String,
    generation: u64,
    ids: Vec<u64>,
    pub(crate) targets: Vec<ReviewTarget>,
    target: Option<usize>,
}
pub(crate) struct DiffReviewView {
    pub worktree: PathBuf,
    pub base: String,
    pub repository: Arc<ReviewRepository>,
    env: Rc<Environment>,
    pub background: Hsla,
    pub foreground: Hsla,
    pub font_family: String,
    pub font_size: gpui::Pixels,
    pub diff: Option<ReviewDiff>,
    pub notes: Option<ReviewSnapshot>,
    pub error: Option<String>,
    pub loading: bool,
    generation: u64,
    file: usize,
    files_open: bool,
    selection: Option<(usize, usize, ReviewSide)>,
    selected_notes: HashSet<u64>,
    attachments: HashMap<u64, Option<chda_core::ReviewAttachment>>,
    attachment_generation: u64,
    notes_page: usize,
    pub(crate) editor: Option<NoteEditor>,
    pub(crate) batch: Option<Batch>,
    scroll: UniformListScrollHandle,
}
impl EventEmitter<ReviewEvent> for DiffReviewView {}
fn control(id: &'static str, label: impl Into<gpui::SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .cursor_pointer()
        .px_2()
        .py_1()
        .rounded_sm()
        .child(label.into())
}
impl DiffReviewView {
    pub fn new(
        worktree: PathBuf,
        base: String,
        repository: Arc<ReviewRepository>,
        env: Rc<Environment>,
        background: Hsla,
        foreground: Hsla,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut v = Self {
            worktree,
            base,
            repository,
            env,
            background,
            foreground,
            font_family: crate::Settings::default().font_family,
            font_size: crate::Settings::default().font_size,
            diff: None,
            notes: None,
            error: None,
            loading: false,
            generation: 0,
            file: 0,
            files_open: false,
            selection: None,
            selected_notes: HashSet::new(),
            attachments: HashMap::new(),
            attachment_generation: 0,
            notes_page: 0,
            editor: None,
            batch: None,
            scroll: UniformListScrollHandle::new(),
        };
        v.refresh(cx);
        v
    }
    pub fn pending(&self) -> bool {
        self.editor.is_some() || self.batch.is_some()
    }
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        self.loading = true;
        self.batch = None;
        self.selection = None;
        let (path, base, repository) = (
            self.worktree.clone(),
            self.base.clone(),
            self.repository.clone(),
        );
        let operation = self.env.windows.borrow().update.operation();
        let task =
            cx.background_spawn(
                async move { (read_review_diff(&path, &base), repository.snapshot()) },
            );
        cx.spawn(async move |this, cx| {
            let (diff, notes) = task.await;
            let _ = this.update(cx, |v, cx| {
                if v.generation != generation {
                    return;
                }
                v.loading = false;
                match diff {
                    Ok(diff) => {
                        v.file = v.file.min(diff.files.len().saturating_sub(1));
                        v.diff = Some(diff);
                        v.error = None;
                    }
                    Err(e) => {
                        v.diff = None;
                        v.error = Some(e.to_string());
                    }
                }
                v.apply_notes(notes, cx);
            });
            drop(operation);
        })
        .detach();
        cx.notify();
    }
    pub fn reload_notes(&mut self, cx: &mut Context<Self>) {
        let repository = self.repository.clone();
        let operation = self.env.windows.borrow().update.operation();
        let task = cx.background_spawn(async move { repository.snapshot() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |v, cx| v.apply_notes(result, cx));
            drop(operation);
        })
        .detach();
    }
    fn apply_notes(&mut self, result: std::io::Result<ReviewSnapshot>, cx: &mut Context<Self>) {
        match result {
            Ok(snapshot)
                if self
                    .notes
                    .as_ref()
                    .is_none_or(|old| old.generation <= snapshot.generation) =>
            {
                if self.notes.is_none() {
                    self.selected_notes = snapshot
                        .notebook
                        .notes
                        .iter()
                        .filter(|n| n.worktree == self.worktree && !n.resolved)
                        .map(|n| n.id)
                        .collect();
                }
                self.notes = Some(snapshot);
                self.rebuild_attachments(cx);
            }
            Ok(_) => {}
            Err(e) => self.error = Some(format!("Notes could not be loaded: {e}")),
        }
        cx.notify();
    }
    fn rebuild_attachments(&mut self, cx: &mut Context<Self>) {
        self.attachment_generation += 1;
        let generation = self.attachment_generation;
        self.attachments.clear();
        let Some(diff) = self.diff.clone() else {
            return;
        };
        let Some(notes) = self.notes.as_ref() else {
            return;
        };
        let notes: Vec<_> = notes
            .notebook
            .notes
            .iter()
            .filter(|n| n.worktree == self.worktree)
            .cloned()
            .collect();
        let operation = self.env.windows.borrow().update.operation();
        let task = cx.background_spawn(async move {
            notes
                .iter()
                .map(|n| (n.id, n.anchor.reattach(&diff)))
                .collect()
        });
        cx.spawn(async move |this, cx| {
            let attachments = task.await;
            let _ = this.update(cx, |v, cx| {
                if v.attachment_generation == generation {
                    v.attachments = attachments;
                    cx.notify();
                }
            });
            drop(operation);
        })
        .detach();
    }
    pub(crate) fn select_line(
        &mut self,
        row: usize,
        side: ReviewSide,
        extend: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(diff) = &self.diff else {
            return;
        };
        let first = self
            .selection
            .filter(|(_, _, s)| extend && *s == side)
            .map(|(first, _, _)| first)
            .unwrap_or(row);
        let (a, b) = (first.min(row), first.max(row));
        match ReviewAnchor::capture(diff, self.file, a, b, side) {
            Ok(_) => {
                self.selection = Some((a, b, side));
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
        cx.notify();
    }
    pub(crate) fn edit_note(
        &mut self,
        note: Option<ReviewNote>,
        reattach: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.is_some() {
            self.error = Some("Save or cancel the current draft first".into());
            cx.notify();
            return;
        }
        let anchor = if let Some(note) = &note
            && !reattach
        {
            Ok(note.anchor.clone())
        } else {
            self.diff
                .as_ref()
                .zip(self.selection)
                .ok_or_else(|| {
                    std::io::Error::other("Select a diff line or Shift-click a range first")
                })
                .and_then(|(d, (a, b, side))| ReviewAnchor::capture(d, self.file, a, b, side))
        };
        let anchor = match anchor {
            Ok(a) => a,
            Err(e) => {
                self.error = Some(e.to_string());
                cx.notify();
                return;
            }
        };
        let input = cx.new(|cx| {
            let mut i = TextInput::new("Write a review note", self.foreground, self.background, cx);
            i.multiline = true;
            if let Some(note) = &note {
                i.set_text(&note.text, cx);
            }
            i
        });
        let events = cx.subscribe_in(&input, window, |v, _, event, window, cx| match event {
            TextInputEvent::Submit(_) => v.save_note(cx),
            TextInputEvent::Cancel => {
                if !v.editor.as_ref().is_some_and(|e| e.saving) {
                    v.editor = None;
                    window.blur(cx);
                    cx.notify();
                }
            }
        });
        window.focus(&input.focus_handle(cx), cx);
        self.editor = Some(NoteEditor {
            expected: note,
            anchor,
            input,
            _events: events,
            saving: false,
        });
        self.error = None;
        cx.notify();
    }
    pub(crate) fn save_note(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        if editor.saving {
            return;
        }
        let text = editor.input.read(cx).text().to_owned();
        if text.trim().is_empty() {
            self.error = Some("Write a note before saving".into());
            cx.notify();
            return;
        }
        editor.saving = true;
        let (expected, anchor, path, repository) = (
            editor.expected.clone(),
            editor.anchor.clone(),
            self.worktree.clone(),
            self.repository.clone(),
        );
        let edited_id = expected.as_ref().map(|n| n.id);
        let operation = self.env.windows.borrow().update.operation();
        let task = cx.background_spawn(async move {
            repository.save_note(expected.as_ref(), &path, anchor, text)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |v, cx| {
                match result {
                    Ok(snapshot) => {
                        if let Some(note) = snapshot
                            .notebook
                            .notes
                            .iter()
                            .filter(|n| {
                                n.worktree == v.worktree && edited_id.is_none_or(|id| id == n.id)
                            })
                            .max_by_key(|n| n.id)
                        {
                            v.selected_notes.insert(note.id);
                        }
                        v.editor = None;
                        v.error = None;
                        v.apply_notes(Ok(snapshot), cx);
                        cx.emit(ReviewEvent::Changed);
                    }
                    Err(e) => {
                        if let Some(editor) = &mut v.editor {
                            editor.saving = false;
                        }
                        v.error = Some(format!("Save failed. Draft kept: {e}"));
                    }
                }
                cx.notify();
            });
            drop(operation);
        })
        .detach();
        cx.notify();
    }
    fn resolve(&mut self, note: ReviewNote, cx: &mut Context<Self>) {
        let repository = self.repository.clone();
        let operation = self.env.windows.borrow().update.operation();
        let task =
            cx.background_spawn(async move { repository.resolve_note(&note, !note.resolved) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |v, cx| match result {
                Ok(snapshot) => {
                    v.apply_notes(Ok(snapshot), cx);
                    cx.emit(ReviewEvent::Changed);
                }
                Err(e) => {
                    v.error = Some(e.to_string());
                    cx.notify();
                }
            });
            drop(operation);
        })
        .detach();
    }
    pub(crate) fn prepare_batch(&mut self, candidates: Vec<ReviewTarget>, cx: &mut Context<Self>) {
        if self.editor.is_some() {
            self.error = Some("Save or cancel the draft before preparing a batch".into());
            cx.notify();
            return;
        }
        let (path, base, repository, ids) = (
            self.worktree.clone(),
            self.base.clone(),
            self.repository.clone(),
            self.selected_notes.clone(),
        );
        self.generation += 1;
        let generation = self.generation;
        self.loading = true;
        self.batch = None;
        let operation = self.env.windows.borrow().update.operation();
        let task = cx.background_spawn(async move {
            let diff = read_review_diff(&path, &base)?;
            let snapshot = repository.snapshot()?;
            let notes: Vec<_> = snapshot
                .notebook
                .notes
                .iter()
                .filter(|n| n.worktree == path && ids.contains(&n.id))
                .cloned()
                .collect();
            if notes.is_empty() || notes.len() != ids.len() {
                return Err(std::io::Error::other(
                    "Select current notes to prepare a batch",
                ));
            }
            let text = review_batch(&path, &diff, &notes)?;
            let targets = candidates
                .into_iter()
                .filter(|t| chda_core::worktree_root(&t.cwd).is_ok_and(|root| root == path))
                .collect();
            Ok((
                diff,
                snapshot,
                Batch {
                    text,
                    revision: String::new(),
                    generation: 0,
                    ids: notes.iter().map(|n| n.id).collect(),
                    targets,
                    target: None,
                },
            ))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |v, cx| {
                if generation != v.generation {
                    return;
                }
                v.loading = false;
                match result {
                    Ok((diff, snapshot, mut batch)) => {
                        batch.revision = diff.revision.clone();
                        batch.generation = snapshot.generation;
                        v.diff = Some(diff);
                        v.apply_notes(Ok(snapshot), cx);
                        v.batch = Some(batch);
                        v.error = None;
                    }
                    Err(e) => v.error = Some(e.to_string()),
                }
                cx.notify();
            });
            drop(operation);
        })
        .detach();
        cx.notify();
    }
    fn confirm_copy(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let Some(batch) = &self.batch else {
            return;
        };
        let Some(target) = batch.target.and_then(|i| batch.targets.get(i)).cloned() else {
            self.error = Some("Choose a live conversation in this worktree".into());
            cx.notify();
            return;
        };
        let request = ReviewCopyRequest {
            target,
            text: batch.text.clone(),
            generation: batch.generation,
            ids: batch.ids.clone(),
        };
        let (path, base, repository, revision, generation) = (
            self.worktree.clone(),
            self.base.clone(),
            self.repository.clone(),
            batch.revision.clone(),
            self.generation,
        );
        self.loading = true;
        let operation = self.env.windows.borrow().update.operation();
        let task = cx.background_spawn(async move {
            let diff = read_review_diff(&path, &base)?;
            if diff.revision != revision
                || repository.snapshot()?.generation != request.generation
                || chda_core::worktree_root(&request.target.cwd)? != path
            {
                return Err(std::io::Error::other(
                    "Diff, notes or target worktree changed. Prepare the batch again.",
                ));
            }
            Ok(request)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |v, cx| {
                if generation != v.generation {
                    return;
                }
                v.loading = false;
                match result {
                    Ok(request) => cx.emit(ReviewEvent::Copy(request)),
                    Err(e) => v.error = Some(e.to_string()),
                }
                cx.notify();
            });
            drop(operation);
        })
        .detach();
    }
    pub(crate) fn copy_result(
        &mut self,
        request: ReviewCopyRequest,
        accepted: bool,
        cx: &mut Context<Self>,
    ) {
        if !accepted {
            self.error = Some(
                "The target conversation changed. Prepare the batch again; notes are kept.".into(),
            );
            cx.notify();
            return;
        }
        self.batch = None;
        let (repository, path) = (self.repository.clone(), self.worktree.clone());
        let operation = self.env.windows.borrow().update.operation();
        let task = cx.background_spawn(async move {
            let at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            repository.mark_copied(
                request.generation,
                &request.ids,
                &path,
                ReviewCopy {
                    agent: request.target.agent,
                    session: request.target.session,
                    at,
                },
            )
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |v, cx| match result {
                Ok(snapshot) => {
                    v.apply_notes(Ok(snapshot), cx);
                    cx.emit(ReviewEvent::Changed);
                }
                Err(e) => {
                    v.error = Some(format!(
                        "Copied to clipboard; copy status could not be saved: {e}"
                    ));
                    cx.notify();
                }
            });
            drop(operation);
        })
        .detach();
        cx.notify();
    }
    fn row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let line = &self.diff.as_ref().unwrap().files[self.file].lines[index];
        let selected = self
            .selection
            .is_some_and(|(a, b, _)| index >= a && index <= b);
        let color = match line.kind {
            DiffLineKind::Added => gpui::rgb(0xa6e3a1),
            DiffLineKind::Removed => gpui::rgb(0xf38ba8),
            _ => gpui::rgb(0xcdd6f4),
        };
        let mut row = div()
            .id(index)
            .h(self.font_size * 1.6 + px(2.0))
            .font_family(self.font_family.clone())
            .text_size(self.font_size)
            .whitespace_nowrap()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .text_color(color)
            .bg(if selected {
                self.foreground.opacity(0.15)
            } else {
                Hsla::from(color).opacity(0.03)
            });
        for (side, number) in [(ReviewSide::Old, line.old), (ReviewSide::New, line.new)] {
            let cell = div()
                .id(if side == ReviewSide::Old {
                    "old"
                } else {
                    "new"
                })
                .debug_selector(move || {
                    format!(
                        "review-{}-line-{index}",
                        if side == ReviewSide::Old {
                            "old"
                        } else {
                            "new"
                        }
                    )
                })
                .w(px(54.0))
                .flex_shrink_0()
                .text_right()
                .text_color(self.foreground.opacity(0.65))
                .child(number.map(|n| n.to_string()).unwrap_or_default());
            row = row.child(if number.is_some() {
                cell.cursor_pointer()
                    .on_click(cx.listener(move |v, e: &gpui::ClickEvent, _, cx| {
                        v.select_line(index, side, e.modifiers().shift, cx)
                    }))
                    .into_any_element()
            } else {
                cell.into_any_element()
            });
        }
        row.child(match line.kind {
            DiffLineKind::Added => "+",
            DiffLineKind::Removed => "−",
            _ => " ",
        })
        .child(line.text.clone())
        .into_any_element()
    }
    fn note_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        self.notes
            .iter()
            .flat_map(|s| &s.notebook.notes)
            .filter(|n| n.worktree == self.worktree)
            .skip(self.notes_page * 50)
            .take(50)
            .map(|n| {
                let id = n.id;
                let checked = self.selected_notes.contains(&id);
                let attachment = self.attachments.get(&id);
                let stale = attachment.is_some_and(|at| at.is_none());
                let state = if n.resolved {
                    "Resolved"
                } else if stale {
                    "Stale"
                } else if attachment.is_none() {
                    "Checking location…"
                } else {
                    "Open"
                };
                let path = match n.anchor.side {
                    ReviewSide::Old => &n.anchor.old_path,
                    ReviewSide::New => &n.anchor.new_path,
                };
                let location = attachment
                    .and_then(|at| at.as_ref())
                    .map(|a| (a.first, a.last))
                    .unwrap_or((n.anchor.first, n.anchor.last));
                let edit = n.clone();
                let resolve = n.clone();
                let reattach = n.clone();
                let mut row = div()
                    .id(id as usize)
                    .p_2()
                    .border_b_1()
                    .border_color(self.foreground.opacity(0.15))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .id("select")
                                    .cursor_pointer()
                                    .child(if checked { "☑" } else { "☐" })
                                    .on_click(cx.listener(move |v, _, _, cx| {
                                        if !v.selected_notes.remove(&id) {
                                            v.selected_notes.insert(id);
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(format!(
                                "#{id} · {state} · {:?}:{}–{} · {:?}",
                                path, location.0, location.1, n.anchor.side
                            )),
                    )
                    .child(n.text.clone())
                    .children(n.copied.as_ref().map(|c| {
                        div().text_xs().child(format!(
                            "Copied for {} · {} · paste manually",
                            c.agent, c.session
                        ))
                    }))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(control("edit", "Edit").on_click(cx.listener(
                                move |v, _, w, cx| v.edit_note(Some(edit.clone()), false, w, cx),
                            )))
                            .child(
                                control("resolve", if n.resolved { "Reopen" } else { "Resolve" })
                                    .on_click(cx.listener(move |v, _, _, cx| {
                                        v.resolve(resolve.clone(), cx)
                                    })),
                            ),
                    );
                if stale {
                    row = row.child(control("reattach", "Attach to selected lines…").on_click(
                        cx.listener(move |v, _, w, cx| {
                            v.edit_note(Some(reattach.clone()), true, w, cx)
                        }),
                    ));
                }
                row.into_any_element()
            })
            .collect()
    }
}
impl Render for DiffReviewView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let narrow = f32::from(window.viewport_size().width) < 850.0;
        let fg = self.foreground;
        let mut body = div()
            .id("diff-review")
            .debug_selector(|| "diff-review".into())
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .bg(self.background)
            .text_color(fg)
            .text_sm()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(format!(
                                "Diff review · {} · base {}",
                                self.worktree.display(),
                                self.base
                            )),
                    )
                    .child(
                        control(
                            "review-refresh",
                            if self.loading {
                                "Loading…"
                            } else {
                                "Refresh"
                            },
                        )
                        .on_click(cx.listener(|v, _, _, cx| v.refresh(cx))),
                    )
                    .child(control("review-files", "Files").on_click(cx.listener(
                        |v, _, _, cx| {
                            v.files_open = !v.files_open;
                            cx.notify();
                        },
                    ))),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .px_2()
                    .child(
                        control("review-add-note", "Add note to selected lines…")
                            .on_click(cx.listener(|v, _, w, cx| v.edit_note(None, false, w, cx))),
                    )
                    .child(
                        control("review-send", "Send review notes…")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ReviewEvent::Targets))),
                    ),
            );
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .p_2()
                    .text_color(gpui::rgb(0xf38ba8))
                    .child(error.clone()),
            );
        }
        if let Some(batch) = &self.batch {
            let mut preview=div().id("review-batch").debug_selector(||"review-batch".into()).flex_1().min_h_0().flex().flex_col().gap_2().p_3()
                .child("Review batch · direct delivery unavailable · no terminal input will be submitted")
                .child(div().id("review-batch-text").flex_1().min_h_0().overflow_y_scroll().child(batch.text.clone()));
            for (i, t) in batch.targets.iter().enumerate() {
                preview = preview.child(
                    div()
                        .id(i)
                        .debug_selector(move || format!("review-target-{i}"))
                        .cursor_pointer()
                        .p_2()
                        .bg(fg.opacity(if batch.target == Some(i) { 0.15 } else { 0.03 }))
                        .child(format!(
                            "{}{} · {} · {}",
                            if batch.target == Some(i) {
                                "● "
                            } else {
                                "○ "
                            },
                            t.label,
                            t.agent,
                            t.session
                        ))
                        .on_click(cx.listener(move |v, _, _, cx| {
                            if !v.loading
                                && let Some(b) = &mut v.batch
                            {
                                b.target = Some(i);
                            }
                            cx.notify();
                        })),
                );
            }
            if batch.targets.is_empty() {
                preview=preview.child("No live conversation in this worktree. Notes remain saved; open its agent and try again.");
            }
            return body
                .child(preview)
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .p_2()
                        .child(
                            control("review-batch-cancel", "Cancel").on_click(cx.listener(
                                |v, _, _, cx| {
                                    v.batch = None;
                                    v.generation += 1;
                                    v.loading = false;
                                    cx.notify();
                                },
                            )),
                        )
                        .child(
                            control("review-copy-agent", "Copy and go to agent")
                                .on_click(cx.listener(|v, _, _, cx| v.confirm_copy(cx))),
                        ),
                )
                .into_any_element();
        }
        if let Some(diff) = &self.diff {
            let mut files = div()
                .id("review-file-list")
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_1()
                .p_2();
            for (i, f) in diff.files.iter().enumerate() {
                files = files.child(
                    div()
                        .id(i)
                        .cursor_pointer()
                        .p_1()
                        .bg(fg.opacity(if self.file == i { 0.15 } else { 0.0 }))
                        .child(format!("{} · {:?}", f.status, f.new_path))
                        .on_click(cx.listener(move |v, _, _, cx| {
                            if v.editor.is_some() {
                                v.error =
                                    Some("Save or cancel the draft before changing files".into());
                            } else {
                                v.file = i;
                                v.selection = None;
                                v.scroll = UniformListScrollHandle::new();
                                v.files_open = false;
                            }
                            cx.notify();
                        })),
                );
            }
            let mut content = div().flex_1().min_h_0().flex().min_w_0();
            if narrow {
                if self.files_open {
                    body = body.child(files.max_h(px(140.0)));
                }
            } else {
                content = content.child(files.w(px(200.0)).flex_shrink_0());
            }
            if let Some(file) = diff.files.get(self.file) {
                body = body.child(div().px_2().py_1().child(format!(
                    "{} · {:?} → {:?} · old / new line numbers · Shift-click a range",
                    file.status, file.old_path, file.new_path
                )));
                if file.binary {
                    content =
                        content.child(div().p_3().child("Binary diff · no line anchors available"));
                } else if file.lines.is_empty() {
                    content = content.child(
                        div()
                            .p_3()
                            .child("File metadata changed · no text lines to annotate"),
                    );
                } else {
                    let width = px((file
                        .lines
                        .iter()
                        .map(|l| l.text.chars().count())
                        .max()
                        .unwrap_or(0) as f32
                        * f32::from(self.font_size)
                        + 140.0)
                        .max(320.0));
                    content = content.child(
                        div()
                            .id("review-diff-scroll")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_x_scroll()
                            .child(
                                uniform_list(
                                    "review-diff-lines",
                                    file.lines.len(),
                                    cx.processor(|v, range: std::ops::Range<usize>, _, cx| {
                                        range.map(|i| v.row(i, cx)).collect::<Vec<_>>()
                                    }),
                                )
                                .w(width)
                                .h_full()
                                .track_scroll(&self.scroll),
                            ),
                    );
                }
            } else {
                content = content.child(div().p_3().child("No changes against this base"));
            }
            body = body.child(content);
        } else {
            body = body.child(div().flex_1().p_3().child(if self.loading {
                "Loading diff…"
            } else {
                "Diff unavailable · Refresh to retry"
            }));
        }
        if let Some(editor) = &self.editor {
            let input = editor.input.clone();
            body =
                body.child(
                    div()
                        .id("review-note-editor")
                        .debug_selector(|| "review-note-editor".into())
                        .p_2()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(format!(
                            "{:?}:{}–{} · {:?} · Shift ↵ New line",
                            match editor.anchor.side {
                                ReviewSide::Old => &editor.anchor.old_path,
                                ReviewSide::New => &editor.anchor.new_path,
                            },
                            editor.anchor.first,
                            editor.anchor.last,
                            editor.anchor.side
                        ))
                        .child(
                            div()
                                .id("review-editor-scroll")
                                .max_h(px(120.0))
                                .overflow_y_scroll()
                                .child(input.clone()),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    control(
                                        "review-note-save",
                                        if editor.saving {
                                            "Saving…"
                                        } else {
                                            "Save note"
                                        },
                                    )
                                    .on_click(cx.listener(|v, _, _, cx| v.save_note(cx))),
                                )
                                .child(control("review-note-copy-draft", "Copy draft").on_click(
                                    cx.listener(move |_, _, _, cx| {
                                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                            input.read(cx).text().into(),
                                        ))
                                    }),
                                ))
                                .child(control("review-note-cancel", "Cancel").on_click(
                                    cx.listener(|v, _, w, cx| {
                                        if !v.editor.as_ref().is_some_and(|e| e.saving) {
                                            v.editor = None;
                                            w.blur(cx);
                                            cx.notify();
                                        }
                                    }),
                                )),
                        ),
                );
        } else {
            let count = self
                .notes
                .as_ref()
                .map(|s| {
                    s.notebook
                        .notes
                        .iter()
                        .filter(|n| n.worktree == self.worktree)
                        .count()
                })
                .unwrap_or(0);
            self.notes_page = self.notes_page.min(count.saturating_sub(1) / 50);
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .child(format!(
                        "Notes {count} · page {} · {} selected",
                        self.notes_page + 1,
                        self.selected_notes.len()
                    ))
                    .child(
                        control("review-select-none", "Select none").on_click(cx.listener(
                            |v, _, _, cx| {
                                v.selected_notes.clear();
                                cx.notify();
                            },
                        )),
                    )
                    .children((self.notes_page > 0).then(|| {
                        control("review-notes-previous", "Previous").on_click(cx.listener(
                            |v, _, _, cx| {
                                v.notes_page -= 1;
                                cx.notify();
                            },
                        ))
                    }))
                    .children(((self.notes_page + 1) * 50 < count).then(|| {
                        control("review-notes-next", "Next").on_click(cx.listener(|v, _, _, cx| {
                            v.notes_page += 1;
                            cx.notify();
                        }))
                    })),
            );
            body = body.child(
                div()
                    .id("review-notes")
                    .max_h(px(170.0))
                    .overflow_y_scroll()
                    .children(self.note_rows(cx)),
            );
        }
        body.into_any_element()
    }
}
