//! The inspector's Part section: the part summary and actions when no
//! feature is selected, or the selected feature's parameters, built from the
//! same scrubbable fields and segmented controls as the rest of the panel.
//! Also the part-level actions shown for meshes and multi-object selections
//! (Convert to part, Combine).
use super::assets::{Icon, icon};
use super::commands::operation_icon;
use super::components::*;
use super::timeline::{ChipState, chip_hint, chip_state, feature_icon};
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

fn stat(t: Colors, label: &'static str, value: String) -> Div {
    col()
        .flex_1()
        .min_w(px(0.))
        .px(px(SP_2 + 2.))
        .py(px(SP_1_5))
        .gap(px(1.))
        .rounded(px(R_SM))
        .bg(rgb(t.field))
        .child(caption(t, label).text_size(px(TEXT_XS)))
        .child(
            div()
                .tabular()
                .font_weight(WEIGHT_MEDIUM)
                .truncate()
                .child(value),
        )
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
                .child(icon(
                    operation_icon(operation),
                    ICON_SM,
                    if active { t.text } else { t.text_muted },
                ))
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
                        format!(
                            "Face at {}, {}, {} · positive pulls out",
                            short(push.face.point.x),
                            short(push.face.point.y),
                            short(push.face.point.z)
                        ),
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
                        .rounded(px(R_SM))
                        .border_1()
                        .border_color(rgb(t.hairline))
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
            "Embedded copy".to_owned(),
            "The source sketch was deleted · Edit sketch recreates it from this copy",
        ),
    };
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
                    .rounded(px(R_SM))
                    .bg(rgb(t.field))
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
    let (badge_text, tone) = match (&state, feature.suppressed, rolled_back) {
        (ChipState::Error(_), _, _) => ("Failed", BadgeTone::Danger),
        (_, true, _) => ("Suppressed", BadgeTone::Neutral),
        (_, _, true) => ("Rolled back", BadgeTone::Neutral),
        _ if part.is_pending() => ("Regenerating", BadgeTone::Accent),
        (ChipState::Warning(_), _, _) => ("Check", BadgeTone::Neutral),
        _ => ("Active", BadgeTone::Neutral),
    };
    let header = row()
        .gap(px(SP_2))
        .pb(px(SP_1))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(H_SM))
                .flex_shrink_0()
                .rounded(px(R_SM + 1.))
                .bg(rgb(t.accent_soft))
                .child(icon(feature_icon(feature), ICON_SM, t.accent_text)),
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
            .flex_1()
            .min_w(px(0.))
            .font_weight(WEIGHT_MEDIUM),
        )
        .child(badge(t, badge_text, tone))
        .child(on_command(
            icon_button(t, "feature-deselect", Icon::Close, false, H_SM).tooltip(tip(
                t,
                "Back to the part",
                Some("Esc"),
            )),
            Command::Timeline(TimelineCommand::Deselect),
            cx,
        ));
    let meta = caption(
        t,
        format!(
            "{} · step {} of {}",
            feature.kind.label(),
            selected.index + 1,
            part.features.len()
        ),
    )
    .pb(px(SP_1));
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
        .pt(px(SP_2))
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
        )
        .child(on_command(
            icon_button(t, "feature-delete", Icon::Delete, false, H_SM)
                .tooltip(command_tip(t, Command::Timeline(TimelineCommand::Delete))),
            Command::Timeline(TimelineCommand::Delete),
            cx,
        ));
    col()
        .child(header)
        .child(meta)
        .when_some(notice_row, |d, notice| d.child(notice.mb(px(SP_1))))
        .child(parameters(t, s, feature, stacked, cx))
        .when_some(sketch_link(t, s, feature, cx), |d, link| d.child(link))
        .child(actions)
        .child(
            div()
                .pt(px(SP_2))
                .text_size(px(TEXT_SM))
                .text_color(rgb(t.text_faint))
                .child("Drag a value to scrub · the part regenerates live"),
        )
}

