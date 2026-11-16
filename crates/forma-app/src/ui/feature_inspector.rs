//! The inspector's Part section: the part summary and actions when no
//! feature is selected, or the selected feature's parameters, built from the
//! same scrubbable fields and segmented controls as the rest of the panel.
//! Also the part-level actions shown for meshes and multi-object selections
//! (Convert to part, Combine).
use super::assets::{Icon, icon};
use super::commands::operation_icon;
use super::components::*;
use super::timeline::{ChipState, chip_hint, chip_state, feature_glyph, feature_icon};
use super::tokens::*;
use crate::app::{Command, Field, Studio};
use crate::cad::CadCommand;
use crate::theme::Colors;
use crate::timeline::{self, ExtentMode, FeatureField, TimelineCommand};
use forma_core::{BaseBody, Feature, FeatureKind, FeatureOperation, PartData, PrimitiveShape};
use gpui::{prelude::*, *};

/// A tinted callout for feature issues and cautions.
fn notice(t: Colors, glyph: Icon, ink: u32, title: String, detail: Option<String>) -> Div {
    row()
        .items_start()
        .gap(px(SP_2))
        .p(px(SP_2))
        .rounded(px(R_MD))
        .bg(tint(ink, 0.09))
        .border_1()
        .border_color(tint(ink, 0.22))
        .child(div().pt(px(1.)).child(icon(glyph, ICON_SM, ink)))
        .child(
            col()
                .flex_1()
                .min_w(px(0.))
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(TEXT_SM))
                        .line_height(px(16.))
                        .text_color(rgb(t.text))
                        .child(title),
                )
                .when_some(detail, |d, detail| {
                    d.child(
                        div()
                            .text_size(px(TEXT_SM))
                            .line_height(px(16.))
                            .text_color(rgb(t.text_muted))
                            .child(detail),
                    )
                }),
        )
}

/// A label above a full-width segmented control (operation, extent): the
/// choices need the whole row to stay legible in a narrow sidebar.
fn choice_row(t: Colors, label: &'static str, control: impl IntoElement) -> Div {
    col()
        .gap(px(SP_1))
        .pt(px(SP_1))
        .pb(px(SP_1))
        .child(div().text_color(rgb(t.text_muted)).child(label))
        .child(control)
}

fn thousands(value: usize) -> String {
    if value >= 10_000 {
        format!("{:.0}k", value as f64 / 1000.)
    } else if value >= 1000 {
        format!("{:.1}k", value as f64 / 1000.)
    } else {
        value.to_string()
    }
}

/// Operation choices for a feature kind.
fn operations(feature: &Feature) -> Option<(FeatureOperation, &'static [FeatureOperation])> {
    use FeatureOperation::*;
    const SOLID: &[FeatureOperation] = &[Join, Cut, Intersect, NewBody];
    const BOOLEAN: &[FeatureOperation] = &[Join, Cut, Intersect];
    match &feature.kind {
        FeatureKind::Primitive(f) => Some((f.operation, SOLID)),
        FeatureKind::Extrude(f) => Some((f.operation, SOLID)),
        FeatureKind::Revolve(f) => Some((f.operation, SOLID)),
        FeatureKind::BaseBody(f) => Some((f.operation, SOLID)),
        FeatureKind::Combine(f) => Some((f.operation, BOOLEAN)),
        FeatureKind::PushPull(_) | FeatureKind::Chamfer(_) => None,
    }
}

fn operation_control(
    t: Colors,
    feature: &Feature,
    icons_only: bool,
    cx: &mut Context<Studio>,
) -> Option<Div> {
    let (current, choices) = operations(feature)?;
    let control = segmented(t)
        .w_full()
        .children(choices.iter().map(|&operation| {
            let active = operation == current;
            let command = Command::Timeline(TimelineCommand::SetOperation(operation));
            let label = match operation {
                FeatureOperation::NewBody => "New",
                other => other.label(),
            };
            on_command(
                segment(
                    t,
                    SharedString::from(format!("feature-operation-{operation:?}")),
                    active,
                    22.,
                )
                .flex_1()
                .min_w(px(0.))
                // Four labelled segments only fit as text; a narrow sidebar
                // keeps the icons, named by their tooltips.
                .when(icons_only, |d| {
                    d.child(icon(
                        operation_icon(operation),
                        ICON_SM,
                        if active { t.text } else { t.text_muted },
                    ))
                })
                .when(!icons_only, |d| d.child(label))
                .tooltip(command_tip(t, command)),
                command,
                cx,
            )
        }));
    Some(choice_row(t, "Operation", control))
}

