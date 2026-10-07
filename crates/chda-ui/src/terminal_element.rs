//! Paints a terminal [`Frame`] as a fixed-width cell grid.

use std::collections::HashSet;
use std::sync::Arc;

use chda_term::{
    Cell, CellWidth, CursorShape, Frame, ImageLayer, Rgb, SearchMark, Size, Underline,
};
use gpui::{
    App, BorderStyle, Bounds, ContentMask, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, FocusHandle, Font, FontStyle, FontWeight, GlobalElementId, Hitbox, HitboxBehavior,
    Hsla, InspectorElementId, IntoElement, LayoutId, Pixels, Point, RenderImage, Rgba, ShapedLine,
    StrikethroughStyle, Style, TextAlign, TextRun, UnderlineStyle, Window, fill, outline, point,
    px, relative, size,
};

use crate::terminal_view::{GridGeometry, LinkOpen, TerminalView};

/// Cell dimensions for the current font.
#[derive(Clone, Copy, Debug)]
struct Metrics {
    cell_width: Pixels,
    line_height: Pixels,
    font_size: Pixels,
}

#[derive(Clone)]
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

/// Shaped text and background rects for one frame, reused while the frame,
/// metrics and origin stay the same (cursor blinks do not re-shape text).
pub struct CachedLayout {
    pub key: (u64, Pixels, Pixels, Point<Pixels>, bool),
    rects: Vec<(Bounds<Pixels>, Hsla)>,
    text: Vec<TextBatch>,
}

pub struct Layout {
    hitbox: Hitbox,
    background: Hsla,
    minimum_contrast: f32,
    rects: Vec<(Bounds<Pixels>, Hsla)>,
    text: Vec<TextBatch>,
    cursor: Option<CursorLayout>,
    /// IME composition drawn over the cursor cell.
    marked: Option<(Bounds<Pixels>, ShapedLine)>,
    /// Underlines for the link under the mouse while cmd is held.
    link: Vec<(Bounds<Pixels>, Hsla)>,
    /// Kitty graphics, in `z` order.
    images: Vec<(ImageLayer, Bounds<Pixels>, Arc<RenderImage>)>,
    /// Textures no longer shown, to free from the atlas.
    dropped_images: Vec<Arc<RenderImage>>,
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
        let advance = text_system
            .advance(font_id, font_size, 'm')
            .map(|s| s.width)
            .unwrap_or(px(8.0));
        let ascent = text_system.ascent(font_id, font_size);
        let descent = text_system.descent(font_id, font_size);
        // As Ghostty does: the advance and ascent to descent (negative), no
        // extra leading, each rounded to whole device pixels.
        let scale = window.scale_factor();
        let device_round = |v: Pixels| px((f32::from(v) * scale).round().max(1.0) / scale);
        let cell_width = device_round(advance);
        let line_height = device_round(ascent - descent);
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
        let scale = window.scale_factor();
        self.view.update(cx, |view, _| {
            view.set_grid(grid, metrics.cell_width, metrics.line_height, scale);
            view.geometry = Some(GridGeometry {
                origin: bounds.origin,
                cell_width: metrics.cell_width,
                line_height: metrics.line_height,
                size: grid,
                scale,
            });
        });

        let (frame, marked_text, selection, minimum_contrast, blink_on, cached) = {
            let view = self.view.read(cx);
            (
                view.frame(),
                view.marked_text.clone(),
                (
                    view.settings.selection_background.map(hsla),
                    view.settings.selection_foreground.map(hsla),
                ),
                view.settings.minimum_contrast,
                view.blink_on,
                view.layout_cache.clone(),
            )
        };
        let focused = self.focus.is_focused(window);
        let mut layout = Layout {
            hitbox,
            background: hsla(frame.background),
            minimum_contrast,
            rects: Vec::new(),
            text: Vec::new(),
            cursor: None,
            marked: None,
            link: Vec::new(),
            images: Vec::new(),
            dropped_images: Vec::new(),
            metrics,
        };
        self.view.update(cx, |view, _| {
            let mut used = HashSet::new();
            for p in &frame.images {
                let Some(texture) = view.image_textures.get(p) else {
                    continue;
                };
                used.insert(crate::terminal_images::key(p));
                let origin = point(
                    bounds.origin.x
                        + metrics.cell_width * p.col as f32
                        + px(p.x_offset as f32 / scale),
                    bounds.origin.y
                        + metrics.line_height * p.row as f32
                        + px(p.y_offset as f32 / scale),
                );
                let extent = size(px(p.width as f32 / scale), px(p.height as f32 / scale));
                layout
                    .images
                    .push((p.layer, Bounds::new(origin, extent), texture));
            }
            layout.dropped_images = view.image_textures.retain(&used);
        });
        if let Some(link) = self.view.read(cx).hovered_link.as_ref() {
            let color = hsla(frame.foreground);
            // Files get a solid underline, folders a dotted one.
            let dotted = matches!(link.open, LinkOpen::Path { is_dir: true, .. });
            for &(row, from, to) in &link.cells {
                let origin = point(
                    bounds.origin.x + metrics.cell_width * f32::from(from),
                    bounds.origin.y + metrics.line_height * f32::from(row + 1) - px(1.0),
                );
                let width = metrics.cell_width * f32::from(to - from + 1);
                if !dotted {
                    layout
                        .link
                        .push((Bounds::new(origin, size(width, px(1.0))), color));
                    continue;
                }
                let mut x = px(0.0);
                while x < width {
                    let dot = point(origin.x + x, origin.y);
                    layout
                        .link
                        .push((Bounds::new(dot, size(px(2.0), px(1.0))), color));
                    x += px(4.0);
                }
            }
        }
        let key = (
            frame.generation,
            metrics.cell_width,
            metrics.line_height,
            bounds.origin,
            focused,
        );
        match cached.filter(|c| c.key == key) {
            Some(c) => {
                layout.rects = c.rects.clone();
                layout.text = c.text.clone();
                layout.cursor =
                    cursor_layout(&frame, &font, bounds.origin, focused, &layout, window);
            }
            None => {
                layout_frame(
                    &frame,
                    &font,
                    bounds.origin,
                    focused,
                    selection,
                    &mut layout,
                    window,
                );
                let cache = std::rc::Rc::new(CachedLayout {
                    key,
                    rects: layout.rects.clone(),
                    text: layout.text.clone(),
                });
                self.view
                    .update(cx, |view, _| view.layout_cache = Some(cache));
            }
        }
        if focused && !blink_on {
            layout.cursor = None;
        }