/// One feature row in the summary's history list.
fn history_row(
    t: Colors,
    s: &Studio,
    part: &PartData,
    index: usize,
    feature: &Feature,
    cx: &mut Context<Studio>,
) -> Stateful<Div> {
    let id = feature.id;
    let rolled_back = index >= timeline::marker(part);
    let state = chip_state(s, part, feature);
    row()
        .id(("part-history", id))
        .h(px(H_SM + 2.))
        .px(px(SP_1_5))
        .gap(px(SP_2))
        .rounded(px(R_SM))
        .cursor_pointer()
        .hover(move |d| d.bg(rgb(t.control_hover)))
        .when(rolled_back, |d| d.opacity(0.45))
        .child(
            div()
                .w(px(14.))
                .flex_shrink_0()
                .tabular()
                .text_size(px(TEXT_XS))
                .text_color(rgb(t.text_faint))
                .child((index + 1).to_string()),
        )
        .child(icon(
            feature_icon(feature),
            ICON_SM,
            if matches!(state, ChipState::Error(_)) {
                t.danger
            } else {
                t.text_muted
            },
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
            caption(t, chip_hint(feature))
                .tabular()
                .text_size(px(TEXT_XS))
                .text_color(rgb(t.text_faint)),
        )
        .map(|d| match &state {
            ChipState::Error(_) => d.child(icon(Icon::Error, 13., t.danger)),
            ChipState::Warning(_) => d.child(icon(Icon::Warning, 13., t.warning)),
            ChipState::Ok if feature.suppressed => d.child(icon(Icon::EyeOff, 13., t.text_faint)),
            ChipState::Ok => d,
        })
        .on_click(cx.listener(move |s, event: &ClickEvent, w, cx| {
            cx.stop_propagation();
            s.execute(Command::Timeline(TimelineCommand::Select(id)), w, cx);
            if event.click_count() >= 2 {
                s.execute(Command::Timeline(TimelineCommand::Edit), w, cx);
            }
        }))
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
    let (status, tone) = if part.is_pending() {
        ("Regenerating…".to_owned(), BadgeTone::Accent)
    } else if part.regeneration_failed() {
        // Viewport, OBJ/STL and STEP/convert would otherwise disagree silently.
        (
            "Failed · showing the previous result".to_owned(),
            BadgeTone::Danger,
        )
    } else if issues.is_empty() {
        ("Up to date".to_owned(), BadgeTone::Neutral)
    } else {
        (
            format!(
                "{} issue{}",
                issues.len(),
                if issues.len() == 1 { "" } else { "s" }
            ),
            BadgeTone::Danger,
        )
    };
    let suppressed = part.features.iter().filter(|f| f.suppressed).count();
    let faces = s.cad.faces.get(&part.id).map(Vec::len);
    let triangles = s.scene.object_mesh(object).map(|mesh| mesh.faces.len());
    let stats = s
        .cad
        .last_stats
        .as_ref()
        .filter(|(id, _)| *id == part.id)
        .map(|(_, stats)| stats.clone());
    let marker = timeline::marker(part);
    let mut summary = col()
        .gap(px(SP_1))
        .child(property_row(
            t,
            "Status",
            row().gap(px(SP_1_5)).child(badge(t, status, tone)).when(
                marker < part.features.len(),
                |d| {
                    d.child(badge(
                        t,
                        format!("Rolled back · {marker} of {}", part.features.len()),
                        BadgeTone::Neutral,
                    ))
                },
            ),
        ))
        .child(
            row()
                .gap(px(SP_1))
                .py(px(SP_1))
                .child(stat(
                    t,
                    "Features",
                    if suppressed > 0 {
                        format!("{} · {suppressed} off", part.features.len())
                    } else {
                        part.features.len().to_string()
                    },
                ))
                .child(stat(t, "Faces", faces.map_or("—".into(), thousands)))
                .child(stat(
                    t,
                    "Triangles",
                    triangles.map_or("—".into(), thousands),
                )),
        )
        .when_some(stats, |d, stats| {
            d.child(caption(
                t,
                if stats.evaluate_ms < 0.5 {
                    format!(
                        "Regenerated in {:.0} ms · history reused, tessellation only",
                        stats.total_ms
                    )
                } else {
                    format!(
                        "Regenerated in {:.0} ms · solid {:.0} ms · mesh {:.0} ms",
                        stats.total_ms, stats.evaluate_ms, stats.tessellate_ms
                    )
                },
            ))
        });
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
        .children(
            part.features
                .iter()
                .enumerate()
                .map(|(index, feature)| history_row(t, s, part, index, feature, cx)),
        )
        .child(
            div()
                .pt(px(SP_1))
                .text_size(px(TEXT_SM))
                .text_color(rgb(t.text_faint))
                .child("Click a feature here or in the timeline to edit it"),
        )
        .children(combine_row(t, s, cx))
        .child(part_actions(t, cx))
}

fn part_actions(t: Colors, cx: &mut Context<Studio>) -> Div {
    col()
        .gap(px(SP_1))
        .pt(px(SP_3))
        .child(
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
        .child(
            div()
                .text_size(px(TEXT_SM))
                .line_height(px(16.))
                .text_color(rgb(t.text_faint))
                .child("Converting bakes the history into editable polygons: the timeline and its parameters are gone (undo brings them back)."),
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
) -> (Div, SharedString) {
    match s.selected_feature() {
        Some(selected) => {
            let detail = format!("Feature {} of {}", selected.index + 1, part.features.len());
            (feature_page(t, s, &selected, stacked, cx), detail.into())
        }
        None => (
            part_summary(t, s, object, part, cx),
            format!(
                "{} feature{}",
                part.features.len(),
                if part.features.len() == 1 { "" } else { "s" }
            )
            .into(),
        ),
    }
}
