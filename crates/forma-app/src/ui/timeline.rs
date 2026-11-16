//! The feature timeline: the active part's history as a strip of chips in
//! the bottom-centre viewport dock (see `slots::viewport_dock`).
//!
//! Chips show each feature's kind, name, a compact parameter hint and its
//! state: failed and warned features carry a glyph, suppressed features are
//! struck through, and features past the rollback marker are dimmed. The
//! marker is a draggable accent bar in the gaps between chips. Click selects,
//! double-click edits (a sketch-based feature opens its sketch), right-click
//! opens the feature menu. Behaviour lives in `crate::timeline`.
use super::assets::{Icon, icon};
use super::commands::{operation_icon, timeline_icon};
use super::components::*;
use super::hud::HUD_INSET;
use super::overlays::{backdrop, contain, pop_in};
use super::tokens::*;
use crate::app::{Command, Studio};
use crate::theme::Colors;
use crate::timeline::{self, TimelineCommand};
use forma_core::{
    BaseBody, ExtrudeExtent, Feature, FeatureKind, FeatureOperation, PartData, PrimitiveShape,
};
use gpui::{prelude::*, *};

/// Widest a chip's name may grow before it truncates.
const NAME_MAX: f32 = 132.;

/// The icon for a feature's kind (and, where it reads better, operation).
pub(crate) fn feature_icon(feature: &Feature) -> Icon {
    match &feature.kind {
        FeatureKind::Primitive(primitive) => match primitive.shape {
            PrimitiveShape::Box { .. } => Icon::Cube,
            PrimitiveShape::Cylinder { .. } => Icon::Cylinder,
            PrimitiveShape::Sphere { .. } => Icon::Sphere,
            PrimitiveShape::Cone { .. } => Icon::Cone,
            PrimitiveShape::Torus { .. } => Icon::Torus,
        },
        FeatureKind::Extrude(extrude) if extrude.operation == FeatureOperation::Cut => {
            Icon::ExtrudeCut
        }
        FeatureKind::Extrude(_) => Icon::Extrude,
        FeatureKind::Revolve(_) => Icon::Revolve,
        FeatureKind::Combine(combine) => operation_icon(combine.operation),
        FeatureKind::PushPull(_) => Icon::PushPull,
        FeatureKind::Chamfer(_) => Icon::Bevel,
        FeatureKind::BaseBody(base) => match base.body {
            BaseBody::Mesh { .. } => Icon::Mesh,
            BaseBody::Step { .. } => Icon::Import,
        },
    }
}

fn short(value: f32) -> String {
    compact_number(crate::transform::format_precision_value(value), 5)
}

/// A compact parameter summary for a chip, e.g. `0.6 m` or `2.2×0.3×1.5`.
pub(crate) fn chip_hint(feature: &Feature) -> String {
    match &feature.kind {
        FeatureKind::Primitive(primitive) => match primitive.shape {
            PrimitiveShape::Box { size } => {
                format!("{}×{}×{}", short(size.x), short(size.y), short(size.z))
            }
            PrimitiveShape::Cylinder { radius, height }
            | PrimitiveShape::Cone {
                bottom_radius: radius,
                height,
                ..
            } => format!("r {} × {}", short(radius), short(height)),
            PrimitiveShape::Sphere { radius } => format!("r {}", short(radius)),
            PrimitiveShape::Torus {
                major_radius,
                minor_radius,
            } => format!("{} / {}", short(major_radius), short(minor_radius)),
        },
        FeatureKind::Extrude(extrude) => match extrude.extent {
            ExtrudeExtent::OneSide => format!("{} m", short(extrude.distance)),
            ExtrudeExtent::Symmetric => format!("±{} m", short(extrude.distance.abs() * 0.5)),
            ExtrudeExtent::TwoSides { back_distance } => {
                format!("{} / {} m", short(extrude.distance), short(back_distance))
            }
        },
        FeatureKind::Revolve(revolve) => format!("{}°", short(revolve.angle.to_degrees())),
        FeatureKind::Combine(combine) => format!(
            "{} tool{}",
            combine.tools.len(),
            if combine.tools.len() == 1 { "" } else { "s" }
        ),
        FeatureKind::PushPull(push) => format!("{} m", short(push.distance)),
        FeatureKind::Chamfer(chamfer) => format!("{} m", short(chamfer.distance)),
        FeatureKind::BaseBody(base) => match &base.body {
            BaseBody::Mesh { mesh } => format!("{} faces", mesh.faces.len()),
            BaseBody::Step { .. } => "STEP".into(),
        },
    }
}