/// Rows for the selected feature's values.
fn parameters(
    t: Colors,
    s: &Studio,
    feature: &Feature,
    stacked: bool,
    cx: &mut Context<Studio>,
) -> Div {
    let field = Field::Feature;
    let mut rows = col();
    match &feature.kind {
        FeatureKind::Primitive(primitive) => {
            rows = match primitive.shape {
                PrimitiveShape::Box { .. } => rows.child(vector_row(
                    t,
                    s,
                    "Size",
                    "m",
                    "feature-size",
                    |axis| field(FeatureField::Size(axis)),
                    ["Size X", "Size Y", "Size Z"],
                    stacked,
                    cx,
                )),
                PrimitiveShape::Cylinder { .. } => rows
                    .child(numeric_row(
                        t,
                        s,
                        "feature-radius",
                        "Radius",
                        "m",
                        field(FeatureField::Radius),
                        cx,
                    ))
                    .child(numeric_row(
                        t,
                        s,
                        "feature-height",
                        "Height",
                        "m",
                        field(FeatureField::Height),
                        cx,
                    )),
                PrimitiveShape::Sphere { .. } => rows.child(numeric_row(
                    t,
                    s,
                    "feature-radius",
                    "Radius",
                    "m",
                    field(FeatureField::Radius),
                    cx,
                )),
                PrimitiveShape::Cone { .. } => rows
                    .child(numeric_row(
                        t,
                        s,
                        "feature-radius",
                        "Base radius",
                        "m",
                        field(FeatureField::Radius),
                        cx,
                    ))
                    .child(numeric_row(
                        t,
                        s,
                        "feature-top-radius",
                        "Top radius",
                        "m",
                        field(FeatureField::TopRadius),
                        cx,
                    ))
                    .child(numeric_row(
                        t,
                        s,
                        "feature-height",
                        "Height",
                        "m",
                        field(FeatureField::Height),
                        cx,
                    )),
                PrimitiveShape::Torus { .. } => rows
                    .child(numeric_row(
                        t,
                        s,
                        "feature-major",
                        "Ring radius",
                        "m",
                        field(FeatureField::MajorRadius),
                        cx,
                    ))
                    .child(numeric_row(
                        t,
                        s,
                        "feature-minor",
                        "Tube radius",
                        "m",
                        field(FeatureField::MinorRadius),
                        cx,
                    )),
            };
            rows = rows
                .child(vector_row(
                    t,
                    s,
                    "Position",
                    "m",
                    "feature-origin",
                    |axis| field(FeatureField::Origin(axis)),
                    [
                        "Position X · part space",
                        "Position Y · part space",
                        "Position Z · part space",
                    ],
                    stacked,
                    cx,
                ))
                .child(vector_row(
                    t,
                    s,
                    "Rotation",
                    "°",
                    "feature-rotation",
                    |axis| field(FeatureField::Rotation(axis)),
                    ["Rotation X", "Rotation Y", "Rotation Z"],
                    stacked,
                    cx,
                ));
        }
        FeatureKind::Extrude(extrude) => {
            rows = rows.child(numeric_row(
                t,
                s,
                "feature-distance",
                "Distance",
                "m",
                field(FeatureField::Distance),
                cx,
            ));
            let mode = ExtentMode::of(extrude.extent);
            let extent = segmented(t)
                .w_full()
                .children(ExtentMode::ALL.into_iter().map(|choice| {
                    let command = Command::Timeline(TimelineCommand::SetExtent(choice));
                    on_command(
                        segment(
                            t,
                            SharedString::from(format!("feature-extent-{choice:?}")),
                            choice == mode,
                            22.,
                        )
                        .flex_1()
                        .min_w(px(0.))
                        .child(choice.label())
                        .tooltip(command_tip(t, command)),
                        command,
                        cx,
                    )
                }));
            rows = rows.child(choice_row(t, "Extent", extent));
            if mode == ExtentMode::TwoSides {
                rows = rows.child(numeric_row(
                    t,
                    s,
                    "feature-back-distance",
                    "Back",
                    "m",
                    field(FeatureField::BackDistance),
                    cx,
                ));
            }
            rows = rows.child(switch_row(
                t,
                "feature-flip",
                "Flip direction",
                Some("Extrude to the other side of the sketch"),
                extrude.flip,
                Command::Timeline(TimelineCommand::ToggleFlip),
                cx,
            ));
        }
        FeatureKind::Revolve(_) => {
            rows = rows.child(numeric_row(
                t,
                s,
                "feature-angle",
                "Angle",
                "°",
                field(FeatureField::Angle),
                cx,
            ));
        }
        FeatureKind::PushPull(push) => {
            rows = rows
                .child(numeric_row(
                    t,
                    s,
                    "feature-distance",
                    "Distance",
                    "m",
                    field(FeatureField::Distance),
                    cx,
                ))
                .child(
                    caption(
                        t,
                        typeset_minus(&format!(
                            "Face at {}, {}, {} · positive pulls out",
                            short(push.face.point.x),
                            short(push.face.point.y),
                            short(push.face.point.z)
                        )),
                    )
                    .pt(px(SP_1)),
                );
        }
        FeatureKind::Chamfer(chamfer) => {
            rows = rows
                .child(numeric_row(
                    t,
                    s,
                    "feature-distance",
                    "Distance",
                    "m",
                    field(FeatureField::Distance),
                    cx,
                ))
                .child(readonly_row(
                    t,
                    "Edges",
                    "",
                    chamfer.edges.len().to_string(),
                    "Straight edges between planar faces, picked in the viewport",
                ));
        }
        FeatureKind::Combine(combine) => {
            rows = rows.children(combine.tools.iter().enumerate().map(|(index, tool)| {
                property_row(
                    t,
                    if index == 0 { "Tools" } else { "" },
                    row()
                        .flex_1()
                        .min_w(px(0.))
                        .h(px(H_SM))
                        .px(px(SP_2))
                        .gap(px(SP_1_5))
                        .child(icon(Icon::Part, ICON_SM, t.text_faint))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .child(tool.name.clone()),
                        )
                        .child(
                            caption(
                                t,
                                format!(
                                    "{} feature{}",
                                    tool.features.len(),
                                    if tool.features.len() == 1 { "" } else { "s" }
                                ),
                            )
                            .text_color(rgb(t.text_faint)),
                        ),
                )
            }));
        }
        FeatureKind::BaseBody(base) => {
            let (label, value, reason) = match &base.body {
                BaseBody::Mesh { mesh } => (
                    "Polygons",
                    mesh.faces.len().to_string(),
                    "A converted mesh: planar polygons became B-rep faces",
                ),
                BaseBody::Step { text } => (
                    "STEP data",
                    format!("{:.1} MB", text.len() as f64 / 1.0e6),
                    "An imported STEP body, kept exactly as imported",
                ),
            };
            rows = rows.child(readonly_row(t, label, "", value, reason));
        }
    }
    if let Some(control) = operation_control(t, feature, stacked, cx) {
        rows = rows.child(control);
    }
    rows
}

