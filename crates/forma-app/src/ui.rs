//! Native workspace chrome. Rendering and interaction state live in `Studio`.
use crate::app::{Command, Field, Studio, Tool};
use crate::platform_shortcut;
use crate::shading_pie::{CARD_HALF_SIZE, CHOICES};
use forma_core::{Primitive, ShaderKind, TextureMapping, TextureSlot};
use forma_render::{RenderMode, StudioLight};
use gpui::{prelude::*, *};

// Neutral charcoal surfaces keep the scene's color distinct from editor chrome.
const SHELL: u32 = 0x1d1e20;
const PANEL: u32 = 0x292a2d;
const CARD: u32 = 0x333437;
const RAISED: u32 = 0x3c3e42;
const INPUT: u32 = 0x46484c;
const WELL: u32 = 0x222326;
const LINE: u32 = 0x1c1d1f;
const EDGE: u32 = 0x4b4d51;
// Ink, primary to faintest.
pub(crate) const TEXT: u32 = 0xe1e2e4;
pub(crate) const MUTED: u32 = 0xa5a7ad;
const FAINT: u32 = 0x7d8087;
// Accent and state.
pub(crate) const ACCENT: u32 = 0x96d5c2;
pub(crate) const ACCENT_LINE: u32 = 0x597d71;
pub(crate) const ACTIVE: u32 = 0x354e46;
const ACTIVE_HOVER: u32 = 0x405f55;
pub(crate) const ALERT: u32 = 0xd7a175;
/// Fully transparent fill for the resting state of ghost controls.
fn clear() -> Rgba {
    rgba(0x00000000)
}
/// Axis identity, shared with the viewport gizmo.
pub(crate) const AXIS: [u32; 3] = [0xe77778, 0x83c799, 0x7b9ee8];
/// The same identity dimmed for small field labels.
const AXIS_INK: [u32; 3] = [0xe0a0a1, 0xa7cfb1, 0xa5bde9];

#[derive(Clone, Copy)]
enum PanelSection {
    Transform,
    Surface,
    Geometry,
    Render,
    AddGeometry,
}

impl PanelSection {
    fn contains_field(self, field: Field) -> bool {
        matches!(
            (self, field),
            (
                Self::Transform,
                Field::Translation(_) | Field::Rotation(_) | Field::Scale(_)
            ) | (
                Self::Surface,
                Field::Color(_)
                    | Field::Roughness
                    | Field::Metallic
                    | Field::Emission
                    | Field::Ior
                    | Field::NormalStrength
                    | Field::TextureScale(_)
                    | Field::TextureOffset(_)
            ) | (
                Self::Render,
                Field::Samples | Field::Bounces | Field::Exposure | Field::WorldStrength
            )
        )
    }
}

/// Workspace disclosure state is independent of the document and undo history.
pub(crate) struct PanelState {
    open: [bool; 5],
    collection_open: bool,
}

impl Default for PanelState {
    fn default() -> Self {
        Self {
            open: [true, true, false, true, false],
            collection_open: true,
        }
    }
}

/// Surface presets: swatch color, name, and the linear base color they apply.
const PRESETS: [(u32, &str, [f32; 3]); 5] = [
    (0x83b7a6, "Jade", [0.045, 0.42, 0.32]),
    (0xe4e4dc, "Porcelain", [0.73, 0.76, 0.73]),
    (0xc38967, "Copper", [0.76, 0.32, 0.13]),
    (0x525860, "Graphite", [0.055, 0.055, 0.055]),
    (0xf3e9c9, "Light", [0.9, 0.83, 0.66]),
];

struct Tooltip(&'static str);

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(8.))
            .py(px(5.))
            .rounded(px(5.))
            .bg(rgb(PANEL))
            .border_1()
            .border_color(rgb(EDGE))
            .shadow_md()
            .text_color(rgb(TEXT))
            .text_size(px(11.))
            .child(self.0)
    }
}

fn command_hint(command: Command) -> &'static str {
    match command {
        Command::New => platform_shortcut("New project · ⌘ N", "New project · Ctrl+N"),
        Command::Open => platform_shortcut("Open project · ⌘ O", "Open project · Ctrl+O"),
        Command::Save => platform_shortcut("Save project · ⌘ S", "Save project · Ctrl+S"),
        Command::SaveAs => {
            platform_shortcut("Save project as · ⇧ ⌘ S", "Save project as · Ctrl+Shift+S")
        }
        Command::ImportObj => "Import Wavefront OBJ",
        Command::ExportObj => "Export scene geometry as OBJ",
        Command::ExportImage => "Export current rendered image as PNG",
        Command::Add(Primitive::Cube) => "Add cube",
        Command::Add(Primitive::Sphere) => "Add sphere",
        Command::Add(Primitive::Cylinder) => "Add cylinder",
        Command::Add(Primitive::Torus) => "Add torus",
        Command::Add(Primitive::Plane) => "Add plane",
        Command::Select(_) => "Select object",
        Command::ToggleVisible(_) => "Toggle object visibility",
        Command::Delete => "Delete selection · Delete",
        Command::Duplicate => "Duplicate selection · ⇧ D",
        Command::Undo => platform_shortcut("Undo · ⌘ Z", "Undo · Ctrl+Z"),
        Command::Redo => platform_shortcut("Redo · ⇧ ⌘ Z", "Redo · Ctrl+Shift+Z"),
        Command::FrameSelected => "Frame selected · F",
        Command::ViewFront => "Front view · 1",
        Command::ViewRight => "Right view · 3",
        Command::ViewTop => "Top view · 7",
        Command::ViewPerspective => "Perspective view · 0",
        Command::ToggleProjection => "Toggle perspective / orthographic · 5",
        Command::SetMode(RenderMode::Wireframe) => "Wireframe · Z, then 4",
        Command::SetMode(RenderMode::Solid) => "Solid shading · X",
        Command::SetMode(RenderMode::MaterialPreview) => "Material preview · C",
        Command::SetMode(RenderMode::Rendered) => "Progressive path tracing · V",
        Command::SetTool(Tool::Select) => "Select · Q",
        Command::SetTool(Tool::Move) => "Move · G",
        Command::SetTool(Tool::Rotate) => "Rotate · R",
        Command::SetTool(Tool::Scale) => "Scale · S",
        Command::ToggleEdit => "Toggle object / face edit · Tab",
        Command::Extrude => "Extrude selected face · E",
        Command::Subdivide => "Subdivide selected mesh",
        Command::ToggleGrid => "Toggle ground grid",
        Command::MaterialPreset(_) => "Apply surface preset",
        Command::SetShader(_) => "Choose a surface shader",
        Command::SetTextureMapping(_) => "Choose generated image coordinates",
        Command::LoadTexture(_) => "Choose a PNG or JPEG image texture",
        Command::ClearTexture(_) => "Remove this texture",
        Command::EditShader => "Edit custom surface code",
        Command::ApplyShader => platform_shortcut(
            "Compile and apply · ⌘ Return",
            "Compile and apply · Ctrl+Return",
        ),
        Command::CloseShader => "Close shader editor · Esc",
        Command::TogglePalette => {
            platform_shortcut("Workspace commands · ⌘ K", "Workspace commands · Ctrl+K")
        }
        Command::ToggleHelp => "Keyboard reference · ?",
        Command::TogglePreviewSettings => "Material preview lighting",
        Command::SetPreviewStudio(_) => "Use this studio environment for material preview",
        Command::TogglePreviewWorld => "Use the project's world instead of a studio environment",
        Command::TogglePreviewAo => "Ambient occlusion for grounded contact shading",
        Command::LoadPreviewHdri => "Load a Radiance HDR environment",
        Command::ResetPreview => "Reset material preview lighting",
    }
}

fn row() -> Div {
    div().flex().items_center()
}
fn col() -> Div {
    div().flex().flex_col()
}
fn divider() -> Div {
    div().w(px(1.)).h(px(16.)).flex_shrink_0().bg(rgb(EDGE))
}
fn key(text: &str) -> Div {
    div()
        .px(px(5.))
        .h(px(17.))
        .flex_shrink_0()
        .rounded(px(3.))
        .bg(rgb(WELL))
        .border_1()
        .border_color(rgb(LINE))
        .text_size(px(10.))
        .text_color(rgb(FAINT))
        .flex()
        .items_center()
        .justify_center()
        .child(text.to_owned())
}
/// Quiet section label for popovers and editor headers.
fn section(text: &str) -> Div {
    row()
        .h(px(24.))
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(MUTED))
        .child(text.to_owned())
}
fn caption(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(10.))
        .text_color(rgb(FAINT))
        .child(text.into())
}
/// Inset track that groups mutually exclusive choices.
fn segmented() -> Div {
    row()
        .p(px(1.))
        .gap(px(1.))
        .rounded(px(4.))
        .bg(rgb(WELL))
        .border_1()
        .border_color(rgb(LINE))
}

/// Each editor has a crisp boundary against the narrow workspace gutters.
fn editor() -> Div {
    col()
        .min_w(px(0.))
        .min_h(px(0.))
        .rounded(px(5.))
        .bg(rgb(PANEL))
        .border_1()
        .border_color(rgb(LINE))
        .overflow_hidden()
}

fn editor_header() -> Div {
    row()
        .h(px(29.))
        .flex_shrink_0()
        .px(px(8.))
        .gap(px(7.))
        .bg(rgb(PANEL))
        .border_b_1()
        .border_color(rgb(LINE))
}

