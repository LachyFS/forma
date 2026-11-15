//! The tool HUD for the viewport dock, the adjust-last-operation panel and
//! status bar hints. Built only from design-system components and tokens.
use super::sketch_tool::{Operation, SketchToolKind};
use super::{AdjustParam, Tool, ToolCommand};
use crate::app::{Command, Field, Studio};
use crate::theme::Colors;
use crate::ui::assets::{Icon, icon};
use crate::ui::components::*;
use crate::ui::tokens::*;
use gpui::{AnyElement, Context, Div, MouseMoveEvent, SharedString, div, prelude::*, px, rgb};

pub(crate) fn operation_icon(operation: Operation) -> Icon {
    match operation {
        Operation::NewPart => Icon::OpNew,
        Operation::Join => Icon::OpJoin,
        Operation::Cut => Icon::OpCut,
        Operation::Intersect => Icon::OpIntersect,
    }
}

/// A read-only value well, styled like the numeric fields: the typed text
/// with a caret, or the live value.
fn value_well(t: Colors, text: String, typing: bool, invalid: bool, width: f32) -> Div {
    row()
        .min_w(px(width))
        .h(px(H_SM))
        .px(px(SP_2))
        .justify_end()
        .rounded(px(R_SM))
        .bg(rgb(if typing { t.sunken } else { t.field }))
        .border_1()
        .border_color(rgb(if invalid {
            t.danger
        } else if typing {
            t.focus
        } else {
            t.hairline
        }))
        .tabular()
        .text_size(px(TEXT_MD))
        .text_color(rgb(if invalid { t.danger } else { t.text }))
        .child(text)
        .when(typing, |d| {
            d.child(div().ml(px(1.)).w(px(1.5)).h(px(14.)).bg(rgb(t.focus)))
        })
}

fn title(t: Colors, glyph: Icon, label: &'static str) -> Div {
    row()
        .h(px(H_MD))
        .pl(px(SP_2))
        .pr(px(SP_1))
        .gap(px(SP_1_5))
        .child(icon(glyph, ICON_MD, t.accent_text))
        .child(
            div()
                .font_weight(WEIGHT_SEMIBOLD)
                .text_color(rgb(t.text))
                .child(label),
        )
}

fn done_cancel(t: Colors, compact: bool, cx: &mut Context<Studio>) -> Div {
    let cancel = command_icon(
        t,
        "cad-tool-cancel",
        Icon::Close,
        Command::CadTool(ToolCommand::Cancel),
        false,
        H_MD,
        cx,
    );
    let done = if compact {
        on_command(
            button_base(t, "cad-tool-done", Tone::Primary, H_MD)
                .w(px(H_MD))
                .child(icon(Icon::Check, ICON_MD, t.on_accent))
                .tooltip(command_tip(t, Command::CadTool(ToolCommand::Done))),
            Command::CadTool(ToolCommand::Done),
            cx,
        )
    } else {
        command_button(
            t,
            "cad-tool-done",
            "Done",
            Some(Icon::Check),
            Command::CadTool(ToolCommand::Done),
            Tone::Primary,
            cx,
        )
        .h(px(H_MD))
        .px(px(SP_3))
    };
    row().gap(px(2.)).child(cancel).child(done)
}

