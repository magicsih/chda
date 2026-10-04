//! Virtualized, read-only ancestry tab. A refresh invalidates in-flight pages.
use std::path::PathBuf;
use std::sync::Arc;

use chda_core::{GraphLayout, GraphQuery, GraphRow, HistoryPage};
use gpui::{
    AnyElement, Context, Hsla, PathBuilder, Render, UniformListScrollHandle, Window, canvas, div,
    point, prelude::*, px, uniform_list,
};

pub(crate) struct GitGraphView {
    pub(crate) repo: PathBuf,
    pub(crate) rows: Vec<GraphRow>,
    layout: GraphLayout,
    query: Arc<GraphQuery>,
    pub(crate) generation: u64,
    pub(crate) loading: bool,
    pub(crate) has_more: bool,
    pub(crate) error: Option<String>,
    pub(crate) scroll: UniformListScrollHandle,
    columns: usize,
    pub(crate) background: Hsla,
    pub(crate) foreground: Hsla,
}

impl GitGraphView {
    pub(crate) fn new(
        repo: PathBuf,
        background: Hsla,
        foreground: Hsla,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self {
            query: Arc::new(GraphQuery::new(repo.clone())),
            repo,
            rows: Vec::new(),
            layout: GraphLayout::default(),
            generation: 0,
            loading: false,
            has_more: true,
            error: None,
            scroll: UniformListScrollHandle::new(),
            columns: 3,
            background,
            foreground,
        };
        view.load_next(cx);
        view
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.rows.clear();
        self.layout = GraphLayout::default();
        self.query = Arc::new(GraphQuery::new(self.repo.clone()));
        self.loading = false;
        self.has_more = true;
        self.error = None;
        self.columns = 3;
        self.scroll = UniformListScrollHandle::new();
        self.load_next(cx);
    }

    pub(crate) fn load_next(&mut self, cx: &mut Context<Self>) {
        if self.loading || !self.has_more || self.error.is_some() {
            return;
        }
        self.loading = true;
        let generation = self.generation;
        let query = self.query.clone();
        let task = cx.background_spawn(async move { query.next_page() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| view.apply_page(generation, result, cx));
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn apply_page(
        &mut self,
        generation: u64,
        result: std::io::Result<HistoryPage>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.generation {
            return;
        }
        self.loading = false;
        match result {
            Ok(page) => {
                self.has_more = page.has_more;
                for commit in page.commits {
                    let row = self.layout.append(commit);
                    self.columns = self.columns.max(row.columns);
                    self.rows.push(row);
                }
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn row(&self, index: usize) -> AnyElement {
        let row = &self.rows[index];
        let lines = row.lines.clone();
        let color = self.foreground;
        let lane_x = |column| px(12.0 + column as f32 * 18.0);
        let graph = div()
            .relative()
            .h(px(48.0))
            .w(px(24.0 + self.columns as f32 * 18.0))
            .flex_shrink_0()
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        for line in &lines {
                            let mut path = PathBuilder::stroke(px(1.5));
                            path.move_to(
                                bounds.origin + point(lane_x(line.from.0), px(48.0 * line.from.1)),
                            );
                            path.line_to(
                                bounds.origin + point(lane_x(line.to.0), px(48.0 * line.to.1)),
                            );
                            if let Ok(path) = path.build() {
                                window.paint_path(path, color.opacity(0.65));
                            }
                        }
                    },
                )
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .left(lane_x(row.column) - px(4.0))
                    .top(px(20.0))
                    .size(px(8.0))
                    .rounded_full()
                    .bg(color),
            );
        let labels = row
            .commit
            .refs
            .iter()
            .map(|reference| reference.label())
            .collect::<Vec<_>>()
            .join(" · ");
        let tooltip = format!("{}\n{}\n{}", row.commit.subject, row.commit.id, labels);
        div()
            .id(index)
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .h(px(48.0))
            .flex()
            .items_center()
            .gap_2()
            .pr_3()
            .tooltip(crate::tooltip::text(tooltip))
            .hover(|style| style.bg(color.opacity(0.06)))
            .child(graph)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .gap_1()
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .gap_2()
                            .min_w_0()
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_color(color.opacity(0.6))
                                    .child(row.commit.id[..8].to_owned()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(row.commit.subject.clone()),
                            ),
                    )
                    .children((!labels.is_empty()).then(|| {
                        div()
                            .w_full()
                            .text_xs()
                            .text_color(color.opacity(0.75))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(labels)
                    })),
            )
            .into_any_element()
    }
}

impl Render for GitGraphView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let fg = self.foreground;
        let name = self
            .repo
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let path = self.repo.display().to_string();
        let mut content = div()
            .id("git-graph")
            .debug_selector(|| "git-graph".into())
            .size_full()
            .flex()
            .flex_col()
            .bg(self.background)
            .text_color(fg)
            .text_sm()
            .child(
                div()
                    .p_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .border_b_1()
                    .border_color(fg.opacity(0.12))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(format!("Git tree · {name}"))
                            .child(
                                div()
                                    .id("graph-path")
                                    .text_xs()
                                    .text_color(fg.opacity(0.6))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .tooltip(crate::tooltip::text(path.clone()))
                                    .child(path),
                            ),
                    )
                    .child(
                        div()
                            .id("graph-refresh")
                            .debug_selector(|| "graph-refresh".into())
                            .cursor_pointer()
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(fg.opacity(0.12))
                            .on_click(cx.listener(|view, _, _, cx| view.refresh(cx)))
                            .child("Refresh"),
                    ),
            );
        if self.rows.is_empty() {
            let text: &str = if self.error.is_some() {
                "Could not read history"
            } else if self.loading {
                "Loading commits…"
            } else {
                "No commits yet"
            };
            content = content.child(div().flex_1().p_4().child(text));
        } else {
            content = content.child(
                uniform_list(
                    "graph-rows",
                    self.rows.len(),
                    cx.processor(|view, range: std::ops::Range<usize>, _, cx| {
                        let approaching_end = range.end + 20 >= view.rows.len();
                        let rows = range.map(|index| view.row(index)).collect::<Vec<_>>();
                        if approaching_end {
                            view.load_next(cx);
                        }
                        rows
                    }),
                )
                .w_full()
                .flex_1()
                .min_h_0()
                .track_scroll(&self.scroll),
            );
        }
        if let Some(error) = &self.error {
            content = content.child(
                div().p_3().child(error.clone()).child(
                    div()
                        .id("graph-retry")
                        .debug_selector(|| "graph-retry".into())
                        .cursor_pointer()
                        .mt_2()
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(fg.opacity(0.12))
                        .on_click(cx.listener(|view, _, _, cx| view.refresh(cx)))
                        .child("Retry"),
                ),
            );
        } else {
            let state = if self.loading {
                "Loading commits…"
            } else if self.has_more {
                "Scroll for more commits"
            } else {
                "End of history"
            };
            content = content.child(
                div()
                    .px_3()
                    .py_2()
                    .text_xs()
                    .text_color(fg.opacity(0.6))
                    .child(format!("{} commits · {state}", self.rows.len())),
            );
        }
        content
    }
}
