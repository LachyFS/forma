//! Shading pie: four directional targets around the cursor.
use super::super::assets::icon;
use super::super::commands::shading_icon;
use super::super::components::*;
use super::super::tokens::*;
use super::{float_border, float_shadow};
use crate::app::{Command, Studio};
use crate::shading_pie::{CARD_HALF_SIZE, CHOICES};
use crate::theme::Colors;
use forma_render::RenderMode;
use gpui::{prelude::*, *};

fn label(mode: RenderMode) -> &'static str {
    match mode {
        RenderMode::Wireframe => "Wireframe",
        RenderMode::Solid => "Solid",
        RenderMode::MaterialPreview => "Material preview",
        RenderMode::Rendered => "Rendered",
    }
}

pub(super) fn overlay(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    let pie = s.shading_pie.as_ref().unwrap();
    let center = pie.center;
    let scale = pie.scale;
    let hovered = pie.hovered;
    let current = s.settings.mode;
    let ring = canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            use std::f32::consts::{FRAC_PI_4, TAU};
            let origin = bounds.center();
            let at = |radius: f32, angle: f32| {
                origin
                    + point(
                        px(radius * scale * angle.cos()),
                        px(radius * scale * angle.sin()),
                    )
            };
            let segment = |start: f32, end: f32, color: Hsla, window: &mut Window| {
                let mut ring = PathBuilder::fill();
                ring.move_to(at(58., start));
                for i in 1..=32 {
                    ring.line_to(at(58., start + (end - start) * i as f32 / 32.));
                }
                for i in (0..=32).rev() {
                    ring.line_to(at(40., start + (end - start) * i as f32 / 32.));
                }
                ring.close();
                if let Ok(path) = ring.build() {
                    window.paint_path(path, color);
                }
            };
            // One uninterrupted annulus, with the selection layered over it so
            // no background shows between modes.
            segment(0., TAU, hex(t.overlay), window);
            let selected = hovered.unwrap_or(current);
            if let Some(choice) = CHOICES.into_iter().find(|choice| choice.mode == selected) {
                let angle = choice.offset.y.atan2(choice.offset.x);
                let (start, end) = (angle - FRAC_PI_4, angle + FRAC_PI_4);
                segment(
                    start,
                    end,
                    hex(if hovered.is_some() {
                        t.accent_soft_hover
                    } else {
                        t.accent_soft
                    }),
                    window,
                );
                let mut accent = PathBuilder::stroke(px(2.5 * scale));
                accent.move_to(at(58., start));
                for i in 1..=32 {
                    accent.line_to(at(58., start + (end - start) * i as f32 / 32.));
                }
                if let Ok(path) = accent.build() {
                    window.paint_path(path, hex(t.accent));
                }
            }
            for angle in [1., 3., 5., 7.].map(|k: f32| k * FRAC_PI_4) {
                let mut divider = PathBuilder::stroke(px(scale.max(0.75)));
                divider.move_to(at(40., angle));
                divider.line_to(at(58., angle));
                if let Ok(path) = divider.build() {
                    window.paint_path(path, hex(t.hairline));
                }
            }
        },
    )
    .absolute()
    .left(px(center.x - 60. * scale))
    .top(px(center.y - 60. * scale))
    .size(px(120. * scale));
    div()
        .id("shading-pie-overlay")
        .absolute()
        .inset_0()
        .occlude()
        .bg(tint(0x000000, if t.dark { 0.22 } else { 0.1 }))
        .on_mouse_move(cx.listener(Studio::mouse_move))
        .on_mouse_down(MouseButton::Left, cx.listener(Studio::mouse_down))
        .on_mouse_down(MouseButton::Right, cx.listener(Studio::mouse_down))
        .on_mouse_down(MouseButton::Middle, cx.listener(Studio::mouse_down))
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .child(ring)
        .child(
            col()
                .absolute()
                .left(px(center.x - 34. * scale))
                .top(px(center.y - 34. * scale))
                .size(px(68. * scale))
                .rounded_full()
                .bg(rgb(t.overlay))
                .border_1()
                .border_color(float_border(t))
                .shadow(float_shadow(t))
                .items_center()
                .justify_center()
                .gap(px(2. * scale))
                .child(
                    div()
                        .text_size(px(18. * scale))
                        .font_weight(WEIGHT_SEMIBOLD)
                        .text_color(rgb(t.text))
                        .child("Z"),
                )
                .child(
                    div()
                        .text_size(px(TEXT_XS * scale))
                        .text_color(rgb(t.text_muted))
                        .child("Shading"),
                ),
        )
        .children(CHOICES.into_iter().map(|choice| {
            let mode = choice.mode;
            let origin = center + (choice.offset - CARD_HALF_SIZE) * scale;
            let active = current == mode;
            let highlighted = hovered == Some(mode);
            row()
                .id(SharedString::from(format!("shading-pie-{}", choice.key)))
                .absolute()
                .left(px(origin.x))
                .top(px(origin.y))
                .w(px(CARD_HALF_SIZE.x * 2. * scale))
                .h(px(CARD_HALF_SIZE.y * 2. * scale))
                .px(px(10. * scale))
                .gap(px(8. * scale))
                .rounded(px(R_LG * scale))
                .border_1()
                .border_color(if highlighted {
                    hex(t.focus)
                } else if active {
                    hex(t.accent_border)
                } else {
                    float_border(t)
                })
                .bg(rgb(if highlighted {
                    t.accent_soft
                } else {
                    t.overlay
                }))
                .shadow(float_shadow(t))
                .cursor_pointer()
                .child(icon(
                    shading_icon(mode),
                    ICON_LG * scale,
                    if active || highlighted {
                        t.accent_text
                    } else {
                        t.text_muted
                    },
                ))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(TEXT_LG * scale))
                        .font_weight(WEIGHT_MEDIUM)
                        .text_color(rgb(t.text))
                        .child(label(mode)),
                )
                .child(keycap(t, choice.key))
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