fn short(value: f32) -> String {
    compact_number(crate::transform::format_precision_value(value), 6)
}

/// The sketch a sketch-based feature follows, with the way back into Draw.
fn sketch_link(t: Colors, s: &Studio, feature: &Feature, cx: &mut Context<Studio>) -> Option<Div> {
    let sketch = timeline::feature_sketch(feature)?;
    let source = feature
        .source
        .filter(|id| s.scene.object_sketch(*id).is_some())
        .and_then(|id| s.scene.object(id));
    let picks = match &feature.kind {
        FeatureKind::Extrude(extrude) => extrude.profiles.len(),
        FeatureKind::Revolve(revolve) => revolve.profiles.len(),
        _ => 0,
    };
    let regions = if picks == 0 {
        "Outer regions".to_owned()
    } else {
        format!("{picks} region{}", if picks == 1 { "" } else { "s" })
    };
    let (name, detail) = match source {
        Some(object) => (
            object.name.clone(),
            if object.visible {
                "Linked · edits to the sketch update this feature"
            } else {
                "Linked hidden sketch · edits update this feature"
            },
        ),
        None => (
            // Beside the "Sketch" label: an embedded sketch.
            "Embedded".to_owned(),
            "Stored in this feature · Edit sketch opens it in Draw as a linked sketch",
        ),
    };
    // Sidebar minus the section insets, label column, row padding, icon and
    // the curve count: GPUI cannot ellipsize this nested text by itself.
    let room =
        s.panels.laid_out_sidebar() - 2. * SP_3 - LABEL_W - 3. * SP_2 - ICON_SM - 2. * SP_1_5 - 46.;
    let name = fit_text(cx, name.into(), TEXT_MD, room.max(24.));
    Some(
        col()
            .gap(px(SP_1))
            .pt(px(SP_1))
            .child(property_row(
                t,
                "Sketch",
                row()
                    .id("feature-sketch-link")
                    .flex_1()
                    .min_w(px(0.))
                    .h(px(H_SM))
                    .px(px(SP_2))
                    .gap(px(SP_1_5))
                    .child(icon(
                        Icon::Sketch,
                        ICON_SM,
                        if source.is_some() {
                            t.accent_text
                        } else {
                            t.text_faint
                        },
                    ))
                    .child(div().flex_1().min_w(px(0.)).truncate().child(name))
                    .child(
                        caption(
                            t,
                            format!(
                                "{} curve{}",
                                sketch.entities.len(),
                                if sketch.entities.len() == 1 { "" } else { "s" }
                            ),
                        )
                        .tabular()
                        .text_color(rgb(t.text_faint)),
                    )
                    .tooltip(tip_detail(t, regions.clone(), detail, None)),
            ))
            .child(property_row(
                t,
                "",
                command_button(
                    t,
                    "feature-edit-sketch",
                    "Edit sketch",
                    Some(Icon::DrawMode),
                    Command::Timeline(TimelineCommand::EditSketch),
                    Tone::Secondary,
                    cx,
                )
                .flex_1()
                .min_w(px(0.)),
            )),
    )
}

