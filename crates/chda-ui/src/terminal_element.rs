//! Paints a terminal [`Frame`] as a fixed-width cell grid.

use chda_term::{Cell, CellWidth, CursorShape, Frame, Rgb, Size, Underline};
use gpui::{
    App, BorderStyle, Bounds, ContentMask, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, FocusHandle, Font, FontStyle, FontWeight, GlobalElementId, Hitbox, HitboxBehavior,
    Hsla, InspectorElementId, IntoElement, LayoutId, Pixels, Point, Rgba, ShapedLine,
    StrikethroughStyle, Style, TextAlign, TextRun, UnderlineStyle, Window, fill, outline, point,
    px, relative, size,
};

use crate::terminal_view::TerminalView;

/// Cell dimensions for the current font.
#[derive(Clone, Copy, Debug)]
struct Metrics {
    cell_width: Pixels,
    line_height: Pixels,
    font_size: Pixels,
}

struct TextBatch {
    origin: Point<Pixels>,
    line: ShapedLine,
}

struct CursorLayout {
    bounds: Bounds<Pixels>,
    shape: CursorShape,
    color: Hsla,
    /// The glyph under a block cursor, drawn in the background color.
    text: Option<ShapedLine>,
}

pub struct Layout {
    hitbox: Hitbox,
    background: Hsla,
    rects: Vec<(Bounds<Pixels>, Hsla)>,
    text: Vec<TextBatch>,
    cursor: Option<CursorLayout>,
    /// IME composition drawn over the cursor cell.
    marked: Option<(Bounds<Pixels>, ShapedLine)>,
    metrics: Metrics,
}

pub struct TerminalElement {
    view: Entity<TerminalView>,
    focus: FocusHandle,
}

impl TerminalElement {
    pub fn new(view: Entity<TerminalView>, focus: FocusHandle) -> Self {
        Self { view, focus }
    }

    fn metrics(&self, window: &Window, cx: &App) -> (Font, Metrics) {
        let settings = &self.view.read(cx).settings;
        let font_size = settings.font_size;
        let font = settings.font();
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&font);
        let cell_width = text_system
            .advance(font_id, font_size, 'm')
            .map(|s| s.width)
            .unwrap_or(px(8.0));
        let ascent = text_system.ascent(font_id, font_size);
        let descent = text_system.descent(font_id, font_size);
        // Descent is negative. Ghostty sizes cells the same way: ascent to
        // descent, no extra leading.
        let line_height = px(f32::from(ascent - descent).ceil());
        (
            font,
            Metrics {
                cell_width,
                line_height,
                font_size,
            },
        )
    }
}

