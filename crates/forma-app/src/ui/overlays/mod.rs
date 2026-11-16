//! Everything that floats above the workspace: dropdown menus, the command
//! palette, dialogs and popovers. Each overlay has a backdrop that consumes
//! pointer input, so a dismissing click never selects in the viewport.
pub(crate) mod menu;
pub(crate) mod palette;
mod pie;
mod preview;
mod reference;
mod shader;
mod stl;
mod theme_picker;
mod updates;

use super::tokens::*;
use crate::app::Studio;
use crate::theme::Colors;
use gpui::{prelude::*, *};

/// Full-window backdrop. `dismiss` runs on a left or right press outside the
/// overlay's content; every other pointer event is swallowed.
pub(super) fn backdrop(
    t: Colors,
    id: &'static str,
    scrim: bool,
    dismiss: impl Fn(&mut Studio, &mut Window, &mut Context<Studio>) + 'static,
    cx: &mut Context<Studio>,
) -> Stateful<Div> {
    let dismiss = std::rc::Rc::new(dismiss);
    let right = dismiss.clone();
    div()
        .id(id)
        .absolute()
        .inset_0()
        .occlude()
        .when(scrim, |d| d.bg(scrim_color(t)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |s, _, w, cx| {
                cx.stop_propagation();
                dismiss(s, w, cx);
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |s, _, w, cx| {
                cx.stop_propagation();
                right(s, w, cx);
            }),
        )
        .on_mouse_down(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
}

/// The command palette's backdrop: deeper than a dialog's, so the title bar
/// and panels recede with the viewport instead of competing with the one
/// card that holds the keyboard.
pub(super) fn deep_scrim(t: Colors) -> Hsla {
    let mut scrim = hex_alpha(t.scrim);
    scrim.a = if t.dark { 0.58 } else { 0.34 };
    scrim
}

/// Modal backdrop tint. Dark themes cap it at 40% so the scene stays
/// readable behind a dialog; separation comes from the card's shadow and
/// border instead.
fn scrim_color(t: Colors) -> Hsla {
    let mut scrim = hex_alpha(t.scrim);
    scrim.a = scrim.a.min(0.4);
    scrim
}

/// Outline of floating cards: a faint light edge on dark themes, where a
/// dark shadow alone disappears against the scene.
pub(super) fn float_border(t: Colors) -> Hsla {
    if t.dark {
        tint(0xffffff, 0.08)
    } else {
        hex(t.border)
    }
}

/// Soft shadow under the palette, menus and dialogs.
pub(super) fn float_shadow(t: Colors) -> Vec<BoxShadow> {
    let deep = hex_alpha(t.shadow);
    let mut near = deep;
    near.a *= 0.5;
    vec![
        BoxShadow {
            color: deep,
            offset: point(px(0.), px(12.)),
            blur_radius: px(24.),
            spread_radius: px(-2.),
        },
        BoxShadow {
            color: near,
            offset: point(px(0.), px(1.)),
            blur_radius: px(3.),
            spread_radius: px(0.),
        },
    ]
}

/// Fill of the keyboard-selected row in lists: a neutral tint, so the accent
/// is left for the 2px bar beside it and for matched text.
pub(super) fn selected_fill(t: Colors) -> Hsla {
    tint(t.text, if t.dark { 0.075 } else { 0.06 })
}

/// Fill of a row under the pointer that is not selected.
pub(super) fn hover_fill(t: Colors) -> Hsla {
    tint(t.text, if t.dark { 0.04 } else { 0.035 })
}

/// The 2px accent bar at the left edge of a selected row.
pub(super) fn selection_bar(color: u32, inset: f32) -> Div {
    div()
        .absolute()
        .left_0()
        .top(px(inset))
        .bottom(px(inset))
        .w(px(2.))
        .rounded_full()
        .bg(rgb(color))
}

/// Section heading inside menus and the palette.
/// Stronger than the panels' quiet group label: in a floating list the
/// headings are what the eye scans first.
pub(super) fn heading(t: Colors, text: impl Into<SharedString>) -> Div {
    super::components::group_label(t, text)
        .flex_shrink_0()
        .font_weight(WEIGHT_SEMIBOLD)
        .text_color(rgb(t.text_muted))
}

/// Content inside a backdrop: presses stay inside instead of dismissing.
pub(super) fn contain<E: InteractiveElement>(element: E) -> E {
    element
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
}

/// Fade and settle a floating card as it opens.
pub(super) fn animate_in(
    element: Stateful<Div>,
    id: &'static str,
) -> AnimationElement<Stateful<Div>> {
    element.with_animation(id, enter(), |element, delta| {
        element.opacity(delta).mt(px((1. - delta) * 6.))
    })
}

/// A small, fast reveal for menus and popovers.
pub(super) fn pop_in(
    element: Stateful<Div>,
    id: impl Into<ElementId>,
) -> AnimationElement<Stateful<Div>> {
    element.with_animation(id, enter_fast(), |element, delta| {
        element.opacity(delta).mt(px((1. - delta) * -3.))
    })
}

pub(super) fn overlays(
    t: Colors,
    s: &Studio,
    window: &Window,
    cx: &mut Context<Studio>,
) -> Vec<AnyElement> {
    let mut layers = Vec::new();
    if !s.palette_open {
        palette::rest();
    }
    if s.preview_open {
        layers.push(preview::overlay(t, s, cx));
    }
    if s.palette_open {
        layers.push(palette::overlay(t, s, window, cx));
    }
    if s.theme_picker.is_some() {
        layers.push(theme_picker::overlay(t, s, cx));
    }
    if s.help_open {
        layers.push(reference::overlay(t, cx));
    }
    if s.shader_editor.is_some() {
        layers.push(shader::overlay(t, s, cx));
    }
    if s.shading_pie.is_some() {
        layers.push(pie::overlay(t, s, cx));
    }
    if s.stl_dialog.is_some() {
        layers.push(stl::overlay(t, s, cx));
    }
    if s.panels.menu.is_some() {
        layers.push(menu::overlay(t, s, cx));
    }
    if let Some(menu) = super::timeline::context_menu(t, s, cx) {
        layers.push(menu);
    }
    if s.updates.open || s.updates.restart_pending {
        layers.push(updates::overlay(t, s, cx));
    }
    layers
}
