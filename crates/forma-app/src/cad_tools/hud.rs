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
        Operation::NewPart => Icon::NewPart,
        Operation::Join => Icon::Join,
        Operation::Cut => Icon::Subtract,
        Operation::Intersect => Icon::Intersect,
    }
}

/// The live value, styled like the inspector's numeric fields: the typed
/// text with a caret, or the live value. Its resting outline says it takes
/// typing (digits go straight to it while the tool runs).
fn value_well(t: Colors, text: String, typing: bool, invalid: bool, width: f32) -> Div {
    row()
        .min_w(px(width))
        .h(px(H_MD))
        .px(px(SP_2 + 2.))
        .justify_end()
        .rounded(px(R_SM + 1.))
        .bg(rgb(if typing { t.sunken } else { t.field }))
        .border_1()
        .border_color(rgb(if invalid {
            t.danger
        } else if typing {
            t.focus
        } else {
            t.border_strong
        }))
        .tabular()
        .text_size(px(TEXT_MD))
        .font_weight(WEIGHT_MEDIUM)
        .text_color(rgb(if invalid { t.danger } else { t.text }))
        .child(text)
        .when(typing, |d| {
            d.child(div().ml(px(1.)).w(px(1.5)).h(px(14.)).bg(rgb(t.focus)))
        })
}

/// Quiet text in the HUD naming what the tool acts on.
fn hud_detail(t: Colors, text: impl Into<SharedString>) -> Div {
    div()
        .px(px(SP_1))
        .max_w(px(220.))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_size(px(TEXT_SM))
        .text_color(rgb(t.text_muted))
        .child(text.into())
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The operation panel's title: what is running and what it acts on.
fn panel_header(t: Colors, glyph: Icon, name: &'static str, subject: String) -> Div {
    row()
        .gap(px(SP_2 + 2.))
        .pb(px(SP_2))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(32.))
                .flex_shrink_0()
                .rounded(px(R_MD))
                .bg(rgb(t.accent_soft))
                .child(icon(glyph, ICON_LG, t.accent_text)),
        )
        .child(
            col()
                .flex_1()
                .min_w(px(0.))
                .gap(px(1.))
                .child(
                    div()
                        .text_size(px(TEXT_XL))
                        .font_weight(WEIGHT_SEMIBOLD)
                        .text_color(rgb(t.text))
                        .child(name),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(px(TEXT_SM))
                        .text_color(rgb(t.text_muted))
                        .child(subject),
                ),
        )
}

/// A labelled row of the operation panel.
fn panel_row(t: Colors, label: &'static str, unit: &'static str, value: impl IntoElement) -> Div {
    row()
        .min_h(px(ROW_H))
        .gap(px(SP_2))
        .child(
            div()
                .w(px(LABEL_W))
                .flex_shrink_0()
                .text_color(rgb(t.text_muted))
                .child(unit_label(t, label, unit)),
        )
        .child(row().flex_1().min_w(px(0.)).gap(px(SP_1)).child(value))
}

/// A label above a full-width choice, with an optional note on the right.
fn stacked_row(
    t: Colors,
    label: &'static str,
    note: Option<&'static str>,
    control: impl IntoElement,
) -> Div {
    col()
        .gap(px(SP_1))
        .pt(px(SP_1))
        .pb(px(SP_1))
        .child(
            row()
                .child(div().flex_1().text_color(rgb(t.text_muted)).child(label))
                .when_some(note, |d, note| {
                    d.child(caption(t, note).text_color(rgb(t.text_faint)))
                }),
        )
        .child(control)
}

/// What the operation acts on, as a tinted chip ("2 edges").
fn selection_chip(t: Colors, glyph: Icon, text: String) -> Div {
    row()
        .h(px(H_SM))
        .px(px(SP_2))
        .gap(px(SP_1_5))
        .flex_shrink_0()
        .rounded(px(R_SM))
        .bg(rgb(t.accent_soft))
        .text_size(px(TEXT_SM))
        .font_weight(WEIGHT_MEDIUM)
        .text_color(rgb(t.accent_text))
        .child(icon(glyph, 13., t.accent_text))
        .child(text)
}

/// The live value in the operation panel. Typing goes to the tool from
/// anywhere, so the well shows the typed text and caret without a click.
fn live_value(t: Colors, text: String, typing: bool, invalid: bool) -> gpui::Stateful<Div> {
    value_well(t, text, typing, invalid, 0.)
        .h(px(H_SM))
        .flex_1()
        .id("cad-panel-value")
        .tooltip(tip_detail(
            t,
            "Live value",
            "Type a number or expression such as 25mm or (2+3)*10cm · Return applies",
            None,
        ))
}