fn panel_card(
    s: &Studio,
    panel: PanelSection,
    title: &'static str,
    detail: impl Into<SharedString>,
    contents: impl IntoElement,
    cx: &mut Context<Studio>,
) -> Div {
    let open = s.panels.open[panel as usize];
    col()
        .flex_shrink_0()
        .rounded(px(4.))
        .border_1()
        .border_color(rgb(EDGE))
        .bg(rgb(CARD))
        .overflow_hidden()
        .child(
            row()
                .id(SharedString::from(format!("panel-{}", panel as usize)))
                .h(px(26.))
                .flex_shrink_0()
                .px(px(7.))
                .gap(px(6.))
                .cursor_pointer()
                .hover(|d| d.bg(rgb(RAISED)))
                .child(icon(
                    if open {
                        Icon::Chevron
                    } else {
                        Icon::ChevronRight
                    },
                    MUTED,
                    11.,
                ))
                .child(div().font_weight(FontWeight::MEDIUM).child(title))
                .child(div().flex_1())
                .child(caption(detail))
                .on_click(cx.listener(move |s, _, _, cx| {
                    cx.stop_propagation();
                    s.panels.open[panel as usize] = !s.panels.open[panel as usize];
                    // A collapsed field must not keep consuming keyboard input.
                    if !s.panels.open[panel as usize]
                        && s.active_field
                            .as_ref()
                            .is_some_and(|(field, _)| panel.contains_field(*field))
                    {
                        s.active_field = None;
                        s.status = "Edit cancelled".into();
                    }
                    cx.notify();
                })),
        )
        .when(open, |d| {
            d.child(col().px(px(8.)).pb(px(8.)).pt(px(3.)).child(contents))
        })
}

#[derive(Clone, Copy, PartialEq)]
enum Icon {
    Cube,
    Sphere,
    Cylinder,
    Torus,
    Plane,
    Select,
    Move,
    Rotate,
    Scale,
    Grid,
    Eye,
    Hidden,
    Folder,
    Save,
    Export,
    Frame,
    Search,
    Help,
    Wire,
    Solid,
    Material,
    Render,
    Chevron,
    ChevronRight,
    Plus,
}

/// Stroke icons on a 16 unit grid, scaled to `side` and painted in one pass.
fn icon(kind: Icon, color: u32, side: f32) -> AnyElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            const TAU: f32 = std::f32::consts::TAU;
            const PI: f32 = std::f32::consts::PI;
            let arc = |x: f32, y: f32, rx: f32, ry: f32, from: f32, to: f32| {
                let steps = ((to - from).abs() / TAU * 32.).ceil().max(4.) as usize;
                (0..=steps)
                    .map(|i| {
                        let a = from + (to - from) * i as f32 / steps as f32;
                        (x + a.cos() * rx, y + a.sin() * ry)
                    })
                    .collect::<Vec<_>>()
            };
            let ellipse = |x: f32, y: f32, rx: f32, ry: f32| arc(x, y, rx, ry, 0., TAU);
            let circle = |x: f32, y: f32, r: f32| arc(x, y, r, r, 0., TAU);
            let dial = |detail: Vec<Vec<(f32, f32)>>| {
                let mut paths = vec![circle(8., 8., 6.)];
                paths.extend(detail);
                paths
            };
            let paths: Vec<Vec<(f32, f32)>> = match kind {
                Icon::Cube => vec![
                    vec![
                        (8., 1.6),
                        (14., 5.1),
                        (14., 11.9),
                        (8., 15.4),
                        (2., 11.9),
                        (2., 5.1),
                        (8., 1.6),
                    ],
                    vec![(2., 5.1), (8., 8.6), (14., 5.1)],
                    vec![(8., 8.6), (8., 15.4)],
                ],
                Icon::Sphere => vec![
                    circle(8., 8., 6.2),
                    ellipse(8., 8., 2.6, 6.2),
                    arc(8., 8., 6.2, 2.3, 0., PI),
                ],
                Icon::Cylinder => vec![
                    ellipse(8., 4.3, 5., 2.3),
                    vec![(3., 4.3), (3., 11.7)],
                    vec![(13., 4.3), (13., 11.7)],
                    arc(8., 11.7, 5., 2.3, 0., PI),
                ],
                Icon::Torus => vec![ellipse(8., 8., 6.6, 5.), ellipse(8., 8., 3.3, 2.5)],
                Icon::Plane => vec![
                    vec![
                        (1.2, 12.4),
                        (5.8, 4.6),
                        (14.8, 4.6),
                        (10.2, 12.4),
                        (1.2, 12.4),
                    ],
                    vec![(3.5, 8.5), (12.5, 8.5)],
                ],
                Icon::Select => vec![vec![(3., 2.), (13., 9.), (8., 10.), (5., 14.), (3., 2.)]],
                Icon::Move => vec![
                    vec![(8., 1.), (8., 15.)],
                    vec![(1., 8.), (15., 8.)],
                    vec![(5., 4.), (8., 1.), (11., 4.)],
                    vec![(12., 5.), (15., 8.), (12., 11.)],
                    vec![(5., 12.), (8., 15.), (11., 12.)],
                    vec![(4., 5.), (1., 8.), (4., 11.)],
                ],
                Icon::Rotate => vec![
                    arc(8., 8., 6., 6., -1.9, 3.5),
                    vec![(10., 6.), (13.2, 5.6), (13.8, 2.4)],
                ],
                Icon::Scale => vec![
                    vec![(2., 10.), (2., 14.), (6., 14.), (6., 10.), (2., 10.)],
                    vec![(7., 9.), (14., 2.)],
                    vec![(9., 2.), (14., 2.), (14., 7.)],
                ],
                Icon::Grid => vec![
                    vec![(2., 2.), (14., 2.), (14., 14.), (2., 14.), (2., 2.)],
                    vec![(6., 2.), (6., 14.)],
                    vec![(10., 2.), (10., 14.)],
                    vec![(2., 6.), (14., 6.)],
                    vec![(2., 10.), (14., 10.)],
                ],
                Icon::Eye => vec![
                    vec![
                        (1., 8.),
                        (4., 4.),
                        (8., 3.),
                        (12., 4.),
                        (15., 8.),
                        (12., 12.),
                        (8., 13.),
                        (4., 12.),
                        (1., 8.),
                    ],
                    circle(8., 8., 2.),
                ],
                Icon::Hidden => vec![
                    vec![
                        (1., 8.),
                        (4., 4.),
                        (8., 3.),
                        (12., 4.),
                        (15., 8.),
                        (12., 12.),
                        (8., 13.),
                        (4., 12.),
                        (1., 8.),
                    ],
                    vec![(2., 1.5), (14., 14.5)],
                ],
                Icon::Folder => vec![vec![
                    (2., 13.),
                    (2., 3.),
                    (6., 3.),
                    (8., 5.),
                    (14., 5.),
                    (14., 13.),
                    (2., 13.),
                ]],
                Icon::Save => vec![
                    vec![
                        (2., 2.),
                        (12., 2.),
                        (14., 4.),
                        (14., 14.),
                        (2., 14.),
                        (2., 2.),
                    ],
                    vec![(5., 2.), (5., 6.), (11., 6.), (11., 2.)],
                    vec![(5., 14.), (5., 10.), (11., 10.), (11., 14.)],
                ],
                Icon::Export => vec![
                    vec![(8., 11.), (8., 1.)],
                    vec![(4., 5.), (8., 1.), (12., 5.)],
                    vec![(2., 10.), (2., 14.), (14., 14.), (14., 10.)],
                ],
                Icon::Frame => vec![
                    vec![(6., 2.), (2., 2.), (2., 6.)],
                    vec![(10., 2.), (14., 2.), (14., 6.)],
                    vec![(14., 10.), (14., 14.), (10., 14.)],
                    vec![(6., 14.), (2., 14.), (2., 10.)],
                ],
                Icon::Search => vec![circle(6.8, 6.8, 4.6), vec![(10.2, 10.2), (14.6, 14.6)]],
                Icon::Help => vec![
                    vec![
                        (5., 5.2),
                        (5., 3.2),
                        (7., 2.),
                        (10., 2.),
                        (12., 4.2),
                        (11., 6.2),
                        (8., 8.2),
                        (8., 10.2),
                    ],
                    vec![(8., 13.), (8., 14.)],
                ],
                Icon::Wire => dial(vec![
                    vec![(2., 8.), (14., 8.)],
                    vec![
                        (8., 2.),
                        (5., 5.),
                        (5., 11.),
                        (8., 14.),
                        (11., 11.),
                        (11., 5.),
                        (8., 2.),
                    ],
                ]),
                Icon::Solid => dial(
                    [8., 10., 12.]
                        .into_iter()
                        .map(|x| vec![(x, 4.), (x, 12.)])
                        .collect(),
                ),
                Icon::Material => dial(vec![vec![(4., 12.), (12., 4.)], circle(5.6, 5.6, 1.1)]),
                Icon::Render => dial(vec![
                    circle(8., 8., 2.5),
                    vec![(8., 0.4), (8., 3.)],
                    vec![(8., 13.), (8., 15.6)],
                ]),
                Icon::Chevron => vec![vec![(5., 6.5), (8., 9.5), (11., 6.5)]],
                Icon::ChevronRight => vec![vec![(6.5, 4.), (10., 8.), (6.5, 12.)]],
                Icon::Plus => vec![vec![(3., 8.), (13., 8.)], vec![(8., 3.), (8., 13.)]],
            };
            let weight = px((side / 16. * 1.25).max(1.));
            for points in paths {
                let mut path = PathBuilder::stroke(weight);
                for (i, (x, y)) in points.into_iter().enumerate() {
                    let p = bounds.origin + point(px(x * side / 16.), px(y * side / 16.));
                    if i == 0 {
                        path.move_to(p)
                    } else {
                        path.line_to(p)
                    }
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, rgb(color));
                }
            }
        },
    )
    .size(px(side))
    .flex_shrink_0()
    .into_any_element()
}

