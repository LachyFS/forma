//! Everything that floats above the workspace: dropdown menus, the command
//! palette, dialogs and popovers. Each overlay has a backdrop that consumes
//! pointer input, so a dismissing click never selects in the viewport.
pub(crate) mod menu;
mod palette;
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
        .when(scrim, |d| d.bg(rgba(t.scrim)))
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
