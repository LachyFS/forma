//! Floating viewport controls. The viewport fills its region edge to edge;
//! tools, filters and view controls float above it in translucent clusters,
//! leaving the scene as large as possible.
use super::assets::{Icon, icon};
use super::commands::shading_icon;
use super::components::*;
use super::tokens::*;
use super::{Anchor, MenuKind, VIEWPORT_MIN_WIDTH, slots};
use crate::app::{Command, EditMode, Studio, Tool};
use crate::theme::Colors;
use forma_core::Primitive;
use forma_render::RenderMode;
use gpui::{prelude::*, *};

/// Distance between floating clusters and the viewport edge.
pub(crate) const HUD_INSET: f32 = 12.;
const TOOL: f32 = H_LG;

fn tool_palette(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> Div {
    let menu_open = s
        .panels
        .menu
        .as_ref()
        .is_some_and(|menu| menu.kind == MenuKind::Add);
    let separator = || {
        div()
            .mx(px(SP_1_5))
            .my(px(3.))
            .h(px(1.))
            .bg(rgba(t.hud_border))
    };
    let mut palette = hud(t).flex_col().items_stretch().children(
        [
            (Tool::Select, Icon::Select),
            (Tool::Move, Icon::Move),
            (Tool::Rotate, Icon::Rotate),
            (Tool::Scale, Icon::Scale),
        ]
        .into_iter()
        .map(|(tool, glyph)| {
            command_icon(
                t,
                SharedString::from(format!("tool-{tool:?}")),
                glyph,
                Command::SetTool(tool),
                s.tool == tool,
                TOOL,
                cx,
            )
        }),
    );
    palette = palette.child(separator());
    palette = if s.edit_mode.is_component() {
        palette
            .child(command_icon(
                t,
                "rail-extrude",
                Icon::Extrude,
                Command::Extrude,
                false,
                TOOL,
                cx,
            ))
            .child(command_icon(
                t,
                "rail-inset",
                Icon::Inset,
                Command::Inset,
                false,
                TOOL,
                cx,
            ))
    } else {
        palette.child(command_icon(
            t,
            "rail-bevel",
            Icon::Bevel,
            Command::Bevel,
            false,
            TOOL,
            cx,
        ))
    };
    palette.child(separator()).child(
        icon_button(t, "tool-add", Icon::Plus, menu_open, TOOL)
            .relative()
            .child(s.panels.anchor_cell(Anchor::ToolRail).element())
            .tooltip(tip_detail(t, "Add", "Primitives and imported meshes", None))
            .on_click(cx.listener(|s, _, _, cx| {
                cx.stop_propagation();
                s.open_menu(MenuKind::Add, Anchor::ToolRail, cx);
            })),
    )
}

/// Vertex / edge / face filter, shown while editing components.
fn component_filter(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> Div {
    hud(t).children(
        [
            (EditMode::Vertex, Icon::Vertex, "Vertex"),
            (EditMode::Edge, Icon::Edge, "Edge"),
            (EditMode::Face, Icon::Face, "Face"),
        ]
        .into_iter()
        .map(|(mode, glyph, label)| {
            let active = s.edit_mode == mode;
            on_command(
                button_base(
                    t,
                    SharedString::from(format!("select-{label}")),
                    if active { Tone::Selected } else { Tone::Ghost },
                    H_MD,
                )
                .px(px(SP_2))
                .font_weight(WEIGHT_MEDIUM)
                .child(icon(
                    glyph,
                    ICON_MD,
                    if active { t.accent_text } else { t.text_muted },
                ))
                .child(label)
                .tooltip(command_tip(t, Command::SetEditMode(mode))),
                Command::SetEditMode(mode),
                cx,
            )
        }),
    )
}

/// Short name of the current view for the views menu button.
pub(crate) fn view_name(camera: forma_core::Camera) -> &'static str {
    match crate::viewport::camera_view_label(camera) {
        "User perspective" => "Perspective",
        "User orthographic" => "Orthographic",
        "Front orthographic" => "Front",
        "Back orthographic" => "Back",
        "Right orthographic" => "Right",
        "Left orthographic" => "Left",
        "Top orthographic" => "Top",
        "Bottom orthographic" => "Bottom",
        other => other,
    }
}

fn view_controls(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> Div {
    let menu_open = s
        .panels
        .menu
        .as_ref()
        .is_some_and(|menu| menu.kind == MenuKind::View);
    let ortho = s.scene.camera.orthographic;
    let views = hud(t)
        .child(
            button_base(
                t,
                "view-menu",
                if menu_open {
                    Tone::Selected
                } else {
                    Tone::Ghost
                },
                H_MD,
            )
            .relative()
            .px(px(SP_2))
            .font_weight(WEIGHT_MEDIUM)
            .child(icon(Icon::Views, ICON_MD, t.text_muted))
            .child(
                div()
                    .min_w(px(78.))
                    .text_color(rgb(t.text))
                    .child(view_name(s.scene.camera)),
            )
            .child(icon(Icon::ChevronDown, ICON_SM, t.text_faint))
            .child(s.panels.anchor_cell(Anchor::ViewHud).element())
            .tooltip(tip_detail(
                t,
                "Views",
                "Front, side and top views · numpad-style keys 1, 3, 7",
                None,
            ))
            .on_click(cx.listener(|s, _, _, cx| {
                cx.stop_propagation();
                s.open_menu(MenuKind::View, Anchor::ViewHud, cx);
            })),
        )
        .child(command_icon(
            t,
            "projection",
            if ortho {
                Icon::Orthographic
            } else {
                Icon::Perspective
            },
            Command::ToggleProjection,
            false,
            H_MD,
            cx,
        ));
    let display = hud(t)
        .child(command_icon(
            t,
            "frame-selected",
            Icon::FrameSelected,
            Command::FrameSelected,
            false,
            H_MD,
            cx,
        ))
        .child(command_icon(
            t,
            "frame-all",
            Icon::FrameAll,
            Command::FrameAll,
            false,
            H_MD,
            cx,
        ))
        .child(command_icon(
            t,
            "toggle-grid",
            Icon::Grid,
            Command::ToggleGrid,
            s.settings.show_grid,
            H_MD,
            cx,
        ))
        .child(command_icon(
            t,
            "toggle-measurements",
            Icon::Measure,
            Command::ToggleMeasurements,
            s.show_measurements,
            H_MD,
            cx,
        ));
    let mode = s.settings.mode;
    let shading = hud(t)
        .children(
            [
                RenderMode::Wireframe,
                RenderMode::Solid,
                RenderMode::MaterialPreview,
                RenderMode::Rendered,
            ]
            .into_iter()
            .map(|candidate| {
                command_icon(
                    t,
                    SharedString::from(format!("render-mode-{}", candidate as usize)),
                    shading_icon(candidate),
                    Command::SetMode(candidate),
                    mode == candidate,
                    H_MD,
                    cx,
                )
            }),
        )
        .when(mode == RenderMode::MaterialPreview, |d| {
            d.child(div().w(px(1.)).h(px(16.)).mx(px(2.)).bg(rgba(t.hud_border)))
                .child(command_icon(
                    t,
                    "shading-lighting",
                    Icon::Lighting,
                    Command::TogglePreviewSettings,
                    s.preview_open,
                    H_MD,
                    cx,
                ))
        });
    row()
        .gap(px(SP_1_5))
        .child(views)
        .child(display)
        .child(shading)
}

/// Readout for a modal operation: tool, constraint, typed value and keys.
fn operation_hud(t: Colors, s: &Studio) -> Option<Div> {
    let drag = s.transform_drag.as_ref()?;
    let (glyph, label) = match drag.mesh_tool {
        Some(crate::modelling::MeshTool::Extrude) => (Icon::Extrude, "Extrude"),
        Some(crate::modelling::MeshTool::Inset) => (Icon::Inset, "Inset"),
        Some(crate::modelling::MeshTool::Bevel) => (Icon::Bevel, "Bevel"),
        None => match drag.tool {
            Tool::Rotate => (Icon::Rotate, "Rotate"),
            Tool::Scale => (Icon::Scale, "Scale"),
            _ => (Icon::Move, "Move"),
        },
    };
    let constraint = match drag.mesh_tool {
        Some(crate::modelling::MeshTool::Bevel) => "All edges".to_owned(),
        Some(crate::modelling::MeshTool::Inset) => "Face region".to_owned(),
        _ => drag.constraint.label(),
    };
    Some(
        hud(t)
            .flex_col()
            .items_stretch()
            .p(px(SP_1))
            .gap(px(SP_1))
            .child(
                row()
                    .h(px(H_MD))
                    .px(px(SP_1_5))
                    .gap(px(SP_2))
                    .child(icon(glyph, ICON_MD, t.accent_text))
                    .child(
                        div()
                            .font_weight(WEIGHT_SEMIBOLD)
                            .text_color(rgb(t.text))
                            .child(label),
                    )
                    .child(badge(t, constraint, BadgeTone::Accent))
                    .child(
                        row()
                            .min_w(px(72.))
                            .h(px(H_SM - 2.))
                            .px(px(SP_2))
                            .rounded(px(R_SM))
                            .bg(rgb(t.field))
                            .border_1()
                            .border_color(rgb(if drag.numeric.is_empty() {
                                t.hairline
                            } else {
                                t.focus
                            }))
                            .tabular()
                            .text_color(rgb(if drag.numeric.is_empty() {
                                t.text_faint
                            } else {
                                t.text
                            }))
                            .child(if drag.numeric.is_empty() {
                                "Type a value".to_owned()
                            } else {
                                drag.numeric.clone()
                            }),
                    )
                    .child(div().w(px(SP_2)))
                    .child(hint(t, "Return", "Apply"))
                    .child(hint(t, "Esc", "Cancel")),
            )
            .when_some(drag.error.clone(), |d: Div, error: String| {
                d.child(
                    row()
                        .px(px(SP_1_5))
                        .pb(px(SP_1))
                        .gap(px(SP_1_5))
                        .text_size(px(TEXT_SM))
                        .text_color(rgb(t.danger))
                        .child(icon(Icon::Error, ICON_SM, t.danger))
                        .child(error),
                )
            }),
    )
}

/// World dimensions and edge lengths, while measurements are on.
fn measurements_card(t: Colors, s: &Studio) -> Option<Div> {
    if !s.show_measurements {
        return None;
    }
    let measurements = s.selection_measurements();
    let mut card = hud(t)
        .flex_col()
        .items_stretch()
        .min_w(px(176.))
        .px(px(SP_3))
        .py(px(SP_2 + 2.))
        .gap(px(SP_1_5))
        .child(
            row()
                .gap(px(SP_1_5))
                .child(icon(Icon::Measure, ICON_SM, t.text_muted))
                .child(
                    div()
                        .flex_1()
                        .font_weight(WEIGHT_MEDIUM)
                        .text_color(rgb(t.text))
                        .child("World dimensions"),
                )
                .child(div().text_color(rgb(t.text_faint)).child("m")),
        );
    if let Some(bounds) = measurements.bounds {
        let size = bounds.dimensions();
        card = card.children((0..3).map(|axis| {
            row()
                .gap(px(SP_2))
                .child(
                    div()
                        .w(px(2.))
                        .h(px(12.))
                        .rounded_full()
                        .bg(rgb(t.axis[axis])),
                )
                .child(
                    div()
                        .w(px(12.))
                        .text_color(rgb(t.axis_text[axis]))
                        .child(["X", "Y", "Z"][axis]),
                )
                .child(
                    div()
                        .flex_1()
                        .text_right()
                        .tabular()
                        .text_color(rgb(t.text))
                        .child(crate::viewport::measurement_value(size[axis] as f64)),
                )
        }));
        if measurements.edge_count > 0 {
            card = card.child(hrule(t).bg(rgba(t.hud_border))).child(
                row()
                    .gap(px(SP_2))
                    .text_color(rgb(t.text_muted))
                    .child(format!(
                        "{} edge{}",
                        measurements.edge_count,
                        if measurements.edge_count == 1 {
                            ""
                        } else {
                            "s"
                        }
                    ))
                    .child(spacer())
                    .child(
                        div()
                            .tabular()
                            .text_color(rgb(t.accent_text))
                            .child(crate::viewport::measurement_value(measurements.edge_length)),
                    ),
            );
        }
    } else {
        card = card.child(
            div()
                .text_color(rgb(t.text_muted))
                .child("Select geometry to measure"),
        );
    }
    Some(card)
}

/// Guidance and quick starts when the scene has no objects.
fn empty_scene(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> Option<Div> {
    if !s.scene.objects.is_empty() || s.frame.is_none() {
        return None;
    }
    Some(
        hud(t)
            .flex_col()
            .items_center()
            .px(px(SP_6))
            .py(px(SP_5))
            .gap(px(SP_3))
            .child(empty_state(
                t,
                Icon::Cube,
                "Empty scene",
                "Add a primitive to start modelling, or import an OBJ or STL mesh.",
            ))
            .child(
                row().gap(px(SP_1)).children(
                    [
                        (Primitive::Cube, Icon::Cube, "Cube"),
                        (Primitive::Sphere, Icon::Sphere, "Sphere"),
                        (Primitive::Cylinder, Icon::Cylinder, "Cylinder"),
                        (Primitive::Plane, Icon::Plane, "Plane"),
                    ]
                    .into_iter()
                    .map(|(primitive, glyph, label)| {
                        command_button(
                            t,
                            SharedString::from(format!("empty-add-{label}")),
                            label,
                            Some(glyph),
                            Command::Add(primitive),
                            Tone::Secondary,
                            cx,
                        )
                    }),
                ),
            )
            .child(
                row()
                    .relative()
                    .gap(px(SP_1))
                    .child(command_button(
                        t,
                        "empty-import-obj",
                        "Import OBJ…",
                        Some(Icon::Import),
                        Command::ImportObj,
                        Tone::Ghost,
                        cx,
                    ))
                    .child(command_button(
                        t,
                        "empty-import-stl",
                        "Import STL…",
                        Some(Icon::Import),
                        Command::ImportStl,
                        Tone::Ghost,
                        cx,
                    )),
            ),
    )
}

/// The viewport region: the GPU canvas plus every floating cluster and the
/// reserved slots from `slots`.
pub(super) fn viewport_area(
    t: Colors,
    s: &Studio,
    viewport: AnyElement,
    cx: &mut Context<Studio>,
) -> impl IntoElement {
    let tools = tool_palette(t, s, cx);
    let filter = s
        .edit_mode
        .is_component()
        .then(|| component_filter(t, s, cx));
    let views = view_controls(t, s, cx);
    let operation = operation_hud(t, s);
    let dock = slots::viewport_dock(s, t, cx);
    let adjust = slots::adjust_last_operation(s, t, cx);
    let measurements = measurements_card(t, s);
    let empty = empty_scene(t, s, cx);
    div()
        .relative()
        .flex_1()
        .min_w(px(VIEWPORT_MIN_WIDTH))
        .h_full()
        .overflow_hidden()
        .bg(rgb(t.sunken))
        .child(viewport)
        .child(
            div()
                .absolute()
                .top(px(HUD_INSET))
                .left(px(HUD_INSET))
                .child(tools),
        )
        .when_some(filter, |d, filter| {
            d.child(
                div()
                    .absolute()
                    .top(px(HUD_INSET))
                    .left(px(HUD_INSET + TOOL + 8. + SP_2))
                    .child(filter),
            )
        })
        .child(
            div()
                .absolute()
                .top(px(HUD_INSET))
                .right(px(HUD_INSET))
                .child(views),
        )
        .when_some(empty, |d, empty| {
            d.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(empty),
            )
        })
        .when(operation.is_some() || dock.is_some(), |d| {
            d.child(
                col()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(px(HUD_INSET))
                    .items_center()
                    .gap(px(SP_2))
                    .children(operation)
                    .children(dock),
            )
        })
        .when_some(adjust, |d, adjust| {
            d.child(
                div()
                    .absolute()
                    .left(px(HUD_INSET))
                    .bottom(px(HUD_INSET))
                    .child(adjust),
            )
        })
        .when_some(measurements, |d, card| {
            d.child(
                div()
                    .absolute()
                    .right(px(HUD_INSET))
                    .bottom(px(HUD_INSET))
                    .child(card),
            )
        })
}
