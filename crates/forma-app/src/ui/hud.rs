//! Floating viewport controls. The viewport fills its region edge to edge;
//! tools, filters and view controls float above it in translucent clusters,
//! leaving the scene as large as possible.
use super::assets::{Icon, icon};
use super::commands::{
    MESH_DETAIL, PART_DETAIL, SKETCH_DETAIL, command_icon as command_glyph, shading_icon,
};
use super::components::*;
use super::tokens::*;
use super::{Anchor, MenuKind, VIEWPORT_MIN_WIDTH, slots};
use crate::app::{Command, EditMode, Studio, Tool};
use crate::cad::{CadCommand, PartShape};
use crate::platform_shortcut;
use crate::sketch::{DrawCommand, DrawTarget};
use crate::theme::Colors;
use forma_core::{Primitive, SketchPlane};
use forma_render::RenderMode;
use gpui::{prelude::*, *};

/// Distance between floating clusters and the viewport edge.
pub(crate) const HUD_INSET: f32 = 12.;
const TOOL: f32 = H_LG;

fn tool_palette(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> Div {
    // Only the + that opened the Add menu lights up; the Scene panel has one too.
    let menu_open = s
        .panels
        .menu
        .as_ref()
        .is_some_and(|menu| menu.kind == MenuKind::Add && menu.from == Anchor::ToolRail);
    let separator = || {
        div()
            .mx(px(SP_1_5))
            .my(px(3.))
            .h(px(1.))
            .bg(rgba(t.hud_border))
    };
    // While a sketch is open the Draw toolbar holds the tools that matter;
    // the Object tools stay usable but step back.
    let drawing = s.is_drawing();
    let step_back = |icon: Stateful<Div>, idle: bool| {
        if drawing {
            icon.opacity(0.35)
        } else {
            icon.when(idle, |icon| icon.opacity(0.4))
        }
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
            // Transform tools have nothing to act on in an empty scene.
            let idle = tool != Tool::Select && s.scene.objects.is_empty();
            let icon = command_icon(
                t,
                SharedString::from(format!("tool-{tool:?}")),
                glyph,
                Command::SetTool(tool),
                s.tool == tool && !drawing,
                TOOL,
                cx,
            );
            step_back(icon, idle)
        }),
    );
    palette = palette.child(separator());
    let push_pull = Command::CadTool(crate::cad_tools::ToolCommand::PushPull);
    let pushing = matches!(s.cad_tool.tool, Some(crate::cad_tools::Tool::PushPull(_)));
    palette = if s.part_editing().is_some() {
        // B-rep component editing: faces push and pull, edges chamfer.
        palette
            .child(step_back(
                command_icon(
                    t,
                    "rail-push-pull",
                    Icon::PushPull,
                    push_pull,
                    pushing,
                    TOOL,
                    cx,
                ),
                false,
            ))
            .child(step_back(
                command_icon(
                    t,
                    "rail-chamfer",
                    Icon::Bevel,
                    Command::CadTool(crate::cad_tools::ToolCommand::Chamfer),
                    matches!(s.cad_tool.tool, Some(crate::cad_tools::Tool::Chamfer(_))),
                    TOOL,
                    cx,
                ),
                false,
            ))
    } else if s.edit_mode.is_component() {
        palette
            .child(step_back(
                command_icon(
                    t,
                    "rail-extrude",
                    Icon::Extrude,
                    Command::Extrude,
                    false,
                    TOOL,
                    cx,
                ),
                false,
            ))
            .child(step_back(
                command_icon(
                    t,
                    "rail-inset",
                    Icon::Inset,
                    Command::Inset,
                    false,
                    TOOL,
                    cx,
                ),
                false,
            ))
    } else {
        // Like the transform tools, these need something to act on.
        let empty = s.scene.objects.is_empty();
        palette
            .child(step_back(
                command_icon(
                    t,
                    "rail-push-pull",
                    Icon::PushPull,
                    push_pull,
                    pushing,
                    TOOL,
                    cx,
                ),
                empty,
            ))
            .child(step_back(
                command_icon(
                    t,
                    "rail-bevel",
                    Icon::Bevel,
                    Command::Bevel,
                    false,
                    TOOL,
                    cx,
                ),
                empty,
            ))
    };
    palette.child(separator()).child(
        icon_button(t, "tool-add", Icon::Plus, menu_open, TOOL)
            .relative()
            .child(s.panels.anchor_cell(Anchor::ToolRail).element())
            .tooltip(tip_detail(
                t,
                "Add",
                "Meshes, parts, sketches and imports",
                None,
            ))
            .on_click(cx.listener(|s, _, _, cx| {
                cx.stop_propagation();
                s.open_menu(MenuKind::Add, Anchor::ToolRail, cx);
            })),
    )
}