/// The selected feature: name, state, values, sketch link and actions.
fn feature_page(
    t: Colors,
    s: &Studio,
    selected: &timeline::SelectedFeature<'_>,
    stacked: bool,
    cx: &mut Context<Studio>,
) -> Div {
    let feature = selected.feature;
    let part = selected.part;
    let rolled_back = selected.index >= timeline::marker(part);
    let state = chip_state(s, part, feature);
    // One header: a breadcrumb back to the part with the feature's kind and
    // place in the history, then the feature's icon and editable name.
    // States that need attention get a notice below, never a pill.
    let part_name = s
        .selected
        .and_then(|id| s.scene.object(id))
        .map(|object| object.name.clone())
        .unwrap_or_default();
    let crumb = row()
        .h(px(H_SM))
        .gap(px(SP_1))
        .child(on_command(
            row()
                .id("feature-back")
                .h(px(H_SM - 4.))
                .px(px(SP_1_5))
                .ml(px(-SP_1_5))
                .min_w(px(0.))
                .rounded(px(R_SM))
                .cursor_pointer()
                .text_size(px(TEXT_SM))
                .font_weight(WEIGHT_MEDIUM)
                .text_color(rgb(t.text_muted))
                .hover(move |d| d.bg(rgb(t.control_hover)).text_color(rgb(t.text)))
                .child(div().truncate().child(part_name))
                .tooltip(tip(t, "Back to the part summary", Some("Esc"))),
            Command::Timeline(TimelineCommand::Deselect),
            cx,
        ))
        .child(icon(Icon::ChevronRight, 12., t.text_faint))
        .child(caption(t, feature.kind.label()))
        .child(spacer())
        .child(
            caption(
                t,
                format!("Step {} of {}", selected.index + 1, part.features.len()),
            )
            .tabular()
            .text_color(rgb(t.text_faint)),
        );
    let header = row()
        .gap(px(SP_2))
        .pt(px(SP_1))
        .pb(px(SP_1_5))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(H_MD))
                .flex_shrink_0()
                .rounded(px(R_MD))
                .bg(rgb(t.accent_soft))
                .child(icon(feature_icon(feature), ICON_MD, t.accent_text)),
        )
        .child(
            numeric_field(
                t,
                s,
                "feature-name",
                Field::Feature(FeatureField::Name),
                FieldMark::None,
                "Feature name",
                cx,
            )
            .h(px(H_MD))
            .flex_1()
            .min_w(px(0.))
            .text_size(px(TEXT_LG))
            .font_weight(WEIGHT_MEDIUM),
        )
        .when(part.is_pending(), |d| {
            d.child(badge(t, "Regenerating", BadgeTone::Accent))
        });
    let notice_row = match &state {
        ChipState::Error(message) => {
            let (title, detail) = match message.split_once(" · ") {
                Some((title, detail)) => (title.to_owned(), Some(detail.to_owned())),
                None => (message.clone(), None),
            };
            Some(notice(
                t,
                Icon::Error,
                t.danger,
                title,
                Some(detail.unwrap_or_else(|| "The history continues without this feature".into())),
            ))
        }
        ChipState::Warning(message) => Some(notice(
            t,
            Icon::Warning,
            t.warning,
            message.clone(),
            Some("Edit the sketch to pick the regions again, or keep the rest".into()),
        )),
        ChipState::Ok if feature.suppressed => Some(notice(
            t,
            Icon::EyeOff,
            t.text_muted,
            "Suppressed · kept in the history, not evaluated".into(),
            None,
        )),
        ChipState::Ok if rolled_back => Some(notice(
            t,
            Icon::History,
            t.text_muted,
            "After the rollback marker · not evaluated".into(),
            Some("Roll to end to see its result".into()),
        )),
        ChipState::Ok => None,
    };
    let actions = row()
        .gap(px(SP_1))
        .child(
            command_button(
                t,
                "feature-suppress",
                if feature.suppressed {
                    "Unsuppress"
                } else {
                    "Suppress"
                },
                Some(if feature.suppressed {
                    Icon::Eye
                } else {
                    Icon::EyeOff
                }),
                Command::Timeline(TimelineCommand::ToggleSuppress),
                Tone::Secondary,
                cx,
            )
            .flex_1()
            .min_w(px(0.))
            .px(px(SP_1_5)),
        )
        .child(
            command_button(
                t,
                "feature-roll-back",
                if stacked {
                    "Roll back"
                } else {
                    "Roll back here"
                },
                Some(Icon::History),
                Command::Timeline(TimelineCommand::RollBackHere),
                Tone::Secondary,
                cx,
            )
            .flex_1()
            .min_w(px(0.))
            .px(px(SP_1_5)),
        );
    // Destructive, so apart from the history actions and in danger ink.
    let delete = on_command(
        row()
            .id("feature-delete")
            .h(px(H_SM))
            .w_full()
            .justify_center()
            .gap(px(SP_1_5))
            .rounded(px(R_SM + 1.))
            .cursor_pointer()
            .text_size(px(TEXT_MD))
            .font_weight(WEIGHT_MEDIUM)
            .text_color(rgb(t.danger))
            .hover(move |d| d.bg(tint(t.danger, 0.1)))
            .active(move |d| d.bg(tint(t.danger, 0.16)))
            .child(icon(Icon::Delete, ICON_SM, t.danger))
            .child("Delete feature")
            .tooltip(command_tip(t, Command::Timeline(TimelineCommand::Delete))),
        Command::Timeline(TimelineCommand::Delete),
        cx,
    );
    col()
        .child(crumb)
        .child(header)
        .when_some(notice_row, |d, notice| d.child(notice.mb(px(SP_1))))
        .child(parameters(t, s, feature, stacked, cx))
        .when_some(sketch_link(t, s, feature, cx), |d, link| d.child(link))
        .child(
            col()
                .gap(px(SP_1))
                .mt(px(SP_3))
                .pt(px(SP_3))
                .border_t_1()
                .border_color(rgb(t.hairline))
                .child(actions)
                .child(delete),
        )
}

