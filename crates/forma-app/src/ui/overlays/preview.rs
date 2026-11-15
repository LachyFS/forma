//! Material Preview lighting popover, anchored under the viewport controls.
use super::super::assets::{Icon, icon};
use super::super::components::*;
use super::super::hud::HUD_INSET;
use super::super::inspector::preview_light_name;
use super::super::tokens::*;
use super::{backdrop, contain, pop_in};
use crate::app::{Command, Field, Studio};
use crate::theme::Colors;
use forma_render::StudioLight;
use gpui::{prelude::*, *};

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
    .h(px(44.))
    .into_any_element()
}

/// A preview field: element id, label, unit and the field it edits.
type PreviewField = (&'static str, &'static str, &'static str, Field);

fn field_row(
    t: Colors,
    s: &Studio,
    (id, label, unit, field): PreviewField,
    enabled: bool,
    cx: &mut Context<Studio>,
) -> AnyElement {
    if enabled {
        numeric_row(t, s, id, label, unit, field, cx).into_any_element()
    } else {
        readonly_row(
            t,
            label,
            unit,
            s.field_value(field),
            "Uses the scene world · turn off Scene world to adjust",
        )
        .into_any_element()
    }
}

pub(super) fn overlay(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    let bounds = s.bounds.get();
    let width = 320.;
    let right = f32::from(bounds.right()) - HUD_INSET;
    let left = (right - width).max(f32::from(bounds.left()) + HUD_INSET);
    let top = f32::from(bounds.top()) + HUD_INSET + H_MD + 8. + SP_2;
    let max_height = (f32::from(bounds.bottom()) - top - HUD_INSET).max(260.);
    let preview = &s.settings.preview;
    let studio_enabled = !preview.use_scene_world;
    let environments = row().gap(px(SP_1_5)).children(
        [
            StudioLight::Studio,
            StudioLight::Courtyard,
            StudioLight::Sunset,
        ]
        .into_iter()
        .map(|studio| {
            let active = preview.hdri_path.is_none() && preview.studio == studio && studio_enabled;
            col()
                .id(SharedString::from(format!(
                    "preview-studio-{}",
                    studio as u32
                )))
                .flex_1()
                .min_w(px(0.))
                .gap(px(SP_1 + 1.))
                .child(
                    div()
                        .rounded(px(R_MD))
                        .overflow_hidden()
                        .border_2()
                        .border_color(if active { hex(t.focus) } else { clear() })
                        .child(
                            div()
                                .rounded(px(R_SM))
                                .overflow_hidden()
                                .child(studio_swatch(studio)),
                        ),
                )
                .child(
                    div()
                        .text_center()
                        .text_size(px(TEXT_SM))
                        .text_color(rgb(if active { t.accent_text } else { t.text_muted }))
                        .child(studio.label()),
                )
                .when(studio_enabled, |d| {
                    d.cursor_pointer()
                        .tooltip(command_tip(t, Command::SetPreviewStudio(studio)))
                        .on_click(cx.listener(move |s, _, w, cx| {
                            s.execute(Command::SetPreviewStudio(studio), w, cx)
                        }))
                })
                .when(!studio_enabled, |d| d.opacity(0.4))
        }),
    );
    let source = row()
        .h(px(H_MD))
        .gap(px(SP_2))
        .child(icon(
            if preview.hdri_path.is_some() {
                Icon::Image
            } else {
                Icon::Lighting
            },
            ICON_SM,
            t.text_faint,
        ))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_color(rgb(if studio_enabled { t.text } else { t.text_faint }))
                .child(if s.preview_loading {
                    "Loading environment…".to_owned()
                } else {
                    preview_light_name(s)
                }),
        )
        .when(studio_enabled && !s.preview_loading, |d| {
            d.child(command_button(
                t,
                "preview-load-hdr",
                "Load HDR…",
                Some(Icon::Open),
                Command::LoadPreviewHdri,
                Tone::Secondary,
                cx,
            ))
        });
    let panel = popover(t)
        .id("preview-lighting-panel")
        .w(px(width))
        .max_h(px(max_height))
        .p(px(0.))
        .child(
            row()
                .h(px(40.))
                .flex_shrink_0()
                .px(px(SP_3))
                .gap(px(SP_2))
                .border_b_1()
                .border_color(rgb(t.hairline))
                .child(icon(Icon::Lighting, ICON_MD, t.accent_text))
                .child(
                    div()
                        .flex_1()
                        .font_weight(WEIGHT_SEMIBOLD)
                        .child("Preview lighting"),
                )
                .child(badge(t, "Viewport only", BadgeTone::Neutral)),
        )
        .child(
            col()
                .id("preview-lighting-scroll")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .px(px(SP_3))
                .py(px(SP_3))
                .gap(px(SP_2))
                .child(environments)
                .child(source)
                .child(
                    col()
                        .child(field_row(
                            t,
                            s,
                            ("preview-rotation", "Rotation", "°", Field::PreviewRotation),
                            studio_enabled,
                            cx,
                        ))
                        .child(field_row(
                            t,
                            s,
                            ("preview-strength", "Strength", "×", Field::PreviewStrength),
                            studio_enabled,
                            cx,
                        ))
                        .child(field_row(
                            t,
                            s,
                            ("preview-opacity", "Background", "%", Field::PreviewOpacity),
                            true,
                            cx,
                        ))
                        .child(field_row(
                            t,
                            s,
                            ("preview-blur", "Blur", "%", Field::PreviewBlur),
                            studio_enabled,
                            cx,
                        )),
                )
                .child(hrule(t))
                .child(switch_row(
                    t,
                    "preview-scene-world",
                    "Scene world",
                    Some("Use the project's world color and strength"),
                    preview.use_scene_world,
                    Command::TogglePreviewWorld,
                    cx,
                ))
                .child(switch_row(
                    t,
                    "preview-ao",
                    "Contact shading",
                    Some("Ambient occlusion near surfaces"),
                    preview.ambient_occlusion,
                    Command::TogglePreviewAo,
                    cx,
                )),
        )
        .child(
            row()
                .flex_shrink_0()
                .px(px(SP_3))
                .py(px(SP_2))
                .border_t_1()
                .border_color(rgb(t.hairline))
                .bg(rgb(t.raised))
                .child(caption(t, "Never rendered or saved").text_color(rgb(t.text_faint)))
                .child(spacer())
                .child(command_button(
                    t,
                    "preview-reset",
                    "Reset",
                    Some(Icon::Reset),
                    Command::ResetPreview,
                    Tone::Ghost,
                    cx,
                )),
        );
    backdrop(
        t,
        "preview-lighting-overlay",
        false,
        |s, w, cx| s.execute(Command::TogglePreviewSettings, w, cx),
        cx,
    )
    .on_mouse_move(|_, _, cx| cx.stop_propagation())
    .child(
        div()
            .absolute()
            .left(px(left))
            .top(px(top))
            .child(pop_in(contain(panel), "preview-lighting-enter")),
    )
    .into_any_element()
}