/// Clickable base: hint tooltip and command dispatch, without any styling.
fn action(
    id: impl Into<SharedString>,
    command: Command,
    cx: &mut Context<Studio>,
) -> Stateful<Div> {
    row()
        .id(ElementId::from(id.into()))
        .flex_shrink_0()
        .cursor_pointer()
        .tooltip(move |_, cx| cx.new(|_| Tooltip(command_hint(command))).into())
        .on_click(cx.listener(move |s, _, window, cx| {
            cx.stop_propagation();
            s.execute(command, window, cx);
        }))
}

/// Labelled button. `active` gives it the accented on state.
fn button(
    id: impl Into<SharedString>,
    label: &str,
    glyph: Option<Icon>,
    command: Command,
    active: bool,
    cx: &mut Context<Studio>,
) -> Stateful<Div> {
    let ink = if active { ACCENT } else { MUTED };
    action(id, command, cx)
        .h(px(23.))
        .px(px(7.))
        .gap(px(5.))
        .rounded(px(3.))
        .text_color(rgb(ink))
        .bg(if active { rgb(ACTIVE) } else { clear() })
        .hover(move |s| {
            if active {
                s.bg(rgb(ACTIVE_HOVER))
            } else {
                s.bg(rgb(RAISED)).text_color(rgb(TEXT))
            }
        })
        .when_some(glyph, |d, glyph| d.child(icon(glyph, ink, 13.)))
        .when(!label.is_empty(), |d| d.child(label.to_owned()))
}

/// Square icon-only button, for rails and row affordances.
fn icon_button(
    id: impl Into<SharedString>,
    glyph: Icon,
    command: Command,
    active: bool,
    side: f32,
    cx: &mut Context<Studio>,
) -> Stateful<Div> {
    let ink = if active { ACCENT } else { MUTED };
    action(id, command, cx)
        .size(px(side))
        .justify_center()
        .rounded(px(3.))
        .bg(if active { rgb(ACTIVE) } else { clear() })
        .hover(move |s| {
            if active {
                s.bg(rgb(ACTIVE_HOVER))
            } else {
                s.bg(rgb(RAISED))
            }
        })
        .child(icon(glyph, ink, (side * 0.55).round()))
}

/// The single emphasised action in the window.
fn primary(
    id: impl Into<SharedString>,
    label: &str,
    glyph: Icon,
    command: Command,
    cx: &mut Context<Studio>,
) -> Stateful<Div> {
    action(id, command, cx)
        .h(px(24.))
        .px(px(9.))
        .gap(px(6.))
        .rounded(px(4.))
        .bg(rgb(ACTIVE))
        .border_1()
        .border_color(rgb(ACCENT_LINE))
        .text_color(rgb(ACCENT))
        .hover(|s| s.bg(rgb(ACTIVE_HOVER)))
        .child(icon(glyph, ACCENT, 13.))
        .child(label.to_owned())
}

fn titlebar(s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    row()
        .h(px(36.))
        .flex_shrink_0()
        .pl(px(if cfg!(target_os = "macos") { 80. } else { 12. }))
        .pr(px(8.))
        .gap(px(8.))
        .border_b_1()
        .border_color(rgb(LINE))
        .bg(rgb(PANEL))
        .child(
            row()
                .gap(px(9.))
                .flex_shrink_0()
                .child(icon(Icon::Cube, ACCENT, 16.))
                .child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .text_size(px(12.))
                        .text_color(rgb(TEXT))
                        .child("FORMA"),
                ),
        )
        .child(divider())
        .child(
            row()
                .gap(px(7.))
                .min_w(px(0.))
                .child(
                    div()
                        .max_w(px(260.))
                        .min_w(px(0.))
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_color(rgb(TEXT))
                        .child(s.project_name.clone()),
                )
                .when(s.dirty, |d| {
                    d.child(
                        div()
                            .size(px(5.))
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(rgb(ACCENT)),
                    )
                }),
        )
        .child(div().flex_1())
        .child(
            button(
                "commands",
                "Commands",
                Some(Icon::Search),
                Command::TogglePalette,
                s.palette_open,
                cx,
            )
            .child(key(platform_shortcut("⌘K", "Ctrl+K"))),
        )
        .child(divider())
        .child(button(
            "open",
            "Open",
            Some(Icon::Folder),
            Command::Open,
            false,
            cx,
        ))
        .child(button(
            "save",
            "Save",
            Some(Icon::Save),
            Command::Save,
            false,
            cx,
        ))
        .child(primary(
            "export-image",
            "Export image",
            Icon::Export,
            Command::ExportImage,
            cx,
        ))
        .into_any_element()
}

/// Selection-mode segment. Only the inactive half dispatches the toggle.
fn mode_segment(
    id: &'static str,
    label: &'static str,
    active: bool,
    cx: &mut Context<Studio>,
) -> Stateful<Div> {
    row()
        .id(id)
        .h(px(22.))
        .px(px(9.))
        .rounded(px(3.))
        .justify_center()
        .text_color(rgb(if active { ACCENT } else { MUTED }))
        .bg(if active { rgb(ACTIVE) } else { clear() })
        .child(label)
        .when(!active, |d| {
            d.cursor_pointer()
                .hover(|s| s.bg(rgb(RAISED)).text_color(rgb(TEXT)))
                .tooltip(|_, cx| {
                    cx.new(|_| Tooltip(command_hint(Command::ToggleEdit)))
                        .into()
                })
                .on_click(cx.listener(|s, _, window, cx| {
                    cx.stop_propagation();
                    s.execute(Command::ToggleEdit, window, cx);
                }))
        })
}

/// Everything that acts on the viewport: what you select, how it shades, where
/// the camera looks.
fn toolbar(s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    editor_header()
        .h(px(32.))
        .child(icon(Icon::Cube, MUTED, 14.))
        .child(
            segmented()
                .child(mode_segment("mode-object", "Object", !s.edit_mode, cx))
                .child(mode_segment("mode-face", "Face", s.edit_mode, cx)),
        )
        .child(divider())
        .child(
            row()
                .id("add-menu")
                .h(px(23.))
                .px(px(7.))
                .gap(px(5.))
                .rounded(px(3.))
                .cursor_pointer()
                .text_color(rgb(MUTED))
                .hover(|d| d.bg(rgb(RAISED)).text_color(rgb(TEXT)))
                .tooltip(|_, cx| cx.new(|_| Tooltip("Add geometry")).into())
                .child(icon(Icon::Plus, MUTED, 12.))
                .child("Add")
                .child(icon(Icon::Chevron, MUTED, 10.))
                .on_click(cx.listener(|s, _, w, cx| {
                    s.execute(Command::TogglePalette, w, cx);
                    s.palette_query = "Add ".into();
                    s.palette_index = 0;
                })),
        )
        .child(div().flex_1())
        .child(
            row()
                .gap(px(1.))
                .child(button(
                    "view-front",
                    "Front",
                    None,
                    Command::ViewFront,
                    false,
                    cx,
                ))
                .child(button(
                    "view-right",
                    "Right",
                    None,
                    Command::ViewRight,
                    false,
                    cx,
                ))
                .child(button("view-top", "Top", None, Command::ViewTop, false, cx))
                .child(icon_button(
                    "projection",
                    Icon::Cube,
                    Command::ToggleProjection,
                    s.scene.camera.orthographic,
                    23.,
                    cx,
                )),
        )
        .child(divider())
        .child(shading_controls(s, cx))
        .into_any_element()
}

fn outliner(s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    let objects =
        s.scene
            .objects
            .iter()
            .enumerate()
            .map(|(index, object)| {
                let id = object.id;
                let selected = s.selected == Some(id);
                let ink = if !object.visible { FAINT } else { TEXT };
                row()
                    .id(SharedString::from(format!("object-{id}")))
                    .h(px(22.))
                    .flex_shrink_0()
                    .pr(px(5.))
                    .gap(px(6.))
                    .bg(rgb(if selected {
                        ACTIVE
                    } else if index % 2 == 0 {
                        PANEL
                    } else {
                        0x2c2d30
                    }))
                    .cursor_pointer()
                    .hover(move |d| d.bg(rgb(if selected { ACTIVE_HOVER } else { RAISED })))
                    .child(div().w(px(2.)).h_full().bg(if selected {
                        rgb(ACCENT)
                    } else {
                        clear()
                    }))
                    .child(
                        div()
                            .w(px(19.))
                            .h_full()
                            .flex_shrink_0()
                            .border_r_1()
                            .border_color(rgb(EDGE)),
                    )
                    .child(icon(
                        Icon::Cube,
                        if object.visible { ALERT } else { FAINT },
                        12.,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_color(rgb(ink))
                            .child(object.name.clone()),
                    )
                    .child(icon_button(
                        format!("visibility-{id}"),
                        if object.visible {
                            Icon::Eye
                        } else {
                            Icon::Hidden
                        },
                        Command::ToggleVisible(id),
                        false,
                        20.,
                        cx,
                    ))
                    .on_click(cx.listener(move |s, _, w, cx| s.execute(Command::Select(id), w, cx)))
            })
            .collect::<Vec<_>>();
    editor()
        .h(px(190.))
        .flex_shrink_0()
        .child(
            editor_header()
                .child(icon(Icon::Folder, MUTED, 13.))
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child("Scene Collection"),
                )
                .child(div().flex_1())
                .child(caption(s.scene.objects.len().to_string()))
                .child(icon_button(
                    "scene-commands",
                    Icon::Search,
                    Command::TogglePalette,
                    false,
                    21.,
                    cx,
                )),
        )
        .child(
            col()
                .id("scene-objects")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(
                    row()
                        .id("scene-collection")
                        .h(px(25.))
                        .flex_shrink_0()
                        .px(px(8.))
                        .gap(px(7.))
                        .cursor_pointer()
                        .hover(|d| d.bg(rgb(RAISED)))
                        .child(icon(
                            if s.panels.collection_open {
                                Icon::Chevron
                            } else {
                                Icon::ChevronRight
                            },
                            MUTED,
                            11.,
                        ))
                        .child(icon(Icon::Folder, MUTED, 12.))
                        .child("Collection")
                        .on_click(cx.listener(|s, _, _, cx| {
                            s.panels.collection_open = !s.panels.collection_open;
                            cx.notify();
                        })),
                )
                .when(s.panels.collection_open, |d| d.children(objects)),
        )
        .into_any_element()
}