        // Composition goes at the cursor cell whether or not the cursor is
        // drawn: TUIs hide it, and the blink hides it half the time.
        let anchor = frame.ime_anchor.map(|(x, y)| {
            let origin = point(
                bounds.origin.x + metrics.cell_width * f32::from(x),
                bounds.origin.y + metrics.line_height * f32::from(y),
            );
            rect(origin, 1, metrics)
        });
        if let (Some(text), Some(anchor)) = (marked_text, anchor) {
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
            let mut b = anchor;
            b.size.width = line.width().max(metrics.cell_width);
            layout.marked = Some((b, line));
        }
        // The IME candidate window follows the same cell.
        let cursor_bounds = layout.cursor.as_ref().map(|c| c.bounds).or(anchor);
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
            let cursor = if layout.link.is_empty() {
                CursorStyle::IBeam
            } else {
                CursorStyle::PointingHand
            };
            window.set_cursor_style(cursor, &layout.hitbox);
            for texture in layout.dropped_images.drain(..) {
                let _ = window.drop_image(texture);
            }
            window.paint_quad(fill(bounds, layout.background));
            paint_images(&layout.images, ImageLayer::BelowBackground, bounds, window);
            for (rect, color) in &layout.rects {
                window.paint_quad(fill(*rect, *color));
            }
            paint_images(&layout.images, ImageLayer::BelowText, bounds, window);
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
            paint_images(&layout.images, ImageLayer::AboveText, bounds, window);
            for (rect, color) in &layout.link {
                window.paint_quad(fill(*rect, *color));
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

/// Search highlight colors: every match, and the current one.
const SEARCH_MATCH: u32 = 0xf9e2af;
const SEARCH_CURRENT: u32 = 0xfab387;
/// Text drawn on a search highlight.
const SEARCH_TEXT: u32 = 0x1e1e2e;

fn search_background(mark: SearchMark) -> Option<Hsla> {
    match mark {
        SearchMark::None => None,
        SearchMark::Match => Some(gpui::rgb(SEARCH_MATCH).into()),
        SearchMark::Current => Some(gpui::rgb(SEARCH_CURRENT).into()),
    }
}

/// Keep text readable when a TUI retains an explicit background across a theme change.
/// Match Ghostty's minimum-contrast behavior: retain adequate colors, otherwise
/// choose the black or white foreground with the higher WCAG contrast ratio.
fn readable_foreground(fg: Rgb, bg: Rgb, minimum: f32) -> Rgb {
    if minimum <= 1.0 {
        return fg;
    }
    let luminance = |color: Rgb| {
        let linear = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
    };
    let a = luminance(fg);
    let b = luminance(bg);
    if (a.max(b) + 0.05) / (a.min(b) + 0.05) >= minimum {
        return fg;
    }
    if (b + 0.05) / 0.05 >= 1.05 / (b + 0.05) {
        Rgb { r: 0, g: 0, b: 0 }
    } else {
        Rgb {
            r: 255,
            g: 255,
            b: 255,
        }
    }
}

impl RunStyle {
    fn of(cell: &Cell, frame: &Frame, selection: SelectionColors, minimum_contrast: f32) -> Self {
        let mut fg = hsla(readable_foreground(
            cell.fg,
            cell.bg.unwrap_or(frame.background),
            minimum_contrast,
        ));
        if cell.style.faint {
            fg.a *= 0.7;
        }
        if cell.selected {
            fg = selection
                .1
                .unwrap_or_else(|| hsla(cell.bg.unwrap_or(frame.background)));
        } else if cell.search != SearchMark::None {
            fg = gpui::rgb(SEARCH_TEXT).into();
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
                search_background(cell.search).or_else(|| cell.bg.map(hsla))
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
        // The cursor cell is shaped alone, as in Ghostty, so a ligature never
        // hides the character under the cursor.
        let cursor_x = frame
            .cursor
            .filter(|c| c.y == y)
            .map(|c| c.x.saturating_sub(u16::from(c.at_wide_tail)));
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
            let style = RunStyle::of(cell, frame, selection, layout.minimum_contrast);
            let alone = cell.width == CellWidth::Wide || cursor_x == Some(x);
            match batch.as_mut() {
                Some((_, end, s, buf)) if !alone && *s == style && *end + 1 == x => {
                    buf.push_str(text);
                    *end = x;
                }
                _ => {
                    flush(&mut batch, layout);
                    batch = Some((x, x, style, text.to_owned()));
                    if alone {
                        flush(&mut batch, layout);
                    }
                }
            }
        }
        flush(&mut batch, layout);
    }

    layout.cursor = cursor_layout(frame, font, origin, focused, layout, window);
}

fn cursor_layout(
    frame: &Frame,
    font: &Font,
    origin: Point<Pixels>,
    focused: bool,
    layout: &Layout,
    window: &Window,
) -> Option<CursorLayout> {
    let m = layout.metrics;
    let text_system = window.text_system();
    let cell_origin = |x: u16, y: u16| {
        point(
            origin.x + m.cell_width * f32::from(x),
            origin.y + m.line_height * f32::from(y),
        )
    };
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
        return Some(CursorLayout {
            bounds,
            shape,
            color,
            text,
        });
    }
    None
}

/// Paint one layer's images, clipped to the terminal.
fn paint_images(
    images: &[(ImageLayer, Bounds<Pixels>, Arc<RenderImage>)],
    layer: ImageLayer,
    clip: Bounds<Pixels>,
    window: &mut Window,
) {
    for (_, image_bounds, texture) in images.iter().filter(|(l, _, _)| *l == layer) {
        let _ = window.paint_image(
            clip,
            *image_bounds,
            Default::default(),
            Arc::clone(texture),
            0,
            false,
        );
    }
}

fn rect(origin: Point<Pixels>, cells: u16, m: Metrics) -> Bounds<Pixels> {
    Bounds::new(
        point(origin.x.floor(), origin.y.floor()),
        size((m.cell_width * f32::from(cells)).ceil(), m.line_height),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chda_term::{ColorConfig, Rgb, Size, Terminal};

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

    #[test]
    fn theme_switch_keeps_cached_prompt_background_readable() {
        let mut terminal = Terminal::new(Size { cols: 20, rows: 3 }, 1024).unwrap();
        let light = ColorConfig {
            foreground: Some(Rgb {
                r: 76,
                g: 79,
                b: 105,
            }),
            background: Some(Rgb {
                r: 239,
                g: 241,
                b: 245,
            }),
            ..ColorConfig::default()
        };
        terminal.set_colors(&light).unwrap();
        // A TUI caches a light composer background but uses the terminal's default text.
        terminal.feed(b"\x1b[48;2;245;245;245m\x1b[39mPrompt text");
        let dark = ColorConfig {
            foreground: Some(Rgb {
                r: 205,
                g: 214,
                b: 244,
            }),
            background: Some(Rgb {
                r: 30,
                g: 30,
                b: 46,
            }),
            ..ColorConfig::default()
        };
        terminal.set_colors(&dark).unwrap();
        let frame = terminal.frame().unwrap();
        assert_eq!(frame.row_text(0), "Prompt text");
        assert_eq!(frame.background, dark.background.unwrap());
        let cell = frame.cell(0, 0).unwrap();
        assert_eq!(
            cell.bg,
            Some(Rgb {
                r: 245,
                g: 245,
                b: 245
            })
        );
        let style = RunStyle::of(
            cell,
            &frame,
            (None, None),
            crate::settings::Settings::default().minimum_contrast,
        );
        let fg = luminance(style.fg);
        let bg = luminance(hsla(cell.bg.unwrap()));
        let contrast = (fg.max(bg) + 0.05) / (fg.min(bg) + 0.05);
        assert!(
            contrast >= 3.0,
            "prompt contrast after the theme switch: {contrast}"
        );
    }
    #[test]
    fn contrast_adjustment_preserves_readable_colors_and_can_be_disabled() {
        let white = Rgb {
            r: 255,
            g: 255,
            b: 255,
        };
        let black = Rgb { r: 0, g: 0, b: 0 };
        let red = Rgb {
            r: 240,
            g: 40,
            b: 40,
        };
        assert_eq!(readable_foreground(red, black, 3.0), red);
        assert_eq!(readable_foreground(white, white, 1.0), white);
        assert_eq!(readable_foreground(white, white, 3.0), black);
        assert_eq!(readable_foreground(black, black, 3.0), white);
    }
}