/// How a feature appears in the timeline right now.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ChipState {
    Ok,
    Warning(String),
    Error(String),
}

pub(crate) fn chip_state(s: &Studio, part: &PartData, feature: &Feature) -> ChipState {
    if let Some(issue) = part.issues().iter().find(|i| i.feature == feature.id) {
        let mut text = timeline::issue_text(issue);
        if !issue.suggestion.is_empty() {
            text = format!("{text} · {}", issue.suggestion);
        }
        // An approximated feature still applied; it is not a failure.
        return if issue.warning {
            ChipState::Warning(text)
        } else {
            ChipState::Error(text)
        };
    }
    if let Some((_, warning)) = s
        .feature_warnings(part)
        .iter()
        .find(|(id, _)| *id == feature.id)
    {
        return ChipState::Warning(warning.clone());
    }
    ChipState::Ok
}

/// The operation glyph shown before a hint, for features that remove or
/// intersect material (joins are the unmarked default).
fn hint_operation(feature: &Feature) -> Option<Icon> {
    match &feature.kind {
        FeatureKind::Primitive(primitive) => Some(primitive.operation),
        FeatureKind::Revolve(revolve) => Some(revolve.operation),
        FeatureKind::BaseBody(base) => Some(base.operation),
        FeatureKind::Extrude(extrude) if extrude.operation == FeatureOperation::Intersect => {
            Some(extrude.operation)
        }
        _ => None,
    }
    .filter(|op| matches!(op, FeatureOperation::Cut | FeatureOperation::Intersect))
    .map(operation_icon)
}

#[allow(clippy::too_many_arguments)]
fn chip(
    t: Colors,
    s: &Studio,
    part: &PartData,
    index: usize,
    feature: &Feature,
    selected: bool,
    compact: bool,
    cx: &mut Context<Studio>,
) -> Stateful<Div> {
    let rolled_back = index >= timeline::marker(part);
    let state = chip_state(s, part, feature);
    let id = feature.id;
    let ink = if selected { t.accent_text } else { t.text };
    let glyph_ink = match (&state, selected) {
        (_, true) => t.accent_text,
        (ChipState::Error(_), false) => t.danger,
        _ => t.text_muted,
    };
    let kind = feature.kind.label();
    let detail = {
        let mut lines = vec![crate::cad::feature_summary(feature)];
        match &state {
            ChipState::Error(message) => lines.push(format!("Failed · {message}")),
            ChipState::Warning(message) => lines.push(message.clone()),
            ChipState::Ok => {}
        }
        if feature.suppressed {
            lines.push("Suppressed · not evaluated".into());
        } else if rolled_back {
            lines.push("After the rollback marker · not evaluated".into());
        }
        lines.push(if timeline::is_sketch_feature(feature) {
            "Double-click to edit the sketch · Right-click for more".into()
        } else {
            "Double-click to edit · Right-click for more".into()
        });
        lines.join("\n")
    };
    row()
        .id(("timeline-chip", id))
        .h(px(H_MD))
        .flex_shrink_0()
        .px(px(SP_2))
        .gap(px(SP_1_5))
        .rounded(px(R_SM + 1.))
        .cursor_pointer()
        .bg(if selected {
            hex(t.accent_soft)
        } else {
            clear()
        })
        .when(!selected, |d| d.hover(move |d| d.bg(rgb(t.control_hover))))
        .when(selected, |d| {
            d.hover(move |d| d.bg(rgb(t.accent_soft_hover)))
        })
        .when(rolled_back && !selected, |d| d.opacity(0.42))
        .when(rolled_back && selected, |d| d.opacity(0.7))
        .child(icon(feature_icon(feature), ICON_SM, glyph_ink))
        .child(
            div()
                .max_w(px(NAME_MAX))
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_size(px(TEXT_SM))
                .font_weight(WEIGHT_MEDIUM)
                .text_color(rgb(if feature.suppressed && !selected {
                    t.text_faint
                } else {
                    ink
                }))
                .when(feature.suppressed, |d| d.line_through())
                .child(feature.name.clone()),
        )
        .when(!compact, |d| {
            d.when_some(hint_operation(feature), |d, glyph| {
                d.child(icon(
                    glyph,
                    12.,
                    if selected {
                        t.accent_text
                    } else {
                        t.text_muted
                    },
                ))
            })
            .child(
                div()
                    .flex_shrink_0()
                    .tabular()
                    .text_size(px(TEXT_SM))
                    .text_color(rgb(if selected {
                        t.accent_text
                    } else {
                        t.text_muted
                    }))
                    .child(chip_hint(feature)),
            )
        })
        .map(|d| match &state {
            ChipState::Error(_) => d.child(icon(Icon::Error, 13., t.danger)),
            ChipState::Warning(_) => d.child(icon(Icon::Warning, 13., t.warning)),
            ChipState::Ok if feature.suppressed => d.child(icon(
                Icon::EyeOff,
                13.,
                if selected {
                    t.accent_text
                } else {
                    t.text_faint
                },
            )),
            ChipState::Ok => d,
        })
        .tooltip(tip_detail(
            t,
            format!("{} · {kind}", feature.name),
            detail,
            None,
        ))
        .on_click(cx.listener(move |s, event: &ClickEvent, w, cx| {
            cx.stop_propagation();
            let command = if event.click_count() >= 2 {
                TimelineCommand::Edit
            } else {
                TimelineCommand::Select(id)
            };
            s.execute(Command::Timeline(command), w, cx);
        }))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |s, event: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                s.open_feature_menu(id, event.position, cx);
            }),
        )
}

