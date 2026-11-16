//! Inspector: a fixed header with the selection's name, then two pages —
//! the selection's properties and the viewport/render settings — built from
//! collapsible sections and label/value property rows.
use super::assets::{Icon, icon};
use super::components::*;
use super::outliner::{kind_icon, kind_label, object_kind};
use super::tokens::*;
use super::{Anchor, InspectorTab, MenuKind, OpenMenu, PanelSection};
use crate::app::{Command, Field, Studio};
use crate::theme::Colors;
use forma_core::{DenoiseQuality, ShaderKind, TextureMapping, TextureSlot};
use forma_render::RenderMode;
use gpui::{prelude::*, *};

/// Below this sidebar width, vector rows put their label above the fields.
const STACK_BELOW: f32 = 284.;

/// Surface presets: swatch color, name, and the linear base color they apply.
const PRESETS: [(u32, &str, [f32; 3]); 5] = [
    (0x83b7a6, "Jade", [0.045, 0.42, 0.32]),
    (0xe4e4dc, "Porcelain", [0.73, 0.76, 0.73]),
    (0xc38967, "Copper", [0.76, 0.32, 0.13]),
    (0x525860, "Graphite", [0.055, 0.055, 0.055]),
    (0xf3e9c9, "Light", [0.9, 0.83, 0.66]),
];

pub(crate) fn swatch_color(v: glam::Vec3) -> u32 {
    fn channel(v: f32) -> u32 {
        let v = v.clamp(0., 1.);
        let v = if v <= 0.0031308 {
            v * 12.92
        } else {
            1.055 * v.powf(1. / 2.4) - 0.055
        };
        (v * 255.).round() as u32
    }
    channel(v.x) << 16 | channel(v.y) << 8 | channel(v.z)
}

fn header(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> Div {
    let object = s.selected.and_then(|id| s.scene.object(id));
    let selected = s.selected_ids().len();
    let body = match object {
        Some(object) => {
            let kind = object_kind(&s.scene, object);
            let inline = !(s.panels.rename_in_outliner && s.field_is_active(Field::Name));
            row()
                .gap(px(SP_2))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(H_MD))
                        .flex_shrink_0()
                        .rounded(px(R_MD))
                        .bg(rgb(t.accent_soft))
                        .child(icon(kind_icon(kind), ICON_MD, t.accent_text)),
                )
                .child(if inline {
                    numeric_field(
                        t,
                        s,
                        "object-name",
                        Field::Name,
                        FieldMark::None,
                        "Name",
                        cx,
                    )
                    .h(px(H_MD))
                    .flex_1()
                    .min_w(px(0.))
                    .text_size(px(TEXT_LG))
                    .font_weight(WEIGHT_MEDIUM)
                    .into_any_element()
                } else {
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .px(px(SP_2))
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .text_size(px(TEXT_LG))
                        .font_weight(WEIGHT_MEDIUM)
                        .child(object.name.clone())
                        .into_any_element()
                })
                .child(if selected > 1 {
                    badge(t, format!("{selected} selected"), BadgeTone::Accent)
                } else {
                    badge(t, kind_label(kind), BadgeTone::Neutral)
                })
        }
        None => row()
            .gap(px(SP_2))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(H_MD))
                    .rounded(px(R_MD))
                    .bg(rgb(t.control))
                    .child(icon(Icon::Select, ICON_MD, t.text_faint)),
            )
            .child(
                div()
                    .flex_1()
                    .text_size(px(TEXT_LG))
                    .font_weight(WEIGHT_MEDIUM)
                    .text_color(rgb(t.text_muted))
                    .child("No selection"),
            ),
    };
    let tab = s.panels.tab;
    col()
        .flex_shrink_0()
        .border_b_1()
        .border_color(rgb(t.hairline))
        .child(body.h(px(48.)).px(px(SP_3)))
        .child(
            row()
                .h(px(30.))
                .px(px(SP_2))
                .gap(px(SP_2))
                .child(tab_button(t, InspectorTab::Object, "Object", tab, cx))
                .child(tab_button(t, InspectorTab::Render, "Render", tab, cx)),
        )
}

// ---------------------------------------------------------------------------
// Context blocks: the top of the Object page says what the user is doing
// (editing a part's faces or edges, editing a mesh) and offers its actions.

fn context_block(t: Colors) -> Div {
    col()
        .flex_shrink_0()
        .px(px(SP_3))
        .pt(px(SP_3))
        .pb(px(SP_3))
        .gap(px(SP_2 + 2.))
        .border_b_1()
        .border_color(rgb(t.hairline))
}

/// Icon tile, title and one line of facts.
fn context_title(t: Colors, glyph: Icon, title: String, detail: String) -> Div {
    row()
        .gap(px(SP_2 + 2.))
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
                        .truncate()
                        .text_size(px(TEXT_XL))
                        .font_weight(WEIGHT_SEMIBOLD)
                        .text_color(rgb(t.text))
                        .child(title),
                )
                .child(
                    div()
                        .truncate()
                        .tabular()
                        .text_size(px(TEXT_SM))
                        .text_color(rgb(t.text_muted))
                        .child(detail),
                ),
        )
}

/// A tool button: icon, label and the command's shortcut as keycaps. Unavailable tools stay visible, dimmed, with the reason on hover.
fn action_button(
    t: Colors,
    id: &'static str,
    glyph: Icon,
    label: &'static str,
    command: Command,
    unavailable: Option<&'static str>,
    cx: &mut Context<Studio>,
) -> AnyElement {
    let shortcut = super::commands::command_info(command).shortcut;
    let button = row()
        .id(id)
        .h(px(H_MD))
        .min_w(px(0.))
        .pl(px(SP_2))
        .pr(px(SP_1_5))
        .gap(px(SP_2))
        .rounded(px(R_SM + 1.))
        .bg(rgb(t.control))
        .border_1()
        .border_color(rgb(t.border))
        .child(icon(glyph, ICON_SM, t.text_muted))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .font_weight(WEIGHT_MEDIUM)
                .text_color(rgb(t.text))
                .child(label),
        )
        .when_some(shortcut, |d, shortcut| d.child(keycaps(t, shortcut)));
    match unavailable {
        Some(reason) => disabled(button)
            .tooltip(tip_detail(t, label, reason, None))
            .into_any_element(),
        None => on_command(
            button
                .cursor_pointer()
                .hover(move |d| d.bg(rgb(t.control_hover)))
                .active(move |d| d.bg(rgb(t.control_active)))
                .tooltip(command_tip(t, command)),
            command,
            cx,
        )
        .into_any_element(),
    }
}

