//! Keyboard and pointer reference sheet.
use super::super::assets::Icon;
use super::super::components::*;
use super::super::tokens::*;
use super::{animate_in, backdrop, contain, float_border, float_shadow};
use crate::app::{Command, Studio};
use crate::platform_shortcut;
use crate::theme::Colors;
use gpui::{prelude::*, *};

type Group = (&'static str, Vec<(&'static str, &'static str)>);

fn groups() -> [Vec<Group>; 3] {
    [
        vec![
            (
                "Navigate",
                vec![
                    ("Orbit", "MMB drag"),
                    ("Pan", "⇧ MMB drag"),
                    ("Zoom", "Ctrl MMB drag"),
                    ("Orbit", "two finger drag"),
                    ("Pan", "⇧ two finger drag"),
                    ("Zoom", platform_shortcut("Pinch / Wheel", "Wheel")),
                    ("Focus selection", "` / . / F"),
                    ("Frame all", "Home"),
                    ("Front / right / top", "1 / 3 / 7"),
                    (
                        "Back / left / bottom",
                        platform_shortcut("⌥ 1 / 3 / 7", "Alt 1 / 3 / 7"),
                    ),
                    ("Perspective / orthographic", "5"),
                    ("Reset perspective", "0"),
                ],
            ),
            (
                "Shading",
                vec![
                    ("Shading pie", "Z"),
                    ("Modes inside the pie", "4 6 2 8"),
                    ("Solid / material / rendered", "X / C / V"),
                    ("Measurements", "⇧ M"),
                ],
            ),
        ],
        vec![
            (
                "Select",
                vec![
                    ("Select", "Click"),
                    ("Extend selection", "⇧ Click"),
                    ("All / none", platform_shortcut("A / ⌥ A", "A / Alt A")),
                    ("Invert", platform_shortcut("⌘ I", "Ctrl I")),
                    ("Linked components", "L"),
                    ("Box select", "B"),
                    ("Object / edit mode", "Tab"),
                    ("Vertex / edge / face", "1 / 2 / 3"),
                ],
            ),
            (
                "Transform",
                vec![
                    ("Select tool", "Q"),
                    ("Move / rotate / scale", "G / R / S"),
                    ("Axis constraint", "X / Y / Z"),
                    ("Plane constraint", "⇧ X / Y / Z"),
                    ("Snap / precision", "Ctrl / ⇧"),
                    ("Apply / cancel", "Return / Esc"),
                ],
            ),
            (
                "Part",
                vec![
                    ("Extrude / revolve a sketch", "E / ⇧ R"),
                    ("Push/pull a face", "P"),
                    ("Edit faces and edges", "Tab"),
                    ("Faces / edges", "3 / 2"),
                    ("Chamfer selected edges", platform_shortcut("⌘ B", "Ctrl B")),
                    ("Sketch on selected face", "D"),
                ],
            ),
        ],
        vec![
            (
                "Model",
                vec![
                    ("Extrude / inset", "E / I"),
                    ("Bevel convex edges", platform_shortcut("⌘ B", "Ctrl B")),
                    ("Duplicate", "⇧ D"),
                    ("Delete", "Delete"),
                ],
            ),
            (
                "Draw",
                vec![
                    ("New sketch / edit sketch", "D / Return"),
                    ("Line / rectangle / circle", "L / R / C"),
                    ("Arc / polygon / trim", "A / P / T"),
                    ("Construction", "X"),
                    ("Exact value / next value", "0–9 / Tab"),
                    ("Place without snapping", "Ctrl"),
                    ("Cancel step, then finish", "Esc"),
                ],
            ),
            (
                "Workspace",
                vec![
                    ("Commands", platform_shortcut("⌘ K", "Ctrl K")),
                    ("Undo", platform_shortcut("⌘ Z", "Ctrl Z")),
                    ("Redo", platform_shortcut("⇧ ⌘ Z", "Ctrl ⇧ Z")),
                    (
                        "Save / open",
                        platform_shortcut("⌘ S / ⌘ O", "Ctrl S / Ctrl O"),
                    ),
                    ("Color theme", platform_shortcut("⇧ ⌘ T", "Ctrl ⇧ T")),
                    ("This reference", "?"),
                ],
            ),
        ],
    ]
}

pub(super) fn overlay(t: Colors, cx: &mut Context<Studio>) -> AnyElement {
    let columns = groups().into_iter().map(|groups| {
        col()
            .flex_1()
            .min_w(px(0.))
            .gap(px(SP_4))
            .children(groups.into_iter().map(|(title, keys)| {
                col()
                    .child(
                        div()
                            .pb(px(SP_1_5))
                            .text_size(px(TEXT_SM))
                            .font_weight(WEIGHT_SEMIBOLD)
                            .text_color(rgb(t.accent_text))
                            .child(title),
                    )
                    .children(keys.into_iter().map(|(label, shortcut)| {
                        row()
                            .h(px(26.))
                            .gap(px(SP_3))
                            .border_b_1()
                            .border_color(rgb(t.hairline))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_color(rgb(t.text))
                                    .child(label),
                            )
                            .child(keycaps(t, shortcut))
                    }))
            }))
    });
    backdrop(
        t,
        "help-backdrop",
        true,
        |s, w, cx| s.execute(Command::ToggleHelp, w, cx),
        cx,
    )
    .flex()
    .items_center()
    .justify_center()
    .child(animate_in(
        contain(
            dialog(t)
                .id("help-panel")
                .w(px(860.))
                .max_w(relative(0.94))
                .border_color(float_border(t))
                .shadow(float_shadow(t))
                .max_h(relative(0.9))
                .child(dialog_header(
                    t,
                    Some(Icon::Keyboard),
                    "Keyboard shortcuts",
                    Some(
                        platform_shortcut(
                            "The viewport needs focus. ⌘ is Command, ⌥ is Option, ⇧ is Shift.",
                            "The viewport needs focus. ⇧ is Shift.",
                        )
                        .into(),
                    ),
                ))
                .child(
                    div()
                        .id("help-scroll")
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scroll()
                        .px(px(SP_5))
                        .pb(px(SP_5))
                        .pt(px(SP_1))
                        .child(row().items_start().gap(px(SP_6)).children(columns)),
                ),
        ),
        "help-enter",
    ))
    .into_any_element()
}