/// Everything that puts new geometry in the scene.
fn sources() -> [(Icon, &'static str, Command); 6] {
    [
        (Icon::Cube, "Cube", Command::Add(Primitive::Cube)),
        (Icon::Sphere, "Sphere", Command::Add(Primitive::Sphere)),
        (
            Icon::Cylinder,
            "Cylinder",
            Command::Add(Primitive::Cylinder),
        ),
        (Icon::Torus, "Torus", Command::Add(Primitive::Torus)),
        (Icon::Plane, "Plane", Command::Add(Primitive::Plane)),
        (Icon::Folder, "OBJ file", Command::ImportObj),
    ]
}

fn toolrail(s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    col()
        .w(px(36.))
        .h_full()
        .flex_shrink_0()
        .items_center()
        .py(px(5.))
        .gap(px(2.))
        .bg(rgb(PANEL))
        .border_r_1()
        .border_color(rgb(LINE))
        .children(
            [
                (Tool::Select, Icon::Select),
                (Tool::Move, Icon::Move),
                (Tool::Rotate, Icon::Rotate),
                (Tool::Scale, Icon::Scale),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, (tool, glyph))| {
                icon_button(
                    format!("tool-{i}"),
                    glyph,
                    Command::SetTool(tool),
                    s.tool == tool,
                    28.,
                    cx,
                )
            }),
        )
        .child(div().w(px(16.)).h(px(1.)).my(px(5.)).bg(rgb(LINE)))
        .child(icon_button(
            "rail-frame",
            Icon::Frame,
            Command::FrameSelected,
            false,
            28.,
            cx,
        ))
        .child(icon_button(
            "rail-grid",
            Icon::Grid,
            Command::ToggleGrid,
            s.settings.show_grid,
            28.,
            cx,
        ))
        .into_any_element()
}

fn viewport_panel(s: &Studio, viewport: AnyElement, cx: &mut Context<Studio>) -> AnyElement {
    editor()
        .flex_1()
        .min_w(px(220.))
        .h_full()
        .child(toolbar(s, cx))
        .child(
            row()
                .flex_1()
                .min_h(px(0.))
                .overflow_hidden()
                .child(toolrail(s, cx))
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_w(px(0.))
                        .h_full()
                        .overflow_hidden()
                        .child(viewport),
                ),
        )
        .into_any_element()
}

/// Shading lives in the viewport header, alongside its other display controls.
fn shading_controls(s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    segmented()
        .children(
            [
                (RenderMode::Wireframe, Icon::Wire),
                (RenderMode::Solid, Icon::Solid),
                (RenderMode::MaterialPreview, Icon::Material),
                (RenderMode::Rendered, Icon::Render),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, (mode, glyph))| {
                icon_button(
                    format!("render-mode-{i}"),
                    glyph,
                    Command::SetMode(mode),
                    s.settings.mode == mode,
                    22.,
                    cx,
                )
            }),
        )
        .when(s.settings.mode == RenderMode::MaterialPreview, |d| {
            d.child(icon_button(
                "shading-lighting",
                Icon::Chevron,
                Command::TogglePreviewSettings,
                s.preview_open,
                20.,
                cx,
            ))
        })
        .into_any_element()
}

/// Axis tint for a single-letter field label.
fn axis_ink(label: &str) -> u32 {
    match label {
        "X" | "R" => AXIS_INK[0],
        "Y" | "G" => AXIS_INK[1],
        "Z" | "B" => AXIS_INK[2],
        _ => MUTED,
    }
}

/// Editable numeric well. Callers give it its width.
fn field(s: &Studio, id: &str, label: &str, f: Field, cx: &mut Context<Studio>) -> Stateful<Div> {
    let active = s.field_is_active(f);
    let value = match &s.active_field {
        Some((field, text)) if *field == f => format!("{text}│"),
        _ => s.field_value(f),
    };
    row()
        .id(SharedString::from(id.to_owned()))
        .h(px(21.))
        .flex_shrink_0()
        .px(px(7.))
        .gap(px(5.))
        .rounded(px(3.))
        .bg(rgb(if active { WELL } else { INPUT }))
        .border_1()
        .border_color(if active { rgb(ACCENT) } else { clear() })
        .cursor_pointer()
        .hover(move |d| {
            if active {
                d.border_color(rgb(ACCENT))
            } else {
                d.bg(rgb(0x53565b))
            }
        })
        .when(!label.is_empty(), |d| {
            d.child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(axis_ink(label)))
                    .child(label.to_owned()),
            )
        })
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .text_ellipsis()
                .text_color(rgb(if active { ACCENT } else { TEXT }))
                .text_right()
                .when(!label.is_empty(), |d| d.text_center())
                .when(f == Field::Name, |d| d.text_left())
                .child(value),
        )
        .on_click(cx.listener(move |s, _, w, cx| {
            cx.stop_propagation();
            s.begin_field(f, w, cx);
        }))
}

/// Label on the left, editable value on the right.
fn property(s: &Studio, id: &str, label: &str, f: Field, cx: &mut Context<Studio>) -> AnyElement {
    row()
        .h(px(23.))
        .gap(px(8.))
        .text_color(rgb(MUTED))
        .child(
            div()
                .w(px(92.))
                .flex_shrink_0()
                .text_right()
                .child(label.to_owned()),
        )
        .child(field(s, id, "", f, cx).flex_1().min_w(px(0.)))
        .into_any_element()
}

/// Blender-style vector stack: one group label and contiguous axis controls.
fn axis_row(
    s: &Studio,
    label: &str,
    id: &str,
    axes: [&str; 3],
    field_of: impl Fn(usize) -> Field,
    cx: &mut Context<Studio>,
) -> AnyElement {
    row()
        .items_start()
        .gap(px(8.))
        .child(
            div()
                .w(px(73.))
                .flex_shrink_0()
                .pt(px(3.))
                .text_right()
                .text_color(rgb(MUTED))
                .child(label.to_owned()),
        )
        .child(
            col()
                .flex_1()
                .min_w(px(0.))
                .gap(px(1.))
                .rounded(px(3.))
                .overflow_hidden()
                .children((0..3).map(|axis| {
                    field(s, &format!("{id}-{axis}"), axes[axis], field_of(axis), cx)
                        .w_full()
                        .rounded(px(0.))
                })),
        )
        .into_any_element()
}