fn title(t: Colors, glyph: Icon, label: &'static str) -> Div {
    row()
        .h(px(H_MD))
        .pl(px(SP_2))
        .pr(px(SP_1_5))
        .gap(px(SP_2))
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
            .p(px(SP_1))
            .gap(px(SP_1_5))
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
                        SketchToolKind::Extrude => {
                            crate::sketch::overlay::format_length(tool.distance)
                        }
                        SketchToolKind::Revolve => super::format_sweep(tool.angle),
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
                                .text_size(px(TEXT_SM))
                                .text_color(rgb(t.text_faint))
                                .child("Auto"),
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
                        let mut choice =
                            |id: &'static str, label: &'static str, y: bool, active: bool| {
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
                        let line_active =
                            matches!(axis, Some(super::sketch_tool::AxisSource::Line(_)));
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
                    .child(value_well(t, value_text, typing, invalid, 96.))
                    .child(vrule(t, 18.))
                    .child(done_cancel(t, compact, cx))
            }
            Tool::PushPull(tool) => {
                error = tool.active.as_ref().and_then(|a| a.error.clone());
                let (detail, value) = match &tool.active {
                    Some(active) => {
                        let face = &active.faces[0];
                        let kind = if face.is_part() {
                            "Part face"
                        } else {
                            "Mesh face"
                        };
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
                                if face.is_part() {
                                    "Part face"
                                } else {
                                    "Mesh face"
                                },
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
                    .child(hud_detail(t, detail));
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
                            .child(value_well(t, text, typing, invalid, 96.))
                            .child(vrule(t, 18.))
                            .when(!modal && !compact, |d| {
                                d.child(div().px(px(SP_1)).child(hint(
                                    t,
                                    "Drag",
                                    "Apply on release",
                                )))
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
                    .child(hud_detail(t, plural(tool.edges.len(), "edge", "edges")))
                    .child(value_well(t, text, typing, tool.typed().is_err(), 96.))
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

    /// The running tool as the inspector's top panel: what it does, what it
    /// acts on, the live value and the options the tool really has. It
    /// mirrors the HUD pill, which stays beside the work in the viewport.
    /// `narrow` drops segment labels a narrow sidebar cannot fit.
    pub(crate) fn cad_tool_panel(
        &self,
        t: Colors,
        narrow: bool,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let tool = self.cad_tool.tool.as_ref()?;
        let mut rows = col();
        let (glyph, name, subject, error, can_apply);
        match tool {
            Tool::Chamfer(tool) => {
                glyph = Icon::Bevel;
                name = "Chamfer";
                subject = "Equal distance on both faces".to_owned();
                error = tool.error.clone();
                can_apply = true;
                let typing = !tool.numeric.is_empty();
                let text = if typing {
                    tool.numeric.clone()
                } else {
                    crate::sketch::overlay::format_length(tool.distance)
                };
                rows = rows
                    .child(panel_row(
                        t,
                        "Edges",
                        "",
                        selection_chip(t, Icon::Edge, plural(tool.edges.len(), "edge", "edges")),
                    ))
                    .child(panel_row(
                        t,
                        "Distance",
                        "",
                        live_value(t, text, typing, tool.typed().is_err()),
                    ));
            }
            Tool::PushPull(tool) => {
                glyph = Icon::PushPull;
                name = "Push/Pull";
                error = tool.active.as_ref().and_then(|a| a.error.clone());
                match &tool.active {
                    Some(active) => {
                        let face = &active.faces[0];
                        subject = format!(
                            "{} face{} along {} normal",
                            if face.is_part() { "Part" } else { "Mesh" },
                            if active.faces.len() == 1 { "" } else { "s" },
                            if active.faces.len() == 1 {
                                "its"
                            } else {
                                "their"
                            },
                        );
                        can_apply = true;
                        let typing = !active.numeric.is_empty();
                        let text = if typing {
                            active.numeric.clone()
                        } else {
                            crate::sketch::overlay::format_length(active.distance)
                        };
                        rows = rows
                            .child(panel_row(
                                t,
                                "Faces",
                                "",
                                selection_chip(
                                    t,
                                    Icon::Face,
                                    plural(active.faces.len(), "face", "faces"),
                                ),
                            ))
                            .child(panel_row(
                                t,
                                "Distance",
                                "",
                                live_value(t, text, typing, active.typed().is_err()),
                            ))
                            .child(panel_row(
                                t,
                                "Direction",
                                "",
                                command_button(
                                    t,
                                    "cad-panel-flip",
                                    "Flip",
                                    Some(Icon::FlipDirection),
                                    Command::CadTool(ToolCommand::Flip),
                                    Tone::Secondary,
                                    cx,
                                )
                                .flex_1(),
                            ));
                    }
                    None => {
                        subject = "Pick a face".to_owned();
                        can_apply = false;
                        let hint = match &tool.hover {
                            Some(face) if !face.planar => "Curved faces can't be pushed".to_owned(),
                            Some(face) => format!("{} · drag to push or pull", face.name),
                            None => "Point at a flat face of a part or mesh, then drag".to_owned(),
                        };
                        rows = rows.child(
                            div()
                                .py(px(SP_1))
                                .text_size(px(TEXT_SM))
                                .line_height(px(16.))
                                .text_color(rgb(t.text_muted))
                                .child(hint),
                        );
                    }
                }
            }
            Tool::Sketch(tool) => {
                glyph = match tool.kind {
                    SketchToolKind::Extrude => Icon::Extrude,
                    SketchToolKind::Revolve => Icon::Revolve,
                };
                name = tool.kind.label();
                subject = match &tool.target {
                    Some(target) => format!("Sketch on {}", target.name),
                    None => "Sketch regions into a new part".to_owned(),
                };
                error = tool
                    .error
                    .clone()
                    .or_else(|| tool.revolve_problem().map(str::to_owned))
                    .or_else(|| self.preview_issue());
                can_apply = !tool.selected.is_empty();
                let typing = !tool.numeric.is_empty();
                let invalid = tool.typed().is_err();
                let regions = tool.selected.len();
                rows = rows.child(panel_row(
                    t,
                    "Profile",
                    "",
                    if regions == 0 {
                        caption(t, "Click a region").text_color(rgb(t.text_faint))
                    } else {
                        selection_chip(t, Icon::Sketch, plural(regions, "region", "regions"))
                    },
                ));
                let operation = tool.operation();
                let mut ops = segmented(t).w_full();
                for candidate in Operation::ALL {
                    let available = candidate == Operation::NewPart || tool.target.is_some();
                    let active = operation == candidate;
                    let command = Command::CadTool(ToolCommand::SetOperation(candidate));
                    let element = segment(
                        t,
                        SharedString::from(format!("cad-panel-op-{}", candidate.label())),
                        active,
                        22.,
                    )
                    .flex_1()
                    .min_w(px(0.))
                    .when(narrow, |d| {
                        d.child(icon(
                            operation_icon(candidate),
                            ICON_SM,
                            if active { t.text } else { t.text_muted },
                        ))
                    })
                    .when(!narrow, |d| d.child(candidate.label()))
                    .tooltip(command_tip(t, command));
                    ops = ops.child(if available {
                        on_command(element, command, cx).into_any_element()
                    } else {
                        disabled(element).into_any_element()
                    });
                }
                rows = rows.child(stacked_row(
                    t,
                    "Operation",
                    tool.inferred().then_some("Auto from direction"),
                    ops,
                ));
                match tool.kind {
                    SketchToolKind::Extrude => {
                        let extent = segmented(t).w_full().children(
                            [(false, "One side"), (true, "Symmetric")].into_iter().map(
                                |(symmetric, label)| {
                                    let active = tool.symmetric == symmetric;
                                    let element = segment(
                                        t,
                                        SharedString::from(format!("cad-panel-extent-{label}")),
                                        active,
                                        22.,
                                    )
                                    .flex_1()
                                    .min_w(px(0.))
                                    .child(label)
                                    .tooltip(command_tip(
                                        t,
                                        Command::CadTool(ToolCommand::ToggleSymmetric),
                                    ));
                                    if active {
                                        element
                                    } else {
                                        on_command(
                                            element,
                                            Command::CadTool(ToolCommand::ToggleSymmetric),
                                            cx,
                                        )
                                    }
                                },
                            ),
                        );
                        rows = rows.child(stacked_row(t, "Extent", None, extent));
                    }
                    SketchToolKind::Revolve => {
                        let axis = tool.axis.map(|a| a.source);
                        let line = matches!(axis, Some(super::sketch_tool::AxisSource::Line(_)));
                        let mut axes = segmented(t).w_full().child(
                            segment(t, "cad-panel-axis-line", line, 22.)
                                .flex_1()
                                .min_w(px(0.))
                                .child(icon(
                                    Icon::DrawLine,
                                    ICON_SM,
                                    if line { t.text } else { t.text_muted },
                                ))
                                .when(!narrow, |d| d.child("Line"))
                                .tooltip(tip_detail(
                                    t,
                                    "Sketch line",
                                    "Click a straight line of the sketch",
                                    None,
                                )),
                        );
                        for (y, label, source) in [
                            (false, "X", super::sketch_tool::AxisSource::SketchX),
                            (true, "Y", super::sketch_tool::AxisSource::SketchY),
                        ] {
                            let command = Command::CadTool(ToolCommand::SketchAxis(y));
                            axes = axes.child(on_command(
                                segment(
                                    t,
                                    SharedString::from(format!("cad-panel-axis-{label}")),
                                    axis == Some(source),
                                    22.,
                                )
                                .flex_1()
                                .min_w(px(0.))
                                .child(label)
                                .tooltip(command_tip(t, command)),
                                command,
                                cx,
                            ));
                        }
                        rows = rows.child(stacked_row(t, "Axis", None, axes));
                    }
                }
                let (label, text) = match tool.kind {
                    SketchToolKind::Extrude => (
                        "Distance",
                        crate::sketch::overlay::format_length(tool.distance),
                    ),
                    SketchToolKind::Revolve => ("Angle", super::format_sweep(tool.angle)),
                };
                rows = rows
                    .child(panel_row(
                        t,
                        label,
                        "",
                        live_value(
                            t,
                            if typing { tool.numeric.clone() } else { text },
                            typing,
                            invalid,
                        ),
                    ))
                    .child(panel_row(
                        t,
                        "Direction",
                        "",
                        command_button(
                            t,
                            "cad-panel-flip",
                            "Flip",
                            Some(Icon::FlipDirection),
                            Command::CadTool(ToolCommand::Flip),
                            Tone::Secondary,
                            cx,
                        )
                        .flex_1(),
                    ));
            }
        }
        let footer = row()
            .gap(px(SP_1))
            .pt(px(SP_2))
            .child(
                command_button(
                    t,
                    "cad-panel-cancel",
                    if can_apply { "Cancel" } else { "Close" },
                    None,
                    Command::CadTool(ToolCommand::Cancel),
                    Tone::Secondary,
                    cx,
                )
                .h(px(H_MD))
                .flex_1(),
            )
            .when(can_apply, |d| {
                d.child(
                    command_button(
                        t,
                        "cad-panel-apply",
                        "Apply",
                        Some(Icon::Check),
                        Command::CadTool(ToolCommand::Done),
                        Tone::Primary,
                        cx,
                    )
                    .h(px(H_MD))
                    .flex_1(),
                )
            });
        Some(
            col()
                .flex_shrink_0()
                .px(px(SP_3))
                .pt(px(SP_3))
                .pb(px(SP_3))
                .border_b_1()
                .border_color(rgb(t.hairline))
                .child(panel_header(t, glyph, name, subject))
                .child(rows.gap(px(2.)))
                .when_some(error, |d, error| {
                    d.child(
                        row()
                            .items_start()
                            .gap(px(SP_1_5))
                            .pt(px(SP_2))
                            .child(div().pt(px(1.)).child(icon(Icon::Error, ICON_SM, t.danger)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .text_size(px(TEXT_SM))
                                    .line_height(px(16.))
                                    .text_color(rgb(t.danger))
                                    .child(error),
                            ),
                    )
                })
                .child(footer),
        )
    }

    /// The kernel's message for the previewed feature, if it failed.
    fn preview_issue(&self) -> Option<String> {
        let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_ref() else {
            return None;
        };
        let preview = tool.preview.as_ref()?;
        let issue = preview
            .outcome
            .as_ref()?
            .issues
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
            "Cut" => Icon::Subtract,
            "Intersect" => Icon::Intersect,
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
            panel = panel.child(row().pt(px(SP_1)).child(spacer()).child(command_button(
                t,
                "adjust-flip",
                "Flip",
                Some(Icon::FlipDirection),
                Command::CadTool(ToolCommand::AdjustFlip),
                Tone::Ghost,
                cx,
            )));
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
            None if self.active_field.is_some() => None,
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

    /// Status bar selection readout for the tools and part editing.
    pub(crate) fn cad_tool_summary(&self) -> Option<String> {
        let plural =
            |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        match self.cad_tool.tool.as_ref() {
            Some(Tool::Sketch(tool)) => Some(format!(
                "{} picked",
                plural(tool.selected.len(), "region", "regions")
            )),
            Some(Tool::PushPull(tool)) => tool
                .active
                .as_ref()
                .map(|a| plural(a.faces.len(), "face", "faces")),
            Some(Tool::Chamfer(tool)) => Some(plural(tool.edges.len(), "edge", "edges")),
            None => self.part_edit_summary(),
        }
    }
}
