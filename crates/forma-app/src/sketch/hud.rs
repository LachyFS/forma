//! The Draw toolbar, shown in the shell's bottom-centre viewport dock while a
//! sketch is open: the plane, drawing tools, the active tool's options,
//! construction mode and Finish. Built only from design-system components so
//! it follows every theme; the shell positions it and keeps viewport presses
//! from reaching the sketch underneath.
use super::{DrawCommand, DrawTool};
use crate::app::{Command, Studio};
use crate::theme::Colors;
use crate::ui::assets::{Icon, icon};
use crate::ui::components::*;
use crate::ui::tokens::*;
use gpui::{AnyElement, Context, MouseMoveEvent, SharedString, div, prelude::*, px, rgb};

pub(crate) fn tool_icon(tool: DrawTool) -> Icon {
    match tool {
        DrawTool::Select => Icon::Select,
        DrawTool::Line => Icon::DrawLine,
        DrawTool::Rectangle => Icon::DrawRectangle,
        DrawTool::Circle => Icon::DrawCircle,
        DrawTool::Arc => Icon::DrawArc,
        DrawTool::Polygon => Icon::DrawPolygon,
        DrawTool::Trim => Icon::DrawTrim,
    }
}

impl Studio {
    /// The Draw toolbar for `ui::slots::viewport_dock`, while drawing.
    pub(crate) fn draw_toolbar(&self, t: Colors, cx: &mut Context<Self>) -> Option<AnyElement> {
        let session = self.sketch.session.as_ref()?;
        let draw = Command::Draw;
        let mut bar = hud(t)
            .id("draw-toolbar")
            .items_center()
            // Pointer moves still pass through for navigation, but while the
            // pointer rests on the toolbar the sketch preview steps aside.
            .on_mouse_move(cx.listener(|studio, event: &MouseMoveEvent, _, cx| {
                if studio.navigation.is_none() && event.pressed_button.is_none() {
                    cx.stop_propagation();
                    studio.draw_pointer_in_hud(cx);
                }
            }))
            .child(
                row()
                    .h(px(H_MD))
                    .pl(px(SP_2))
                    .pr(px(SP_1))
                    .gap(px(SP_1_5))
                    .max_w(px(180.))
                    .child(icon(Icon::Sketch, ICON_SM, t.accent_text))
                    .child(
                        div()
                            .min_w(px(0.))
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_size(px(TEXT_SM))
                            .font_weight(WEIGHT_MEDIUM)
                            .text_color(rgb(t.text_muted))
                            .child(session.label.clone()),
                    ),
            )
            .child(vrule(t, 18.));
        for tool in DrawTool::ALL {
            bar = bar.child(command_icon(
                t,
                SharedString::from(format!("draw-tool-{}", tool.key())),
                tool_icon(tool),
                draw(DrawCommand::Tool(tool)),
                session.tool == tool,
                H_MD,
                cx,
            ));
        }
        bar = bar.child(vrule(t, 18.)).child(command_icon(
            t,
            "draw-construction",
            Icon::DrawConstruction,
            draw(DrawCommand::ToggleConstruction),
            session.construction,
            H_MD,
            cx,
        ));
        match session.tool {
            DrawTool::Rectangle => {
                let center = session.options.rect_center;
                let mut choice = |id: &'static str, label: &'static str, glyph, active: bool| {
                    on_command(
                        segment(t, id, active, H_SM)
                            .child(icon(
                                glyph,
                                ICON_SM,
                                if active { t.text } else { t.text_muted },
                            ))
                            .child(label)
                            .tooltip(command_tip(t, draw(DrawCommand::ToggleRectangleMode))),
                        draw(DrawCommand::ToggleRectangleMode),
                        cx,
                    )
                };
                let corner = choice("draw-rect-corner", "Corner", Icon::DrawRectangle, !center);
                let centre = choice(
                    "draw-rect-center",
                    "Centre",
                    Icon::DrawCenterRectangle,
                    center,
                );
                bar = bar
                    .child(vrule(t, 18.))
                    .child(segmented(t).child(corner).child(centre));
            }
            DrawTool::Polygon => {
                let sides = session.options.polygon_sides;
                let minus = command_icon(
                    t,
                    "draw-sides-minus",
                    Icon::Minus,
                    draw(DrawCommand::PolygonSides(-1)),
                    false,
                    H_SM,
                    cx,
                );
                let plus = command_icon(
                    t,
                    "draw-sides-plus",
                    Icon::Plus,
                    draw(DrawCommand::PolygonSides(1)),
                    false,
                    H_SM,
                    cx,
                );
                bar = bar.child(vrule(t, 18.)).child(
                    row()
                        .gap(px(2.))
                        .child(minus)
                        .child(
                            div()
                                .min_w(px(52.))
                                .flex()
                                .justify_center()
                                .tabular()
                                .text_size(px(TEXT_SM))
                                .text_color(rgb(t.text))
                                .child(format!("{sides} sides")),
                        )
                        .child(plus),
                );
            }
            _ => {}
        }
        bar = bar.child(vrule(t, 18.)).child(
            command_button(
                t,
                "draw-finish",
                "Finish",
                Some(Icon::Check),
                draw(DrawCommand::Finish),
                Tone::Primary,
                cx,
            )
            .h(px(H_MD))
            .px(px(SP_3)),
        );
        Some(bar.into_any_element())
    }
}
