//! Title bar: document identity, the primary mode switcher, command search
//! and file actions. Empty stretches drag the window.
use super::TITLEBAR_HEIGHT;
use super::assets::{Icon, icon};
use super::components::*;
use super::modes::{MODES, ModeState};
use super::tokens::*;
use crate::app::{Command, Studio};
use crate::platform_shortcut;
use crate::theme::Colors;
use gpui::{prelude::*, *};

/// macOS draws its window controls over the left of the title bar.
const TRAFFIC_LIGHT_RESERVE: f32 = 76.;

/// Empty title bar space: drags the window and zooms it on double click.
fn drag_region(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_1()
        .min_w(px(SP_2))
        .h_full()
        .window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |event, window, _| {
            if cfg!(target_os = "linux") && event.click_count == 1 {
                window.start_window_move();
            }
        })
        .on_click(|event, window, _| {
            if event.click_count() == 2 {
                window.titlebar_double_click();
            }
        })
}

fn mode_switcher(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> impl IntoElement {
    segmented(t).children(MODES.iter().map(|mode| {
        let state = (mode.state)(s);
        let active = state == ModeState::Active;
        let ink = if active { t.text } else { t.text_muted };
        let mut segment = segment(
            t,
            SharedString::from(format!("mode-{}", mode.id)),
            active,
            24.,
        )
        .px(px(SP_2 + 2.))
        .child(icon(
            mode.icon,
            ICON_SM,
            if active { t.accent_text } else { ink },
        ))
        .child(mode.label);
        segment = match state {
            ModeState::Active => segment.tooltip(tip_detail(t, mode.label, mode.hint, None)),
            ModeState::Available(command) => on_command(
                segment.tooltip(tip_detail(t, mode.label, mode.hint, mode.shortcut)),
                command,
                cx,
            ),
            ModeState::Unavailable(reason) => disabled(segment)
                .cursor_default()
                .tooltip(tip_detail(t, mode.label, reason, None)),
        };
        segment
    }))
}

/// Opens the command palette. While the palette is open the field rests: the
/// palette's own input has the caret, and two focused inputs read as two
/// places to type.
fn search_field(t: Colors, compact: bool, s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    let open = s.palette_open && !s.palette_exports_only;
    if compact {
        return command_icon(
            t,
            "commands",
            Icon::Search,
            Command::TogglePalette,
            open,
            H_MD,
            cx,
        )
        .into_any_element();
    }
    on_command(
        row()
            .id("commands")
            .w(px(232.))
            .h(px(H_MD))
            .px(px(SP_2 + 2.))
            .gap(px(SP_2))
            .rounded(px(R_MD))
            .bg(rgb(t.field))
            .border_1()
            .border_color(rgb(if t.dark { t.hairline } else { t.border }))
            .cursor_pointer()
            .hover(move |d| d.bg(rgb(t.field_hover)).border_color(rgb(t.border)))
            .child(icon(Icon::Search, ICON_SM, t.text_faint))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_color(rgb(t.text_faint))
                    .child("Search commands"),
            )
            .child(keycaps(t, platform_shortcut("⌘ K", "Ctrl K")))
            .tooltip(command_tip(t, Command::TogglePalette)),
        Command::TogglePalette,
        cx,
    )
    .into_any_element()
}