fn swatch_color(v: glam::Vec3) -> u32 {
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

fn preview_light_name(s: &Studio) -> String {
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

/// Sampling and lighting controls for the modes that have them.
fn render_settings(s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    let mode = s.settings.mode;
    let preview = mode == RenderMode::MaterialPreview;
    let progressive = mode.progressive();
    let contents = col()
        .gap(px(1.))
        .when(preview, |d| {
            d.child(
                row()
                    .h(px(23.))
                    .gap(px(8.))
                    .child(
                        div()
                            .w(px(92.))
                            .flex_shrink_0()
                            .text_right()
                            .text_color(rgb(MUTED))
                            .child("Lighting"),
                    )
                    .child(
                        action(
                            "inspector-preview-settings",
                            Command::TogglePreviewSettings,
                            cx,
                        )
                        .flex_1()
                        .min_w(px(0.))
                        .h(px(21.))
                        .px(px(7.))
                        .gap(px(5.))
                        .rounded(px(3.))
                        .bg(rgb(WELL))
                        .border_1()
                        .border_color(rgb(EDGE))
                        .hover(|d| d.bg(rgb(RAISED)))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(preview_light_name(s)),
                        )
                        .child(icon(Icon::Chevron, MUTED, 10.)),
                    ),
            )
        })
        .when(progressive, |d| {
            d.child(property(s, "samples", "Max samples", Field::Samples, cx))
                .child(property(s, "bounces", "Light bounces", Field::Bounces, cx))
        })
        .child(property(s, "exposure", "Exposure", Field::Exposure, cx))
        .when(
            progressive || (preview && s.settings.preview.use_scene_world),
            |d| {
                d.child(property(
                    s,
                    "world-strength",
                    "World strength",
                    Field::WorldStrength,
                    cx,
                ))
            },
        )
        .when(progressive, |d| {
            d.child(
                row()
                    .mt(px(6.))
                    .justify_between()
                    .child(caption("Samples"))
                    .child(caption(format!(
                        "{} / {}",
                        s.samples, s.settings.max_samples
                    ))),
            )
            .child(
                div()
                    .h(px(3.))
                    .mt(px(4.))
                    .rounded_full()
                    .bg(rgb(WELL))
                    .child(
                        div()
                            .h_full()
                            .rounded_full()
                            .w(relative(
                                (s.samples as f32 / s.settings.max_samples.max(1) as f32)
                                    .clamp(0., 1.),
                            ))
                            .bg(rgb(ACCENT)),
                    ),
            )
        });
    panel_card(
        s,
        PanelSection::Render,
        if preview {
            "Material Preview"
        } else {
            "Render"
        },
        if progressive {
            "Path tracing"
        } else {
            "Lighting"
        },
        contents,
        cx,
    )
    .into_any_element()
}

fn geometry_sources(cx: &mut Context<Studio>) -> Div {
    col()
        .gap(px(3.))
        .children(sources().chunks(2).enumerate().map(|(r, pair)| {
            row().gap(px(3.)).children(pair.iter().enumerate().map(
                |(c, (glyph, label, command))| {
                    button(
                        format!("add-{r}-{c}"),
                        label,
                        Some(*glyph),
                        *command,
                        false,
                        cx,
                    )
                    .flex_1()
                    .min_w(px(0.))
                    .bg(rgb(RAISED))
                },
            ))
        }))
}

fn inspector(s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    let object = s.selected_object();
    let mut contents = col().flex_shrink_0().p(px(5.)).gap(px(4.));
    if let Some(object) = object {
        let base = object.material.base_color;
        let transform = col()
            .gap(px(7.))
            .child(axis_row(
                s,
                "Location",
                "transform-0",
                ["X", "Y", "Z"],
                Field::Translation,
                cx,
            ))
            .child(axis_row(
                s,
                "Rotation · °",
                "transform-1",
                ["X", "Y", "Z"],
                Field::Rotation,
                cx,
            ))
            .child(axis_row(
                s,
                "Scale",
                "transform-2",
                ["X", "Y", "Z"],
                Field::Scale,
                cx,
            ));
        contents = contents.child(panel_card(
            s,
            PanelSection::Transform,
            "Transform",
            "XYZ",
            transform,
            cx,
        ));

        let surface = col()
            .gap(px(7.))
            .child(
                row()
                    .gap(px(3.))
                    .children(PRESETS.into_iter().enumerate().map(
                        |(index, (swatch, name, color))| {
                            let applied =
                                (0..3).all(|axis| (base[axis] - color[axis]).abs() < 0.002);
                            action(
                                format!("preset-{index}"),
                                Command::MaterialPreset(index),
                                cx,
                            )
                            .flex_col()
                            .flex_1()
                            .min_w(px(0.))
                            .py(px(4.))
                            .gap(px(4.))
                            .rounded(px(3.))
                            .border_1()
                            .border_color(if applied { rgb(ACCENT_LINE) } else { clear() })
                            .text_color(rgb(if applied { ACCENT } else { MUTED }))
                            .bg(rgb(if applied { ACTIVE } else { PANEL }))
                            .hover(move |d| d.bg(rgb(if applied { ACTIVE_HOVER } else { RAISED })))
                            .child(
                                div()
                                    .w(px(25.))
                                    .h(px(14.))
                                    .rounded(px(3.))
                                    .bg(rgb(swatch))
                                    .border_1()
                                    .border_color(rgb(if applied { ACCENT } else { EDGE })),
                            )
                            .child(div().text_size(px(9.)).child(name))
                        },
                    )),
            )
            .child(
                col()
                    .gap(px(4.))
                    .child(
                        row().gap(px(3.)).children(
                            [ShaderKind::Pbr, ShaderKind::Glass, ShaderKind::Custom]
                                .into_iter()
                                .map(|kind| {
                                    button(
                                        format!("shader-{kind:?}"),
                                        kind.label(),
                                        None,
                                        Command::SetShader(kind),
                                        object.material.shader == kind,
                                        cx,
                                    )
                                    .flex_1()
                                    .justify_center()
                                }),
                        ),
                    )
                    .when(object.material.shader == ShaderKind::Custom, |d| {
                        d.child(button(
                            "edit-custom-shader",
                            "Edit shader code…",
                            Some(Icon::Material),
                            Command::EditShader,
                            false,
                            cx,
                        ))
                    })
                    .child(
                        row()
                            .gap(px(6.))
                            .child(div().text_color(rgb(MUTED)).child("Base color"))
                            .child(div().flex_1())
                            .child(caption("sRGB"))
                            .child(
                                div()
                                    .w(px(24.))
                                    .h(px(12.))
                                    .rounded(px(2.))
                                    .bg(rgb(swatch_color(base)))
                                    .border_1()
                                    .border_color(rgb(EDGE)),
                            ),
                    )
                    .child(
                        row()
                            .gap(px(1.))
                            .rounded(px(3.))
                            .overflow_hidden()
                            .children((0..3).map(|axis| {
                                field(
                                    s,
                                    &format!("base-color-{axis}"),
                                    ["R", "G", "B"][axis],
                                    Field::Color(axis),
                                    cx,
                                )
                                .flex_1()
                                .min_w(px(0.))
                                .rounded(px(0.))
                            })),
                    ),
            )
            .child(
                col()
                    .gap(px(1.))
                    .child(property(s, "roughness", "Roughness", Field::Roughness, cx))
                    .when(object.material.shader != ShaderKind::Glass, |d| {
                        d.child(property(s, "metallic", "Metallic", Field::Metallic, cx))
                    })
                    .child(property(s, "ior", "IOR", Field::Ior, cx))
                    .child(property(s, "emission", "Emission", Field::Emission, cx)),
            )
            .child(texture_inspector(s, object.material, cx));
        contents = contents.child(panel_card(
            s,
            PanelSection::Surface,
            "Surface",
            "Material",
            surface,
            cx,
        ));

        let geometry = col()
            .gap(px(7.))
            .child(
                row().gap(px(4.)).children(
                    [
                        ("Vertices", object.mesh.positions.len()),
                        ("Faces", object.mesh.faces.len()),
                    ]
                    .into_iter()
                    .map(|(label, count)| {
                        row()
                            .flex_1()
                            .px(px(7.))
                            .h(px(25.))
                            .gap(px(6.))
                            .rounded(px(3.))
                            .bg(rgb(PANEL))
                            .child(caption(label))
                            .child(div().flex_1())
                            .child(count.to_string())
                    }),
                ),
            )
            .child(
                row()
                    .gap(px(4.))
                    .child(
                        button(
                            "subdivide",
                            "Subdivide",
                            Some(Icon::Grid),
                            Command::Subdivide,
                            false,
                            cx,
                        )
                        .flex_1()
                        .justify_center()
                        .bg(rgb(RAISED)),
                    )
                    .child(
                        button(
                            "extrude",
                            "Extrude",
                            Some(Icon::Export),
                            Command::Extrude,
                            false,
                            cx,
                        )
                        .flex_1()
                        .justify_center()
                        .bg(rgb(RAISED)),
                    ),
            );
        contents = contents.child(panel_card(
            s,
            PanelSection::Geometry,
            "Geometry",
            format!("{} faces", object.mesh.faces.len()),
            geometry,
            cx,
        ));
    } else {
        contents = contents.child(
            col().p(px(14.)).gap(px(8.)).rounded(px(4.)).border_1().border_color(rgb(EDGE))
                .bg(rgb(CARD)).child(icon(Icon::Select, MUTED, 18.))
                .child("Nothing selected")
                .child(div().text_size(px(10.)).line_height(px(15.)).text_color(rgb(MUTED))
                    .child("Select an object in the viewport or Collection to edit its properties.")),
        );
    }
    if s.settings.mode == RenderMode::MaterialPreview || s.settings.mode.progressive() {
        contents = contents.child(render_settings(s, cx));
    }
    let sources = geometry_sources(cx);
    contents = contents.child(panel_card(
        s,
        PanelSection::AddGeometry,
        "Add Geometry",
        "Primitives / OBJ",
        sources,
        cx,
    ));
    editor()
        .flex_1()
        .child(
            editor_header()
                .child(icon(Icon::Scale, MUTED, 13.))
                .child(div().font_weight(FontWeight::MEDIUM).child("Properties"))
                .child(div().flex_1())
                .child(caption(if s.edit_mode { "Face" } else { "Object" })),
        )
        .child(
            row()
                .h(px(35.))
                .flex_shrink_0()
                .px(px(8.))
                .gap(px(7.))
                .child(icon(Icon::Cube, ALERT, 13.))
                .child(if object.is_some() {
                    field(s, "object-name", "", Field::Name, cx)
                        .flex_1()
                        .min_w(px(0.))
                        .bg(rgb(WELL))
                        .border_1()
                        .border_color(rgb(if s.field_is_active(Field::Name) {
                            ACCENT
                        } else {
                            EDGE
                        }))
                        .tooltip(|_, cx| {
                            cx.new(|_| Tooltip("Rename object · Enter to apply")).into()
                        })
                        .into_any_element()
                } else {
                    div()
                        .text_color(rgb(MUTED))
                        .child("Scene")
                        .into_any_element()
                }),
        )
        .child(
            col()
                .id("properties-scroll")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(contents),
        )
        .into_any_element()
}

fn footer(s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    let face = s.edit_mode.then(|| {
        s.selected_face
            .map(|face| format!("Face {}", face + 1))
            .unwrap_or_else(|| "Click a face".to_owned())
    });
    row()
        .h(px(22.))
        .flex_shrink_0()
        .px(px(8.))
        .gap(px(10.))
        .bg(rgb(PANEL))
        .border_t_1()
        .border_color(rgb(LINE))
        .text_size(px(10.))
        .text_color(rgb(FAINT))
        .child(div().size(px(5.)).flex_shrink_0().rounded_full().bg(rgb(
            if s.render_error.is_some() {
                ALERT
            } else {
                ACCENT
            },
        )))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .text_ellipsis()
                .child(s.status.clone()),
        )
        .when_some(face, |d, face| {
            d.child(div().text_color(rgb(MUTED)).child(face))
                .child(divider().h(px(12.)))
        })
        .child(
            div()
                .text_color(rgb(MUTED))
                .child(format!("{:.1} ms", s.render_ms)),
        )
        .child(divider().h(px(12.)))
        .child(s.device_name.clone())
        .child(icon_button(
            "footer-help",
            Icon::Help,
            Command::ToggleHelp,
            s.help_open,
            22.,
            cx,
        ))
        .into_any_element()
}

