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
        // Narrow viewports drop the plane name, then option labels and the
        // Finish label, so the bar always fits (about 370 px at its smallest).
        let width = f32::from(self.bounds.get().size.width);
        let show_label = width >= 780.;
        let compact = width < 580.;
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
            .when(show_label, |bar| {
                bar.child(
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
                .child(vrule(t, 18.))
            });
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
                            .when(!compact, |segment| segment.child(label))
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
                                .min_w(px(if compact { 22. } else { 52. }))
                                .flex()
                                .justify_center()
                                .tabular()
                                .text_size(px(TEXT_SM))
                                .text_color(rgb(t.text))
                                .child(if compact {
                                    sides.to_string()
                                } else {
                                    format!("{sides} sides")
                                }),
                        )
                        .child(plus),
                );
            }
            _ => {}
        }
        let finish = if compact {
            on_command(
                button_base(t, "draw-finish", Tone::Primary, H_MD)
                    .w(px(H_MD))
                    .child(icon(Icon::Check, ICON_MD, t.on_accent))
                    .tooltip(command_tip(t, draw(DrawCommand::Finish))),
                draw(DrawCommand::Finish),
                cx,
            )
        } else {
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
            .px(px(SP_3))
        };
        // Closed regions can go straight into the Extrude tool.
        let profiles = self.sketch_analysis().map_or(0, |a| a.profiles.len());
        if profiles > 0 {
            let extrude = Command::CadTool(crate::cad_tools::ToolCommand::Extrude);
            bar = bar.child(vrule(t, 18.)).child(if compact {
                command_icon(t, "draw-extrude", Icon::Extrude, extrude, false, H_MD, cx)
            } else {
                command_button(
                    t,
                    "draw-extrude",
                    "Extrude",
                    Some(Icon::Extrude),
                    extrude,
                    Tone::Ghost,
                    cx,
                )
                .h(px(H_MD))
                .px(px(SP_2))
            });
        }
        bar = bar.child(vrule(t, 18.)).child(finish);
        Some(bar.into_any_element())
    }
}

impl Studio {
    /// Inspector page for a selected sketch object: what it contains and a
    /// way back into Draw mode.
    pub(crate) fn sketch_inspector(&self, t: Colors, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let id = self.selected?;
        let sketch = self.scene.object_sketch(id)?;
        let profiles = if self.session_object() == Some(id) {
            self.sketch_analysis().map_or(0, |a| a.profiles.len())
        } else {
            self.sketch_displays()
                .iter()
                .find(|display| display.id == id)
                .map_or(0, |display| display.fills.len())
        };
        let construction = sketch.entities.iter().filter(|e| e.construction).count();
        let matrix = self.scene.world_transform(id).unwrap_or_default();
        let normal = matrix
            .transform_vector3(sketch.plane.normal())
            .normalize_or_zero();
        let orientation = if normal.y.abs() > 0.999 {
            "Horizontal"
        } else if normal.z.abs() > 0.999 {
            "Front"
        } else if normal.x.abs() > 0.999 {
            "Side"
        } else if normal.y.abs() < 1.0e-3 {
            "Vertical"
        } else {
            "Tilted"
        };
        let drawing = self.session_object() == Some(id);
        // `section` pads its contents like every other inspector section.
        let contents = col()
            .gap(px(SP_1))
            .child(
                stat_tiles([
                    stat_tile(t, "Curves", sketch.entities.len().to_string()),
                    stat_tile(t, "Profiles", profiles.to_string()),
                    stat_tile(t, "Construction", construction.to_string()),
                ])
                .id("sketch-stats")
                .tooltip(tip(
                    t,
                    "Curves: lines, arcs and circles · Profiles: closed regions features can extrude · Construction: guides that never form profiles",
                    None,
                )),
            )
            .child(readonly_row(
                t,
                "Plane",
                "",
                orientation.into(),
                "The sketch plane's orientation in the world",
            ))
            .child(
                command_button(
                    t,
                    "sketch-edit",
                    if drawing {
                        "Finish sketch"
                    } else {
                        "Edit sketch"
                    },
                    Some(if drawing { Icon::Check } else { Icon::DrawMode }),
                    Command::Draw(if drawing {
                        DrawCommand::Finish
                    } else {
                        DrawCommand::EditSketch
                    }),
                    Tone::Primary,
                    cx,
                )
                .mt(px(SP_2))
                .w_full(),
            );
        Some(
            col().child(section(
                t,
                self,
                crate::ui::PanelSection::Sketch,
                "Sketch",
                Some(
                    format!(
                        "{} curve{}",
                        sketch.entities.len(),
                        if sketch.entities.len() == 1 { "" } else { "s" }
                    )
                    .into(),
                ),
                contents,
                cx,
            )),
        )
    }
}
