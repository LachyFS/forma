//! Native workspace chrome. Rendering and interaction state live in `Studio`.
use crate::app::{Command, Field, Studio, Tool};
use crate::shading_pie::{CARD_HALF_SIZE, CHOICES};
use forma_core::Primitive;
use forma_render::{RenderMode, StudioLight};
use gpui::{prelude::*, *};

const BG: u32 = 0x141719;
const PANEL: u32 = 0x1b1e21;
const RAISED: u32 = 0x23272b;
const LINE: u32 = 0x30353a;
const TEXT: u32 = 0xd7dcdf;
const MUTED: u32 = 0x899298;
const FAINT: u32 = 0x626d74;
const ACCENT: u32 = 0x8bc7b6;
const ACTIVE: u32 = 0x283d39;

struct Tooltip(&'static str);

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(9.))
            .py(px(6.))
            .rounded(px(4.))
            .bg(rgb(RAISED))
            .border_1()
            .border_color(rgb(LINE))
            .shadow_md()
            .text_color(rgb(TEXT))
            .text_size(px(11.))
            .child(self.0)
    }
}

fn command_hint(command: Command) -> &'static str {
    match command {
        Command::New => "New project · ⌘ N",
        Command::Open => "Open project · ⌘ O",
        Command::Save => "Save project · ⌘ S",
        Command::SaveAs => "Save project as · ⇧ ⌘ S",
        Command::ImportObj => "Import Wavefront OBJ",
        Command::ExportObj => "Export scene geometry as OBJ",
        Command::ExportImage => "Export current rendered image as PNG",
        Command::Add(_) => "Add geometry at the origin",
        Command::Select(_) => "Select object",
        Command::ToggleVisible(_) => "Toggle object visibility",
        Command::Delete => "Delete selection · Delete",
        Command::Duplicate => "Duplicate selection · ⇧ D",
        Command::Undo => "Undo · ⌘ Z",
        Command::Redo => "Redo · ⇧ ⌘ Z",
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
        Command::TogglePalette => "Workspace commands · ⌘ K",
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
fn separator() -> Div {
    div().w(px(1.)).h(px(16.)).bg(rgb(LINE)).mx(px(5.))
}
fn key(text: &str) -> Div {
    div()
        .px(px(5.))
        .h(px(18.))
        .rounded(px(3.))
        .bg(rgb(BG))
        .border_1()
        .border_color(rgb(LINE))
        .text_size(px(10.))
        .text_color(rgb(MUTED))
        .flex()
        .items_center()
        .justify_center()
        .child(text.to_owned())
}
fn section_title(text: &str) -> Div {
    row()
        .h(px(30.))
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(MUTED))
        .child(text.to_owned())
}

#[derive(Clone, Copy)]
enum Icon {
    Cube,
    Select,
    Move,
    Rotate,
    Scale,
    Plus,
    Grid,
    Eye,
    Hidden,
    Folder,
    Save,
    Arrow,
    Frame,
    Spark,
    Search,
    Help,
    Wire,
    Solid,
    Material,
    Render,
    Chevron,
    Undo,
    Redo,
}