fn studio_swatch(studio: StudioLight) -> AnyElement {
    let (sky, horizon, ground) = match studio {
        StudioLight::Studio => (0x68747d, 0xa5aaa9, 0x373e43),
        StudioLight::Courtyard => (0x789bad, 0xc9ceba, 0x57695d),
        StudioLight::Sunset => (0x646785, 0xdca886, 0x605353),
    };
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let width = f32::from(bounds.size.width);
            let height = f32::from(bounds.size.height);
            let mut polygon = |points: &[(f32, f32)], color| {
                let mut path = PathBuilder::fill();
                for (i, &(x, y)) in points.iter().enumerate() {
                    let p = bounds.origin + point(px(x * width), px(y * height));
                    if i == 0 {
                        path.move_to(p);
                    } else {
                        path.line_to(p);
                    }
                }
                path.close();
                if let Ok(path) = path.build() {
                    window.paint_path(path, rgb(color));
                }
            };
            polygon(&[(0., 0.), (1., 0.), (1., 0.52), (0., 0.52)], sky);
            polygon(&[(0., 0.52), (1., 0.52), (1., 1.), (0., 1.)], ground);
            polygon(&[(0., 0.43), (1., 0.43), (1., 0.59), (0., 0.59)], horizon);
            match studio {
                StudioLight::Studio => {
                    polygon(
                        &[(0.15, 0.10), (0.37, 0.16), (0.37, 0.71), (0.15, 0.83)],
                        0xdce5e6,
                    );
                    polygon(
                        &[(0.74, 0.18), (0.84, 0.12), (0.84, 0.80), (0.74, 0.72)],
                        0x959ea4,
                    );
                }
                StudioLight::Courtyard => {
                    polygon(
                        &[(0., 0.17), (0.16, 0.24), (0.24, 0.61), (0., 0.70)],
                        0x667466,
                    );
                    polygon(
                        &[(0.77, 0.24), (1., 0.16), (1., 0.68), (0.77, 0.60)],
                        0xa69c83,
                    );
                }
                StudioLight::Sunset => {
                    let sun: Vec<_> = (0..32)
                        .map(|i| {
                            let a = i as f32 * std::f32::consts::TAU / 32.;
                            (0.72 + a.cos() * 0.045, 0.42 + a.sin() * 0.10)
                        })
                        .collect();
                    polygon(&sun, 0xffe0b5);
                    polygon(
                        &[
                            (0., 0.72),
                            (0.20, 0.54),
                            (0.46, 0.68),
                            (0.80, 0.58),
                            (1., 0.70),
                            (1., 1.),
                            (0., 1.),
                        ],
                        0x61575b,
                    );
                }
            }
        },
    )
    .w_full()
    .h(px(40.))
    .into_any_element()
}

fn preview_toggle(
    id: &'static str,
    label: &'static str,
    detail: &'static str,
    active: bool,
    command: Command,
    cx: &mut Context<Studio>,
) -> AnyElement {
    action(id, command, cx)
        .h(px(42.))
        .w_full()
        .justify_between()
        .child(
            col()
                .gap(px(3.))
                .child(div().text_color(rgb(TEXT)).child(label))
                .child(caption(detail)),
        )
        .child(
            row()
                .w(px(28.))
                .h(px(16.))
                .px(px(3.))
                .flex_shrink_0()
                .rounded_full()
                .bg(rgb(if active { ACTIVE } else { WELL }))
                .border_1()
                .border_color(rgb(if active { ACCENT_LINE } else { LINE }))
                .when(active, |d| d.justify_end())
                .child(div().size(px(9.)).rounded_full().bg(rgb(if active {
                    ACCENT
                } else {
                    FAINT
                }))),
        )
        .into_any_element()
}

fn preview_property(
    s: &Studio,
    id: &str,
    label: &str,
    field_id: Field,
    enabled: bool,
    cx: &mut Context<Studio>,
) -> AnyElement {
    if enabled {
        return property(s, id, label, field_id, cx);
    }
    row()
        .h(px(23.))
        .gap(px(8.))
        .text_color(rgb(FAINT))
        .child(
            div()
                .w(px(92.))
                .flex_shrink_0()
                .text_right()
                .child(label.to_owned()),
        )
        .child(
            row()
                .flex_1()
                .min_w(px(0.))
                .h(px(21.))
                .px(px(7.))
                .justify_end()
                .rounded(px(3.))
                .bg(rgb(WELL))
                .child(s.field_value(field_id)),
        )
        .into_any_element()
}

/// Floating panel for viewport-only lighting, anchored to the viewport.
fn preview_overlay(s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    let bounds = s.bounds.get();
    let width = 312.;
    let left = (f32::from(bounds.right()) - width - 10.).max(f32::from(bounds.left()) + 10.);
    let top = f32::from(bounds.top()) + 10.;
    let height = (f32::from(bounds.size.height) - 20.).clamp(280., 500.);
    let preview = &s.settings.preview;
    let studio_enabled = !preview.use_scene_world;
    div()
        .id("preview-lighting-overlay")
        .absolute()
        .inset_0()
        .occlude()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|s, _, w, cx| {
                s.execute(Command::TogglePreviewSettings, w, cx);
                cx.stop_propagation();
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(|s, _, w, cx| {
                s.execute(Command::TogglePreviewSettings, w, cx);
                cx.stop_propagation();
            }),
        )
        .on_mouse_down(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
        .on_mouse_move(|_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .child(
            col()
                .id("preview-lighting-panel")
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(width))
                .max_h(px(height))
                .rounded(px(5.))
                .bg(rgb(PANEL))
                .border_1()
                .border_color(rgb(EDGE))
                .shadow_lg()
                .overflow_hidden()
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
                .child(
                    row()
                        .h(px(32.))
                        .flex_shrink_0()
                        .px(px(14.))
                        .gap(px(9.))
                        .border_b_1()
                        .border_color(rgb(LINE))
                        .child(icon(Icon::Material, ACCENT, 15.))
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .child("Preview lighting"),
                        )
                        .child(div().flex_1())
                        .child(key("ESC")),
                )
                .child(
                    col()
                        .id("preview-lighting-scroll")
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scroll()
                        .px(px(14.))
                        .py(px(10.))
                        .gap(px(8.))
                        .child(section("STUDIO ENVIRONMENT"))
                        .child(
                            row().gap(px(6.)).children(
                                [
                                    StudioLight::Studio,
                                    StudioLight::Courtyard,
                                    StudioLight::Sunset,
                                ]
                                .into_iter()
                                .map(|studio| {
                                    let active = preview.hdri_path.is_none()
                                        && preview.studio == studio
                                        && studio_enabled;
                                    col()
                                        .id(SharedString::from(format!(
                                            "preview-studio-{}",
                                            studio as u32
                                        )))
                                        .flex_1()
                                        .min_w(px(0.))
                                        .rounded(px(7.))
                                        .overflow_hidden()
                                        .border_1()
                                        .border_color(rgb(if active { ACCENT_LINE } else { LINE }))
                                        .bg(rgb(if active { ACTIVE } else { WELL }))
                                        .when(studio_enabled, |d| {
                                            d.cursor_pointer()
                                                .hover(|d| d.border_color(rgb(EDGE)))
                                                .on_click(cx.listener(move |s, _, w, cx| {
                                                    s.execute(
                                                        Command::SetPreviewStudio(studio),
                                                        w,
                                                        cx,
                                                    )
                                                }))
                                        })
                                        .when(!studio_enabled, |d| d.opacity(0.4))
                                        .child(studio_swatch(studio))
                                        .child(
                                            row()
                                                .h(px(24.))
                                                .justify_center()
                                                .text_size(px(10.))
                                                .text_color(rgb(if active {
                                                    ACCENT
                                                } else {
                                                    MUTED
                                                }))
                                                .child(studio.label()),
                                        )
                                        .into_any_element()
                                }),
                            ),
                        )
                        .child(
                            row()
                                .h(px(28.))
                                .gap(px(8.))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.))
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .text_size(px(10.))
                                        .text_color(rgb(if studio_enabled { MUTED } else { FAINT }))
                                        .child(if s.preview_loading {
                                            "Loading environment…".to_owned()
                                        } else {
                                            preview_light_name(s)
                                        }),
                                )
                                .when(studio_enabled && !s.preview_loading, |d| {
                                    d.child(button(
                                        "preview-load-hdr",
                                        "Load HDR…",
                                        Some(Icon::Folder),
                                        Command::LoadPreviewHdri,
                                        false,
                                        cx,
                                    ))
                                }),
                        )
                        .child(
                            col()
                                .child(preview_property(
                                    s,
                                    "preview-rotation",
                                    "Rotation · °",
                                    Field::PreviewRotation,
                                    studio_enabled,
                                    cx,
                                ))
                                .child(preview_property(
                                    s,
                                    "preview-strength",
                                    "Strength",
                                    Field::PreviewStrength,
                                    studio_enabled,
                                    cx,
                                ))
                                .child(preview_property(
                                    s,
                                    "preview-opacity",
                                    "Background · %",
                                    Field::PreviewOpacity,
                                    true,
                                    cx,
                                ))
                                .child(preview_property(
                                    s,
                                    "preview-blur",
                                    "Blur · %",
                                    Field::PreviewBlur,
                                    studio_enabled,
                                    cx,
                                )),
                        )
                        .child(div().h(px(1.)).flex_shrink_0().bg(rgb(LINE)).my(px(2.)))
                        .child(
                            col()
                                .child(preview_toggle(
                                    "preview-scene-world",
                                    "Scene world",
                                    "Use the project's world color and strength",
                                    preview.use_scene_world,
                                    Command::TogglePreviewWorld,
                                    cx,
                                ))
                                .child(preview_toggle(
                                    "preview-ao",
                                    "Contact shading",
                                    "Ambient occlusion around nearby surfaces",
                                    preview.ambient_occlusion,
                                    Command::TogglePreviewAo,
                                    cx,
                                )),
                        )
                        .child(
                            row()
                                .justify_between()
                                .child(caption("Viewport only · never rendered"))
                                .child(button(
                                    "preview-reset",
                                    "Reset",
                                    None,
                                    Command::ResetPreview,
                                    false,
                                    cx,
                                )),
                        ),
                ),
        )
        .into_any_element()
}