/// One-click image export, plus a chevron that lists every export format.
/// With nothing in the scene there is nothing to export, so the pair steps
/// back from the primary accent to a neutral button.
fn export_controls(
    t: Colors,
    compact: bool,
    s: &Studio,
    cx: &mut Context<Studio>,
) -> impl IntoElement {
    let formats_open = s.palette_open && s.palette_exports_only;
    let primary = !s.scene.objects.is_empty();
    let (tone, ink, divider, open_fill) = if primary {
        (Tone::Primary, t.on_accent, t.accent_hover, t.accent_hover)
    } else {
        (Tone::Secondary, t.text_muted, t.border, t.control_active)
    };
    row()
        .flex_shrink_0()
        .rounded(px(R_MD))
        .overflow_hidden()
        .child(on_command(
            button_base(t, "export-image", tone, H_MD)
                .px(px(SP_2 + 2.))
                .rounded_r(px(0.))
                .when(!primary, |d| d.border_r_0().text_color(rgb(ink)))
                .font_weight(WEIGHT_MEDIUM)
                .child(icon(Icon::ExportImage, ICON_SM, ink))
                .when(!compact, |d| d.child("Export"))
                .tooltip(command_tip(t, Command::ExportImage)),
            Command::ExportImage,
            cx,
        ))
        .child(div().w(px(1.)).h(px(H_MD)).bg(rgb(divider)))
        .child(
            button_base(t, "export-formats", tone, H_MD)
                .w(px(24.))
                .rounded_l(px(0.))
                .when(!primary, |d| d.border_l_0())
                .when(formats_open, |d| d.bg(rgb(open_fill)))
                .child(icon(Icon::ChevronDown, ICON_SM, ink))
                .tooltip(tip_detail(
                    t,
                    "Export formats",
                    "PNG image, Wavefront OBJ, STL or STEP",
                    None,
                ))
                .on_click(cx.listener(|s, _, window, cx| {
                    cx.stop_propagation();
                    s.execute(Command::TogglePalette, window, cx);
                    if s.palette_open {
                        s.palette_exports_only = true;
                        s.palette_query.clear();
                        s.palette_index = 0;
                    }
                })),
        )
}

pub(super) fn titlebar(
    t: Colors,
    s: &Studio,
    window: &Window,
    cx: &mut Context<Studio>,
) -> AnyElement {
    let width = f32::from(window.viewport_size().width);
    let compact = width < 1240.;
    row()
        .h(px(TITLEBAR_HEIGHT))
        .flex_shrink_0()
        .px(px(SP_2))
        .bg(rgb(t.surface))
        .border_b_1()
        .border_color(rgb(t.hairline))
        // Left: identity. Equal flex on both sides keeps the switcher centered.
        .child(
            row()
                .flex_1()
                .flex_basis(px(0.))
                .min_w(px(0.))
                .h_full()
                .gap(px(SP_2))
                .when(cfg!(target_os = "macos"), |d| {
                    d.child(
                        drag_region("titlebar-traffic-lights")
                            .flex_none()
                            .w(px(TRAFFIC_LIGHT_RESERVE - SP_2)),
                    )
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(H_MD))
                        .flex_shrink_0()
                        .child(icon(Icon::Logo, ICON_LG, t.accent)),
                )
                .child(
                    row()
                        .id("document-title")
                        .min_w(px(0.))
                        .gap(px(SP_1_5))
                        .child(
                            div()
                                .min_w(px(0.))
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .text_size(px(TEXT_LG))
                                .font_weight(WEIGHT_MEDIUM)
                                .text_color(rgb(t.text))
                                .child(s.project_name.clone()),
                        )
                        .when(s.dirty, |d| d.child(dot(t.text_muted, 6.)))
                        .tooltip(tip(
                            t,
                            if s.dirty {
                                "Unsaved changes"
                            } else {
                                "All changes saved"
                            },
                            Some(platform_shortcut("⌘ S", "Ctrl+S")),
                        )),
                )
                .child(drag_region("titlebar-drag-left")),
        )
        .child(mode_switcher(t, s, cx))
        .child(
            row()
                .flex_1()
                .flex_basis(px(0.))
                .min_w(px(0.))
                .h_full()
                .gap(px(SP_1))
                .child(drag_region("titlebar-drag-right"))
                .child(search_field(t, compact, s, cx))
                .child(command_icon(
                    t,
                    "color-theme",
                    Icon::Theme,
                    Command::ToggleTheme,
                    s.theme_picker.is_some(),
                    H_MD,
                    cx,
                ))
                .child(div().mx(px(SP_1)).child(vrule(t, 16.)))
                .child(command_icon(
                    t,
                    "open",
                    Icon::Open,
                    Command::Open,
                    false,
                    H_MD,
                    cx,
                ))
                .child(command_icon(
                    t,
                    "save",
                    Icon::Save,
                    Command::Save,
                    false,
                    H_MD,
                    cx,
                ))
                .child(div().w(px(SP_1)))
                .child(export_controls(t, compact, s, cx)),
        )
        .into_any_element()
}
