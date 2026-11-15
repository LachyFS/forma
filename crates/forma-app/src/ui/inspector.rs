//! Inspector: a fixed header with the selection's name, then two pages —
//! the selection's properties and the viewport/render settings — built from
//! collapsible sections and label/value property rows.
use super::assets::{Icon, icon};
use super::commands::shading_icon;
use super::components::*;
use super::outliner::{kind_icon, kind_label};
use super::tokens::*;
use super::{InspectorTab, PanelSection};
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
            let kind = object.kind();
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
                .child(tab_button(t, InspectorTab::Render, "Render", tab, cx))
                .child(spacer())
                .child(
                    div()
                        .pr(px(SP_1))
                        .text_size(px(TEXT_SM))
                        .text_color(rgb(t.text_faint))
                        .child(match s.settings.mode {
                            RenderMode::Wireframe => "Wireframe",
                            RenderMode::Solid => "Solid",
                            RenderMode::MaterialPreview => "Material preview",
                            RenderMode::Rendered => "Rendered",
                        }),
                ),
        )
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
                        .px(px(SP_1 + 2.))
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
        .child(
            div()
                .pt(px(SP_1 + 2.))
                .text_size(px(TEXT_SM))
                .text_color(rgb(t.text_faint))
                .child("Type exact values: 25mm · 1/8in · (2+3)*10cm"),
        )
}

fn surface(
    t: Colors,
    s: &Studio,
    material: &forma_core::Material,
    cx: &mut Context<Studio>,
) -> Div {
    let base = material.base_color;
    let presets = row()
        .gap(px(SP_1 + 2.))
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
                                    .text_size(px(TEXT_XS))
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
                        .gap(px(SP_1 + 2.))
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
                .child(
                    row()
                        .flex_1()
                        .min_w(px(0.))
                        .gap(px(SP_1))
                        .children((0..3).map(|axis| {
                            numeric_field(
                                t,
                                s,
                                SharedString::from(format!("base-color-{axis}")),
                                Field::Color(axis),
                                FieldMark::Axis(axis),
                                ["Red · sRGB", "Green · sRGB", "Blue · sRGB"][axis],
                                cx,
                            )
                            .flex_1()
                            .flex_basis(px(0.))
                        })),
                ),
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
                    .gap(px(SP_1 + 2.))
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

fn geometry(t: Colors, object: &forma_core::MeshInstance, cx: &mut Context<Studio>) -> Div {
    let stat = |label: &'static str, value: usize| {
        col()
            .flex_1()
            .px(px(SP_2 + 2.))
            .py(px(SP_1 + 2.))
            .gap(px(1.))
            .rounded(px(R_SM))
            .bg(rgb(t.field))
            .child(caption(t, label).text_size(px(TEXT_XS)))
            .child(
                div()
                    .tabular()
                    .font_weight(WEIGHT_MEDIUM)
                    .child(value.to_string()),
            )
    };
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
            row()
                .gap(px(SP_1))
                .child(stat("Vertices", object.mesh.positions.len()))
                .child(stat("Faces", object.mesh.faces.len())),
        )
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
}

fn object_page(t: Colors, s: &Studio, stacked: bool, cx: &mut Context<Studio>) -> Div {
    let Some(object) = s.selected_object() else {
        let message = if s.scene.objects.is_empty() {
            (
                "Nothing to edit yet",
                "Add a primitive or import a mesh, then select it to edit its properties.",
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
    col()
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
        .child(section(
            t,
            s,
            PanelSection::Geometry,
            "Geometry",
            Some(format!("{} faces", object.mesh.faces.len()).into()),
            geometry(t, &object, cx),
            cx,
        ))
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
                .gap(px(SP_1 + 2.))
                .child(dot(tone, 6.))
                .child(caption(t, status).text_color(rgb(tone)))
                .child(spacer())
                .child(caption(t, "Open Image Denoise").text_color(rgb(t.text_faint))),
        )
}

fn render_page(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> Div {
    let mode = s.settings.mode;
    let shading = segmented(t).flex_1().children(
        [
            RenderMode::Wireframe,
            RenderMode::Solid,
            RenderMode::MaterialPreview,
            RenderMode::Rendered,
        ]
        .into_iter()
        .map(|candidate| {
            let active = mode == candidate;
            on_command(
                segment(
                    t,
                    SharedString::from(format!("inspector-mode-{}", candidate as usize)),
                    active,
                    22.,
                )
                .flex_1()
                .child(icon(
                    shading_icon(candidate),
                    ICON_SM,
                    if active { t.accent_text } else { t.text_muted },
                ))
                .tooltip(command_tip(t, Command::SetMode(candidate))),
                Command::SetMode(candidate),
                cx,
            )
        }),
    );
    let mut page = col().child(
        col()
            .px(px(SP_3))
            .py(px(SP_2))
            .border_b_1()
            .border_color(rgb(t.hairline))
            .child(property_row(t, "Shading", shading)),
    );
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
                        .gap(px(SP_1 + 2.))
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
                    None,
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
                            .gap(px(SP_1 + 2.))
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
    page
}

pub(super) fn inspector(t: Colors, s: &Studio, width: f32, cx: &mut Context<Studio>) -> Div {
    let stacked = width < STACK_BELOW;
    let page = match s.panels.tab {
        InspectorTab::Object => object_page(t, s, stacked, cx),
        InspectorTab::Render => render_page(t, s, cx),
    };
    col().flex_1().min_h(px(0.)).child(header(t, s, cx)).child(
        col()
            .id("properties-scroll")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .track_scroll(&s.panels.inspector_scroll)
            .child(page.flex_shrink_0().pb(px(SP_4))),
    )
}
