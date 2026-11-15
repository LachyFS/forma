//! The Draw HUD: a compact tool strip floating at the bottom centre of the
//! viewport, with the active tool's options, the Finish button and a hint.
//! Styled only with theme tokens so it follows every color theme.
use super::{DrawCommand, DrawTool};
use crate::app::{Command, Studio};
use crate::theme::Colors;
use gpui::{
    AnyElement, Context, Div, MouseButton, PathBuilder, SharedString, Stateful, Window, canvas,
    div, point, prelude::*, px, rgb,
};

#[derive(Clone, Copy, PartialEq)]
enum Glyph {
    Tool(DrawTool),
    Construction,
    Finish,
    CenterRectangle,
    Minus,
    Plus,
}

/// Stroke icons on a 16 unit grid.
fn glyph(kind: Glyph, color: u32, side: f32) -> AnyElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            use std::f32::consts::{PI, TAU};
            let arc = |x: f32, y: f32, r: f32, from: f32, to: f32| {
                let steps = ((to - from).abs() / TAU * 40.).ceil().max(6.) as usize;
                (0..=steps)
                    .map(|i| {
                        let a = from + (to - from) * i as f32 / steps as f32;
                        (x + a.cos() * r, y + a.sin() * r)
                    })
                    .collect::<Vec<_>>()
            };
            let square = |x: f32, y: f32, s: f32| {
                vec![
                    (x - s, y - s),
                    (x + s, y - s),
                    (x + s, y + s),
                    (x - s, y + s),
                    (x - s, y - s),
                ]
            };
            let paths: Vec<Vec<(f32, f32)>> = match kind {
                Glyph::Tool(DrawTool::Select) => vec![vec![
                    (4.5, 2.5),
                    (4.5, 13.0),
                    (7.2, 10.4),
                    (9.0, 14.2),
                    (10.8, 13.4),
                    (9.0, 9.7),
                    (12.6, 9.7),
                    (4.5, 2.5),
                ]],
                Glyph::Tool(DrawTool::Line) => vec![
                    vec![(4.0, 12.0), (12.0, 4.0)],
                    square(3.5, 12.5, 1.4),
                    square(12.5, 3.5, 1.4),
                ],
                Glyph::Tool(DrawTool::Rectangle) => vec![vec![
                    (2.5, 4.5),
                    (13.5, 4.5),
                    (13.5, 11.5),
                    (2.5, 11.5),
                    (2.5, 4.5),
                ]],
                Glyph::CenterRectangle => vec![
                    vec![
                        (2.5, 4.5),
                        (13.5, 4.5),
                        (13.5, 11.5),
                        (2.5, 11.5),
                        (2.5, 4.5),
                    ],
                    vec![(6.5, 8.0), (9.5, 8.0)],
                    vec![(8.0, 6.5), (8.0, 9.5)],
                ],
                Glyph::Tool(DrawTool::Circle) => {
                    vec![arc(8.0, 8.0, 5.6, 0.0, TAU), vec![(7.4, 8.0), (8.6, 8.0)]]
                }
                Glyph::Tool(DrawTool::Arc) => vec![
                    arc(8.0, 11.0, 5.5, PI, TAU),
                    square(2.5, 11.0, 1.2),
                    square(13.5, 11.0, 1.2),
                ],
                Glyph::Tool(DrawTool::Polygon) => vec![
                    (0..=6)
                        .map(|i| {
                            let a = i as f32 / 6.0 * TAU - PI / 2.0;
                            (8.0 + a.cos() * 5.8, 8.2 + a.sin() * 5.8)
                        })
                        .collect(),
                ],
                Glyph::Tool(DrawTool::Trim) => vec![
                    arc(4.6, 11.6, 2.0, 0.0, TAU),
                    arc(11.4, 11.6, 2.0, 0.0, TAU),
                    vec![(6.0, 10.2), (11.5, 3.0)],
                    vec![(10.0, 10.2), (4.5, 3.0)],
                ],
                Glyph::Construction => vec![
                    vec![(3.0, 13.0), (5.0, 11.0)],
                    vec![(7.0, 9.0), (9.0, 7.0)],
                    vec![(11.0, 5.0), (13.0, 3.0)],
                ],
                Glyph::Finish => vec![vec![(3.5, 8.5), (6.6, 11.6), (12.8, 4.6)]],
                Glyph::Minus => vec![vec![(4.0, 8.0), (12.0, 8.0)]],
                Glyph::Plus => vec![vec![(4.0, 8.0), (12.0, 8.0)], vec![(8.0, 4.0), (8.0, 12.0)]],
            };
            let weight = px((side / 16. * 1.3).max(1.));
            for points in paths {
                let mut path = PathBuilder::stroke(weight);
                for (i, (x, y)) in points.into_iter().enumerate() {
                    let p = bounds.origin + point(px(x * side / 16.), px(y * side / 16.));
                    if i == 0 {
                        path.move_to(p)
                    } else {
                        path.line_to(p)
                    }
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, rgb(color));
                }
            }
        },
    )
    .size(px(side))
    .flex_shrink_0()
    .into_any_element()
}