fn command_overlay(s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    let commands = crate::app::palette_commands(&s.palette_query);
    let selected = s.palette_index;
    div()
        .absolute()
        .inset_0()
        .bg(rgba(0x080b0dbf))
        .occlude()
        .flex()
        .justify_center()
        .pt(px(96.))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|s, _, w, cx| s.execute(Command::TogglePalette, w, cx)),
        )
        .child(
            col()
                .id("command-menu")
                .occlude()
                .w(px(460.))
                .h(px(66. + 32. * commands.len().clamp(1, 12) as f32))
                .rounded(px(12.))
                .bg(rgb(PANEL))
                .border_1()
                .border_color(rgb(EDGE))
                .shadow_lg()
                .overflow_hidden()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    row()
                        .h(px(48.))
                        .flex_shrink_0()
                        .px(px(16.))
                        .gap(px(10.))
                        .border_b_1()
                        .border_color(rgb(LINE))
                        .child(icon(Icon::Search, ACCENT, 15.))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .overflow_hidden()
                                .text_ellipsis()
                                .text_size(px(13.))
                                .text_color(rgb(if s.palette_query.is_empty() {
                                    FAINT
                                } else {
                                    TEXT
                                }))
                                .child(if s.palette_query.is_empty() {
                                    "Type a command…".into()
                                } else {
                                    format!("{}│", s.palette_query)
                                }),
                        )
                        .child(key("ESC")),
                )
                .child(
                    col()
                        .p(px(6.))
                        .gap(px(1.))
                        .when(commands.is_empty(), |d| {
                            d.child(
                                div()
                                    .p(px(12.))
                                    .text_color(rgb(MUTED))
                                    .child("No matching commands"),
                            )
                        })
                        .children(
                            commands
                                .into_iter()
                                .enumerate()
                                .skip(selected.saturating_sub(11))
                                .take(12)
                                .map(|(i, (label, shortcut, command))| {
                                    row()
                                        .id(SharedString::from(format!("palette-command-{i}")))
                                        .h(px(31.))
                                        .px(px(10.))
                                        .rounded(px(6.))
                                        .text_color(rgb(MUTED))
                                        .when(i == selected, |d| {
                                            d.bg(rgb(ACTIVE)).text_color(rgb(ACCENT))
                                        })
                                        .cursor_pointer()
                                        .hover(|d| d.bg(rgb(ACTIVE)).text_color(rgb(ACCENT)))
                                        .child(label)
                                        .child(div().flex_1())
                                        .child(caption(shortcut))
                                        .on_click(cx.listener(move |s, _, w, cx| {
                                            s.palette_open = false;
                                            s.execute(command, w, cx);
                                        }))
                                        .into_any_element()
                                }),
                        ),
                ),
        )
        .into_any_element()
}

fn help_overlay(cx: &mut Context<Studio>) -> AnyElement {
    let columns = [
        vec![
            (
                "NAVIGATION",
                vec![
                    ("Middle drag", "Orbit view"),
                    ("Shift + middle drag", "Pan view"),
                    ("Two fingers", "Orbit view"),
                    ("Shift + two fingers", "Pan view"),
                    (platform_shortcut("Pinch / wheel", "Mouse wheel"), "Zoom"),
                    (
                        platform_shortcut("Ctrl / ⌘ + two fingers", "Ctrl + two fingers"),
                        "Zoom",
                    ),
                    ("F", "Frame selection"),
                    ("1 / 3 / 7", "Front / right / top"),
                    ("5", "Toggle projection"),
                ],
            ),
            (
                "MODELING",
                vec![
                    ("Q", "Select tool"),
                    ("G / R / S", "Move / rotate / scale"),
                    ("X / Y / Z", "Constrain a transform"),
                    ("Enter / Escape", "Confirm / cancel"),
                    ("Tab", "Object / face edit"),
                    ("E", "Extrude selected face"),
                    ("Shift + D", "Duplicate object"),
                ],
            ),
        ],
        vec![(
            "WORKSPACE",
            vec![
                ("Z", "Shading pie · hold and flick"),
                ("4 / 6 / 2 / 8", "Modes inside the pie"),
                ("X / C / V", "Solid / material / rendered"),
                (
                    platform_shortcut("⌘ Z / ⇧ ⌘ Z", "Ctrl+Z / Ctrl+Shift+Z"),
                    "Undo / redo",
                ),
                (
                    platform_shortcut("⌘ S / ⌘ O", "Ctrl+S / Ctrl+O"),
                    "Save / open",
                ),
                (platform_shortcut("⌘ K", "Ctrl+K"), "Workspace commands"),
                ("?", "This reference"),
            ],
        )],
    ];
    div()
        .absolute()
        .inset_0()
        .bg(rgba(0x080b0dbf))
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|s, _, w, cx| s.execute(Command::ToggleHelp, w, cx)),
        )
        .child(
            col()
                .id("help-panel")
                .occlude()
                .w(px(if cfg!(target_os = "macos") {
                    600.
                } else {
                    760.
                }))
                .max_w(relative(0.92))
                .max_h(relative(0.88))
                .overflow_y_scroll()
                .rounded(px(12.))
                .bg(rgb(PANEL))
                .border_1()
                .border_color(rgb(EDGE))
                .shadow_lg()
                .p(px(20.))
                .gap(px(18.))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    row()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(15.))
                                .text_color(rgb(TEXT))
                                .child("Keyboard reference"),
                        )
                        .child(key("ESC")),
                )
                .child(
                    row()
                        .items_start()
                        .gap(px(28.))
                        .children(columns.into_iter().map(|groups| {
                            col().flex_1().min_w(px(0.)).gap(px(14.)).children(
                                groups.into_iter().map(|(title, keys)| {
                                    col().gap(px(2.)).child(section(title)).children(
                                        keys.into_iter().map(|(shortcut, label)| {
                                            row()
                                                .justify_between()
                                                .gap(px(10.))
                                                .h(px(24.))
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w(px(0.))
                                                        .overflow_hidden()
                                                        .text_ellipsis()
                                                        .text_color(rgb(MUTED))
                                                        .child(label),
                                                )
                                                .child(key(shortcut))
                                        }),
                                    )
                                }),
                            )
                        })),
                ),
        )
        .into_any_element()
}

fn shading_overlay(s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    let pie = s.shading_pie.as_ref().unwrap();
    let center = pie.center;
    let scale = pie.scale;
    let hovered = pie.hovered;
    let current = s.settings.mode;
    div()
        .id("shading-pie-overlay")
        .absolute()
        .inset_0()
        .occlude()
        .bg(rgba(0x080b0d40))
        .on_mouse_move(cx.listener(Studio::mouse_move))
        .on_mouse_down(MouseButton::Left, cx.listener(Studio::mouse_down))
        .on_mouse_down(MouseButton::Right, cx.listener(Studio::mouse_down))
        .on_mouse_down(MouseButton::Middle, cx.listener(Studio::mouse_down))
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let origin = bounds.center();
                    let at = |radius: f32, angle: f32| {
                        origin
                            + point(
                                px(radius * scale * angle.cos()),
                                px(radius * scale * angle.sin()),
                            )
                    };

                    let paint_ring_segment =
                        |start: f32, end: f32, color: u32, window: &mut Window| {
                            let mut ring = PathBuilder::fill();
                            ring.move_to(at(60., start));
                            for i in 1..=32 {
                                ring.line_to(at(60., start + (end - start) * i as f32 / 32.));
                            }
                            for i in (0..=32).rev() {
                                ring.line_to(at(38., start + (end - start) * i as f32 / 32.));
                            }
                            ring.close();
                            if let Ok(path) = ring.build() {
                                window.paint_path(path, rgb(color));
                            }
                        };

                    // Paint one uninterrupted annulus first. Selection is then
                    // layered over it, so no background can show between modes.
                    paint_ring_segment(0., std::f32::consts::TAU, PANEL, window);

                    let selected = hovered.unwrap_or(current);
                    if let Some(choice) = CHOICES.into_iter().find(|choice| choice.mode == selected)
                    {
                        let angle = choice.offset.y.atan2(choice.offset.x);
                        let start = angle - std::f32::consts::FRAC_PI_4;
                        let end = angle + std::f32::consts::FRAC_PI_4;
                        paint_ring_segment(
                            start,
                            end,
                            if hovered.is_some() {
                                ACTIVE_HOVER
                            } else {
                                ACTIVE
                            },
                            window,
                        );

                        // The selected quarter has one continuous accent edge,
                        // rather than a detached arc floating inside the ring.
                        let mut accent = PathBuilder::stroke(px(2. * scale));
                        accent.move_to(at(60., start));
                        for i in 1..=32 {
                            accent.line_to(at(60., start + (end - start) * i as f32 / 32.));
                        }
                        if let Ok(path) = accent.build() {
                            window.paint_path(path, rgb(ACCENT));
                        }
                    }

                    // Hairline boundaries preserve the four directional targets
                    // without breaking the ring into separate pieces.
                    for angle in [
                        std::f32::consts::FRAC_PI_4,
                        3. * std::f32::consts::FRAC_PI_4,
                        5. * std::f32::consts::FRAC_PI_4,
                        7. * std::f32::consts::FRAC_PI_4,
                    ] {
                        let mut divider = PathBuilder::stroke(px(scale.max(0.75)));
                        divider.move_to(at(38., angle));
                        divider.line_to(at(60., angle));
                        if let Ok(path) = divider.build() {
                            window.paint_path(path, rgb(LINE));
                        }
                    }
                },
            )
            .absolute()
            .left(px(center.x - 62. * scale))
            .top(px(center.y - 62. * scale))
            .size(px(124. * scale)),
        )
        .child(
            col()
                .absolute()
                .left(px(center.x - 32. * scale))
                .top(px(center.y - 32. * scale))
                .size(px(64. * scale))
                .rounded_full()
                .bg(rgb(PANEL))
                .border_1()
                .border_color(rgb(LINE))
                .items_center()
                .justify_center()
                .gap(px(1. * scale))
                .child(
                    div()
                        .text_size(px(19. * scale))
                        .text_color(rgb(TEXT))
                        .child("Z"),
                )
                .child(
                    div()
                        .text_size(px(8. * scale))
                        .text_color(rgb(FAINT))
                        .child("SHADING"),
                ),
        )
        .children(CHOICES.into_iter().map(|choice| {
            let mode = choice.mode;
            let number = choice.key;
            let glyph = match mode {
                RenderMode::Wireframe => Icon::Wire,
                RenderMode::Solid => Icon::Solid,
                RenderMode::MaterialPreview => Icon::Material,
                RenderMode::Rendered => Icon::Render,
            };
            let origin = center + (choice.offset - CARD_HALF_SIZE) * scale;
            let active = current == mode;
            let highlighted = hovered == Some(mode);
            row()
                .id(SharedString::from(format!("shading-pie-{number}")))
                .absolute()
                .left(px(origin.x))
                .top(px(origin.y))
                .w(px(CARD_HALF_SIZE.x * 2. * scale))
                .h(px(CARD_HALF_SIZE.y * 2. * scale))
                .px(px(10. * scale))
                .gap(px(9. * scale))
                .rounded(px(9. * scale))
                .border_1()
                .border_color(rgb(if highlighted {
                    ACCENT
                } else if active {
                    ACCENT_LINE
                } else {
                    EDGE
                }))
                .bg(rgb(if highlighted { ACTIVE } else { PANEL }))
                .shadow_lg()
                .cursor_pointer()
                .child(icon(
                    glyph,
                    if active || highlighted { ACCENT } else { MUTED },
                    18. * scale,
                ))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(11. * scale))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(if highlighted { 0xe4f4ee } else { TEXT }))
                        .child(mode.label()),
                )
                .child(
                    div()
                        .text_size(px(10. * scale))
                        .text_color(rgb(if highlighted { ACCENT } else { FAINT }))
                        .child(number),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |s, _, w, cx| {
                        s.execute(Command::SetMode(mode), w, cx);
                        cx.stop_propagation();
                    }),
                )
        }))
        .into_any_element()
}