/// Vertex / edge / face filter, shown while editing components.
/// How much room the floating clusters have, from the viewport width.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Density {
    /// Below 640 px: icon-only filter stacked under the tools, no view label.
    Tight,
    /// Below 900 px: icon-only filter beside the tools.
    Compact,
    Full,
}

impl Density {
    fn for_width(width: f32) -> Self {
        if width < 640. {
            Self::Tight
        } else if width < 900. {
            Self::Compact
        } else {
            Self::Full
        }
    }
}

fn component_filter(t: Colors, s: &Studio, density: Density, cx: &mut Context<Studio>) -> Div {
    let labels = density == Density::Full;
    // Parts edit B-rep faces and edges; they have no free vertices.
    let part = s.part_editing().is_some();
    hud(t)
        .when(density == Density::Tight, |d| d.flex_col())
        .children(
            [
                (EditMode::Vertex, Icon::Vertex, "Vertex"),
                (EditMode::Edge, Icon::Edge, "Edge"),
                (EditMode::Face, Icon::Face, "Face"),
            ]
            .into_iter()
            .filter(move |(mode, _, _)| !(part && *mode == EditMode::Vertex))
            .map(|(mode, glyph, label)| {
                let active = s.edit_mode == mode;
                on_command(
                    button_base(
                        t,
                        SharedString::from(format!("select-{label}")),
                        if active { Tone::Selected } else { Tone::Ghost },
                        if labels { H_MD } else { TOOL },
                    )
                    .when(labels, |d| d.px(px(SP_2)))
                    .when(!labels, |d| d.w(px(TOOL)))
                    .font_weight(WEIGHT_MEDIUM)
                    .child(icon(
                        glyph,
                        ICON_MD,
                        if active { t.accent_text } else { t.text_muted },
                    ))
                    .when(labels, |d| d.child(label))
                    .tooltip(command_tip(t, Command::SetEditMode(mode))),
                    Command::SetEditMode(mode),
                    cx,
                )
            }),
        )
}

/// Short name of the current view for the views menu button and the views
/// menu's check mark: "Front" for the front orthographic view.
pub(crate) fn view_name(camera: forma_core::Camera) -> &'static str {
    match crate::viewport::camera_view_label(camera) {
        "User perspective" => "Perspective",
        "User orthographic" => "Orthographic",
        named => named.strip_suffix(" · Ortho").unwrap_or(named),
    }
}