struct Tip(SharedString, Colors);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(8.))
            .py(px(5.))
            .rounded(px(5.))
            .bg(rgb(self.1.panel))
            .border_1()
            .border_color(rgb(self.1.edge))
            .text_color(rgb(self.1.text))
            .text_size(px(11.))
            .child(self.0.clone())
    }
}

/// Swallows presses so clicks on the HUD never reach the sketch below.
fn shield(element: Stateful<Div>) -> Stateful<Div> {
    element
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
}

fn button(
    t: Colors,
    id: impl Into<SharedString>,
    command: DrawCommand,
    active: bool,
    cx: &mut Context<Studio>,
) -> Stateful<Div> {
    let tip: SharedString = command.hint().into();
    shield(div().id(id.into()))
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .rounded(px(6.))
        .cursor_pointer()
        .bg(rgb(if active { t.active } else { t.panel }))
        .when(active, |d| d.border_1().border_color(rgb(t.accent_line)))
        .hover(move |s| s.bg(rgb(if active { t.active_hover } else { t.raised })))
        .tooltip(move |_, cx| cx.new(|_| Tip(tip.clone(), t)).into())
        .on_click(cx.listener(move |studio, _, window, cx| {
            cx.stop_propagation();
            studio.execute(Command::Draw(command), window, cx);
        }))
}

fn tool_button(t: Colors, tool: DrawTool, active: bool, cx: &mut Context<Studio>) -> AnyElement {
    let ink = if active { t.accent } else { t.text };
    button(
        t,
        format!("draw-tool-{}", tool.key()),
        DrawCommand::Tool(tool),
        active,
        cx,
    )
    .relative()
    .size(px(32.))
    .child(glyph(Glyph::Tool(tool), ink, 17.))
    .child(
        div()
            .absolute()
            .right(px(3.))
            .bottom(px(1.))
            .text_size(px(8.))
            .text_color(rgb(if active { t.accent } else { t.faint }))
            .child(tool.key()),
    )
    .into_any_element()
}

fn separator(t: Colors) -> Div {
    div()
        .w(px(1.))
        .h(px(20.))
        .mx(px(4.))
        .flex_shrink_0()
        .bg(rgb(t.edge))
}