/// The last regeneration of part `id`: its total time ("725 ms") and the
/// breakdown, for the part summary's tooltip and the status bar.
pub(super) fn regeneration_timing(s: &Studio, id: u64) -> Option<(String, String)> {
    let (_, stats) = s.cad.last_stats.as_ref().filter(|(part, _)| *part == id)?;
    let total = if stats.total_ms < 1000. {
        format!("{:.0} ms", stats.total_ms)
    } else {
        format!("{:.1} s", stats.total_ms / 1000.)
    };
    let detail = if stats.evaluate_ms < 0.5 {
        "History reused · tessellation only".to_owned()
    } else {
        format!(
            "Solid {:.0} ms · mesh {:.0} ms",
            stats.evaluate_ms, stats.tessellate_ms
        )
    };
    Some((total, detail))
}

/// One feature row in the summary's history list. `number` is its place in
/// the list, which a pending feature before it shifts by one. While a tool
/// runs the rows are a read-only map of where its feature lands: no hover
/// fill or click competes with the pending row.
fn history_row(
    t: Colors,
    s: &Studio,
    part: &PartData,
    index: usize,
    number: usize,
    feature: &Feature,
    cx: &mut Context<Studio>,
) -> Stateful<Div> {
    let id = feature.id;
    let rolled_back = index >= timeline::marker(part);
    let state = chip_state(s, part, feature);
    let live = s.cad_tool_active();
    row()
        .id(("part-history", id))
        .h(px(H_SM + 2.))
        .px(px(SP_1_5))
        .gap(px(SP_2))
        .rounded(px(R_SM))
        .when(!live, |d| {
            d.cursor_pointer()
                .hover(move |d| d.bg(rgb(t.control_hover)))
        })
        .when(rolled_back, |d| d.opacity(0.45))
        .child(
            div()
                .w(px(14.))
                .flex_shrink_0()
                .tabular()
                .text_size(px(TEXT_SM))
                .text_color(rgb(t.text_faint))
                .child(number.to_string()),
        )
        .child(feature_glyph(
            feature,
            ICON_SM,
            if matches!(state, ChipState::Error(_)) {
                t.danger
            } else {
                t.text_muted
            },
            hex(t.surface),
        ))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_color(rgb(if feature.suppressed {
                    t.text_faint
                } else {
                    t.text
                }))
                .when(feature.suppressed, |d| d.line_through())
                .child(feature.name.clone()),
        )
        .child(
            caption(t, typeset_minus(&chip_hint(feature)))
                .tabular()
                .text_color(rgb(t.text_faint)),
        )
        .map(|d| match &state {
            ChipState::Error(_) => d.child(icon(Icon::Error, 13., t.danger)),
            ChipState::Warning(_) => d.child(icon(Icon::Warning, 13., t.warning)),
            ChipState::Ok if feature.suppressed => d.child(icon(Icon::EyeOff, 13., t.text_faint)),
            ChipState::Ok => d,
        })
        .when(!live, |d| {
            d.on_click(cx.listener(move |s, event: &ClickEvent, w, cx| {
                cx.stop_propagation();
                s.execute(Command::Timeline(TimelineCommand::Select(id)), w, cx);
                if event.click_count() >= 2 {
                    s.execute(Command::Timeline(TimelineCommand::Edit), w, cx);
                }
            }))
        })
}