impl Studio {
    /// The running tool's HUD for `ui::slots::viewport_dock`.
    pub(crate) fn cad_tool_hud(&self, t: Colors, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tool = self.cad_tool.tool.as_ref()?;
        let width = f32::from(self.bounds.get().size.width);
        let compact = width < 760.;
        let bar = hud(t)
            .id("cad-tool-hud")
            .items_center()
            .on_mouse_move(cx.listener(|studio, event: &MouseMoveEvent, _, cx| {
                if studio.navigation.is_none() && event.pressed_button.is_none() {
                    cx.stop_propagation();
                    studio.cad_tool_pointer_in_hud(cx);
                }
            }));
        let error;
        let bar = match tool {
            Tool::Sketch(tool) => {
                error = tool
                    .error
                    .clone()
                    .or_else(|| tool.revolve_problem().map(str::to_owned))
                    .or_else(|| self.preview_issue());
                let operation = tool.operation();
                let mut ops = segmented(t);
                for candidate in Operation::ALL {
                    let available = candidate == Operation::NewPart || tool.target.is_some();
                    let active = operation == candidate;
                    let segment_el = segment(
                        t,
                        SharedString::from(format!("cad-op-{}", candidate.label())),
                        active,
                        H_SM,
                    )
                    .child(icon(
                        operation_icon(candidate),
                        ICON_SM,
                        if active { t.text } else { t.text_muted },
                    ))
                    .when(!compact, |d| d.child(candidate.label()))
                    .tooltip(command_tip(
                        t,
                        Command::CadTool(ToolCommand::SetOperation(candidate)),
                    ));
                    ops = ops.child(if available {
                        on_command(
                            segment_el,
                            Command::CadTool(ToolCommand::SetOperation(candidate)),
                            cx,
                        )
                        .into_any_element()
                    } else {
                        disabled(segment_el).into_any_element()
                    });
                }
                let typing = !tool.numeric.is_empty();
                let invalid = tool.typed().is_err();
                let value_text = if typing {
                    tool.numeric.clone()
                } else {
                    match tool.kind {
                        SketchToolKind::Extrude => crate::sketch::overlay::format_length(tool.distance),
                        SketchToolKind::Revolve => crate::sketch::overlay::format_angle(tool.angle),
                    }
                };
                let mut bar = bar
                    .child(title(
                        t,
                        match tool.kind {
                            SketchToolKind::Extrude => Icon::Extrude,
                            SketchToolKind::Revolve => Icon::Revolve,
                        },
                        tool.kind.label(),
                    ))
                    .child(vrule(t, 18.))
                    .child(ops)
                    .when(tool.inferred() && !compact, |d| {
                        d.child(
                            div()
                                .px(px(SP_1))
                                .text_size(px(TEXT_XS))
                                .text_color(rgb(t.text_faint))
                                .child("auto"),
                        )
                    })
                    .child(vrule(t, 18.));
                bar = match tool.kind {
                    SketchToolKind::Extrude => bar
                        .child(command_icon(
                            t,
                            "cad-symmetric",
                            Icon::Symmetric,
                            Command::CadTool(ToolCommand::ToggleSymmetric),
                            tool.symmetric,
                            H_MD,
                            cx,
                        ))
                        .child(command_icon(
                            t,
                            "cad-flip",
                            Icon::FlipDirection,
                            Command::CadTool(ToolCommand::Flip),
                            false,
                            H_MD,
                            cx,
                        )),
                    SketchToolKind::Revolve => {
                        let axis = tool.axis.map(|a| a.source);
                        let mut choice = |id: &'static str, label: &'static str, y: bool, active: bool| {
                            on_command(
                                segment(t, id, active, H_SM)
                                    .child(label)
                                    .tooltip(command_tip(
                                        t,
                                        Command::CadTool(ToolCommand::SketchAxis(y)),
                                    )),
                                Command::CadTool(ToolCommand::SketchAxis(y)),
                                cx,
                            )
                        };
                        let line_active = matches!(axis, Some(super::sketch_tool::AxisSource::Line(_)));
                        bar.child(
                            row()
                                .gap(px(SP_1_5))
                                .child(
                                    div()
                                        .pl(px(SP_1))
                                        .text_size(px(TEXT_SM))
                                        .text_color(rgb(t.text_muted))
                                        .child("Axis"),
                                )
                                .child(
                                    segmented(t)
                                        .child(
                                            segment(t, "cad-axis-line", line_active, H_SM)
                                                .child(icon(
                                                    Icon::DrawLine,
                                                    ICON_SM,
                                                    if line_active { t.text } else { t.text_muted },
                                                ))
                                                .tooltip(tip_detail(
                                                    t,
                                                    "Sketch line",
                                                    "Click a straight line of the sketch",
                                                    None,
                                                )),
                                        )
                                        .child(choice(
                                            "cad-axis-x",
                                            "X",
                                            false,
                                            axis == Some(super::sketch_tool::AxisSource::SketchX),
                                        ))
                                        .child(choice(
                                            "cad-axis-y",
                                            "Y",
                                            true,
                                            axis == Some(super::sketch_tool::AxisSource::SketchY),
                                        )),
                                ),
                        )
                        .child(command_icon(
                            t,
                            "cad-flip",
                            Icon::FlipDirection,
                            Command::CadTool(ToolCommand::Flip),
                            false,
                            H_MD,
                            cx,
                        ))
                    }
                };
                bar.child(vrule(t, 18.))
                    .child(value_well(t, value_text, typing, invalid, 84.))
                    .child(vrule(t, 18.))
                    .child(done_cancel(t, compact, cx))
            }
            Tool::PushPull(tool) => {
                error = tool.active.as_ref().and_then(|a| a.error.clone());
                let (detail, value) = match &tool.active {
                    Some(active) => {
                        let face = &active.faces[0];
                        let kind = if face.is_part() { "Part face" } else { "Mesh face" };
                        let detail = if active.faces.len() > 1 {
                            format!("{} faces", active.faces.len())
                        } else {
                            format!("{kind} · {}", face.name)
                        };
                        let typing = !active.numeric.is_empty();
                        let text = if typing {
                            active.numeric.clone()
                        } else {
                            crate::sketch::overlay::format_length(active.distance)
                        };
                        (
                            detail,
                            Some((text, typing, active.typed().is_err(), active.modal)),
                        )
                    }
                    None => (
                        match &tool.hover {
                            Some(face) if !face.planar => "Curved faces can't be pushed".to_owned(),
                            Some(face) => format!(
                                "{} · {}",
                                if face.is_part() { "Part face" } else { "Mesh face" },
                                face.name
                            ),
                            None => "Point at a flat face".to_owned(),
                        },
                        None,
                    ),
                };
                let mut bar = bar
                    .child(title(t, Icon::PushPull, "Push/Pull"))
                    .child(vrule(t, 18.))
                    .child(
                        div()
                            .px(px(SP_2))
                            .max_w(px(220.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(TEXT_SM))
                            .text_color(rgb(t.text_muted))
                            .child(detail),
                    );
                match value {
                    Some((text, typing, invalid, modal)) => {
                        bar = bar
                            .child(vrule(t, 18.))
                            .child(command_icon(
                                t,
                                "cad-flip",
                                Icon::FlipDirection,
                                Command::CadTool(ToolCommand::Flip),
                                false,
                                H_MD,
                                cx,
                            ))
                            .child(value_well(t, text, typing, invalid, 84.))
                            .child(vrule(t, 18.))
                            .when(!modal && !compact, |d| {
                                d.child(div().px(px(SP_1)).child(hint(t, "Drag", "Apply on release")))
                            })
                            .child(done_cancel(t, compact, cx));
                    }
                    None => {
                        bar = bar.child(vrule(t, 18.)).child(command_icon(
                            t,
                            "cad-tool-cancel",
                            Icon::Close,
                            Command::CadTool(ToolCommand::Cancel),
                            false,
                            H_MD,
                            cx,
                        ));
                    }
                }
                bar
            }
            Tool::Chamfer(tool) => {
                error = tool.error.clone().or_else(|| self.preview_issue());
                let typing = !tool.numeric.is_empty();
                let text = if typing {
                    tool.numeric.clone()
                } else {
                    crate::sketch::overlay::format_length(tool.distance)
                };
                bar.child(title(t, Icon::Bevel, "Chamfer"))
                    .child(vrule(t, 18.))
                    .child(badge(
                        t,
                        format!(
                            "{} edge{}",
                            tool.edges.len(),
                            if tool.edges.len() == 1 { "" } else { "s" }
                        ),
                        BadgeTone::Accent,
                    ))
                    .child(div().w(px(SP_1)))
                    .child(value_well(t, text, typing, tool.typed().is_err(), 84.))
                    .child(vrule(t, 18.))
                    .child(done_cancel(t, compact, cx))
            }
        };
        let card = col()
            .items_center()
            .gap(px(SP_1_5))
            .child(bar)
            .when_some(error, |d, error| {
                d.child(
                    hud(t)
                        .px(px(SP_2))
                        .gap(px(SP_1_5))
                        .text_size(px(TEXT_SM))
                        .text_color(rgb(t.danger))
                        .child(icon(Icon::Error, ICON_SM, t.danger))
                        .child(error),
                )
            });
        Some(card.into_any_element())
    }

    /// The kernel's message for the previewed feature, if it failed.
    fn preview_issue(&self) -> Option<String> {
        let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_ref() else {
            return None;
        };
        let preview = tool.session.preview.as_ref()?;
        let part = self.scene.part(preview.part)?;
        let issue = part
            .issues()
            .iter()
            .find(|i| i.feature == preview.feature || i.feature == 0)?;
        Some(if issue.suggestion.is_empty() {
            issue.message.clone()
        } else {
            format!("{} · {}", issue.message, issue.suggestion)
        })
    }

    /// The bottom-left panel adjusting the last committed operation.
    pub(crate) fn adjust_panel(&self, t: Colors, cx: &mut Context<Self>) -> Option<AnyElement> {
        let last = self.adjustable_operation()?;
        let glyph = match last.label() {
            "Revolve" => Icon::Revolve,
            "Push/Pull" => Icon::PushPull,
            "Chamfer" => Icon::Bevel,
            "Cut" => Icon::OpCut,
            "Intersect" => Icon::OpIntersect,
            _ => Icon::Extrude,
        };
        let param = last.param();
        let (label, unit) = match param {
            AdjustParam::Distance => ("Distance", "m"),
            AdjustParam::Angle => ("Angle", "°"),
        };
        let mut panel = hud(t)
            .id("adjust-last-operation")
            .flex_col()
            .items_stretch()
            .w(px(252.))
            .px(px(SP_2 + 2.))
            .pt(px(SP_1_5))
            .pb(px(SP_2))
            .gap(px(2.))
            .child(
                row()
                    .h(px(H_MD))
                    .gap(px(SP_1_5))
                    .child(icon(glyph, ICON_SM, t.accent_text))
                    .child(
                        div()
                            .font_weight(WEIGHT_SEMIBOLD)
                            .text_color(rgb(t.text))
                            .child(last.label()),
                    )
                    .child(spacer())
                    .child(caption(t, "Adjust").text_color(rgb(t.text_faint))),
            )
            .child(numeric_row(
                t,
                self,
                "adjust-value",
                label,
                unit,
                Field::CadAdjust(param),
                cx,
            ));
        if let Some(operation) = last.operation()
            && last.part.is_some_and(|_| {
                self.scene
                    .object_part(last.object)
                    .is_some_and(|p| p.features.first().map(|f| f.id) != Some(last.feature))
            })
        {
            let mut ops = segmented(t);
            for candidate in [Operation::Join, Operation::Cut, Operation::Intersect] {
                let active = operation == candidate;
                ops = ops.child(on_command(
                    segment(
                        t,
                        SharedString::from(format!("adjust-op-{}", candidate.label())),
                        active,
                        H_XS,
                    )
                    .flex_1()
                    .text_size(px(TEXT_SM))
                    .child(icon(
                        operation_icon(candidate),
                        ICON_SM,
                        if active { t.text } else { t.text_muted },
                    ))
                    .child(candidate.label()),
                    Command::CadTool(ToolCommand::AdjustOperation(candidate)),
                    cx,
                ));
            }
            panel = panel.child(property_row(t, "Operation", ops.flex_1()));
        }
        if let Some(symmetric) = last.symmetric() {
            panel = panel.child(switch_row(
                t,
                "adjust-symmetric",
                "Symmetric",
                None,
                symmetric,
                Command::CadTool(ToolCommand::AdjustSymmetric),
                cx,
            ));
        }
        if last.edge_count() == 0 {
            panel = panel.child(
                row().pt(px(SP_1)).child(spacer()).child(command_button(
                    t,
                    "adjust-flip",
                    "Flip",
                    Some(Icon::FlipDirection),
                    Command::CadTool(ToolCommand::AdjustFlip),
                    Tone::Ghost,
                    cx,
                )),
            );
        }
        Some(panel.into_any_element())
    }

    /// Status bar keycap hints for the tools and part editing.
    pub(crate) fn cad_tool_hints(&self) -> Option<Vec<(&'static str, &'static str)>> {
        let cmd_b = crate::platform_shortcut("⌘ B", "Ctrl B");
        match self.cad_tool.tool.as_ref() {
            Some(Tool::Sketch(tool)) => Some(match tool.kind {
                SketchToolKind::Extrude => vec![
                    ("Drag", "Arrow or region"),
                    ("⇧ Click", "Add region"),
                    ("0–9", "Distance"),
                    ("Tab", "Operation"),
                    ("S", "Symmetric"),
                    ("F", "Flip"),
                    ("R", "Revolve"),
                    ("Return", "Apply"),
                    ("Ctrl", "Free"),
                ],
                SketchToolKind::Revolve => vec![
                    ("Click", "Axis line"),
                    ("X Y", "Sketch axis"),
                    ("Drag", "Angle"),
                    ("0–9", "Angle"),
                    ("Tab", "Operation"),
                    ("E", "Extrude"),
                    ("Return", "Apply"),
                    ("Ctrl", "Free"),
                ],
            }),
            Some(Tool::PushPull(tool)) => Some(match &tool.active {
                Some(active) if active.modal => vec![
                    ("Move", "Distance"),
                    ("0–9", "Exact value"),
                    ("Click", "Apply"),
                    ("Esc", "Cancel"),
                    ("Ctrl", "Free"),
                ],
                Some(_) => vec![
                    ("Drag", "Arrow"),
                    ("0–9", "Distance"),
                    ("Return", "Apply"),
                    ("F", "Flip"),
                    ("Esc", "Deselect"),
                ],
                None => vec![
                    ("Drag", "Push or pull a face"),
                    ("Click", "Select face"),
                    ("Ctrl", "Free"),
                    ("P", "Done"),
                ],
            }),
            Some(Tool::Chamfer(_)) => Some(vec![
                ("Move", "Distance"),
                ("0–9", "Exact value"),
                ("Click", "Apply"),
                ("Esc", "Cancel"),
                ("Ctrl", "Free"),
            ]),
            None if self.part_editing().is_some() => Some(vec![
                ("2 3", "Edge · Face"),
                ("Click", "Select"),
                ("⇧ Click", "Add"),
                ("P", "Push/Pull"),
                (cmd_b, "Chamfer"),
                ("D", "Sketch on face"),
                ("Tab", "Object"),
            ]),
            None if self.cad_tool.region_hover.is_some() => Some(vec![
                ("E", "Extrude region"),
                ("⇧ R", "Revolve"),
                ("D", "Draw"),
                ("Click", "Select sketch"),
            ]),
            None => None,
        }
    }

    /// Status bar selection readout for part editing.
    pub(crate) fn cad_tool_summary(&self) -> Option<String> {
        if self.cad_tool.active() {
            return None;
        }
        self.part_edit_summary()
    }
}