/// A gap between chips. The one holding the rollback marker shows it and
/// starts a drag; the others move the marker there on click.
fn slot(
    t: Colors,
    s: &Studio,
    index: usize,
    count: usize,
    marker: usize,
    cx: &mut Context<Studio>,
) -> Stateful<Div> {
    let id = ("timeline-slot", index);
    if index == marker {
        let dragging = s.panels.timeline.marker.is_some();
        let at_end = index == count;
        let ink = if dragging { t.accent_hover } else { t.accent };
        // A full-height bar with a grab tab at its middle, like a splitter:
        // it divides evaluated features from rolled-back ones.
        return div()
            .id(id)
            .relative()
            .w(px(14.))
            .h(px(H_MD + 6.))
            .my(px(-3.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .cursor(CursorStyle::ResizeLeftRight)
            .group("timeline-marker")
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(6.))
                    .w(px(2.))
                    .rounded_full()
                    .bg(rgb(ink))
                    .group_hover("timeline-marker", move |d| d.bg(rgb(t.accent_hover))),
            )
            .child(
                row()
                    .w(px(10.))
                    .h(px(18.))
                    .gap(px(2.))
                    .justify_center()
                    .rounded(px(R_XS))
                    .bg(rgb(ink))
                    .group_hover("timeline-marker", move |d| d.bg(rgb(t.accent_hover)))
                    .children((0..2).map(|_| {
                        div()
                            .w(px(1.))
                            .h(px(8.))
                            .rounded_full()
                            .bg(tint(t.on_accent, 0.6))
                    })),
            )
            .tooltip(tip_detail(
                t,
                if at_end {
                    "History marker"
                } else {
                    "Rolled back here"
                },
                "Drag to roll the history back or forward · ⇧ ← / ⇧ → step",
                None,
            ))
            .on_mouse_down(MouseButton::Left, cx.listener(Studio::begin_marker_drag));
    }
    div()
        .id(id)
        .w(px(6.))
        .h(px(H_MD))
        .flex_shrink_0()
        .flex()
        .justify_center()
        .items_center()
        .cursor_pointer()
        .group("timeline-slot")
        .child(
            div()
                .w(px(2.))
                .h(px(12.))
                .rounded_full()
                .group_hover("timeline-slot", move |d| d.bg(rgb(t.accent_border))),
        )
        .on_click(cx.listener(move |s, _, _, cx| {
            cx.stop_propagation();
            s.place_marker(index, cx);
        }))
}