/// The name a running tool's feature gets on Done, as the history shows it
/// while previewing: numbered past the features it would repeat.
fn pending_name(part: &PartData, name: &str, count: usize) -> String {
    if count > 1 {
        format!("{count} × {name}")
    } else {
        timeline::numbered_name(part, name)
    }
}

/// The feature a running tool adds on Done, where it will land in the
/// history: accent ink, the live value, and no actions until it exists.
fn pending_row(t: Colors, number: usize, glyph: Icon, name: String, hint: String) -> Stateful<Div> {
    row()
        .id("part-history-pending")
        .h(px(H_SM + 2.))
        .px(px(SP_1_5))
        .gap(px(SP_2))
        .rounded(px(R_SM))
        .bg(rgb(t.accent_soft))
        .child(
            div()
                .w(px(14.))
                .flex_shrink_0()
                .tabular()
                .text_size(px(TEXT_SM))
                .text_color(rgb(t.accent_text))
                .child(number.to_string()),
        )
        .child(icon(glyph, ICON_SM, t.accent_text))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .font_weight(WEIGHT_MEDIUM)
                .text_color(rgb(t.text))
                .child(name),
        )
        .child(
            caption(t, typeset_minus(&hint))
                .tabular()
                .text_color(rgb(t.accent_text)),
        )
        .tooltip(tip_detail(
            t,
            "New feature",
            "Added to the history here when you press Done",
            None,
        ))
}

