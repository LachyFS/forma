//! Custom surface shader editor.
use super::super::assets::{Icon, icon};
use super::super::components::*;
use super::super::tokens::*;
use super::{animate_in, backdrop, contain};
use crate::app::{Command, Studio};
use crate::platform_shortcut;
use crate::theme::Colors;
use gpui::{prelude::*, *};

pub(super) fn overlay(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    let language = s.renderer_backend.shader_language().label();
    let diagnostics = s.shader_message.clone().map(|message| {
        let failed = message.contains("failed");
        let applied = message.starts_with("Compiled");
        let ink = if failed {
            t.danger
        } else if applied {
            t.success
        } else {
            t.text_muted
        };
        row()
            .id("shader-diagnostics")
            .items_start()
            .max_h(px(110.))
            .overflow_y_scroll()
            .mx(px(SP_4))
            .p(px(SP_2 + 2.))
            .gap(px(SP_2))
            .rounded(px(R_MD))
            .bg(tint(ink, 0.08))
            .border_1()
            .border_color(tint(ink, 0.3))
            .text_size(px(TEXT_SM))
            .line_height(px(16.))
            .text_color(rgb(ink))
            .child(icon(
                if failed {
                    Icon::Error
                } else if applied {
                    Icon::Success
                } else {
                    Icon::Info
                },
                ICON_SM,
                ink,
            ))
            .child(div().flex_1().min_w(px(0.)).child(message))
    });
    let footer = dialog_footer(t)
        .child(
            caption(
                t,
                platform_shortcut(
                    "Only applied code is saved · ⌘ Z undoes code edits",
                    "Only applied code is saved · Ctrl+Z undoes code edits",
                ),
            )
            .text_color(rgb(t.text_faint)),
        )
        .child(spacer())
        .child(command_button(
            t,
            "close-shader",
            "Close",
            None,
            Command::CloseShader,
            Tone::Secondary,
            cx,
        ))
        .child(
            command_button(
                t,
                "apply-shader",
                if s.shader_compiling {
                    "Compiling…"
                } else {
                    "Compile and apply"
                },
                Some(Icon::Check),
                Command::ApplyShader,
                Tone::Primary,
                cx,
            )
            .child(keycaps(t, platform_shortcut("⌘ Return", "Ctrl Return")).opacity(0.85)),
        );
    let panel = dialog(t)
        .id("shader-editor-panel")
        .w(px(800.))
        .max_w(relative(0.94))
        .max_h(relative(0.94))
        .child(dialog_header(
            t,
            Some(Icon::Code),
            "Custom surface shader",
            Some(format!("{language} function body · shared by Material preview and Rendered").into()),
        ))
        .child(
            div()
                .px(px(SP_4))
                .pb(px(SP_3))
                .text_size(px(TEXT_SM))
                .line_height(px(17.))
                .text_color(rgb(t.text_muted))
                .child("Set surface.color, roughness, metallic, emission, normal or ior, and surface.glass = true for refraction. Read input.uv, generated, position, normal and view_direction. Image textures apply before your code."),
        )
        .child(
            div()
                .mx(px(SP_4))
                .mb(px(SP_3))
                .rounded(px(R_MD))
                .overflow_hidden()
                .border_1()
                .border_color(rgb(t.border))
                .child(s.shader_editor.as_ref().unwrap().clone()),
        )
        .children(diagnostics)
        .child(div().h(px(SP_3)))
        .child(footer);
    backdrop(t, "shader-editor-overlay", true, |_, _, _| {}, cx)
        .on_mouse_move(|_, _, cx| cx.stop_propagation())
        .flex()
        .items_center()
        .justify_center()
        .child(animate_in(contain(panel), "shader-editor-enter"))
        .into_any_element()
}