fn view_controls(t: Colors, s: &Studio, width: f32, cx: &mut Context<Studio>) -> Div {
    let labelled = Density::for_width(width) != Density::Tight;
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
            .when(labelled, |d| {
                d.child(
                    div()
                        .min_w(px(78.))
                        .text_color(rgb(t.text))
                        .child(view_name(s.scene.camera)),
                )
            })
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
        // The lighting button keeps its slot in every mode, so the cluster
        // never shifts under the pointer; outside Material Preview it is quiet
        // and opening it switches to Material Preview.
        .child(div().w(px(1.)).h(px(16.)).mx(px(2.)).bg(rgba(t.hud_border)))
        .child(
            command_icon(
                t,
                "shading-lighting",
                Icon::Lighting,
                Command::TogglePreviewSettings,
                s.preview_open,
                H_MD,
                cx,
            )
            .when(mode != RenderMode::MaterialPreview, |icon| {
                icon.opacity(0.55)
            }),
        );
    row()
        .gap(px(SP_1_5))
        .child(views)
        // Framing, grid and measurements stay reachable from the views menu,
        // shortcuts and the palette when the viewport is very narrow.
        .when(width >= 540., |d| d.child(display))
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

/// What a start-card row does.
#[derive(Clone, Copy)]
enum StartAction {
    Run(Command),
    /// Open Draw facing a world plane through the origin.
    DrawOn {
        plane: fn() -> SketchPlane,
        name: &'static str,
        detail: &'static str,
    },
}

/// One way of making geometry on the start card: what it is, then the
/// things it can start from.
struct StartKind {
    id: &'static str,
    glyph: Icon,
    title: &'static str,
    detail: &'static str,
    actions: &'static [(&'static str, Option<Icon>, StartAction)],
}

const START_KINDS: [StartKind; 3] = [
    StartKind {
        id: "mesh",
        glyph: Icon::Mesh,
        title: "Mesh",
        detail: MESH_DETAIL,
        actions: &[
            (
                "Cube",
                None,
                StartAction::Run(Command::Add(Primitive::Cube)),
            ),
            (
                "Sphere",
                None,
                StartAction::Run(Command::Add(Primitive::Sphere)),
            ),
            (
                "Cylinder",
                None,
                StartAction::Run(Command::Add(Primitive::Cylinder)),
            ),
        ],
    },
    StartKind {
        id: "part",
        glyph: Icon::PartBox,
        title: "Part",
        detail: PART_DETAIL,
        actions: &[
            (
                "Box",
                None,
                StartAction::Run(Command::Cad(CadCommand::AddPart(PartShape::Box))),
            ),
            (
                "Cylinder",
                None,
                StartAction::Run(Command::Cad(CadCommand::AddPart(PartShape::Cylinder))),
            ),
            (
                "Sphere",
                None,
                StartAction::Run(Command::Cad(CadCommand::AddPart(PartShape::Sphere))),
            ),
        ],
    },
    StartKind {
        id: "sketch",
        glyph: Icon::DrawMode,
        title: "Sketch",
        detail: SKETCH_DETAIL,
        actions: &[
            (
                "Ground plane",
                Some(Icon::PlaneGround),
                StartAction::Run(Command::Draw(DrawCommand::NewGroundSketch)),
            ),
            (
                "Front plane",
                Some(Icon::PlaneFront),
                StartAction::DrawOn {
                    plane: SketchPlane::front,
                    name: "Front plane",
                    detail: "Draw a profile facing you, through the origin",
                },
            ),
            (
                "Right plane",
                Some(Icon::PlaneRight),
                StartAction::DrawOn {
                    plane: SketchPlane::right,
                    name: "Right plane",
                    detail: "Draw a side profile, through the origin",
                },
            ),
        ],
    },
];

/// Open Draw on a world plane, as the start card's plane rows do.
fn draw_on_plane(
    s: &mut Studio,
    plane: SketchPlane,
    name: &str,
    window: &mut Window,
    cx: &mut Context<Studio>,
) {
    window.focus(&s.focus);
    s.enter_draw(DrawTarget::Plane(plane, name.to_owned()), cx);
    cx.notify();
}

