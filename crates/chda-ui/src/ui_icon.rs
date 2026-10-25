//! Small application controls drawn as vectors, independent of text fonts.
use gpui::{Hsla, ParentElement, PathBuilder, Styled, canvas, div, point, px};

#[derive(Clone, Copy)]
pub(crate) enum Icon {
    CollapseAll,
    ExpandAll,
    Working,
    Waiting,
    Unknown,
    Children,
    Merged,
    Missing,
    Panes,
    Trash,
    Busy,
    Sidebar,
    Session,
    SectionAdd,
    Bell,
}

pub(crate) fn icon(icon: Icon, color: Hsla) -> gpui::Div {
    div()
        .size(px(12.0))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let lines: &[&[(f32, f32)]] = match icon {
                        Icon::CollapseAll => &[
                            &[(2., 2.), (6., 5.), (10., 2.)],
                            &[(2., 10.), (6., 7.), (10., 10.)],
                        ],
                        Icon::ExpandAll => &[
                            &[(2., 5.), (6., 2.), (10., 5.)],
                            &[(2., 7.), (6., 10.), (10., 7.)],
                        ],
                        Icon::Working => &[&[(3., 2.), (10., 6.), (3., 10.), (3., 2.)]],
                        Icon::Waiting => &[&[(4., 2.), (4., 10.)], &[(8., 2.), (8., 10.)]],
                        Icon::Unknown => &[
                            &[
                                (4., 3.),
                                (5., 2.),
                                (7., 2.),
                                (8., 3.),
                                (8., 5.),
                                (6., 6.),
                                (6., 7.),
                            ],
                            &[(6., 9.), (6., 10.)],
                        ],
                        Icon::Children => &[
                            &[(6., 10.), (6., 6.), (2., 4.), (2., 1.)],
                            &[(6., 6.), (10., 4.), (10., 1.)],
                        ],
                        Icon::Merged => {
                            &[&[(3., 1.), (3., 11.)], &[(10., 1.), (10., 4.), (3., 8.)]]
                        }
                        Icon::Missing => &[
                            &[
                                (1., 10.),
                                (1., 2.),
                                (5., 2.),
                                (6., 4.),
                                (11., 4.),
                                (11., 10.),
                                (1., 10.),
                            ],
                            &[(7., 5.), (7., 7.)],
                            &[(7., 8.5), (7., 9.)],
                        ],
                        Icon::Panes => &[
                            &[(1., 2.), (11., 2.), (11., 10.), (1., 10.), (1., 2.)],
                            &[(6., 2.), (6., 10.)],
                        ],
                        Icon::Trash => &[
                            &[(2., 3.), (10., 3.)],
                            &[(4., 1.), (8., 1.)],
                            &[(3., 4.), (4., 11.), (8., 11.), (9., 4.)],
                        ],
                        Icon::Busy => &[&[
                            (3., 1.),
                            (9., 1.),
                            (9., 3.),
                            (3., 9.),
                            (3., 11.),
                            (9., 11.),
                            (9., 9.),
                            (3., 3.),
                            (3., 1.),
                        ]],
                        Icon::Sidebar => &[
                            &[(1., 1.), (11., 1.), (11., 11.), (1., 11.), (1., 1.)],
                            &[(5., 1.), (5., 11.)],
                        ],
                        Icon::SectionAdd => &[
                            &[
                                (1., 10.),
                                (1., 2.),
                                (5., 2.),
                                (6., 4.),
                                (11., 4.),
                                (11., 10.),
                                (1., 10.),
                            ],
                            &[(6., 5.), (6., 9.)],
                            &[(4., 7.), (8., 7.)],
                        ],
                        Icon::Bell => &[
                            &[
                                (2., 9.),
                                (3., 7.),
                                (3., 4.),
                                (4., 2.),
                                (6., 1.),
                                (8., 2.),
                                (9., 4.),
                                (9., 7.),
                                (10., 9.),
                                (2., 9.),
                            ],
                            &[(5., 11.), (7., 11.)],
                        ],
                        Icon::Session => &[
                            &[(2., 1.), (10., 1.), (10., 11.), (2., 11.), (2., 1.)],
                            &[(4., 4.), (8., 4.)],
                            &[(4., 7.), (7., 7.)],
                        ],
                    };
                    for line in lines {
                        let mut path = PathBuilder::stroke(px(1.25));
                        for (i, &(x, y)) in line.iter().enumerate() {
                            let at = point(bounds.left() + px(x), bounds.top() + px(y));
                            if i == 0 {
                                path.move_to(at);
                            } else {
                                path.line_to(at);
                            }
                        }
                        if let Ok(path) = path.build() {
                            window.paint_path(path, color);
                        }
                    }
                },
            )
            .size(px(12.0)),
        )
}