fn icon(kind: Icon, color: u32, side: f32) -> AnyElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let mut paths: Vec<Vec<(f32, f32)>> = match kind {
                Icon::Cube => vec![
                    vec![
                        (8., 1.5),
                        (14., 5.),
                        (14., 12.),
                        (8., 15.),
                        (2., 12.),
                        (2., 5.),
                        (8., 1.5),
                    ],
                    vec![(2., 5.), (8., 8.5), (14., 5.)],
                    vec![(8., 8.5), (8., 15.)],
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
                    vec![
                        (13., 6.),
                        (12., 3.),
                        (8., 1.5),
                        (4., 3.),
                        (2., 7.),
                        (3., 11.),
                        (6., 14.),
                        (10., 14.),
                        (13., 11.),
                    ],
                    vec![(10., 6.), (13., 6.), (14., 2.)],
                ],
                Icon::Scale => vec![
                    vec![(2., 10.), (2., 14.), (6., 14.), (6., 10.), (2., 10.)],
                    vec![(7., 9.), (14., 2.)],
                    vec![(9., 2.), (14., 2.), (14., 7.)],
                ],
                Icon::Plus => vec![vec![(3., 8.), (13., 8.)], vec![(8., 3.), (8., 13.)]],
                Icon::Grid => vec![
                    vec![(2., 2.), (14., 2.), (14., 14.), (2., 14.), (2., 2.)],
                    vec![(6., 2.), (6., 14.)],
                    vec![(10., 2.), (10., 14.)],
                    vec![(2., 6.), (14., 6.)],
                    vec![(2., 10.), (14., 10.)],
                ],
                Icon::Eye => vec![vec![
                    (1., 8.),
                    (4., 4.),
                    (8., 3.),
                    (12., 4.),
                    (15., 8.),
                    (12., 12.),
                    (8., 13.),
                    (4., 12.),
                    (1., 8.),
                ]],
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
                    vec![(2., 1.), (14., 15.)],
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
                Icon::Arrow => vec![
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
                Icon::Spark => vec![vec![
                    (8., 1.),
                    (10., 6.),
                    (15., 8.),
                    (10., 10.),
                    (8., 15.),
                    (6., 10.),
                    (1., 8.),
                    (6., 6.),
                    (8., 1.),
                ]],
                Icon::Search => vec![vec![(10.5, 10.5), (15., 15.)]],
                Icon::Help => vec![
                    vec![
                        (5., 5.),
                        (5., 3.),
                        (7., 2.),
                        (10., 2.),
                        (12., 4.),
                        (11., 6.),
                        (8., 8.),
                        (8., 10.),
                    ],
                    vec![(8., 13.), (8., 14.)],
                ],
                Icon::Chevron => vec![vec![(5., 6.), (8., 9.), (11., 6.)]],
                Icon::Undo => vec![
                    vec![(5., 3.), (1., 7.), (5., 11.)],
                    vec![(1., 7.), (10., 7.), (13., 9.), (13., 13.)],
                ],
                Icon::Redo => vec![
                    vec![(11., 3.), (15., 7.), (11., 11.)],
                    vec![(15., 7.), (6., 7.), (3., 9.), (3., 13.)],
                ],
                _ => vec![],
            };
            let circle = |x: f32, y: f32, r: f32| {
                (0..=24)
                    .map(|i| {
                        let a = i as f32 * std::f32::consts::TAU / 24.;
                        (x + a.cos() * r, y + a.sin() * r)
                    })
                    .collect::<Vec<_>>()
            };
            match kind {
                Icon::Eye => paths.push(circle(8., 8., 2.)),
                Icon::Search => paths.push(circle(6.5, 6.5, 4.5)),
                Icon::Wire | Icon::Solid | Icon::Material | Icon::Render => {
                    paths.push(circle(8., 8., 6.));
                    match kind {
                        Icon::Wire => {
                            paths.push(vec![(2., 8.), (14., 8.)]);
                            paths.push(vec![
                                (8., 2.),
                                (5., 5.),
                                (5., 11.),
                                (8., 14.),
                                (11., 11.),
                                (11., 5.),
                                (8., 2.),
                            ]);
                        }
                        Icon::Solid => {
                            for x in [8., 10., 12.] {
                                paths.push(vec![(x, 4.), (x, 12.)]);
                            }
                        }
                        Icon::Material => {
                            paths.push(vec![(4., 12.), (12., 4.)]);
                            paths.push(circle(5.5, 5.5, 1.));
                        }
                        Icon::Render => {
                            paths.push(circle(8., 8., 2.5));
                            paths.push(vec![(8., 0.), (8., 3.)]);
                            paths.push(vec![(8., 13.), (8., 16.)]);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            for points in paths {
                let mut path = PathBuilder::stroke(px(1.2));
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

fn button(
    id: impl Into<SharedString>,
    label: &str,
    glyph: Option<Icon>,
    command: Command,
    active: bool,
    cx: &mut Context<Studio>,
) -> AnyElement {
    let color = if active { ACCENT } else { MUTED };
    row()
        .id(id.into())
        .h(px(28.))
        .px(px(8.))
        .gap(px(6.))
        .rounded(px(4.))
        .text_color(rgb(color))
        .bg(if matches!(command, Command::ToggleVisible(_)) {
            rgba(0x00000000)
        } else {
            rgb(if active { ACTIVE } else { PANEL })
        })
        .cursor_pointer()
        .tooltip(move |_, cx| cx.new(|_| Tooltip(command_hint(command))).into())
        .hover(|s| s.bg(rgb(RAISED)).text_color(rgb(TEXT)))
        .when_some(glyph, |s, g| s.child(icon(g, color, 14.)))
        .when(!label.is_empty(), |s| s.child(label.to_owned()))
        .on_click(cx.listener(move |s, _, window, cx| {
            cx.stop_propagation();
            s.execute(command, window, cx);
        }))
        .into_any_element()
}

fn titlebar(s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    row()
        .h(px(43.))
        .flex_shrink_0()
        .pl(px(82.))
        .pr(px(15.))
        .gap(px(12.))
        .border_b_1()
        .border_color(rgb(LINE))
        .bg(rgb(PANEL))
        .child(
            row()
                .gap(px(8.))
                .w(px(92.))
                .flex_shrink_0()
                .child(icon(Icon::Cube, ACCENT, 19.))
                .child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .text_size(px(13.))
                        .text_color(rgb(TEXT))
                        .child("F O R M A"),
                ),
        )
        .child(separator())
        .child(
            div()
                .max_w(px(245.))
                .min_w(px(0.))
                .overflow_hidden()
                .text_ellipsis()
                .text_color(rgb(TEXT))
                .child(s.project_name.clone()),
        )
        .when(s.dirty, |d| {
            d.child(div().size(px(5.)).rounded_full().bg(rgb(ACCENT)))
        })
        .child(div().flex_1())
        .child(button(
            "commands",
            "Commands",
            Some(Icon::Search),
            Command::TogglePalette,
            s.palette_open,
            cx,
        ))
        .child(key("⌘ K"))
        .child(separator())
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
        .child(button(
            "export-image",
            "Export image",
            Some(Icon::Arrow),
            Command::ExportImage,
            true,
            cx,
        ))
        .into_any_element()
}

fn workspace_bar(s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    row()
        .h(px(39.))
        .flex_shrink_0()
        .px(px(14.))
        .gap(px(7.))
        .border_b_1()
        .border_color(rgb(LINE))
        .bg(rgb(PANEL))
        .child(
            row()
                .w(px(170.))
                .gap(px(8.))
                .child(div().size(px(5.)).rounded_full().bg(rgb(ACCENT)))
                .child(
                    div()
                        .text_color(rgb(TEXT))
                        .font_weight(FontWeight::MEDIUM)
                        .child("Modeling"),
                ),
        )
        .child(button(
            "edit-mode",
            if s.edit_mode {
                "Face edit"
            } else {
                "Object mode"
            },
            Some(Icon::Chevron),
            Command::ToggleEdit,
            s.edit_mode,
            cx,
        ))
        .child(div().flex_1())
        .child(
            row()
                .gap(px(3.))
                .p(px(3.))
                .rounded(px(6.))
                .bg(rgb(BG))
                .children(
                    [
                        (RenderMode::Wireframe, "Wireframe", Icon::Wire),
                        (RenderMode::Solid, "Solid", Icon::Solid),
                        (RenderMode::MaterialPreview, "Material", Icon::Material),
                        (RenderMode::Rendered, "Rendered", Icon::Render),
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(i, (mode, label, glyph))| {
                        button(
                            format!("render-mode-{i}"),
                            label,
                            Some(glyph),
                            Command::SetMode(mode),
                            s.settings.mode == mode,
                            cx,
                        )
                    }),
                ),
        )
        .when(s.settings.mode == RenderMode::MaterialPreview, |d| {
            d.child(button(
                "preview-settings",
                "",
                Some(Icon::Chevron),
                Command::TogglePreviewSettings,
                s.preview_open,
                cx,
            ))
        })
        .child(div().w(px(253.)).flex().justify_end().child(button(
            "help",
            "Shortcuts",
            Some(Icon::Help),
            Command::ToggleHelp,
            s.help_open,
            cx,
        )))
        .into_any_element()
}

fn outliner(s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    let objects = s
        .scene
        .objects
        .iter()
        .map(|object| {
            let id = object.id;
            let selected = s.selected == Some(id);
            row()
                .id(SharedString::from(format!("object-{id}")))
                .h(px(31.))
                .px(px(9.))
                .mx(px(6.))
                .gap(px(7.))
                .rounded(px(4.))
                .bg(rgb(if selected { ACTIVE } else { PANEL }))
                .cursor_pointer()
                .hover(|d| d.bg(rgb(RAISED)))
                .child(icon(Icon::Cube, if selected { ACCENT } else { FAINT }, 13.))
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_color(rgb(if !object.visible {
                            FAINT
                        } else if selected {
                            ACCENT
                        } else {
                            TEXT
                        }))
                        .child(object.name.clone()),
                )
                .child(button(
                    format!("visibility-{id}"),
                    "",
                    Some(if object.visible {
                        Icon::Eye
                    } else {
                        Icon::Hidden
                    }),
                    Command::ToggleVisible(id),
                    false,
                    cx,
                ))
                .on_click(
                    cx.listener(move |s, _, window, cx| s.execute(Command::Select(id), window, cx)),
                )
                .into_any_element()
        })
        .collect::<Vec<_>>();
    col()
        .w(px(190.))
        .flex_shrink_0()
        .h_full()
        .bg(rgb(PANEL))
        .border_r_1()
        .border_color(rgb(LINE))
        .child(
            row()
                .h(px(40.))
                .px(px(14.))
                .justify_between()
                .child(section_title("SCENE"))
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(FAINT))
                        .child(format!("{} objects", s.scene.objects.len())),
                ),
        )
        .child(
            row()
                .h(px(29.))
                .px(px(13.))
                .gap(px(7.))
                .text_color(rgb(MUTED))
                .child(icon(Icon::Chevron, MUTED, 11.))
                .child(icon(Icon::Folder, MUTED, 13.))
                .child("Collection"),
        )
        .child(
            col()
                .id("scene-objects")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .gap(px(2.))
                .children(objects),
        )
        .child(div().h(px(1.)).bg(rgb(LINE)).mx(px(13.)).my(px(15.)))
        .child(
            col()
                .px(px(13.))
                .pb(px(13.))
                .flex_shrink_0()
                .gap(px(6.))
                .child(section_title("ADD GEOMETRY"))
                .children(
                    [
                        Primitive::Cube,
                        Primitive::Sphere,
                        Primitive::Cylinder,
                        Primitive::Torus,
                        Primitive::Plane,
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(i, p)| {
                        row()
                            .id(SharedString::from(format!("add-{i}")))
                            .h(px(31.))
                            .px(px(8.))
                            .gap(px(9.))
                            .rounded(px(4.))
                            .cursor_pointer()
                            .hover(|d| d.bg(rgb(RAISED)))
                            .child(icon(
                                if p == Primitive::Plane {
                                    Icon::Grid
                                } else {
                                    Icon::Cube
                                },
                                MUTED,
                                14.,
                            ))
                            .child(p.label())
                            .child(div().flex_1())
                            .child(icon(Icon::Plus, FAINT, 11.))
                            .on_click(
                                cx.listener(move |s, _, w, cx| s.execute(Command::Add(p), w, cx)),
                            )
                            .into_any_element()
                    }),
                ),
        )
        .child(
            col()
                .p(px(13.))
                .flex_shrink_0()
                .gap(px(8.))
                .border_t_1()
                .border_color(rgb(LINE))
                .child(button(
                    "import-obj",
                    "Import OBJ",
                    Some(Icon::Folder),
                    Command::ImportObj,
                    false,
                    cx,
                ))
                .child(
                    row()
                        .gap(px(6.))
                        .text_size(px(10.))
                        .text_color(rgb(FAINT))
                        .child("LOCAL WORKSPACE")
                        .child(div().flex_1())
                        .child("01"),
                ),
        )
        .into_any_element()
}

fn toolrail(s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    col()
        .w(px(44.))
        .h_full()
        .flex_shrink_0()
        .items_center()
        .py(px(10.))
        .gap(px(7.))
        .bg(rgb(BG))
        .border_r_1()
        .border_color(rgb(LINE))
        .children(
            [
                (Tool::Select, Icon::Select, "Q"),
                (Tool::Move, Icon::Move, "G"),
                (Tool::Rotate, Icon::Rotate, "R"),
                (Tool::Scale, Icon::Scale, "S"),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, (tool, glyph, _))| {
                let active = s.tool == tool;
                row()
                    .id(SharedString::from(format!("tool-{i}")))
                    .size(px(32.))
                    .justify_center()
                    .rounded(px(5.))
                    .bg(rgb(if active { ACTIVE } else { BG }))
                    .cursor_pointer()
                    .tooltip(move |_, cx| {
                        cx.new(|_| Tooltip(command_hint(Command::SetTool(tool))))
                            .into()
                    })
                    .hover(|d| d.bg(rgb(RAISED)))
                    .child(icon(glyph, if active { ACCENT } else { MUTED }, 17.))
                    .on_click(
                        cx.listener(move |s, _, w, cx| s.execute(Command::SetTool(tool), w, cx)),
                    )
                    .into_any_element()
            }),
        )
        .child(div().w(px(20.)).h(px(1.)).my(px(5.)).bg(rgb(LINE)))
        .child(button(
            "rail-frame",
            "",
            Some(Icon::Frame),
            Command::FrameSelected,
            false,
            cx,
        ))
        .child(button(
            "rail-grid",
            "",
            Some(Icon::Grid),
            Command::ToggleGrid,
            s.settings.show_grid,
            cx,
        ))
        .child(div().flex_1())
        .child(button(
            "rail-undo",
            "",
            Some(Icon::Undo),
            Command::Undo,
            false,
            cx,
        ))
        .child(button(
            "rail-redo",
            "",
            Some(Icon::Redo),
            Command::Redo,
            false,
            cx,
        ))
        .into_any_element()
}

fn viewport_panel(s: &Studio, viewport: AnyElement, cx: &mut Context<Studio>) -> AnyElement {
    col()
        .flex_1()
        .min_w(px(180.))
        .h_full()
        .overflow_hidden()
        .bg(rgb(BG))
        .child(
            row()
                .h(px(34.))
                .flex_shrink_0()
                .px(px(12.))
                .gap(px(8.))
                .border_b_1()
                .border_color(rgb(LINE))
                .child(div().text_size(px(11.)).text_color(rgb(MUTED)).child(
                    if s.scene.camera.orthographic {
                        "Orthographic"
                    } else {
                        "Perspective"
                    },
                ))
                .child(div().text_color(rgb(FAINT)).child("/"))
                .child(
                    div()
                        .min_w(px(0.))
                        .max_w(px(160.))
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_color(rgb(FAINT))
                        .child(if s.settings.mode == RenderMode::MaterialPreview {
                            preview_light_name(s)
                        } else {
                            s.settings.mode.label().to_owned()
                        }),
                )
                .child(div().flex_1())
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
                .child(button(
                    "projection",
                    "",
                    Some(Icon::Cube),
                    Command::ToggleProjection,
                    s.scene.camera.orthographic,
                    cx,
                )),
        )
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
        .child(
            row()
                .h(px(31.))
                .flex_shrink_0()
                .px(px(13.))
                .gap(px(7.))
                .border_t_1()
                .border_color(rgb(LINE))
                .text_size(px(10.))
                .text_color(rgb(FAINT))
                .child(key("MMB"))
                .child("Orbit")
                .child(key("⇧ MMB"))
                .child("Pan")
                .child(key("Scroll"))
                .child("Zoom")
                .child(div().flex_1())
                .child(if s.edit_mode {
                    s.selected_face
                        .map(|face| format!("Face {} selected", face + 1))
                        .unwrap_or("Click a face to select".into())
                } else {
                    "Object selection".into()
                }),
        )
        .into_any_element()
}

fn field(
    s: &Studio,
    id: &str,
    label: &str,
    f: Field,
    width: f32,
    cx: &mut Context<Studio>,
) -> AnyElement {
    let active = s.field_is_active(f);
    let value = if let Some((active_field, value)) = &s.active_field {
        if *active_field == f {
            format!("{value}│")
        } else {
            s.field_value(f)
        }
    } else {
        s.field_value(f)
    };
    row()
        .id(SharedString::from(id.to_owned()))
        .h(px(29.))
        .w(px(width))
        .px(px(7.))
        .gap(px(5.))
        .rounded(px(4.))
        .bg(rgb(BG))
        .border_1()
        .border_color(rgb(if active { ACCENT } else { LINE }))
        .cursor_pointer()
        .hover(|d| d.border_color(rgb(MUTED)))
        .when(!label.is_empty(), |d| {
            d.child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(match label {
                        "X" | "R" => 0xba817f,
                        "Y" | "G" => 0x9ab78b,
                        "Z" | "B" => 0x83a2cb,
                        _ => MUTED,
                    }))
                    .child(label.to_owned()),
            )
        })
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .text_ellipsis()
                .text_size(px(11.))
                .text_color(rgb(if active { ACCENT } else { TEXT }))
                .text_right()
                .child(value),
        )
        .on_click(cx.listener(move |s, _, w, cx| s.begin_field(f, w, cx)))
        .into_any_element()
}
fn property(s: &Studio, id: &str, label: &str, f: Field, cx: &mut Context<Studio>) -> AnyElement {
    row()
        .h(px(33.))
        .justify_between()
        .text_color(rgb(MUTED))
        .child(label.to_owned())
        .child(field(s, id, "", f, 105., cx))
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

fn render_settings(s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    let mode = s.settings.mode;
    let preview = mode == RenderMode::MaterialPreview;
    let progressive = mode.progressive();
    col()
        .px(px(15.))
        .py(px(8.))
        .gap(px(2.))
        .child(section_title(if preview { "MATERIAL PREVIEW" } else { "RENDER" }))
        .child(
            row()
                .justify_between()
                .h(px(28.))
                .child(div().text_color(rgb(MUTED)).child("Engine"))
                .child(
                    row()
                        .gap(px(5.))
                        .child(icon(Icon::Spark, ACCENT, 12.))
                        .child(div().text_color(rgb(ACCENT)).child(match mode {
                            RenderMode::MaterialPreview => "Studio IBL",
                            RenderMode::Rendered => "Forma Path",
                            RenderMode::Wireframe | RenderMode::Solid => "Forma View",
                        })),
                ),
        )
        .when(progressive, |d| {
            d.child(property(s, "samples", "Max samples", Field::Samples, cx))
                .child(property(s, "bounces", "Light bounces", Field::Bounces, cx))
        })
        .when(preview || progressive, |d| {
            d.child(property(s, "exposure", "Exposure", Field::Exposure, cx))
        })
        .when(progressive || (preview && s.settings.preview.use_scene_world), |d| {
            d.child(property(s, "world-strength", "World strength", Field::WorldStrength, cx))
        })
        .when(preview, |d| {
            d.child(
                row()
                    .h(px(35.))
                    .gap(px(8.))
                    .child(div().text_color(rgb(MUTED)).child("Lighting"))
                    .child(div().flex_1())
                    .child(
                        div()
                            .max_w(px(100.))
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_color(rgb(TEXT))
                            .child(preview_light_name(s)),
                    )
                    .child(button(
                        "inspector-preview-settings",
                        "",
                        Some(Icon::Chevron),
                        Command::TogglePreviewSettings,
                        s.preview_open,
                        cx,
                    )),
            )
        })
        .child(div().h(px(8.)))
        .child(
            row()
                .justify_between()
                .text_size(px(10.))
                .text_color(rgb(FAINT))
                .child(if progressive { "PROGRESSIVE SAMPLING" } else { "INTERACTIVE VIEW" })
                .child(if progressive {
                    format!("{} / {}", s.samples, s.settings.max_samples)
                } else if preview {
                    "Real-time".to_owned()
                } else {
                    mode.label().to_owned()
                }),
        )
        .when(progressive, |d| {
            d.child(
                div()
                    .h(px(3.))
                    .mt(px(5.))
                    .mb(px(9.))
                    .rounded_full()
                    .bg(rgb(BG))
                    .child(
                        div()
                            .h_full()
                            .rounded_full()
                            .w(relative((s.samples as f32 / s.settings.max_samples.max(1) as f32).clamp(0., 1.)))
                            .bg(rgb(ACCENT)),
                    ),
            )
        })
        .child(
            div()
                .mt(px(7.))
                .text_size(px(10.))
                .line_height(px(15.))
                .text_color(rgb(FAINT))
                .child(match mode {
                    RenderMode::MaterialPreview => "Inspect surfaces under studio lighting. Preview lighting stays separate from your final render.",
                    RenderMode::Rendered => "Rendered mode accumulates light paths as the scene settles.",
                    RenderMode::Wireframe => "See mesh edges and topology through the scene.",
                    RenderMode::Solid => "Neutral studio shading for shaping geometry.",
                }),
        )
        .into_any_element()
}

fn inspector(s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    let object = s.selected_object();
    let mut contents = col().gap(px(2.));
    if let Some(object) = object {
        contents = contents
            .child(
                col()
                    .px(px(15.))
                    .pb(px(13.))
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(section_title("TRANSFORM"))
                    .children(
                        [("Position", 0), ("Rotation", 1), ("Scale", 2)]
                            .into_iter()
                            .map(|(label, kind)| {
                                col()
                                    .gap(px(5.))
                                    .mb(px(9.))
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(rgb(MUTED))
                                            .child(label),
                                    )
                                    .child(row().gap(px(5.)).children((0..3).map(|axis| {
                                        field(
                                            s,
                                            &format!("transform-{kind}-{axis}"),
                                            ["X", "Y", "Z"][axis],
                                            match kind {
                                                0 => Field::Translation(axis),
                                                1 => Field::Rotation(axis),
                                                _ => Field::Scale(axis),
                                            },
                                            75.,
                                            cx,
                                        )
                                    })))
                                    .into_any_element()
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(FAINT))
                            .child("Click a value to edit · Enter to apply"),
                    ),
            )
            .child(
                col()
                    .px(px(15.))
                    .py(px(8.))
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(section_title("SURFACE"))
                    .child(
                        row()
                            .gap(px(10.))
                            .mb(px(10.))
                            .child(
                                div()
                                    .size(px(32.))
                                    .rounded(px(8.))
                                    .bg(rgb(swatch_color(object.material.base_color)))
                                    .border_1()
                                    .border_color(rgb(0x59625f)),
                            )
                            .child(
                                col()
                                    .gap(px(3.))
                                    .child(div().text_color(rgb(TEXT)).child("Principled surface"))
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(rgb(FAINT))
                                            .child("Metallic / roughness"),
                                    ),
                            ),
                    )
                    .child(
                        row().gap(px(7.)).mb(px(8.)).children(
                            [
                                (0, 0x83b7a6, "Jade"),
                                (1, 0xe4e4dc, "Porcelain"),
                                (2, 0xc38967, "Copper"),
                                (3, 0x525860, "Graphite"),
                                (4, 0xf3e9c9, "Light"),
                            ]
                            .into_iter()
                            .map(|(index, color, name)| {
                                col()
                                    .id(SharedString::from(format!("preset-{index}")))
                                    .w(px(41.))
                                    .items_center()
                                    .gap(px(5.))
                                    .cursor_pointer()
                                    .hover(|d| d.text_color(rgb(ACCENT)))
                                    .text_color(rgb(FAINT))
                                    .child(
                                        div()
                                            .size(px(25.))
                                            .rounded_full()
                                            .border_2()
                                            .border_color(rgb(LINE))
                                            .bg(rgb(color)),
                                    )
                                    .child(div().text_size(px(9.)).child(name))
                                    .on_click(cx.listener(move |s, _, w, cx| {
                                        s.execute(Command::MaterialPreset(index), w, cx)
                                    }))
                                    .into_any_element()
                            }),
                        ),
                    )
                    .child(
                        col()
                            .gap(px(5.))
                            .mb(px(9.))
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(MUTED))
                                    .child("Base color · sRGB"),
                            )
                            .child(row().gap(px(5.)).children((0..3).map(|axis| {
                                field(
                                    s,
                                    &format!("base-color-{axis}"),
                                    ["R", "G", "B"][axis],
                                    Field::Color(axis),
                                    75.,
                                    cx,
                                )
                            }))),
                    )
                    .child(property(s, "roughness", "Roughness", Field::Roughness, cx))
                    .child(property(s, "metallic", "Metallic", Field::Metallic, cx))
                    .child(property(s, "emission", "Emission", Field::Emission, cx)),
            )
            .child(
                col()
                    .px(px(15.))
                    .py(px(8.))
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(section_title("GEOMETRY"))
                    .child(
                        row().justify_between().mb(px(10.)).children(
                            [
                                ("Vertices", object.mesh.positions.len()),
                                ("Faces", object.mesh.faces.len()),
                            ]
                            .into_iter()
                            .map(|(label, count)| {
                                col()
                                    .gap(px(4.))
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(rgb(FAINT))
                                            .child(label),
                                    )
                                    .child(div().text_color(rgb(TEXT)).child(count.to_string()))
                            }),
                        ),
                    )
                    .child(
                        row()
                            .gap(px(7.))
                            .child(button(
                                "subdivide",
                                "Subdivide",
                                Some(Icon::Grid),
                                Command::Subdivide,
                                false,
                                cx,
                            ))
                            .child(button(
                                "extrude",
                                "Extrude",
                                Some(Icon::Arrow),
                                Command::Extrude,
                                false,
                                cx,
                            )),
                    ),
            );
    } else {
        contents = contents.child(
            col()
                .p(px(20.))
                .gap(px(10.))
                .child(icon(Icon::Select, FAINT, 24.))
                .child("Nothing selected")
                .child(
                    div()
                        .text_color(rgb(FAINT))
                        .child("Select an object in the scene to edit its geometry and surface."),
                ),
        );
    }
    contents = contents.child(render_settings(s, cx));
    col()
        .w(px(268.))
        .flex_shrink_0()
        .h_full()
        .bg(rgb(PANEL))
        .border_l_1()
        .border_color(rgb(LINE))
        .child(
            row()
                .h(px(41.))
                .flex_shrink_0()
                .px(px(15.))
                .gap(px(9.))
                .border_b_1()
                .border_color(rgb(LINE))
                .child(icon(Icon::Cube, ACCENT, 14.))
                .child(if object.is_some() {
                    div()
                        .id("object-name")
                        .flex_1()
                        .min_w(px(0.))
                        .px(px(5.))
                        .py(px(4.))
                        .rounded(px(4.))
                        .text_color(rgb(if s.field_is_active(Field::Name) {
                            ACCENT
                        } else {
                            TEXT
                        }))
                        .font_weight(FontWeight::MEDIUM)
                        .cursor_pointer()
                        .hover(|d| d.bg(rgb(RAISED)))
                        .tooltip(|_, cx| {
                            cx.new(|_| Tooltip("Rename object · Enter to apply")).into()
                        })
                        .child(div().overflow_hidden().text_ellipsis().child(
                            match &s.active_field {
                                Some((Field::Name, text)) => format!("{text}│"),
                                _ => s.field_value(Field::Name),
                            },
                        ))
                        .on_click(
                            cx.listener(|s, _, window, cx| s.begin_field(Field::Name, window, cx)),
                        )
                        .into_any_element()
                } else {
                    div()
                        .text_color(rgb(TEXT))
                        .child("Scene settings")
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
    row()
        .h(px(26.))
        .flex_shrink_0()
        .px(px(13.))
        .gap(px(9.))
        .bg(rgb(PANEL))
        .border_t_1()
        .border_color(rgb(LINE))
        .text_size(px(10.))
        .text_color(rgb(FAINT))
        .child(
            div()
                .size(px(5.))
                .rounded_full()
                .bg(rgb(if s.render_error.is_some() {
                    0xd29a75
                } else {
                    ACCENT
                })),
        )
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .text_ellipsis()
                .child(s.status.clone()),
        )
        .child(
            div()
                .text_color(rgb(MUTED))
                .child(format!("{:.1} ms", s.render_ms)),
        )
        .child(separator())
        .child(s.device_name.clone())
        .child(separator())
        .child(button(
            "footer-help",
            "?",
            None,
            Command::ToggleHelp,
            s.help_open,
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
    .h(px(43.))
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
    row()
        .id(id)
        .h(px(44.))
        .justify_between()
        .cursor_pointer()
        .child(
            col()
                .gap(px(3.))
                .child(div().text_color(rgb(TEXT)).child(label))
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(FAINT))
                        .child(detail),
                ),
        )
        .child(
            row()
                .w(px(28.))
                .h(px(16.))
                .px(px(3.))
                .rounded_full()
                .bg(rgb(if active { ACTIVE } else { BG }))
                .border_1()
                .border_color(rgb(if active { 0x517f72 } else { LINE }))
                .when(active, |d| d.justify_end())
                .child(div().size(px(9.)).rounded_full().bg(rgb(if active {
                    ACCENT
                } else {
                    FAINT
                }))),
        )
        .on_click(cx.listener(move |s, _, w, cx| s.execute(command, w, cx)))
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
        .h(px(33.))
        .justify_between()
        .text_color(rgb(FAINT))
        .child(label.to_owned())
        .child(
            row()
                .w(px(105.))
                .h(px(29.))
                .px(px(7.))
                .justify_end()
                .rounded(px(4.))
                .bg(rgb(BG))
                .child(s.field_value(field_id)),
        )
        .into_any_element()
}

fn preview_overlay(s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    let bounds = s.bounds.get();
    let width = 324.;
    let left = (f32::from(bounds.right()) - width - 8.).max(f32::from(bounds.left()) + 8.);
    let top = f32::from(bounds.top()) + 8.;
    let height = (f32::from(bounds.size.height) - 16.).clamp(280., 500.);
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
                .h(px(height))
                .rounded(px(9.))
                .bg(rgb(PANEL))
                .border_1()
                .border_color(rgb(0x46514f))
                .shadow_lg()
                .overflow_hidden()
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
                .child(
                    row()
                        .h(px(47.))
                        .flex_shrink_0()
                        .px(px(14.))
                        .gap(px(8.))
                        .border_b_1()
                        .border_color(rgb(LINE))
                        .child(icon(Icon::Material, ACCENT, 16.))
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
                        .p(px(14.))
                        .gap(px(8.))
                        .child(
                            row()
                                .justify_between()
                                .text_size(px(10.))
                                .text_color(rgb(MUTED))
                                .child("STUDIO ENVIRONMENT")
                                .child(div().text_color(rgb(FAINT)).child("Viewport only")),
                        )
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
                                        .rounded(px(5.))
                                        .overflow_hidden()
                                        .border_1()
                                        .border_color(rgb(if active { ACCENT } else { LINE }))
                                        .bg(rgb(if active { ACTIVE } else { BG }))
                                        .when(studio_enabled, |d| {
                                            d.cursor_pointer()
                                                .hover(|d| d.border_color(rgb(MUTED)))
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
                                                .h(px(27.))
                                                .justify_center()
                                                .gap(px(4.))
                                                .text_size(px(10.))
                                                .text_color(rgb(if active {
                                                    ACCENT
                                                } else {
                                                    MUTED
                                                }))
                                                .when(active, |d| {
                                                    d.child(
                                                        div()
                                                            .size(px(4.))
                                                            .rounded_full()
                                                            .bg(rgb(ACCENT)),
                                                    )
                                                })
                                                .child(studio.label()),
                                        )
                                        .into_any_element()
                                }),
                            ),
                        )
                        .child(
                            row()
                                .h(px(29.))
                                .gap(px(8.))
                                .child(
                                    div()
                                        .flex_1()
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
                                .gap(px(1.))
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
                                    "Background blur · %",
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
                        .when(preview.use_scene_world, |d| {
                            d.child(
                                div()
                                    .text_size(px(10.))
                                    .line_height(px(14.))
                                    .text_color(rgb(FAINT))
                                    .child("Studio settings are kept for when you switch back."),
                            )
                        })
                        .child(
                            row()
                                .justify_between()
                                .child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(rgb(FAINT))
                                        .child("Independent of final rendering"),
                                )
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
        .bg(rgba(0x080b0db8))
        .occlude()
        .flex()
        .justify_center()
        .pt(px(90.))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|s, _, w, cx| s.execute(Command::TogglePalette, w, cx)),
        )
        .child(
            col()
                .id("command-menu")
                .occlude()
                .w(px(470.))
                .h(px(70. + 34. * commands.len().clamp(1, 12) as f32))
                .rounded(px(10.))
                .bg(rgb(PANEL))
                .border_1()
                .border_color(rgb(0x46514f))
                .shadow_lg()
                .overflow_hidden()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    row()
                        .h(px(52.))
                        .flex_shrink_0()
                        .px(px(18.))
                        .gap(px(10.))
                        .border_b_1()
                        .border_color(rgb(LINE))
                        .child(icon(Icon::Search, ACCENT, 16.))
                        .child(div().text_size(px(13.)).text_color(rgb(TEXT)).child(
                            if s.palette_query.is_empty() {
                                "Type a command…".into()
                            } else {
                                format!("{}│", s.palette_query)
                            },
                        ))
                        .child(div().flex_1())
                        .child(key("ESC")),
                )
                .child(
                    col()
                        .p(px(7.))
                        .gap(px(2.))
                        .when(commands.is_empty(), |d| {
                            d.child(
                                div()
                                    .p_4()
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
                                        .h(px(32.))
                                        .px(px(12.))
                                        .rounded(px(4.))
                                        .when(i == selected, |d| {
                                            d.bg(rgb(ACTIVE)).text_color(rgb(ACCENT))
                                        })
                                        .cursor_pointer()
                                        .hover(|d| d.bg(rgb(ACTIVE)).text_color(rgb(ACCENT)))
                                        .child(label)
                                        .child(div().flex_1())
                                        .child(
                                            div()
                                                .text_size(px(10.))
                                                .text_color(rgb(FAINT))
                                                .child(shortcut),
                                        )
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
    let shortcuts = [
        (
            "NAVIGATION",
            vec![
                ("Middle drag", "Orbit view"),
                ("Shift + middle drag", "Pan view"),
                ("Scroll", "Zoom"),
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
                ("X / Y / Z during transform", "Constrain axis"),
                ("Enter / Escape", "Confirm / cancel transform"),
                ("Tab", "Object / face edit"),
                ("E", "Extrude selected face"),
                ("Shift + D", "Duplicate object"),
            ],
        ),
        (
            "WORKSPACE",
            vec![
                ("Z", "Shading pie · hold and flick, or tap"),
                ("4 / 6 / 2 / 8 in pie", "Wire / solid / material / rendered"),
                ("X / C / V", "Solid / material / rendered directly"),
                ("⌘ Z / ⇧ ⌘ Z", "Undo / redo"),
                ("⌘ S / ⌘ O", "Save / open"),
                ("⌘ K", "Workspace commands"),
                ("?", "This reference"),
            ],
        ),
    ];
    div()
        .absolute()
        .inset_0()
        .bg(rgba(0x080b0db8))
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
                .w(px(550.))
                .max_h(relative(0.88))
                .overflow_y_scroll()
                .rounded(px(10.))
                .bg(rgb(PANEL))
                .border_1()
                .border_color(rgb(0x46514f))
                .shadow_lg()
                .p(px(22.))
                .gap(px(13.))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    row()
                        .justify_between()
                        .child(
                            col()
                                .gap(px(6.))
                                .child(
                                    div()
                                        .text_size(px(20.))
                                        .text_color(rgb(TEXT))
                                        .child("Make room for making."),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(rgb(MUTED))
                                        .child("Your Forma keyboard reference"),
                                ),
                        )
                        .child(button(
                            "close-help",
                            "Close",
                            None,
                            Command::ToggleHelp,
                            false,
                            cx,
                        )),
                )
                .children(shortcuts.into_iter().map(|(title, keys)| {
                    col()
                        .gap(px(6.))
                        .child(section_title(title))
                        .children(keys.into_iter().map(|(shortcut, label)| {
                            row()
                                .justify_between()
                                .h(px(23.))
                                .child(div().text_color(rgb(MUTED)).child(label))
                                .child(key(shortcut))
                        }))
                })),
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
        .bg(rgba(0x080b0d35))
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
                    for choice in CHOICES {
                        let mode = choice.mode;
                        let angle = choice.offset.y.atan2(choice.offset.x);
                        let a = angle - 0.70;
                        let b = angle + 0.70;
                        let mut wedge = PathBuilder::fill();
                        wedge.move_to(at(38., a));
                        for i in 0..=24 {
                            wedge.line_to(at(60., a + (b - a) * i as f32 / 24.));
                        }
                        for i in (0..=24).rev() {
                            wedge.line_to(at(38., a + (b - a) * i as f32 / 24.));
                        }
                        wedge.close();
                        if let Ok(path) = wedge.build() {
                            window.paint_path(
                                path,
                                rgb(if hovered == Some(mode) { ACTIVE } else { PANEL }),
                            );
                        }
                        if hovered == Some(mode) || current == mode {
                            let mut arc = PathBuilder::stroke(px(2. * scale));
                            let radius = if hovered == Some(mode) { 60. } else { 38. };
                            arc.move_to(at(radius, a));
                            for i in 1..=24 {
                                arc.line_to(at(radius, a + (b - a) * i as f32 / 24.));
                            }
                            if let Ok(path) = arc.build() {
                                window.paint_path(
                                    path,
                                    rgb(if hovered == Some(mode) {
                                        ACCENT
                                    } else {
                                        0x517f72
                                    }),
                                );
                            }
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
                        .text_size(px(20. * scale))
                        .text_color(rgb(TEXT))
                        .child("Z"),
                )
                .child(
                    div()
                        .text_size(px(8. * scale))
                        .text_color(rgb(MUTED))
                        .child("SHADING"),
                ),
        )
        .children(CHOICES.into_iter().map(|choice| {
            let mode = choice.mode;
            let number = choice.key;
            let (glyph, detail) = match mode {
                RenderMode::Wireframe => (Icon::Wire, "Polygon edges"),
                RenderMode::Solid => (Icon::Solid, "Studio clay"),
                RenderMode::MaterialPreview => (Icon::Material, "Studio lighting"),
                RenderMode::Rendered => (Icon::Render, "Scene lighting"),
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
                .gap(px(8. * scale))
                .rounded(px(8. * scale))
                .border_1()
                .border_color(rgb(if highlighted {
                    ACCENT
                } else if active {
                    0x517f72
                } else {
                    LINE
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
                    col()
                        .flex_1()
                        .gap(px(3. * scale))
                        .child(
                            div()
                                .text_size(px(11. * scale))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(if highlighted { 0xe4f4ee } else { TEXT }))
                                .child(mode.label()),
                        )
                        .child(
                            div()
                                .text_size(px(9. * scale))
                                .text_color(rgb(if active { ACCENT } else { MUTED }))
                                .child(if active { "Current mode" } else { detail }),
                        ),
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
        .child(
            col()
                .absolute()
                .left(px(center.x - 166. * scale))
                .top(px(center.y + 143. * scale))
                .w(px(332. * scale))
                .h(px(43. * scale))
                .rounded(px(6. * scale))
                .bg(rgb(PANEL))
                .border_1()
                .border_color(rgb(LINE))
                .shadow_md()
                .items_center()
                .justify_center()
                .gap(px(3. * scale))
                .line_height(px(13. * scale))
                .text_size(px(10. * scale))
                .text_color(rgb(TEXT))
                .child(if pie.trigger_held {
                    "Hold Z, move & release"
                } else {
                    "Choose viewport shading"
                })
                .child(
                    div()
                        .text_size(px(9. * scale))
                        .text_color(rgb(MUTED))
                        .child("Click or press 4 / 6 / 2 / 8   ·   Esc to cancel"),
                ),
        )
        .into_any_element()
}

pub fn render(studio: &Studio, viewport: AnyElement, cx: &mut Context<Studio>) -> AnyElement {
    col()
        .relative()
        .size_full()
        .overflow_hidden()
        .bg(rgb(BG))
        .text_color(rgb(TEXT))
        .text_size(px(11.))
        .font_family(".AppleSystemUIFont")
        .child(titlebar(studio, cx))
        .child(workspace_bar(studio, cx))
        .child(
            row()
                .flex_1()
                .min_h(px(0.))
                .w_full()
                .overflow_hidden()
                .child(outliner(studio, cx))
                .child(viewport_panel(studio, viewport, cx))
                .child(inspector(studio, cx)),
        )
        .child(footer(studio, cx))
        .when(studio.preview_open, |d| {
            d.child(preview_overlay(studio, cx))
        })
        .when(studio.palette_open, |d| {
            d.child(command_overlay(studio, cx))
        })
        .when(studio.help_open, |d| d.child(help_overlay(cx)))
        .when(studio.shading_pie.is_some(), |d| {
            d.child(shading_overlay(studio, cx))
        })
        .into_any_element()
}
