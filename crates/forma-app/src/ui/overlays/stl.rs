//! STL import/export options: units and, for export, scope.
use super::super::assets::{Icon, icon};
use super::super::components::*;
use super::super::tokens::*;
use super::{animate_in, backdrop, contain};
use crate::app::Studio;
use crate::exchange::{StlDirection, StlUnit};
use crate::theme::Colors;
use gpui::{prelude::*, *};

/// Radio-style choice card.
fn choice(t: Colors, id: impl Into<ElementId>, active: bool) -> Stateful<Div> {
    row()
        .id(id.into())
        .min_h(px(H_LG + 4.))
        .px(px(SP_3))
        .gap(px(SP_2 + 2.))
        .rounded(px(R_MD))
        .border_1()
        .border_color(rgb(if active { t.focus } else { t.border }))
        .bg(rgb(if active { t.accent_soft } else { t.raised }))
        .cursor_pointer()
        .hover(move |d| {
            if active {
                d
            } else {
                d.bg(rgb(t.control_hover))
            }
        })
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(14.))
                .flex_shrink_0()
                .rounded_full()
                .border_1()
                .border_color(rgb(if active { t.accent } else { t.border_strong }))
                .when(active, |d| {
                    d.child(div().size(px(6.)).rounded_full().bg(rgb(t.accent)))
                }),
        )
}

pub(super) fn overlay(t: Colors, studio: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    let options = studio.stl_dialog.unwrap();
    let importing = options.direction == StlDirection::Import;
    let selected = studio
        .selected_ids()
        .iter()
        .filter(|&&id| studio.scene.object_mesh(id).is_some())
        .count();
    let visible = studio
        .scene
        .mesh_instances()
        .filter(|mesh| studio.scene.is_effectively_visible(mesh.id))
        .count();
    let units = div().grid().grid_cols(2).gap(px(SP_1 + 2.)).children(
        StlUnit::ALL
            .iter()
            .copied()
            .enumerate()
            .map(|(index, unit)| {
                let active = unit == options.unit;
                choice(t, ("stl-unit", index), active)
                    .child(
                        div()
                            .text_color(rgb(if active { t.accent_text } else { t.text }))
                            .font_weight(WEIGHT_MEDIUM)
                            .child(unit.label()),
                    )
                    .on_click(cx.listener(move |s, _, _, cx| {
                        if let Some(dialog) = s.stl_dialog.as_mut() {
                            dialog.unit = unit;
                        }
                        cx.notify();
                    }))
            }),
    );
    let plural = |count: usize| if count == 1 { "object" } else { "objects" };
    let scopes = col().gap(px(SP_1 + 2.)).children(
        [
            (
                true,
                "Selection",
                format!("{selected} mesh {}", plural(selected)),
            ),
            (
                false,
                "Visible scene",
                format!("{visible} mesh {}", plural(visible)),
            ),
        ]
        .into_iter()
        .map(|(selected_only, label, detail)| {
            let active = selected_only == options.selected_only;
            choice(t, ("stl-scope", usize::from(selected_only)), active)
                .child(
                    div()
                        .flex_1()
                        .text_color(rgb(if active { t.accent_text } else { t.text }))
                        .font_weight(WEIGHT_MEDIUM)
                        .child(label),
                )
                .child(caption(t, detail).tabular())
                .on_click(cx.listener(move |s, _, _, cx| {
                    if let Some(dialog) = s.stl_dialog.as_mut() {
                        dialog.selected_only = selected_only;
                    }
                    cx.notify();
                }))
        }),
    );
    let empty = !importing
        && if options.selected_only {
            selected == 0
        } else {
            visible == 0
        };
    let label = |text: &'static str| {
        div()
            .text_size(px(TEXT_SM))
            .font_weight(WEIGHT_SEMIBOLD)
            .text_color(rgb(t.text_muted))
            .child(text)
    };
    let body = col()
        .px(px(SP_4))
        .pb(px(SP_4))
        .gap(px(SP_2))
        .child(label("Units"))
        .child(units)
        .when(!importing, |d| d.child(div().h(px(SP_1))).child(label("Scope")).child(scopes))
        .child(
            div()
                .pt(px(SP_2))
                .text_size(px(TEXT_SM))
                .line_height(px(17.))
                .text_color(rgb(t.text_muted))
                .child(if importing {
                    "Axes are kept as supplied; Forma uses Y up. Facets become one editable mesh, without materials."
                } else {
                    "Exports triangles with transforms applied. Axes are preserved (Y up); materials are omitted."
                }),
        )
        .when(empty, |d| {
            d.child(
                row()
                    .gap(px(SP_2))
                    .p(px(SP_2 + 2.))
                    .rounded(px(R_MD))
                    .bg(tint(t.warning, 0.1))
                    .text_size(px(TEXT_SM))
                    .text_color(rgb(t.warning))
                    .child(icon(Icon::Warning, ICON_SM, t.warning))
                    .child("No visible mesh objects in this scope. Choose another scope or cancel."),
            )
        });
    let footer = dialog_footer(t)
        .child(hint(t, "← →", "Units"))
        .when(!importing, |d| d.child(hint(t, "Tab", "Scope")))
        .child(spacer())
        .child(
            button(t, "stl-cancel", "Cancel", None, Tone::Secondary).on_click(cx.listener(
                |s, _, _, cx| {
                    s.stl_dialog = None;
                    cx.notify();
                },
            )),
        )
        .child(
            button(
                t,
                "stl-confirm",
                if importing {
                    "Choose file…"
                } else {
                    "Choose destination…"
                },
                Some(if importing {
                    Icon::Import
                } else {
                    Icon::Export
                }),
                Tone::Primary,
            )
            .when(empty, |d| disabled(d).cursor_default())
            .when(!empty, |d| {
                d.on_click(cx.listener(|s, _, _, cx| s.confirm_stl_dialog(cx)))
            }),
        );
    backdrop(
        t,
        "stl-backdrop",
        true,
        |s, _, cx| {
            s.stl_dialog = None;
            cx.notify();
        },
        cx,
    )
    .flex()
    .items_center()
    .justify_center()
    .child(animate_in(
        contain(
            dialog(t)
                .id("stl-options")
                .w(px(460.))
                .max_w(relative(0.92))
                .max_h(relative(0.9))
                .child(dialog_header(
                    t,
                    Some(if importing { Icon::Import } else { Icon::Export }),
                    if importing { "Import STL" } else { "Export STL" },
                    Some(
                        if importing {
                            "STL files do not store units. Choose the unit your file's coordinates use."
                        } else {
                            "Choose the unit your slicer or CAD application expects. Forma converts from metres."
                        }
                        .into(),
                    ),
                ))
                .child(body)
                .child(footer),
        ),
        "stl-enter",
    ))
    .into_any_element()
}