pub fn render(studio: &Studio, viewport: AnyElement, cx: &mut Context<Studio>) -> AnyElement {
    col()
        .relative()
        .size_full()
        .overflow_hidden()
        .bg(rgb(SHELL))
        .text_color(rgb(TEXT))
        .text_size(px(11.))
        .font_family(".SystemUIFont")
        .child(titlebar(studio, cx))
        .child(
            row()
                .flex_1()
                .min_h(px(0.))
                .w_full()
                .overflow_hidden()
                .p(px(4.))
                .gap(px(4.))
                .child(viewport_panel(studio, viewport, cx))
                .child(
                    col()
                        .w(px(292.))
                        .h_full()
                        .flex_shrink_0()
                        .gap(px(4.))
                        .child(outliner(studio, cx))
                        .child(inspector(studio, cx)),
                ),
        )
        .child(footer(studio, cx))
        .when(studio.preview_open, |d| {
            d.child(preview_overlay(studio, cx))
        })
        .when(studio.palette_open, |d| {
            d.child(command_overlay(studio, cx))
        })
        .when(studio.help_open, |d| d.child(help_overlay(cx)))
        .when(studio.shader_editor.is_some(), |d| {
            d.child(shader_overlay(studio, cx))
        })
        .when(studio.shading_pie.is_some(), |d| {
            d.child(shading_overlay(studio, cx))
        })
        .into_any_element()
}

fn texture_inspector(
    s: &Studio,
    material: &forma_core::Material,
    cx: &mut Context<Studio>,
) -> AnyElement {
    let slots: Vec<_> = TextureSlot::ALL
        .into_iter()
        .map(|slot| {
            let image = &material.textures[slot as usize];
            let label = image
                .as_ref()
                .map(|image| format!("{} · {} × {}", image.name, image.width, image.height))
                .unwrap_or_else(|| "Choose image…".into());
            let space = if matches!(slot, TextureSlot::BaseColor | TextureSlot::Emission) {
                "sRGB"
            } else {
                "Linear data"
            };
            let picker = action(
                format!("load-texture-{slot:?}"),
                Command::LoadTexture(slot),
                cx,
            )
            .flex_1()
            .min_w(px(0.))
            .h(px(27.))
            .px(px(7.))
            .rounded(px(5.))
            .bg(rgb(WELL))
            .hover(|d| d.bg(rgb(RAISED)))
            .child(div().truncate().text_size(px(10.)).child(label));
            col()
                .gap(px(3.))
                .child(
                    div()
                        .text_color(rgb(MUTED))
                        .text_size(px(10.))
                        .child(format!("{} · {space}", slot.label())),
                )
                .child(row().gap(px(4.)).child(picker).when(image.is_some(), |d| {
                    d.child(button(
                        format!("clear-texture-{slot:?}"),
                        "×",
                        None,
                        Command::ClearTexture(slot),
                        false,
                        cx,
                    ))
                }))
        })
        .collect();
    let mappings: Vec<_> = [
        TextureMapping::Box,
        TextureMapping::Sphere,
        TextureMapping::Plane,
    ]
    .into_iter()
    .map(|mapping| {
        button(
            format!("texture-mapping-{mapping:?}"),
            mapping.label(),
            None,
            Command::SetTextureMapping(mapping),
            material.mapping == mapping,
            cx,
        )
        .flex_1()
        .justify_center()
        .text_size(px(10.))
    })
    .collect();
    let tiles: Vec<_> = (0..2)
        .map(|axis| {
            field(
                s,
                &format!("texture-scale-{axis}"),
                ["Tile U", "Tile V"][axis],
                Field::TextureScale(axis),
                cx,
            )
            .flex_1()
            .min_w(px(0.))
        })
        .collect();
    let offsets: Vec<_> = (0..2)
        .map(|axis| {
            field(
                s,
                &format!("texture-offset-{axis}"),
                ["Offset U", "Offset V"][axis],
                Field::TextureOffset(axis),
                cx,
            )
            .flex_1()
            .min_w(px(0.))
        })
        .collect();
    col().px(px(14.)).py(px(12.)).gap(px(8.)).border_b_1().border_color(rgb(LINE))
        .child(section("IMAGE TEXTURES"))
        .child(div().text_size(px(10.)).text_color(rgb(MUTED)).child(
            if s.texture_loading { "Loading image…" } else { "PNG / JPEG · Images saved in project" }
        ))
        .children(slots)
        .child(div().text_size(px(10.)).text_color(rgb(MUTED)).child("Generated coordinates"))
        .child(row().gap(px(2.)).children(mappings))
        .child(row().gap(px(6.)).children(tiles))
        .child(row().gap(px(6.)).children(offsets))
        .when(material.textures[TextureSlot::Normal as usize].is_some(), |d| {
            d.child(property(s, "normal-strength", "Normal strength", Field::NormalStrength, cx))
        })
        .child(div().text_size(px(9.)).text_color(rgb(FAINT)).child(
            "Color maps multiply the color above. Roughness and metallic maps replace their values. Normal maps use OpenGL +Y."
        ))
        .into_any_element()
}

fn shader_overlay(s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    let heading = col()
        .gap(px(4.))
        .child(div().text_size(px(16.)).child("Custom surface shader"))
        .child(
            div()
                .text_size(px(10.))
                .text_color(rgb(MUTED))
                .child(format!(
                    "{} · Function body · Shared by Preview and Rendered",
                    s.renderer_backend.shader_language().label()
                )),
        );
    let header = row().justify_between().child(heading).child(button(
        "close-shader",
        "Close",
        None,
        Command::CloseShader,
        false,
        cx,
    ));
    let instructions = div().text_size(px(10.)).text_color(rgb(MUTED)).child(
        "Edit surface.color, roughness, metallic, emission, normal or ior; set surface.glass = true for refraction. Use input.uv, generated, position, normal and view_direction. Image textures are applied before your code."
    );
    let footer = row()
        .justify_between()
        .child(
            div()
                .text_size(px(10.))
                .text_color(rgb(FAINT))
                .child(platform_shortcut(
                    "⌘ Return to apply · ⌘ Z to undo code · Only applied code is saved",
                    "Ctrl+Return to apply · Ctrl+Z to undo code · Only applied code is saved",
                )),
        )
        .child(button(
            "apply-shader",
            if s.shader_compiling {
                "Compiling…"
            } else {
                "Compile & apply"
            },
            None,
            Command::ApplyShader,
            true,
            cx,
        ));
    let panel = col()
        .id("shader-editor-panel")
        .w(px(780.))
        .max_w(relative(0.94))
        .max_h(relative(0.94))
        .overflow_y_scroll()
        .rounded(px(12.))
        .bg(rgb(PANEL))
        .border_1()
        .border_color(rgb(EDGE))
        .shadow_lg()
        .p(px(16.))
        .gap(px(12.))
        .child(header)
        .child(instructions)
        .child(s.shader_editor.as_ref().unwrap().clone())
        .when_some(s.shader_message.clone(), |d, message| {
            let color = if message.contains("failed") {
                ALERT
            } else {
                MUTED
            };
            d.child(
                div()
                    .id("shader-diagnostics")
                    .max_h(px(100.))
                    .overflow_y_scroll()
                    .p(px(8.))
                    .rounded(px(5.))
                    .bg(rgb(WELL))
                    .text_size(px(11.))
                    .text_color(rgb(color))
                    .child(message),
            )
        })
        .child(footer);
    div()
        .id("shader-editor-overlay")
        .absolute()
        .inset_0()
        .occlude()
        .bg(rgba(0x00000088))
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
        .on_mouse_move(|_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .child(panel)
        .into_any_element()
}
