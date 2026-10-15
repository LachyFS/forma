//! Explicit units and scope for unitless mesh exchange.
use crate::app::Studio;
use gpui::{prelude::*, *};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum StlDirection {
    Import,
    Export,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum StlUnit {
    Millimetres,
    Centimetres,
    Metres,
    Inches,
}

impl StlUnit {
    pub(crate) const ALL: [Self; 4] = [
        Self::Millimetres,
        Self::Centimetres,
        Self::Metres,
        Self::Inches,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Millimetres => "Millimetres · mm",
            Self::Centimetres => "Centimetres · cm",
            Self::Metres => "Metres · m",
            Self::Inches => "Inches · in",
        }
    }

    pub(crate) fn metres(self) -> f32 {
        match self {
            Self::Millimetres => 0.001,
            Self::Centimetres => 0.01,
            Self::Metres => 1.,
            Self::Inches => 0.0254,
        }
    }

    pub(crate) fn cycle(self, backwards: bool) -> Self {
        let index = Self::ALL.iter().position(|&unit| unit == self).unwrap();
        Self::ALL[(index + if backwards { 3 } else { 1 }) % 4]
    }
}

#[derive(Clone, Copy)]
pub(crate) struct StlDialog {
    pub direction: StlDirection,
    pub unit: StlUnit,
    pub selected_only: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum MeshImport {
    Obj,
    Stl(StlUnit),
}

pub(crate) fn overlay(studio: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    let t = studio.theme_colors();
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
    let mut contents = div().flex().flex_col().gap(px(16.))
        .child(div().flex().justify_between()
            .child(div().text_size(px(18.)).child(if importing { "Import STL" } else { "Export STL" }))
            .child(div().text_size(px(10.)).text_color(rgb(t.muted)).child("ESC TO CANCEL")))
        .child(div().text_size(px(12.)).line_height(px(18.)).text_color(rgb(t.muted))
            .child(if importing {
                "STL files do not store units. Choose the unit used by coordinates in your file."
            } else {
                "Choose the unit your slicer or CAD application expects. Forma converts metre coordinates on export."
            }))
        .child(div().flex().flex_col().gap(px(6.)).children(StlUnit::ALL.chunks(2).enumerate().map(|(row, units)| {
            div().flex().gap(px(6.)).children(units.iter().copied().enumerate().map(|(column, unit)| {
                let active = unit == options.unit;
                div().id(("stl-unit", row * 2 + column)).flex_1().px(px(12.)).py(px(10.))
                    .rounded(px(5.)).border_1().border_color(rgb(if active { t.accent_line } else { t.edge }))
                    .bg(rgb(if active { t.active } else { t.card }))
                    .text_color(rgb(if active { t.accent } else { t.text }))
                    .cursor_pointer().hover(move |style| style.bg(rgb(t.active_hover)))
                    .child(unit.label())
                    .on_click(cx.listener(move |s, _, _, cx| {
                        if let Some(dialog) = s.stl_dialog.as_mut() { dialog.unit = unit; }
                        cx.notify();
                    }))
            }).collect::<Vec<_>>())
        }).collect::<Vec<_>>()));
    if !importing {
        contents = contents.child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(t.muted))
                        .child("EXPORT SCOPE"),
                )
                .children(
                    [
                        (
                            true,
                            format!(
                                "Selection · {selected} mesh {}",
                                if selected == 1 { "object" } else { "objects" }
                            ),
                        ),
                        (
                            false,
                            format!(
                                "Visible scene · {visible} mesh {}",
                                if visible == 1 { "object" } else { "objects" }
                            ),
                        ),
                    ]
                    .into_iter()
                    .map(|(selected_only, label)| {
                        let active = selected_only == options.selected_only;
                        div()
                            .id(("stl-scope", usize::from(selected_only)))
                            .px(px(12.))
                            .py(px(9.))
                            .rounded(px(5.))
                            .border_1()
                            .border_color(rgb(if active { t.accent_line } else { t.edge }))
                            .bg(rgb(if active { t.active } else { t.card }))
                            .text_color(rgb(if active { t.accent } else { t.text }))
                            .cursor_pointer()
                            .hover(move |style| style.bg(rgb(t.active_hover)))
                            .child(label)
                            .on_click(cx.listener(move |s, _, _, cx| {
                                if let Some(dialog) = s.stl_dialog.as_mut() {
                                    dialog.selected_only = selected_only;
                                }
                                cx.notify();
                            }))
                    })
                    .collect::<Vec<_>>(),
                ),
        );
    }
    let empty = !importing
        && if options.selected_only {
            selected == 0
        } else {
            visible == 0
        };
    contents = contents
        .child(div().text_size(px(11.)).line_height(px(17.)).text_color(rgb(t.muted))
            .child(if importing { "Axes are kept as supplied; Forma uses Y up. Facets become one editable mesh, without materials." }
                else { "Exports triangles with transforms applied. Axes are preserved (Y up); materials are omitted." }))
        .when(empty, |d| d.child(div().text_size(px(11.)).text_color(rgb(t.alert)).child("No visible mesh objects in this scope. Choose another scope or cancel.")))
        .child(div().text_size(px(10.)).text_color(rgb(t.muted))
            .child(if importing { "← → Choose units · Enter continue" } else { "← → Choose units · Tab change scope · Enter continue" }))
        .child(div().flex().justify_end().gap(px(8.))
            .child(div().id("stl-cancel").px(px(14.)).py(px(9.)).rounded(px(5.))
                .bg(rgb(t.raised)).cursor_pointer().child("Cancel")
                .on_click(cx.listener(|s, _, _, cx| { s.stl_dialog = None; cx.notify(); })))
            .child(div().id("stl-confirm").px(px(14.)).py(px(9.)).rounded(px(5.))
                .bg(rgb(if empty { t.raised } else { t.active }))
                .text_color(rgb(if empty { t.muted } else { t.accent }))
                .when(!empty, |d| d.cursor_pointer().on_click(cx.listener(|s, _, _, cx| s.confirm_stl_dialog(cx))))
                .child(if importing { "Choose file…" } else { "Choose destination…" })));
    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(rgba(0x080b0dbf))
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|s, _, _, cx| {
                s.stl_dialog = None;
                cx.notify();
            }),
        )
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .id("stl-options")
                .w(px(440.))
                .max_w(relative(0.92))
                .max_h(relative(0.9))
                .overflow_y_scroll()
                .p(px(20.))
                .rounded(px(10.))
                .border_1()
                .border_color(rgb(t.edge))
                .bg(rgb(t.panel))
                .text_color(rgb(t.text))
                .text_size(px(12.))
                .shadow_lg()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(contents),
        )
        .into_any_element()
}