/// Regeneration state at the right end of the strip.
fn status(t: Colors, s: &Studio, part: &PartData, tight: bool, cx: &mut Context<Studio>) -> Div {
    let count = part.features.len();
    let marker = timeline::marker(part);
    let issues = part.issues();
    let warnings = s.feature_warnings(part).len();
    let mut cluster = row()
        .h(px(H_MD))
        .px(px(SP_1_5))
        .gap(px(SP_1_5))
        .flex_shrink_0();
    if s.part_regenerating(part) {
        cluster = cluster
            .child(
                dot(t.accent, 6.).with_animation(
                    "timeline-regenerating",
                    Animation::new(std::time::Duration::from_millis(900))
                        .repeat()
                        .with_easing(gpui::ease_in_out),
                    |dot, delta| dot.opacity(0.35 + 0.65 * (1. - (delta * 2. - 1.).abs())),
                ),
            )
            .when(!tight, |d| {
                d.child(caption(t, "Regenerating").text_color(rgb(t.accent_text)))
            });
    } else if !issues.is_empty() {
        let label = format!(
            "{} issue{}",
            issues.len(),
            if issues.len() == 1 { "" } else { "s" }
        );
        cluster = cluster
            .child(icon(Icon::Error, ICON_SM, t.danger))
            .when(!tight, |d| {
                d.child(caption(t, label).text_color(rgb(t.danger)))
            });
    } else if warnings > 0 {
        cluster = cluster
            .child(icon(Icon::Warning, ICON_SM, t.warning))
            .when(!tight, |d| {
                d.child(caption(
                    t,
                    format!("{warnings} warning{}", if warnings == 1 { "" } else { "s" }),
                ))
            });
    } else {
        // Timings live in the status bar; the strip only says it is current.
        cluster = cluster.child(
            div()
                .id("timeline-up-to-date")
                .p(px(SP_1))
                .child(dot(t.success, 6.))
                .tooltip(tip(t, "Up to date", None)),
        );
    }
    if marker < count {
        cluster = cluster.child(on_command(
            icon_button(t, "timeline-roll-to-end", Icon::ChevronRight, false, H_SM).tooltip(
                tip_detail(
                    t,
                    "Roll to end",
                    format!("{marker} of {count} features evaluated"),
                    None,
                ),
            ),
            Command::Timeline(TimelineCommand::RollToEnd),
            cx,
        ));
    }
    cluster
}

/// Window-wide capture while the rollback marker is dragged, so fast drags
/// and releases outside the strip still land.
fn marker_capture(cx: &mut Context<Studio>) -> impl IntoElement {
    let on_move = cx.listener(Studio::move_marker_drag);
    let on_up = cx.listener(Studio::end_marker_drag);
    canvas(
        |_, _, _| (),
        move |_, _, window, _| {
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture {
                    on_move(event, window, cx);
                }
            });
            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture {
                    on_up(event, window, cx);
                }
            });
        },
    )
    .absolute()
    .size_full()
}