/// Summary of a part with no feature selected: health, size, history and
/// part-level actions.
fn part_summary(
    t: Colors,
    s: &Studio,
    object: u64,
    part: &PartData,
    cx: &mut Context<Studio>,
) -> Div {
    let issues = part.issues();
    let pending = s.pending_feature(object);
    let (status, ink) = if pending.is_some() {
        // The viewport shows the tool's result, not the part's: say so,
        // and leave the committed solid's counts until Done.
        (
            "Previewing · Done adds it to the history".to_owned(),
            t.accent,
        )
    } else if part.is_pending() {
        ("Regenerating…".to_owned(), t.accent)
    } else if part.regeneration_failed() {
        // Viewport, OBJ/STL and STEP/convert would otherwise disagree silently.
        ("Failed · showing the previous result".to_owned(), t.danger)
    } else if issues.is_empty() {
        ("Up to date".to_owned(), t.success)
    } else if issues.iter().all(|issue| issue.warning) {
        (format!("{} approximated", issues.len()), t.warning)
    } else {
        (
            format!(
                "{} issue{}",
                issues.len(),
                if issues.len() == 1 { "" } else { "s" }
            ),
            t.danger,
        )
    };
    let suppressed = part.features.iter().filter(|f| f.suppressed).count();
    let faces = s.cad.faces.get(&part.id).map(Vec::len);
    let triangles = s.scene.object_mesh(object).map(|mesh| mesh.faces.len());
    let marker = timeline::marker(part);
    // One line of facts after a status dot; timings are on hover.
    let mut facts = vec![status];
    let previewing = pending.is_some();
    if marker < part.features.len() {
        facts.push(format!(
            "rolled back to {marker} of {}",
            part.features.len()
        ));
    }
    if suppressed > 0 {
        facts.push(format!("{suppressed} suppressed"));
    }
    if let Some(faces) = faces.filter(|_| !previewing) {
        facts.push(format!(
            "{} face{}",
            thousands(faces),
            if faces == 1 { "" } else { "s" }
        ));
    }
    if let Some(triangles) = triangles.filter(|_| !previewing) {
        facts.push(format!("{} triangles", thousands(triangles)));
    }
    let timing = regeneration_timing(s, part.id);
    let meta = row()
        .id("part-meta")
        .items_start()
        .gap(px(SP_2))
        .pb(px(SP_1))
        .child(div().pt(px(5.)).child(dot(ink, 7.)))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .tabular()
                .text_size(px(TEXT_SM))
                .line_height(px(17.))
                .text_color(rgb(t.text_muted))
                .child(facts.join(" · ")),
        )
        .when_some(timing, |d, (total, detail)| {
            d.tooltip(tip_detail(
                t,
                format!("Regenerated in {total}"),
                detail,
                None,
            ))
        });
    let mut summary = col().gap(px(SP_1)).child(meta);
    for issue in issues {
        let target = part
            .features
            .iter()
            .find(|feature| feature.id == issue.feature);
        let title = match target {
            Some(feature) => format!("{}: {}", feature.name, timeline::issue_text(issue)),
            None => timeline::issue_text(issue),
        };
        let detail = (!issue.suggestion.is_empty()).then(|| issue.suggestion.clone());
        let card = notice(t, Icon::Error, t.danger, title, detail).mt(px(SP_1));
        summary = summary.child(match target.map(|f| f.id) {
            Some(id) => div()
                .id(("part-issue", id))
                .cursor_pointer()
                .child(card)
                .tooltip(tip(t, "Select the feature", None))
                .on_click(cx.listener(move |s, _, w, cx| {
                    cx.stop_propagation();
                    s.execute(Command::Timeline(TimelineCommand::Select(id)), w, cx);
                }))
                .into_any_element(),
            None => card.into_any_element(),
        });
    }
    summary
        .child(
            div()
                .pt(px(SP_2))
                .pb(px(SP_1))
                .text_size(px(TEXT_SM))
                .font_weight(WEIGHT_MEDIUM)
                .text_color(rgb(t.text_muted))
                .child("History"),
        )
        .children({
            // A running tool's feature lands at the rollback marker, where
            // Done inserts it; the rows after it renumber.
            let shift = pending.as_ref().map_or(0, |_| 1);
            let mut rows: Vec<AnyElement> = part
                .features
                .iter()
                .enumerate()
                .map(|(index, feature)| {
                    let number = index + 1 + if index >= marker { shift } else { 0 };
                    history_row(t, s, part, index, number, feature, cx).into_any_element()
                })
                .collect();
            if let Some(pending) = pending {
                let name = pending_name(part, pending.name, pending.count);
                rows.insert(
                    marker.min(rows.len()),
                    pending_row(t, marker + 1, pending.icon, name, pending.hint).into_any_element(),
                );
            }
            rows
        })
        // While a tool runs, only the history (where its feature lands)
        // stays at full strength.
        .child(
            col()
                .when(s.cad_tool_active(), |d| d.opacity(0.4))
                .children(combine_row(t, s, cx))
                .child(part_actions(t, cx)),
        )
}

fn part_actions(t: Colors, cx: &mut Context<Studio>) -> Div {
    col().gap(px(SP_1)).pt(px(SP_3)).child(
        row()
            .gap(px(SP_1))
            .child(
                command_button(
                    t,
                    "part-to-mesh",
                    "Convert to mesh",
                    Some(Icon::Mesh),
                    Command::Cad(CadCommand::ConvertToMesh),
                    Tone::Secondary,
                    cx,
                )
                .flex_1()
                .min_w(px(0.)),
            )
            .child(
                command_button(
                    t,
                    "part-export-step",
                    "Export STEP…",
                    Some(Icon::Export),
                    Command::Cad(CadCommand::ExportStep),
                    Tone::Secondary,
                    cx,
                )
                .flex_1()
                .min_w(px(0.)),
            ),
    )
}