impl Studio {
    /// The Draw HUD, while a session is active.
    pub(crate) fn draw_hud(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.sketch.session.as_ref()?;
        let t = self.theme_colors();
        let mut strip = shield(div().id("draw-hud"))
            .flex()
            .items_center()
            .gap(px(2.))
            .p(px(4.))
            .rounded(px(9.))
            .bg(rgb(t.panel))
            .border_1()
            .border_color(rgb(t.edge))
            .shadow_md()
            .on_mouse_move(cx.listener(|studio, _, _, cx| {
                cx.stop_propagation();
                studio.draw_pointer_in_hud(cx);
            }));
        for tool in DrawTool::ALL {
            strip = strip.child(tool_button(t, tool, session.tool == tool, cx));
        }
        strip = strip.child(separator(t));
        let construction = session.construction;
        strip = strip.child(
            button(
                t,
                "draw-construction",
                DrawCommand::ToggleConstruction,
                construction,
                cx,
            )
            .h(px(32.))
            .px(px(8.))
            .gap(px(6.))
            .text_size(px(11.))
            .text_color(rgb(if construction { t.accent } else { t.muted }))
            .child(glyph(
                Glyph::Construction,
                if construction { t.accent } else { t.muted },
                15.,
            ))
            .child("Construction"),
        );
        match session.tool {
            DrawTool::Rectangle => {
                let center = session.options.rect_center;
                strip = strip.child(separator(t)).child(
                    button(
                        t,
                        "draw-rect-mode",
                        DrawCommand::ToggleRectangleMode,
                        center,
                        cx,
                    )
                    .h(px(32.))
                    .px(px(8.))
                    .gap(px(6.))
                    .text_size(px(11.))
                    .text_color(rgb(if center { t.accent } else { t.muted }))
                    .child(glyph(
                        if center {
                            Glyph::CenterRectangle
                        } else {
                            Glyph::Tool(DrawTool::Rectangle)
                        },
                        if center { t.accent } else { t.muted },
                        15.,
                    ))
                    .child(if center { "Centre" } else { "Corner" }),
                );
            }
            DrawTool::Polygon => {
                let sides = session.options.polygon_sides;
                strip = strip.child(separator(t)).child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .child(
                            button(
                                t,
                                "draw-sides-minus",
                                DrawCommand::PolygonSides(-1),
                                false,
                                cx,
                            )
                            .size(px(26.))
                            .child(glyph(Glyph::Minus, t.muted, 13.)),
                        )
                        .child(
                            div()
                                .min_w(px(58.))
                                .flex()
                                .justify_center()
                                .text_size(px(11.))
                                .text_color(rgb(t.text))
                                .child(format!("{sides} sides")),
                        )
                        .child(
                            button(
                                t,
                                "draw-sides-plus",
                                DrawCommand::PolygonSides(1),
                                false,
                                cx,
                            )
                            .size(px(26.))
                            .child(glyph(Glyph::Plus, t.muted, 13.)),
                        ),
                );
            }
            _ => {}
        }
        strip = strip.child(separator(t)).child(
            shield(div().id("draw-finish"))
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(32.))
                .pl(px(9.))
                .pr(px(10.))
                .rounded(px(6.))
                .cursor_pointer()
                .bg(rgb(t.active))
                .border_1()
                .border_color(rgb(t.accent_line))
                .text_color(rgb(t.accent))
                .text_size(px(12.))
                .hover(move |s| s.bg(rgb(t.active_hover)))
                .tooltip(move |_, cx| cx.new(|_| Tip(DrawCommand::Finish.hint().into(), t)).into())
                .on_click(cx.listener(|studio, _, window, cx| {
                    cx.stop_propagation();
                    studio.execute(Command::Draw(DrawCommand::Finish), window, cx);
                }))
                .child(glyph(Glyph::Finish, t.accent, 15.))
                .child("Finish"),
        );
        let snapping = "Hold Ctrl to place freely";
        let hint = shield(div().id("draw-hint"))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(10.))
            .py(px(4.))
            .rounded(px(7.))
            .bg(rgb(t.panel))
            .border_1()
            .border_color(rgb(t.edge))
            .text_size(px(11.))
            .child(div().text_color(rgb(t.accent)).child(format!(
                "{} · {}",
                session.label,
                session.tool.label()
            )))
            .child(div().text_color(rgb(t.text)).child(self.draw_hint()))
            .when(session.tool != DrawTool::Select, |d| {
                d.child(div().text_color(rgb(t.faint)).child(snapping))
            });
        Some(
            div()
                .absolute()
                .bottom(px(14.))
                .left_0()
                .right_0()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(6.))
                .child(hint)
                .child(strip)
                .into_any_element(),
        )
    }
}