impl IntoElement for TerminalElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = Layout;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        window.set_focus_handle(&self.focus, cx);

        let (font, metrics) = self.metrics(window, cx);
        let cols = (f32::from(bounds.size.width) / f32::from(metrics.cell_width)).floor() as u16;
        let rows = (f32::from(bounds.size.height) / f32::from(metrics.line_height)).floor() as u16;
        let grid = Size {
            cols: cols.max(2),
            rows: rows.max(2),
        };
        self.view.update(cx, |view, _| {
            view.set_grid(grid, metrics.cell_width, metrics.line_height)
        });

        let (frame, marked_text, selection) = {
            let view = self.view.read(cx);
            (
                view.frame(),
                view.marked_text.clone(),
                (
                    view.settings.selection_background.map(hsla),
                    view.settings.selection_foreground.map(hsla),
                ),
            )
        };
        let focused = self.focus.is_focused(window);
        let mut layout = Layout {
            hitbox,
            background: hsla(frame.background),
            rects: Vec::new(),
            text: Vec::new(),
            cursor: None,
            marked: None,
            metrics,
        };
        layout_frame(
            &frame,
            &font,
            bounds.origin,
            focused,
            selection,
            &mut layout,
            window,
        );

        if let (Some(text), Some(cursor)) = (marked_text, &layout.cursor) {
            let run = TextRun {
                len: text.len(),
                font: font.clone(),
                color: hsla(frame.foreground),
                background_color: None,
                underline: Some(UnderlineStyle {
                    thickness: px(1.0),
                    color: Some(hsla(frame.foreground)),
                    wavy: false,
                }),
                strikethrough: None,
            };
            let line = window.text_system().shape_line(
                text.into(),
                metrics.font_size,
                &[run],
                Some(metrics.cell_width),
            );
            let mut b = cursor.bounds;
            b.size.width = line.width().max(metrics.cell_width);
            layout.marked = Some((b, line));
        }
        let cursor_bounds = layout.cursor.as_ref().map(|c| c.bounds);
        self.view
            .update(cx, |view, _| view.cursor_bounds = cursor_bounds);
        layout
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        layout: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.handle_input(
            &self.focus,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.set_cursor_style(CursorStyle::IBeam, &layout.hitbox);
            window.paint_quad(fill(bounds, layout.background));
            for (rect, color) in &layout.rects {
                window.paint_quad(fill(*rect, *color));
            }
            for batch in &layout.text {
                let _ = batch.line.paint(
                    batch.origin,
                    layout.metrics.line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
            if let Some((bounds, line)) = &layout.marked {
                window.paint_quad(fill(*bounds, layout.background));
                let _ = line.paint(
                    bounds.origin,
                    layout.metrics.line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
                return;
            }
            if let Some(cursor) = &layout.cursor {
                match cursor.shape {
                    CursorShape::Block => {
                        window.paint_quad(fill(cursor.bounds, cursor.color));
                        if let Some(text) = &cursor.text {
                            let _ = text.paint(
                                cursor.bounds.origin,
                                layout.metrics.line_height,
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        }
                    }
                    CursorShape::BlockHollow => {
                        window.paint_quad(outline(cursor.bounds, cursor.color, BorderStyle::Solid));
                    }
                    CursorShape::Bar => {
                        let mut b = cursor.bounds;
                        b.size.width = px(2.0);
                        window.paint_quad(fill(b, cursor.color));
                    }
                    CursorShape::Underline => {
                        let mut b = cursor.bounds;
                        b.origin.y += b.size.height - px(2.0);
                        b.size.height = px(2.0);
                        window.paint_quad(fill(b, cursor.color));
                    }
                }
            }
        });
    }
}

pub fn hsla(c: Rgb) -> Hsla {
    Rgba {
        r: f32::from(c.r) / 255.0,
        g: f32::from(c.g) / 255.0,
        b: f32::from(c.b) / 255.0,
        a: 1.0,
    }
    .into()
}

/// Style key used to batch adjacent cells into one shaped line.
#[derive(Clone, PartialEq)]
struct RunStyle {
    fg: Hsla,
    bold: bool,
    italic: bool,
    underline: Option<UnderlineStyle>,
    strikethrough: Option<StrikethroughStyle>,
}

/// Selection colors from the config; `None` inverts the cell.
type SelectionColors = (Option<Hsla>, Option<Hsla>);

impl RunStyle {
    fn of(cell: &Cell, frame: &Frame, selection: SelectionColors) -> Self {
        let mut fg = hsla(cell.fg);
        if cell.style.faint {
            fg.a *= 0.7;
        }
        if cell.selected {
            fg = selection
                .1
                .unwrap_or_else(|| hsla(cell.bg.unwrap_or(frame.background)));
        }
        let underline = (cell.style.underline != Underline::None).then(|| UnderlineStyle {
            thickness: px(1.0),
            color: Some(cell.underline_color.map(hsla).unwrap_or(fg)),
            wavy: cell.style.underline == Underline::Curly,
        });
        let strikethrough = cell.style.strikethrough.then_some(StrikethroughStyle {
            thickness: px(1.0),
            color: Some(fg),
        });
        Self {
            fg,
            bold: cell.style.bold,
            italic: cell.style.italic,
            underline,
            strikethrough,
        }
    }

    fn text_run(&self, base: &Font, len: usize) -> TextRun {
        TextRun {
            len,
            font: Font {
                weight: if self.bold {
                    FontWeight::BOLD
                } else {
                    base.weight
                },
                style: if self.italic {
                    FontStyle::Italic
                } else {
                    base.style
                },
                ..base.clone()
            },
            color: self.fg,
            background_color: None,
            underline: self.underline,
            strikethrough: self.strikethrough,
        }
    }
}

fn layout_frame(
    frame: &Frame,
    font: &Font,
    origin: Point<Pixels>,
    focused: bool,
    selection: SelectionColors,
    layout: &mut Layout,
    window: &Window,
) {
    let m = layout.metrics;
    let text_system = window.text_system();
    let cell_origin = |x: u16, y: u16| {
        point(
            origin.x + m.cell_width * f32::from(x),
            origin.y + m.line_height * f32::from(y),
        )
    };

    for y in 0..frame.size.rows {
        let cells = frame.row_cells(y);

        // Background rectangles, merged across runs of the same color.
        let mut run: Option<(u16, u16, Hsla)> = None;
        for (x, cell) in cells.iter().enumerate() {
            let x = x as u16;
            let bg = if cell.selected {
                Some(selection.0.unwrap_or_else(|| hsla(cell.fg)))
            } else {
                cell.bg.map(hsla)
            };
            match (run.as_mut(), bg) {
                (Some((_, end, color)), Some(bg)) if *color == bg && *end + 1 == x => *end = x,
                _ => {
                    if let Some((start, end, color)) = run.take() {
                        layout
                            .rects
                            .push((rect(cell_origin(start, y), end - start + 1, m), color));
                    }
                    run = bg.map(|bg| (x, x, bg));
                }
            }
        }
        if let Some((start, end, color)) = run {
            layout
                .rects
                .push((rect(cell_origin(start, y), end - start + 1, m), color));
        }

        // Text batches: adjacent cells with the same style share one shaped line.
        let mut batch: Option<(u16, u16, RunStyle, String)> = None;
        let flush = |batch: &mut Option<(u16, u16, RunStyle, String)>, layout: &mut Layout| {
            if let Some((start, _, style, text)) = batch.take() {
                let run = style.text_run(font, text.len());
                let line =
                    text_system.shape_line(text.into(), m.font_size, &[run], Some(m.cell_width));
                layout.text.push(TextBatch {
                    origin: cell_origin(start, y),
                    line,
                });
            }
        };
        for (x, cell) in cells.iter().enumerate() {
            let x = x as u16;
            if matches!(cell.width, CellWidth::SpacerTail | CellWidth::SpacerHead)
                || !cell.has_text()
                || cell.style.invisible
            {
                flush(&mut batch, layout);
                continue;
            }
            let text = frame.cell_text(cell);
            let has_decoration =
                cell.style.underline != Underline::None || cell.style.strikethrough;
            if text == " " && !has_decoration {
                flush(&mut batch, layout);
                continue;
            }
            let style = RunStyle::of(cell, frame, selection);
            let wide = cell.width == CellWidth::Wide;
            match batch.as_mut() {
                Some((_, end, s, buf)) if !wide && *s == style && *end + 1 == x => {
                    buf.push_str(text);
                    *end = x;
                }
                _ => {
                    flush(&mut batch, layout);
                    batch = Some((x, x, style, text.to_owned()));
                    if wide {
                        flush(&mut batch, layout);
                    }
                }
            }
        }
        flush(&mut batch, layout);
    }

    if let Some(cursor) = frame.cursor {
        let x = cursor.x.saturating_sub(u16::from(cursor.at_wide_tail));
        let cell = frame.cell(x, cursor.y);
        let wide = cell.is_some_and(|c| c.width == CellWidth::Wide);
        let bounds = rect(cell_origin(x, cursor.y), if wide { 2 } else { 1 }, m);
        let color = cursor.color.map(hsla).unwrap_or(hsla(frame.foreground));
        let shape = if focused {
            cursor.shape
        } else {
            CursorShape::BlockHollow
        };
        let text = match (shape, cell) {
            (CursorShape::Block, Some(cell)) if cell.has_text() => {
                let bg = cell.bg.map(hsla).unwrap_or(layout.background);
                let run = TextRun {
                    len: frame.cell_text(cell).len(),
                    font: font.clone(),
                    color: bg,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                Some(text_system.shape_line(
                    frame.cell_text(cell).to_owned().into(),
                    m.font_size,
                    &[run],
                    Some(m.cell_width),
                ))
            }
            _ => None,
        };
        layout.cursor = Some(CursorLayout {
            bounds,
            shape,
            color,
            text,
        });
    }
}

fn rect(origin: Point<Pixels>, cells: u16, m: Metrics) -> Bounds<Pixels> {
    Bounds::new(
        point(origin.x.floor(), origin.y.floor()),
        size((m.cell_width * f32::from(cells)).ceil(), m.line_height),
    )
}