/// A start tile: the kind's glyph, name and one-line description over the
/// rows that start it, in equal columns so the three read as one choice.
fn start_tile(t: Colors, kind: &StartKind, stacked: bool, cx: &mut Context<Studio>) -> Div {
    let actions = kind.actions.iter().map(|(label, glyph, action)| {
        let glyph = glyph.or(match action {
            StartAction::Run(command) => command_glyph(*command),
            StartAction::DrawOn { .. } => None,
        });
        let button = button_base(
            t,
            SharedString::from(format!("empty-{}-{label}", kind.id)),
            Tone::Ghost,
            H_MD,
        )
        .w_full()
        .justify_start()
        .px(px(SP_2))
        .gap(px(SP_2 + 2.))
        .text_size(px(TEXT_LG))
        .text_color(rgb(t.text))
        .when_some(glyph, |d, glyph| {
            d.child(icon(glyph, ICON_MD, t.text_muted))
        })
        .child(*label);
        match *action {
            StartAction::Run(command) => {
                on_command(button.tooltip(command_tip(t, command)), command, cx)
            }
            StartAction::DrawOn {
                plane,
                name,
                detail,
            } => button
                .tooltip(tip_detail(
                    t,
                    format!("Draw on the {}", name.to_lowercase()),
                    detail,
                    None,
                ))
                .on_click(cx.listener(move |s, _, window, cx| {
                    cx.stop_propagation();
                    draw_on_plane(s, plane(), name, window, cx);
                })),
        }
    });
    col()
        .when(!stacked, |d| d.flex_1().flex_basis(px(0.)))
        .min_w(px(0.))
        .p(px(SP_2))
        .gap(px(SP_2))
        .rounded(px(R_LG))
        .bg(tint(t.text, if t.dark { 0.035 } else { 0.025 }))
        .border_1()
        .border_color(rgba(t.hud_border))
        .child(
            col()
                .px(px(SP_2))
                .pt(px(SP_2))
                .pb(px(SP_1))
                .gap(px(3.))
                .child(
                    row()
                        .gap(px(SP_2))
                        .child(icon(kind.glyph, ICON_LG, t.accent_text))
                        .child(
                            div()
                                .text_size(px(TEXT_XL))
                                .font_weight(WEIGHT_SEMIBOLD)
                                .text_color(rgb(t.text))
                                .child(kind.title),
                        ),
                )
                .child(
                    div()
                        .text_size(px(TEXT_MD))
                        .line_height(px(16.))
                        .text_color(rgb(t.text_muted))
                        .child(kind.detail),
                ),
        )
        .child(hrule(t).bg(rgba(t.hud_border)))
        .child(col().gap(px(1.)).children(actions))
}

/// Quick starts when the scene has no objects: a tile for each way of making
/// geometry (mesh, part, sketch), then a row of import buttons.
fn empty_scene(t: Colors, s: &Studio, width: f32, cx: &mut Context<Studio>) -> Option<Div> {
    // Drawing on an empty scene needs the whole view: the sketch only joins
    // the scene with its first curve.
    if !s.scene.objects.is_empty() || s.frame.is_none() || s.is_drawing() {
        return None;
    }
    // Clear of the tool rail on both sides; below about 520 px the tiles
    // stack instead of squeezing their descriptions.
    let card = 680f32.min(width - 2. * (HUD_INSET + TOOL + SP_5));
    let stacked = card < 520.;
    let tiles: Vec<Div> = START_KINDS
        .iter()
        .map(|kind| start_tile(t, kind, stacked, cx))
        .collect();
    let imports = [
        ("OBJ…", Command::ImportObj),
        ("STL…", Command::ImportStl),
        ("STEP…", Command::Cad(CadCommand::ImportStep)),
    ]
    .map(|(label, command)| {
        on_command(
            button_base(
                t,
                SharedString::from(format!("empty-import-{label}")),
                Tone::Ghost,
                H_MD,
            )
            .px(px(SP_2 + 2.))
            .border_1()
            .border_color(rgba(t.hud_border))
            .text_size(px(TEXT_LG))
            .text_color(rgb(t.text))
            .child(label)
            .tooltip(command_tip(t, command)),
            command,
            cx,
        )
    });
    Some(
        hud(t)
            .flex_col()
            .items_stretch()
            .w(px(card))
            .p(px(SP_5))
            .gap(px(SP_4))
            .rounded(px(R_XL + 2.))
            .shadow(elevation(t, 2))
            .child(
                col()
                    .px(px(SP_1))
                    .gap(px(SP_1))
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(WEIGHT_SEMIBOLD)
                            .text_color(rgb(t.text))
                            .child("Start a model"),
                    )
                    .child(
                        div()
                            .text_size(px(TEXT_LG))
                            .text_color(rgb(t.text_muted))
                            .child("Pick a kind of geometry, or bring in a file."),
                    ),
            )
            .child(
                row()
                    .items_stretch()
                    .gap(px(SP_3))
                    .when(stacked, |d| d.flex_col())
                    .children(tiles),
            )
            .child(
                row()
                    .px(px(SP_1))
                    .gap(px(SP_1_5))
                    .child(
                        row()
                            .mr(px(SP_1_5))
                            .gap(px(SP_1_5))
                            .text_size(px(TEXT_LG))
                            .text_color(rgb(t.text_muted))
                            .child(icon(Icon::Import, ICON_MD, t.text_muted))
                            .child("Import"),
                    )
                    .children(imports)
                    .child(spacer())
                    .child(hint(t, platform_shortcut("⌘ K", "Ctrl K"), "All commands")),
            ),
    )
}

