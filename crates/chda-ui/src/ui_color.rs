//! Keep native UI colors readable on their rendered surfaces.
use gpui::Hsla;

pub(crate) fn fit_color(mut color: Hsla, surfaces: &[Hsla], minimum: f32) -> Hsla {
    let row = luminance(surfaces[0]);
    let step = if ratio(row, 0.0) >= ratio(row, 1.0) {
        -0.01
    } else {
        0.01
    };
    loop {
        let light = luminance(color);
        if surfaces
            .iter()
            .all(|s| ratio(light, luminance(*s)) >= minimum)
        {
            return color;
        }
        let next = (color.l + step).clamp(0.0, 1.0);
        if next == color.l {
            return color;
        }
        color.l = next;
    }
}

/// WCAG 2.2 relative luminance of an opaque color.
pub(crate) fn luminance(color: Hsla) -> f32 {
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
pub(crate) fn ratio(a: f32, b: f32) -> f32 {
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