/// The timeline for `slots::viewport_dock`, when a part object is active.
pub(crate) fn timeline(s: &Studio, t: Colors, cx: &mut Context<Studio>) -> Option<AnyElement> {
    let (object, part) = s.timeline_part()?;
    let width = f32::from(s.bounds.get().size.width);
    // Narrow viewports drop hints, then the part label and status text.
    let compact = width < 820.;
    let tight = width < 600.;
    let selected = s.selected_feature().map(|selected| selected.feature.id);
    let count = part.features.len();
    let marker = timeline::marker(part);
    let state = &s.panels.timeline;
    // Keep the selected chip in view when it is chosen and whenever chip
    // widths may change (an edit restamps, a regeneration adds a glyph).
    if let Some(index) = s.selected_feature().map(|selected| selected.index) {
        let key = (
            selected.unwrap_or(0),
            part.stamp,
            part.is_pending(),
            compact,
        );
        if state.reveal.take() || state.revealed.get() != Some(key) {
            state.revealed.set(Some(key));
            // The last chip reveals the end gap too, so the marker shows.
            state.scroll.scroll_to_item(if index + 1 == count {
                2 * count
            } else {
                2 * index + 1
            });
        }
    }
    // Fade the strip's clipped ends, so overflow reads as "more this way".
    let offset = f32::from(state.scroll.offset().x);
    let overflow = f32::from(state.scroll.max_offset().width);
    let clipped_left = offset < -0.5;
    let clipped_right = overflow > 0.5 && offset > -overflow + 0.5;
    let veil = gpui::rgba(t.hud);
    let mut clear_veil: Hsla = veil.into();
    clear_veil.a = 0.;
    let fade = |angle: f32| {
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .w(px(28.))
            .bg(linear_gradient(
                angle,
                linear_color_stop(veil, 0.),
                linear_color_stop(clear_veil, 1.),
            ))
    };
    let mut strip = row()
        .id("timeline-strip")
        .min_w(px(0.))
        .flex_shrink()
        .items_center()
        .overflow_x_scroll()
        .track_scroll(&state.scroll);
    for index in 0..=count {
        strip = strip.child(slot(t, s, index, count, marker, cx));
        if let Some(feature) = part.features.get(index) {
            strip = strip.child(chip(
                t,
                s,
                part,
                index,
                feature,
                selected == Some(feature.id),
                compact,
                cx,
            ));
        }
    }
    let name = s
        .scene
        .object(object)
        .map(|o| o.name.clone())
        .unwrap_or_default();
    let label = row()
        .id("timeline-part")
        .h(px(H_MD))
        .pl(px(SP_2))
        .pr(px(SP_1_5))
        .gap(px(SP_1_5))
        .max_w(px(160.))
        .flex_shrink_0()
        .rounded(px(R_SM + 1.))
        .cursor_pointer()
        .hover(move |d| d.bg(rgb(t.control_hover)))
        .child(icon(Icon::History, ICON_SM, t.accent_text))
        .child(
            div()
                .min_w(px(0.))
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_size(px(TEXT_SM))
                .font_weight(WEIGHT_MEDIUM)
                .text_color(rgb(t.text_muted))
                .child(name.clone()),
        )
        .tooltip(tip_detail(
            t,
            format!("{name} history"),
            "Click for the part summary · ← → step through features",
            None,
        ))
        .on_click(cx.listener(|s, _, w, cx| {
            cx.stop_propagation();
            s.execute(Command::Timeline(TimelineCommand::Deselect), w, cx);
        }));
    let dragging = state.marker.is_some();
    Some(
        hud(t)
            .id("feature-timeline")
            .relative()
            .items_center()
            .max_w(px((width - HUD_INSET * 2.).max(240.)))
            .when(!tight, |d| d.child(label).child(vrule(t, 18.)))
            .child(
                div()
                    .relative()
                    .flex()
                    .min_w(px(0.))
                    .flex_shrink()
                    .child(strip)
                    .when(clipped_left, |d| d.child(fade(90.).left_0()))
                    .when(clipped_right, |d| d.child(fade(270.).right_0())),
            )
            .child(vrule(t, 18.))
            .child(status(t, s, part, tight, cx))
            .when(dragging, |d| d.child(marker_capture(cx)))
            .into_any_element(),
    )
}