/// The viewport region: the GPU canvas plus every floating cluster and the
/// reserved slots from `slots`.
pub(super) fn viewport_area(
    t: Colors,
    s: &Studio,
    viewport: AnyElement,
    width: f32,
    cx: &mut Context<Studio>,
) -> impl IntoElement {
    let density = Density::for_width(width);
    let tools = tool_palette(t, s, cx);
    let filter = s
        .edit_mode
        .is_component()
        .then(|| component_filter(t, s, density, cx));
    let views = view_controls(t, s, width, cx);
    let operation = operation_hud(t, s);
    let dock = slots::viewport_dock(s, t, cx);
    let adjust = slots::adjust_last_operation(s, t, cx);
    let measurements = measurements_card(t, s);
    let notice = update_notice(t, s, cx);
    let empty = empty_scene(t, s, width, cx);
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
                .flex()
                .items_start()
                .gap(px(SP_2))
                .when(density == Density::Tight, |d| d.flex_col())
                .child(tools)
                .children(filter),
        )
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
        .when(measurements.is_some() || notice.is_some(), |d| {
            d.child(
                col()
                    .absolute()
                    .right(px(HUD_INSET))
                    .bottom(px(HUD_INSET))
                    .items_end()
                    .gap(px(SP_2))
                    .children(notice)
                    .children(measurements),
            )
        })
}

/// A downloaded update waits for a restart; say so once, without a dialog.
fn update_notice(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> Option<AnimationElement<Div>> {
    let crate::updater::State::Ready(version) = &s.updates.state else {
        return None;
    };
    if s.updates.open || s.panels.dismissed_update.as_ref() == Some(version) {
        return None;
    }
    let dismissed = version.clone();
    Some(
        toast(
            t,
            Icon::Update,
            t.accent_text,
            "Update ready",
            Some(
                format!("Forma {version} installs when you restart. Your work is saved first.")
                    .into(),
            ),
        )
        .child(
            col()
                .gap(px(SP_1))
                .child(command_button(
                    t,
                    "notice-restart",
                    "Restart",
                    None,
                    Command::RestartUpdate,
                    Tone::Primary,
                    cx,
                ))
                .child(
                    button(t, "notice-later", "Later", None, Tone::Ghost)
                        .tooltip(tip(t, "Hide until the next update", None))
                        .on_click(cx.listener(move |s, _, _, cx| {
                            cx.stop_propagation();
                            s.panels.dismissed_update = Some(dismissed.clone());
                            cx.notify();
                        })),
                ),
        )
        .with_animation("update-notice", enter(), |card, delta| {
            card.opacity(delta).mb(px((1. - delta) * -8.))
        }),
    )
}