/// "Combine N objects into the active one" when several parts or meshes are
/// selected. The active (last selected) object is the target.
pub(super) fn combine_row(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> Option<Div> {
    let ids = s.selected_ids();
    let target = s.selected?;
    let usable = |id: &u64| {
        s.scene
            .object(*id)
            .is_some_and(|o| o.is_part() || o.data.is_editable_mesh())
    };
    let tools = ids.iter().filter(|id| **id != target && usable(id)).count();
    if tools == 0 || !usable(&target) {
        return None;
    }
    let name = s.scene.object(target)?.name.clone();
    let operations = [
        FeatureOperation::Join,
        FeatureOperation::Cut,
        FeatureOperation::Intersect,
    ];
    Some(
        col()
            .gap(px(SP_1))
            .pt(px(SP_3))
            .child(
                row()
                    .gap(px(SP_1_5))
                    .child(icon(Icon::Join, ICON_SM, t.accent_text))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_color(rgb(t.text))
                            .child(format!(
                                "Combine {tools} object{} into {name}",
                                if tools == 1 { "" } else { "s" }
                            )),
                    ),
            )
            .child(
                row()
                    .gap(px(SP_1))
                    .children(operations.into_iter().map(|operation| {
                        command_button(
                            t,
                            SharedString::from(format!("combine-{operation:?}")),
                            operation.label(),
                            Some(operation_icon(operation)),
                            Command::Cad(CadCommand::Combine(operation)),
                            Tone::Secondary,
                            cx,
                        )
                        .flex_1()
                        .min_w(px(0.))
                        .px(px(SP_1_5))
                    })),
            )
            .child(
                div()
                    .text_size(px(TEXT_SM))
                    .line_height(px(16.))
                    .text_color(rgb(t.text_faint))
                    .child("The other objects become editable tools in its history"),
            ),
    )
}

/// The Convert to part action for a mesh, with the closed-manifold check
/// shown before converting.
pub(super) fn convert_to_part(
    t: Colors,
    s: &Studio,
    object: u64,
    cx: &mut Context<Studio>,
) -> Option<Div> {
    let check = s.manifold_check(object)?;
    let (ink, glyph, message) = match &check {
        Ok(()) => (
            t.success,
            Icon::Success,
            "Closed and manifold · converts to a B-rep solid".to_owned(),
        ),
        Err(message) => (t.warning, Icon::Warning, format!("This mesh {message}")),
    };
    Some(
        col()
            .gap(px(SP_1_5))
            .pt(px(SP_1))
            .child(
                row()
                    .items_start()
                    .gap(px(SP_1_5))
                    .child(div().pt(px(1.)).child(icon(glyph, ICON_SM, ink)))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .text_size(px(TEXT_SM))
                            .line_height(px(16.))
                            .text_color(rgb(t.text_muted))
                            .child(message),
                    ),
            )
            .child(if check.is_ok() {
                command_button(
                    t,
                    "mesh-to-part",
                    "Convert to part",
                    Some(Icon::Part),
                    Command::Cad(CadCommand::ConvertToPart),
                    Tone::Secondary,
                    cx,
                )
                .w_full()
                .into_any_element()
            } else {
                disabled(
                    button(
                        t,
                        "mesh-to-part",
                        "Convert to part",
                        Some(Icon::Part),
                        Tone::Secondary,
                    )
                    .w_full()
                    .cursor_default(),
                )
                .tooltip(tip(t, "Only closed, manifold meshes become solids", None))
                .into_any_element()
            }),
    )
}

/// The Part section's contents and header detail.
pub(super) fn part_section(
    t: Colors,
    s: &Studio,
    object: u64,
    part: &PartData,
    stacked: bool,
    cx: &mut Context<Studio>,
) -> (Div, Option<SharedString>) {
    match s.selected_feature() {
        // The feature's breadcrumb says where it sits in the history.
        Some(selected) => (feature_page(t, s, &selected, stacked, cx), None),
        None => {
            // A previewed feature counts: the history below lists it.
            let count =
                part.features.len() + s.pending_feature(object).map_or(0, |pending| pending.count);
            (
                part_summary(t, s, object, part, cx),
                Some(format!("{count} feature{}", if count == 1 { "" } else { "s" }).into()),
            )
        }
    }
}