/// The feature context menu, opened by right-clicking a chip.
pub(super) fn context_menu(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> Option<AnyElement> {
    let menu = s.panels.timeline.menu.as_ref()?;
    let feature = s
        .scene
        .object_part(menu.selection.object)?
        .feature(menu.selection.feature)?;
    let entries = timeline::menu_entries(feature);
    let mut list = popover(t).w(px(212.)).gap(px(1.));
    list = list.child(
        row()
            .h(px(H_SM))
            .px(px(SP_2))
            .gap(px(SP_1_5))
            .child(icon(feature_icon(feature), 12., t.text_faint))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .text_size(px(TEXT_SM))
                    .font_weight(WEIGHT_MEDIUM)
                    .text_color(rgb(t.text_faint))
                    .child(feature.name.clone()),
            ),
    );
    for (index, entry) in entries.into_iter().enumerate() {
        let Some(entry) = entry else {
            list = list.child(
                div()
                    .my(px(SP_1))
                    .mx(px(SP_1))
                    .h(px(1.))
                    .bg(rgb(t.hairline)),
            );
            continue;
        };
        let active = index == menu.index;
        let ink = if entry.danger { t.danger } else { t.text };
        let command = entry.command;
        let glyph = match command {
            TimelineCommand::ToggleSuppress if feature.suppressed => Some(Icon::Eye),
            _ => timeline_icon(command),
        };
        list = list.child(
            row()
                .id(("timeline-menu-item", index))
                .h(px(H_MD))
                .px(px(SP_2))
                .gap(px(SP_2))
                .rounded(px(R_SM + 1.))
                .cursor_pointer()
                .bg(if active {
                    if entry.danger {
                        tint(t.danger, 0.12)
                    } else {
                        hex(t.control_hover)
                    }
                } else {
                    clear()
                })
                .child(match glyph {
                    Some(glyph) => icon(
                        glyph,
                        ICON_MD,
                        if entry.danger {
                            t.danger
                        } else if active {
                            t.text
                        } else {
                            t.text_muted
                        },
                    )
                    .into_any_element(),
                    None => div().w(px(ICON_MD)).into_any_element(),
                })
                .child(div().flex_1().text_color(rgb(ink)).child(entry.label))
                .when_some(entry.shortcut, |d, shortcut| d.child(keycaps(t, shortcut)))
                .on_hover(cx.listener(move |s, hovered, _, cx| {
                    if *hovered
                        && let Some(menu) = s.panels.timeline.menu.as_mut()
                        && menu.index != index
                    {
                        menu.index = index;
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |s, _, w, cx| {
                    cx.stop_propagation();
                    s.panels.timeline.menu = None;
                    s.execute(Command::Timeline(command), w, cx);
                })),
        );
    }
    let position = menu.position;
    Some(
        backdrop(
            t,
            "timeline-menu-backdrop",
            false,
            |s, _, cx| {
                s.panels.timeline.menu = None;
                cx.notify();
            },
            cx,
        )
        .child(
            anchored()
                .position(position)
                // Open upward: the timeline sits at the bottom of the viewport.
                .anchor(Corner::BottomLeft)
                .snap_to_window_with_margin(px(SP_2))
                .child(pop_in(contain(list.id("timeline-menu")), "timeline-menu")),
        )
        .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's prelude exports its own `test` attribute.
    use super::{Icon, chip_hint, feature_icon, hint_operation};
    use forma_core::{
        ExtrudeExtent, ExtrudeFeature, Feature, FeatureKind, FeatureOperation, Placement,
        PrimitiveFeature, PrimitiveShape, Sketch, SketchPlane, Vec3,
    };

    #[test]
    fn hints_are_compact_and_icons_follow_the_operation() {
        let boxed = Feature::new(
            1,
            "Plate",
            FeatureKind::Primitive(PrimitiveFeature {
                shape: PrimitiveShape::Box {
                    size: Vec3::new(2.2, 0.3, 1.5),
                },
                placement: Placement::default(),
                operation: FeatureOperation::Cut,
            }),
        );
        assert_eq!(chip_hint(&boxed), "2.2×0.3×1.5");
        assert_eq!(feature_icon(&boxed), Icon::Cube);
        assert_eq!(hint_operation(&boxed), Some(Icon::Subtract));
        let mut extrude = Feature::new(
            2,
            "Holes",
            FeatureKind::Extrude(ExtrudeFeature {
                sketch: Sketch::new(SketchPlane::ground()),
                profiles: Vec::new(),
                distance: 0.6,
                extent: ExtrudeExtent::Symmetric,
                flip: true,
                operation: FeatureOperation::Cut,
            }),
        );
        assert_eq!(chip_hint(&extrude), "±0.3 m");
        assert_eq!(feature_icon(&extrude), Icon::ExtrudeCut);
        assert_eq!(hint_operation(&extrude), None, "the icon already says cut");
        if let FeatureKind::Extrude(feature) = &mut extrude.kind {
            feature.extent = ExtrudeExtent::TwoSides { back_distance: 0.1 };
        }
        assert_eq!(chip_hint(&extrude), "0.6 / 0.1 m");
    }
}