/// Facts about the selected faces and edges of the part being edited.
#[derive(Default)]
struct PartSelection {
    faces: usize,
    planar: usize,
    /// World area of the selected faces.
    area: f32,
    edges: usize,
    straight: usize,
    /// World length of the selected edges.
    length: f32,
}

fn part_selection_facts(s: &Studio, id: u64) -> Option<PartSelection> {
    use crate::cad_tools::geometry;
    let mesh = s.scene.object_mesh(id)?;
    let matrix = s.scene.world_transform(id).unwrap_or_default();
    let world = |i: u32| matrix.transform_point3(mesh.positions[i as usize]);
    let groups: Vec<u32> = s
        .selected_edit_faces(id)
        .into_iter()
        .map(|(group, _)| group)
        .collect();
    let mut facts = PartSelection {
        faces: groups.len(),
        ..Default::default()
    };
    if !groups.is_empty() {
        // One pass over the tessellation for every selected group.
        let mut members: std::collections::BTreeMap<u32, Vec<usize>> =
            groups.iter().map(|&g| (g, Vec::new())).collect();
        for face in 0..mesh.faces.len() {
            let group = if mesh.has_groups() {
                mesh.groups[face]
            } else {
                face as u32
            };
            if let Some(list) = members.get_mut(&group) {
                list.push(face);
            }
        }
        for faces in members.values() {
            if geometry::group_plane(mesh, faces).is_some() {
                facts.planar += 1;
            }
            for &f in faces {
                let polygon = &mesh.faces[f];
                let Some(&first) = polygon.first() else {
                    continue;
                };
                let a = world(first);
                for pair in polygon.windows(2).skip(1) {
                    facts.area += 0.5 * (world(pair[0]) - a).cross(world(pair[1]) - a).length();
                }
            }
        }
    }
    let edges = s.selected_edit_edges(id);
    facts.edges = edges.len();
    if let Some(topology) = (!edges.is_empty()).then(|| s.part_topology(id)).flatten() {
        for edge in edges.iter().filter_map(|&e| topology.edges.get(e)) {
            facts.straight += usize::from(edge.straight);
            facts.length += edge
                .points
                .windows(2)
                .map(|pair| world(pair[0]).distance(world(pair[1])))
                .sum::<f32>();
        }
    }
    Some(facts)
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn measure(value: f32) -> String {
    compact_number(crate::transform::format_precision_value(value), 4)
}

/// "Planar", "Curved" or "1 planar, 2 curved".
fn mix(count: usize, matching: usize, yes: &str, no: &str) -> String {
    let capital = |word: &str| {
        let mut chars = word.chars();
        chars
            .next()
            .map(|c| c.to_uppercase().chain(chars).collect())
            .unwrap_or_default()
    };
    if matching == count {
        capital(yes)
    } else if matching == 0 {
        capital(no)
    } else {
        format!("{matching} {yes}, {} {no}", count - matching)
    }
}

/// Part edit mode: what is selected, measured, and what can be done to it.
fn part_selection(t: Colors, s: &Studio, id: u64, cx: &mut Context<Studio>) -> Option<Div> {
    use crate::cad_tools::ToolCommand;
    let facts = part_selection_facts(s, id)?;
    let edges_mode = s.edit_mode == crate::app::EditMode::Edge;
    let (glyph, title, detail) = match (facts.faces, facts.edges) {
        (0, 0) if edges_mode => (
            Icon::Edge,
            "No edges selected".to_owned(),
            "Click an edge · Shift adds".to_owned(),
        ),
        (0, 0) => (
            Icon::Face,
            "No faces selected".to_owned(),
            "Click a face · Shift adds".to_owned(),
        ),
        (faces, 0) => (
            Icon::Face,
            plural(faces, "face", "faces"),
            format!(
                "{} · {} m²",
                mix(faces, facts.planar, "planar", "curved"),
                measure(facts.area)
            ),
        ),
        (0, edges) => (
            Icon::Edge,
            plural(edges, "edge", "edges"),
            format!(
                "{} · {} m{}",
                mix(edges, facts.straight, "straight", "curved"),
                measure(facts.length),
                if edges == 1 { " long" } else { " in total" }
            ),
        ),
        (faces, edges) => (
            Icon::Face,
            format!(
                "{} · {}",
                plural(faces, "face", "faces"),
                plural(edges, "edge", "edges")
            ),
            format!(
                "{} m² · {} m of edges",
                measure(facts.area),
                measure(facts.length)
            ),
        ),
    };
    let any = facts.faces + facts.edges > 0;
    let title = context_title(t, glyph, title, detail).when(any, |d| {
        d.child(
            on_command(
                icon_button(t, "part-selection-clear", Icon::Close, false, H_SM)
                    .tooltip(command_tip(t, Command::DeselectAll)),
                Command::DeselectAll,
                cx,
            )
            .flex_shrink_0(),
        )
    });
    let face_actions = facts.faces > 0 || !edges_mode;
    let edge_actions = facts.edges > 0 || edges_mode;
    let mut actions = col().gap(px(SP_1));
    if face_actions {
        actions = actions
            .child(action_button(
                t,
                "part-selection-push-pull",
                Icon::PushPull,
                "Push/Pull",
                Command::CadTool(ToolCommand::PushPull),
                (facts.faces > 0 && facts.planar < facts.faces)
                    .then_some("Curved faces can't be pushed · select flat faces"),
                cx,
            ))
            .child(action_button(
                t,
                "part-selection-sketch",
                Icon::DrawMode,
                "Sketch on face",
                Command::CadTool(ToolCommand::SketchOnFace),
                (facts.faces > 0 && facts.planar == 0)
                    .then_some("Sketches need a flat face · select a planar face"),
                cx,
            ));
    }
    if edge_actions {
        actions = actions.child(action_button(
            t,
            "part-selection-chamfer",
            Icon::Bevel,
            "Chamfer",
            Command::CadTool(ToolCommand::Chamfer),
            if facts.edges == 0 {
                Some("Select straight edges first · Click an edge, Shift adds")
            } else if facts.straight < facts.edges {
                Some("Chamfer works on straight edges between flat faces")
            } else {
                None
            },
            cx,
        ));
    }
    Some(context_block(t).child(title).child(actions))
}

/// Mesh edit mode: the polygon tools, with their shortcuts, and selection
/// helpers. Counts live in the status bar, never here.
fn mesh_edit_tools(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> Div {
    let mode = s.edit_mode.label();
    let no_faces = s
        .selected_faces()
        .is_empty()
        .then_some("Select faces first · 3 for face mode, A selects all");
    let tools = [
        (
            "mesh-tool-extrude",
            Icon::Extrude,
            "Extrude",
            Command::Extrude,
            no_faces,
        ),
        (
            "mesh-tool-inset",
            Icon::Inset,
            "Inset",
            Command::Inset,
            no_faces,
        ),
        (
            "mesh-tool-subdivide",
            Icon::Subdivide,
            "Subdivide",
            Command::Subdivide,
            None,
        ),
        (
            "mesh-tool-triangulate",
            Icon::Triangulate,
            "Triangulate",
            Command::Triangulate,
            None,
        ),
        (
            "mesh-tool-flip",
            Icon::FlipNormals,
            "Flip normals",
            Command::FlipNormals,
            None,
        ),
        (
            "mesh-tool-normals",
            Icon::NormalsOutside,
            "Normals out",
            Command::RecalculateNormals,
            None,
        ),
    ];
    let select = [
        ("mesh-select-all", "All", Command::SelectAll),
        ("mesh-select-linked", "Linked", Command::SelectLinked),
        ("mesh-select-invert", "Invert", Command::InvertSelection),
    ];
    context_block(t)
        .child(context_title(
            t,
            Icon::EditMode,
            "Edit mesh".to_owned(),
            format!("{mode} mode · Tab returns to Object mode"),
        ))
        .child(
            div()
                .grid()
                .grid_cols(2)
                .gap(px(SP_1))
                .children(
                    tools
                        .into_iter()
                        .map(|(id, glyph, label, command, unavailable)| {
                            action_button(t, id, glyph, label, command, unavailable, cx)
                        }),
                ),
        )
        .child(property_row(
            t,
            "Select",
            row()
                .flex_1()
                .gap(px(SP_1))
                .children(select.into_iter().map(|(id, label, command)| {
                    command_button(t, id, label, None, command, Tone::Secondary, cx)
                        .flex_1()
                        .min_w(px(0.))
                        .px(px(SP_1_5))
                })),
        ))
}

/// The context block for the Object page, if the user is editing components.
fn context(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> Option<Div> {
    if s.cad_tool_active() || !s.edit_mode.is_component() {
        return None;
    }
    if let Some(id) = s.part_editing() {
        return part_selection(t, s, id, cx);
    }
    s.selected_object()
        .filter(|object| object.data.is_editable_mesh())
        .map(|_| mesh_edit_tools(t, s, cx))
}

fn tab_button(
    t: Colors,
    page: InspectorTab,
    label: &'static str,
    current: InspectorTab,
    cx: &mut Context<Studio>,
) -> Stateful<Div> {
    tab(
        t,
        SharedString::from(format!("inspector-tab-{label}")),
        label,
        page == current,
    )
    .on_click(cx.listener(move |s, _, _, cx| {
        cx.stop_propagation();
        s.set_inspector_tab(page, cx);
    }))
}

fn transform(t: Colors, s: &Studio, stacked: bool, cx: &mut Context<Studio>) -> Div {
    let actions = [
        ("apply-scale", "Scale", Command::ApplyScale),
        (
            "apply-rotation-scale",
            "Rot & scale",
            Command::ApplyRotationScale,
        ),
    ];
    col()
        .child(vector_row(
            t,
            s,
            "Location",
            "m",
            "transform-0",
            Field::Translation,
            ["Location X", "Location Y", "Location Z"],
            stacked,
            cx,
        ))
        .child(vector_row(
            t,
            s,
            "Rotation",
            "°",
            "transform-1",
            Field::Rotation,
            ["Rotation X", "Rotation Y", "Rotation Z"],
            stacked,
            cx,
        ))
        .child(vector_row(
            t,
            s,
            "Scale",
            "",
            "transform-2",
            Field::Scale,
            ["Scale X", "Scale Y", "Scale Z"],
            stacked,
            cx,
        ))
        .child(vector_row(
            t,
            s,
            "Dimensions",
            "m",
            "transform-3",
            Field::Dimensions,
            [
                "Local dimension X · excludes rotation and parents",
                "Local dimension Y · excludes rotation and parents",
                "Local dimension Z · excludes rotation and parents",
            ],
            stacked,
            cx,
        ))
        .child(property_row(
            t,
            "Apply",
            row()
                .flex_1()
                .gap(px(SP_1))
                .children(actions.into_iter().map(|(id, label, command)| {
                    command_button(t, id, label, None, command, Tone::Secondary, cx)
                        .flex_1()
                        .min_w(px(0.))
                        .px(px(SP_1_5))
                })),
        ))
        .child(property_row(
            t,
            "Origin",
            command_button(
                t,
                "center-origin",
                "To geometry center",
                Some(Icon::Origin),
                Command::CenterOrigin,
                Tone::Secondary,
                cx,
            )
            .flex_1()
            .min_w(px(0.)),
        ))
}

fn surface(
    t: Colors,
    s: &Studio,
    material: &forma_core::Material,
    cx: &mut Context<Studio>,
) -> Div {
    let base = material.base_color;
    let presets = row()
        .gap(px(SP_1_5))
        .children(
            PRESETS
                .into_iter()
                .enumerate()
                .map(|(index, (swatch, name, color))| {
                    let applied = (0..3).all(|axis| (base[axis] - color[axis]).abs() < 0.002);
                    on_command(
                        col()
                            .id(SharedString::from(format!("preset-{index}")))
                            .flex_1()
                            .min_w(px(0.))
                            .items_center()
                            .gap(px(SP_1))
                            .cursor_pointer()
                            .group("preset")
                            .child(
                                div()
                                    .w_full()
                                    .h(px(22.))
                                    .rounded(px(R_SM))
                                    .bg(rgb(swatch))
                                    .border_1()
                                    .border_color(gpui::rgba(if t.dark {
                                        0xffffff1f
                                    } else {
                                        0x0000001f
                                    }))
                                    .when(applied, |d| d.border_2().border_color(rgb(t.focus))),
                            )
                            .child(
                                div()
                                    .text_size(px(TEXT_SM))
                                    .text_color(rgb(if applied {
                                        t.accent_text
                                    } else {
                                        t.text_muted
                                    }))
                                    .child(name),
                            )
                            .tooltip(tip(t, format!("{name} preset"), None)),
                        Command::MaterialPreset(index),
                        cx,
                    )
                }),
        );
    let shader = segmented(t).flex_1().children(
        [ShaderKind::Pbr, ShaderKind::Glass, ShaderKind::Custom]
            .into_iter()
            .map(|kind| {
                let active = material.shader == kind;
                let command = if kind == ShaderKind::Custom {
                    Command::EditShader
                } else {
                    Command::SetShader(kind)
                };
                on_command(
                    segment(
                        t,
                        SharedString::from(format!("shader-{kind:?}")),
                        active,
                        20.,
                    )
                    .flex_1()
                    .child(kind.label())
                    .tooltip(command_tip(t, command)),
                    command,
                    cx,
                )
            }),
    );
    col()
        .gap(px(SP_1))
        .child(div().pb(px(SP_1)).child(presets))
        .child(property_row(t, "Shader", shader))
        .when(material.shader == ShaderKind::Custom, |d| {
            d.child(property_row(
                t,
                "",
                command_button(
                    t,
                    "edit-custom-shader",
                    "Edit shader code…",
                    Some(Icon::Code),
                    Command::EditShader,
                    Tone::Secondary,
                    cx,
                )
                .flex_1(),
            ))
        })
        .child(
            row()
                .min_h(px(ROW_H))
                .gap(px(SP_2))
                .child(
                    row()
                        .w(px(LABEL_W))
                        .flex_shrink_0()
                        .gap(px(SP_1_5))
                        .text_color(rgb(t.text_muted))
                        .child("Color")
                        .child(
                            div()
                                .size(px(14.))
                                .rounded(px(R_XS))
                                .bg(rgb(swatch_color(base)))
                                .border_1()
                                .border_color(gpui::rgba(if t.dark {
                                    0xffffff26
                                } else {
                                    0x00000026
                                })),
                        ),
                )
                .child({
                    // One precision for the three channels.
                    let texts: Vec<String> = (0..3)
                        .map(|axis| s.field_value(Field::Color(axis)))
                        .collect();
                    let mut shown = row_numbers(&texts, 5).into_iter();
                    row()
                        .flex_1()
                        .min_w(px(0.))
                        .gap(px(SP_1))
                        .children((0..3).map(|axis| {
                            numeric_field_showing(
                                t,
                                s,
                                SharedString::from(format!("base-color-{axis}")),
                                Field::Color(axis),
                                FieldMark::Text(["R", "G", "B"][axis]),
                                ["Red · sRGB", "Green · sRGB", "Blue · sRGB"][axis],
                                shown.next(),
                                cx,
                            )
                            .flex_1()
                            .flex_basis(px(0.))
                        }))
                }),
        )
        .child(numeric_row(
            t,
            s,
            "roughness",
            "Roughness",
            "",
            Field::Roughness,
            cx,
        ))
        .when(material.shader != ShaderKind::Glass, |d| {
            d.child(numeric_row(
                t,
                s,
                "metallic",
                "Metallic",
                "",
                Field::Metallic,
                cx,
            ))
        })
        .child(numeric_row(t, s, "ior", "IOR", "", Field::Ior, cx))
        .child(numeric_row(
            t,
            s,
            "emission",
            "Emission",
            "",
            Field::Emission,
            cx,
        ))
}

fn textures(
    t: Colors,
    s: &Studio,
    material: &forma_core::Material,
    cx: &mut Context<Studio>,
) -> Div {
    let slots = TextureSlot::ALL
        .into_iter()
        .map(|slot| {
            let image = &material.textures[slot as usize];
            let space = if matches!(slot, TextureSlot::BaseColor | TextureSlot::Emission) {
                "sRGB"
            } else {
                "Linear"
            };
            let picker = on_command(
                row()
                    .id(SharedString::from(format!("load-texture-{slot:?}")))
                    .flex_1()
                    .min_w(px(0.))
                    .h(px(H_SM))
                    .px(px(SP_2))
                    .gap(px(SP_1_5))
                    .rounded(px(R_SM))
                    .bg(rgb(t.field))
                    .cursor_pointer()
                    .hover(move |d| d.bg(rgb(t.field_hover)))
                    .child(icon(
                        Icon::Image,
                        ICON_SM,
                        if image.is_some() {
                            t.accent_text
                        } else {
                            t.text_faint
                        },
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_color(rgb(if image.is_some() {
                                t.text
                            } else {
                                t.text_faint
                            }))
                            .child(
                                image
                                    .as_ref()
                                    .map(|image| image.name.clone())
                                    .unwrap_or_else(|| "Choose image…".into()),
                            ),
                    )
                    .when_some(image.as_ref(), |d, image| {
                        d.child(
                            caption(t, format!("{}×{}", image.width, image.height))
                                .tabular()
                                .text_color(rgb(t.text_faint)),
                        )
                    })
                    .tooltip(tip_detail(
                        t,
                        format!("{} texture", slot.label()),
                        format!("PNG or JPEG · {space}"),
                        None,
                    )),
                Command::LoadTexture(slot),
                cx,
            );
            property_row(
                t,
                slot.label(),
                row()
                    .flex_1()
                    .min_w(px(0.))
                    .gap(px(SP_1))
                    .child(picker)
                    .when(image.is_some(), |d| {
                        d.child(command_icon(
                            t,
                            SharedString::from(format!("clear-texture-{slot:?}")),
                            Icon::Close,
                            Command::ClearTexture(slot),
                            false,
                            H_SM,
                            cx,
                        ))
                    }),
            )
        })
        .collect::<Vec<_>>();
    let mappings = segmented(t).flex_1().children(
        [
            TextureMapping::Box,
            TextureMapping::Sphere,
            TextureMapping::Plane,
        ]
        .into_iter()
        .map(|mapping| {
            on_command(
                segment(
                    t,
                    SharedString::from(format!("texture-mapping-{mapping:?}")),
                    material.mapping == mapping,
                    20.,
                )
                .flex_1()
                .child(mapping.label())
                .tooltip(command_tip(t, Command::SetTextureMapping(mapping))),
                Command::SetTextureMapping(mapping),
                cx,
            )
        }),
    );
    let pair = |label: &'static str,
                ids: [&'static str; 2],
                fields: [Field; 2],
                cx: &mut Context<Studio>| {
        property_row(
            t,
            label,
            row().flex_1().gap(px(SP_1)).children((0..2).map(|i| {
                numeric_field(
                    t,
                    s,
                    ids[i],
                    fields[i],
                    FieldMark::Text(["U", "V"][i]),
                    ["U", "V"][i],
                    cx,
                )
                .flex_1()
                .flex_basis(px(0.))
            })),
        )
    };
    col()
        .gap(px(SP_1))
        .when(s.texture_loading, |d| {
            d.child(caption(t, "Loading image…").text_color(rgb(t.accent_text)))
        })
        .children(slots)
        .child(property_row(t, "Mapping", mappings))
        .child(pair(
            "Tiling",
            ["texture-scale-0", "texture-scale-1"],
            [Field::TextureScale(0), Field::TextureScale(1)],
            cx,
        ))
        .child(pair(
            "Offset",
            ["texture-offset-0", "texture-offset-1"],
            [Field::TextureOffset(0), Field::TextureOffset(1)],
            cx,
        ))
        .when(
            material.textures[TextureSlot::Normal as usize].is_some(),
            |d| {
                d.child(numeric_row(
                    t,
                    s,
                    "normal-strength",
                    "Normal",
                    "×",
                    Field::NormalStrength,
                    cx,
                ))
            },
        )
        .child(
            div()
                .pt(px(SP_1))
                .text_size(px(TEXT_SM))
                .line_height(px(16.))
                .text_color(rgb(t.text_faint))
                .child("Color maps multiply the color above. Roughness and metallic maps replace their values. Normal maps use OpenGL +Y. Images are saved in the project."),
        )
}

fn geometry(
    t: Colors,
    s: &Studio,
    object: &forma_core::MeshInstance,
    cx: &mut Context<Studio>,
) -> Div {
    let operations = [
        ("extrude", "Extrude", Icon::Extrude, Command::Extrude),
        ("inset", "Inset", Icon::Inset, Command::Inset),
        (
            "subdivide",
            "Subdivide",
            Icon::Subdivide,
            Command::Subdivide,
        ),
        ("bevel", "Bevel", Icon::Bevel, Command::Bevel),
        (
            "triangulate",
            "Triangulate",
            Icon::Triangulate,
            Command::Triangulate,
        ),
        (
            "recalculate-normals",
            "Normals out",
            Icon::NormalsOutside,
            Command::RecalculateNormals,
        ),
        (
            "flip-normals",
            "Flip normals",
            Icon::FlipNormals,
            Command::FlipNormals,
        ),
    ];
    col()
        .gap(px(SP_2))
        .child(
            div()
                .grid()
                .grid_cols(2)
                .gap(px(SP_1))
                .children(operations.into_iter().map(|(id, label, glyph, command)| {
                    command_button(t, id, label, Some(glyph), command, Tone::Secondary, cx)
                        .justify_start()
                        .min_w(px(0.))
                })),
        )
        .children(super::feature_inspector::convert_to_part(
            t,
            s,
            object.object.id,
            cx,
        ))
        .children(super::feature_inspector::combine_row(t, s, cx))
}

/// The selection's properties. While a tool runs (`dimmed`), everything but
/// the part history dims: the history shows where the new feature lands.
fn object_page(
    t: Colors,
    s: &Studio,
    stacked: bool,
    dimmed: bool,
    cx: &mut Context<Studio>,
) -> Div {
    if let Some(page) = s.sketch_inspector(t, cx) {
        return page;
    }
    let Some(object) = s.selected_object() else {
        let message = if s.scene.objects.is_empty() {
            (
                "Nothing to edit yet",
                "Add a mesh, a part or a sketch, then select it to edit its properties.",
            )
        } else {
            (
                "Nothing selected",
                "Click an object in the viewport or the scene list to edit its transform, surface and geometry.",
            )
        };
        return col().child(empty_state(t, Icon::Select, message.0, message.1));
    };
    let material = object.material;
    // Parts lead with their history: the selected feature's values belong
    // at the top, where selecting it in the timeline can be seen.
    let part = s.scene.object_part(object.id).map(|part| {
        let (contents, detail) =
            super::feature_inspector::part_section(t, s, object.id, part, stacked, cx);
        section(t, s, PanelSection::Part, "Part", detail, contents, cx)
    });
    let is_part = part.is_some();
    let rest = col()
        .flex_shrink_0()
        .when(dimmed, |d| d.opacity(0.4))
        .child(section(
            t,
            s,
            PanelSection::Transform,
            "Transform",
            None,
            transform(t, s, stacked, cx),
            cx,
        ))
        .child(section(
            t,
            s,
            PanelSection::Surface,
            "Surface",
            Some(material.shader.label().into()),
            surface(t, s, material, cx),
            cx,
        ))
        .child(section(
            t,
            s,
            PanelSection::Textures,
            "Image textures",
            {
                let count = material.textures.iter().filter(|t| t.is_some()).count();
                (count > 0).then(|| format!("{count}").into())
            },
            textures(t, s, material, cx),
            cx,
        ))
        // Parts show their feature history instead of polygon tools, and in
        // edit mode the polygon tools lead the page instead.
        .when(!is_part && !s.edit_mode.is_component(), |page| {
            page.child(section(
                t,
                s,
                PanelSection::Geometry,
                "Geometry",
                Some(
                    format!(
                        "{} · {}",
                        plural(object.mesh.positions.len(), "vertex", "vertices"),
                        plural(object.mesh.faces.len(), "face", "faces")
                    )
                    .into(),
                ),
                geometry(t, s, &object, cx),
                cx,
            ))
        });
    col().children(part).child(rest)
}

pub(crate) fn preview_light_name(s: &Studio) -> String {
    if s.settings.preview.use_scene_world {
        "Scene world".to_owned()
    } else if let Some(path) = &s.settings.preview.hdri_path {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Custom HDR".to_owned())
    } else {
        s.settings.preview.studio.label().to_owned()
    }
}

fn denoise(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> Div {
    let settings = s.settings.denoise;
    let clean = s.denoised_generation.is_some() && s.samples > 0;
    let (status, tone) = if !settings.viewport {
        ("Showing original samples".to_string(), t.text_muted)
    } else if s.denoise_error.is_some() {
        (
            "Unavailable · showing original samples".to_string(),
            t.danger,
        )
    } else if clean {
        let frame = s.frame.as_ref().unwrap();
        (
            format!(
                "Denoised at {} samples · {}",
                frame.samples,
                frame.denoise_device.as_deref().unwrap_or("Auto")
            ),
            t.success,
        )
    } else if s.samples < settings.start_sample.min(s.settings.max_samples) {
        (
            format!(
                "Starts at sample {}",
                settings.start_sample.min(s.settings.max_samples)
            ),
            t.text_muted,
        )
    } else {
        ("Denoising…".to_string(), t.accent_text)
    };
    col()
        .gap(px(SP_1))
        .child(switch_row(
            t,
            "viewport-denoise",
            "Viewport",
            Some("Clean up progressive renders"),
            settings.viewport,
            Command::ToggleViewportDenoise,
            cx,
        ))
        .when(settings.viewport, |d| {
            d.child(numeric_row(
                t,
                s,
                "denoise-start",
                "Start at",
                "spp",
                Field::DenoiseStart,
                cx,
            ))
            .child(property_row(
                t,
                "Quality",
                segmented(t).flex_1().children(
                    [
                        DenoiseQuality::Fast,
                        DenoiseQuality::Balanced,
                        DenoiseQuality::High,
                    ]
                    .into_iter()
                    .map(|quality| {
                        on_command(
                            segment(
                                t,
                                SharedString::from(format!("denoise-quality-{}", quality as usize)),
                                settings.quality == quality,
                                20.,
                            )
                            .flex_1()
                            .child(quality.label())
                            .tooltip(command_tip(t, Command::SetDenoiseQuality(quality))),
                            Command::SetDenoiseQuality(quality),
                            cx,
                        )
                    }),
                ),
            ))
        })
        .child(switch_row(
            t,
            "export-denoise",
            "Image export",
            Some("High quality · accurate prefilter"),
            settings.render,
            Command::ToggleRenderDenoise,
            cx,
        ))
        .child(
            row()
                .pt(px(SP_1))
                .gap(px(SP_1_5))
                .child(dot(tone, 6.))
                .child(
                    caption(t, status)
                        .flex_shrink()
                        .min_w(px(0.))
                        .truncate()
                        .text_color(rgb(tone)),
                ),
        )
}

/// The Render page's one primary action: save the viewport as an image. In
/// Rendered mode that is a full path-traced render at the sample target;
/// other modes save what the viewport shows. Shading itself is chosen in the
/// viewport toolbar, so the page does not repeat it.
fn render_action(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> Div {
    let mode = s.settings.mode;
    let (label, detail) = if mode == RenderMode::Rendered {
        (
            "Render image…",
            format!(
                "Path-traced PNG at {} samples · keeps rendering while you edit",
                s.settings.max_samples
            ),
        )
    } else {
        (
            "Export image…",
            format!(
                "PNG of the {} viewport · switch to Rendered for a path-traced image",
                mode.label().to_lowercase()
            ),
        )
    };
    col()
        .px(px(SP_3))
        .pt(px(SP_3))
        .pb(px(SP_3))
        .border_b_1()
        .border_color(rgb(t.hairline))
        .child(on_command(
            button(
                t,
                "render-image",
                label,
                Some(Icon::ExportImage),
                Tone::Primary,
            )
            .h(px(H_MD))
            .w_full()
            .tooltip(tip_detail(t, label, detail, None)),
            Command::ExportImage,
            cx,
        ))
}

fn render_page(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> Div {
    let mode = s.settings.mode;
    let mut page = col().child(render_action(t, s, cx));
    match mode {
        RenderMode::Rendered => {
            let progress = (s.samples as f32 / s.settings.max_samples.max(1) as f32).clamp(0., 1.);
            let sampling = col()
                .child(numeric_row(
                    t,
                    s,
                    "samples",
                    "Samples",
                    "max",
                    Field::Samples,
                    cx,
                ))
                .child(numeric_row(
                    t,
                    s,
                    "bounces",
                    "Bounces",
                    "",
                    Field::Bounces,
                    cx,
                ))
                .child(numeric_row(
                    t,
                    s,
                    "exposure",
                    "Exposure",
                    "EV",
                    Field::Exposure,
                    cx,
                ))
                .child(numeric_row(
                    t,
                    s,
                    "world-strength",
                    "World",
                    "×",
                    Field::WorldStrength,
                    cx,
                ))
                .child(
                    col()
                        .pt(px(SP_2))
                        .gap(px(SP_1_5))
                        .child(
                            row().child(caption(t, "Progress")).child(spacer()).child(
                                caption(t, format!("{} / {}", s.samples, s.settings.max_samples))
                                    .tabular(),
                            ),
                        )
                        .child(
                            div().h(px(4.)).rounded_full().bg(rgb(t.field)).child(
                                div()
                                    .h_full()
                                    .rounded_full()
                                    .w(relative(progress))
                                    .bg(rgb(t.accent)),
                            ),
                        ),
                );
            page = page
                .child(section(
                    t,
                    s,
                    PanelSection::Render,
                    "Path tracing",
                    Some("Progressive".into()),
                    sampling,
                    cx,
                ))
                .child(section(
                    t,
                    s,
                    PanelSection::Denoise,
                    "AI denoising",
                    Some("Open Image Denoise".into()),
                    denoise(t, s, cx),
                    cx,
                ));
        }
        RenderMode::MaterialPreview => {
            let lighting = col()
                .child(property_row(
                    t,
                    "Environment",
                    on_command(
                        row()
                            .id("inspector-preview-settings")
                            .flex_1()
                            .min_w(px(0.))
                            .h(px(H_SM))
                            .px(px(SP_2))
                            .gap(px(SP_1_5))
                            .rounded(px(R_SM))
                            .bg(rgb(if s.preview_open {
                                t.accent_soft
                            } else {
                                t.field
                            }))
                            .cursor_pointer()
                            .hover(move |d| d.bg(rgb(t.field_hover)))
                            .child(icon(Icon::Lighting, ICON_SM, t.text_muted))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .truncate()
                                    .child(preview_light_name(s)),
                            )
                            .child(icon(Icon::ChevronRight, ICON_SM, t.text_faint))
                            .tooltip(command_tip(t, Command::TogglePreviewSettings)),
                        Command::TogglePreviewSettings,
                        cx,
                    ),
                ))
                .child(numeric_row(
                    t,
                    s,
                    "exposure",
                    "Exposure",
                    "EV",
                    Field::Exposure,
                    cx,
                ))
                .when(s.settings.preview.use_scene_world, |d| {
                    d.child(numeric_row(
                        t,
                        s,
                        "world-strength",
                        "World",
                        "×",
                        Field::WorldStrength,
                        cx,
                    ))
                });
            page = page.child(section(
                t,
                s,
                PanelSection::Lighting,
                "Preview lighting",
                Some("Viewport only".into()),
                lighting,
                cx,
            ));
        }
        RenderMode::Solid | RenderMode::Wireframe => {
            page = page.child(
                col()
                    .child(empty_state(
                        t,
                        Icon::ShadingRendered,
                        "No render settings",
                        "Solid and wireframe shading are fixed. Switch to Material preview or Rendered to adjust lighting and sampling.",
                    ))
                    .child(
                        row()
                            .justify_center()
                            .gap(px(SP_1))
                            .pb(px(SP_4))
                            .child(command_button(
                                t,
                                "render-tab-preview",
                                "Material preview",
                                Some(Icon::ShadingMaterial),
                                Command::SetMode(RenderMode::MaterialPreview),
                                Tone::Secondary,
                                cx,
                            ))
                            .child(command_button(
                                t,
                                "render-tab-rendered",
                                "Rendered",
                                Some(Icon::ShadingRendered),
                                Command::SetMode(RenderMode::Rendered),
                                Tone::Secondary,
                                cx,
                            )),
                    ),
            );
        }
    }
    page.child(section(
        t,
        s,
        PanelSection::Display,
        "Viewport display",
        None,
        col()
            .child(switch_row(
                t,
                "display-grid",
                "Ground grid",
                None,
                s.settings.show_grid,
                Command::ToggleGrid,
                cx,
            ))
            .child(switch_row(
                t,
                "display-measurements",
                "Measurements",
                Some("World dimensions and selected edge lengths"),
                s.show_measurements,
                Command::ToggleMeasurements,
                cx,
            )),
        cx,
    ))
}

pub(super) fn inspector(t: Colors, s: &Studio, width: f32, cx: &mut Context<Studio>) -> Div {
    if let Some(panel) = super::sketch_inspector::panel(t, s, cx) {
        return panel;
    }
    let stacked = width < STACK_BELOW;
    // An empty scene has nothing to inspect: no header or tabs, just the two
    // ways to start, placed where the eye lands in the panel.
    if s.selected.is_none() && s.scene.objects.is_empty() {
        return empty_scene(t, s, cx);
    }
    let object_tab = s.panels.tab == InspectorTab::Object;
    // A running tool leads, pinned above the page, and the page beneath it
    // dims: it describes the object, not the operation in progress.
    let live = object_tab
        .then(|| s.cad_tool_panel(t, stacked, cx))
        .flatten();
    let context = object_tab.then(|| context(t, s, cx)).flatten();
    let dimmed = live.is_some();
    let page = match s.panels.tab {
        // With nothing selected, the running tool is all there is to show.
        InspectorTab::Object if dimmed && s.selected.is_none() => col(),
        InspectorTab::Object => object_page(t, s, stacked, dimmed, cx),
        InspectorTab::Render => render_page(t, s, cx),
    };
    col()
        .flex_1()
        .min_h(px(0.))
        .child(header(t, s, cx))
        .children(live)
        .child(
            div()
                .relative()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .child(
                    col()
                        .id("properties-scroll")
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scroll()
                        .track_scroll(&s.panels.inspector_scroll)
                        .child(
                            col()
                                .flex_shrink_0()
                                // Room below the last row, so the end of the
                                // page reads as an end and never as a crop.
                                .pb(px(SP_6))
                                .children(context)
                                .child(page.flex_shrink_0()),
                        ),
                )
                .child(scroll_fades(
                    t,
                    &s.panels.inspector_scroll,
                    // A field revealed by Tab may sit at the bottom edge;
                    // it must never be washed out while it is edited.
                    s.active_field
                        .as_ref()
                        .is_some_and(|(field, _)| super::PanelState::is_inspector_field(*field)),
                )),
        )
}

/// Soft fades over a scrolled panel's edges while content continues past
/// them: a short one under the header once scrolled, a taller one at the
/// bottom while more sits below. They are painted, not laid out, after the
/// scroll area has measured itself, so they match this frame's content even
/// when a section opens or closes, and they never take input. `editing`
/// drops the bottom fade so a field being edited at the edge stays clear.
pub(super) fn scroll_fades(t: Colors, scroll: &ScrollHandle, editing: bool) -> impl IntoElement {
    const TOP: f32 = 14.;
    const BOTTOM: f32 = 32.;
    let scroll = scroll.clone();
    let mut solid = hex(t.surface);
    solid.a = 1.;
    let mut clear = solid;
    clear.a = 0.;
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let offset = -f32::from(scroll.offset().y);
            let maximum = f32::from(scroll.max_offset().height);
            let band = |top: Pixels, height: f32| {
                Bounds::new(
                    point(bounds.left(), top),
                    size(bounds.size.width, px(height)),
                )
            };
            if offset > 0.5 {
                window.paint_quad(fill(
                    band(bounds.top(), TOP),
                    linear_gradient(
                        0.,
                        linear_color_stop(clear, 0.),
                        linear_color_stop(solid, 1.),
                    ),
                ));
            }
            if !editing && maximum - offset > 0.5 {
                window.paint_quad(fill(
                    band(bounds.bottom() - px(BOTTOM), BOTTOM),
                    linear_gradient(
                        180.,
                        linear_color_stop(clear, 0.),
                        linear_color_stop(solid, 1.),
                    ),
                ));
            }
        },
    )
    .absolute()
    .inset_0()
    .size_full()
}

/// Open the Add menu below `anchor`, or close it if this button opened it.
/// It drops below its trigger like the view menu, whose slot it borrows:
/// that trigger only lights up for its own menu.
fn open_add_menu(s: &mut Studio, anchor: Bounds<Pixels>, cx: &mut Context<Studio>) {
    if add_menu_open(s) {
        s.panels.menu = None;
    } else {
        s.active_field = None;
        s.palette_open = false;
        s.preview_open = false;
        s.panels.menu = Some(OpenMenu {
            kind: MenuKind::Add,
            from: Anchor::ViewHud,
            anchor,
            index: super::overlays::menu::first_item(MenuKind::Add),
        });
    }
    cx.notify();
}

fn add_menu_open(s: &Studio) -> bool {
    s.panels
        .menu
        .as_ref()
        .is_some_and(|menu| menu.kind == MenuKind::Add && menu.from == Anchor::ViewHud)
}

/// The inspector of an empty scene: what goes here, and the two ways to
/// begin. The viewport's start card offers every kind; this stays quiet.
fn empty_scene(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> Div {
    // The Add button records where it was drawn, so its menu opens beneath it.
    let bounds = std::rc::Rc::new(std::cell::Cell::new(Bounds::<Pixels>::default()));
    let recorder = bounds.clone();
    let tone = if add_menu_open(s) {
        Tone::Selected
    } else {
        Tone::Secondary
    };
    let add = button(t, "inspector-empty-add", "Add", Some(Icon::Plus), tone)
        .h(px(H_MD))
        .flex_1()
        .relative()
        .child(
            canvas(move |b, _, _| recorder.set(b), |_, _, _, _| {})
                .absolute()
                .inset_0()
                .size_full(),
        )
        .tooltip(tip_detail(
            t,
            "Add",
            "Meshes, parts, sketches and imports",
            None,
        ))
        .on_click(cx.listener(move |s, _, _, cx| {
            cx.stop_propagation();
            open_add_menu(s, bounds.get(), cx);
        }));
    let sketch = command_button(
        t,
        "inspector-empty-sketch",
        "Sketch",
        Some(Icon::DrawMode),
        Command::Draw(crate::sketch::DrawCommand::NewGroundSketch),
        Tone::Secondary,
        cx,
    )
    .h(px(H_MD))
    .flex_1();
    col()
        .flex_1()
        .min_h(px(0.))
        .px(px(SP_5))
        .pt(px(SP_6 + SP_4))
        .items_center()
        .gap(px(SP_2))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(36.))
                .rounded(px(R_LG))
                .bg(rgb(t.control))
                .border_1()
                .border_color(rgb(t.hairline))
                .child(icon(Icon::Scene, ICON_LG, t.text_muted)),
        )
        .child(
            div()
                .mt(px(SP_1))
                .text_size(px(TEXT_LG))
                .font_weight(WEIGHT_MEDIUM)
                .text_color(rgb(t.text))
                .child("Empty scene"),
        )
        .child(
            div()
                .max_w(px(232.))
                .text_center()
                .line_height(px(17.))
                .text_color(rgb(t.text_muted))
                .child("Add a mesh or part, or sketch a profile to extrude. Its properties appear here."),
        )
        .child(
            row()
                .w_full()
                .max_w(px(232.))
                .mt(px(SP_2))
                .gap(px(SP_1_5))
                .child(add)
                .child(sketch),
        )
}
