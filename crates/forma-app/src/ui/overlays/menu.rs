//! Dropdown menus (Add, Views): anchored to their trigger, keyboard
//! navigable, and dismissed by Escape or any press outside.
use super::super::assets::{Icon, icon};
use super::super::commands::{command_icon, command_info};
use super::super::components::*;
use super::super::tokens::*;
use super::super::{Anchor, MenuKind, hud::view_name};
use super::{backdrop, contain, pop_in};
use crate::app::{Command, Studio};
use crate::theme::Colors;
use forma_core::Primitive;
use gpui::{prelude::*, *};

const MENU_WIDTH: f32 = 232.;

#[derive(Clone, Copy)]
pub(crate) enum MenuItem {
    Heading(&'static str),
    Action {
        command: Command,
        label: &'static str,
        checked: bool,
    },
    Separator,
}

impl MenuItem {
    pub(crate) fn command(&self) -> Option<Command> {
        match self {
            Self::Action { command, .. } => Some(*command),
            _ => None,
        }
    }
}

fn action(command: Command, label: &'static str) -> MenuItem {
    MenuItem::Action {
        command,
        label,
        checked: false,
    }
}

pub(crate) fn items(kind: MenuKind, camera: forma_core::Camera) -> Vec<MenuItem> {
    match kind {
        MenuKind::Add => vec![
            MenuItem::Heading("Mesh"),
            action(Command::Add(Primitive::Cube), "Cube"),
            action(Command::Add(Primitive::Sphere), "Sphere"),
            action(Command::Add(Primitive::Cylinder), "Cylinder"),
            action(Command::Add(Primitive::Torus), "Torus"),
            action(Command::Add(Primitive::Plane), "Plane"),
            MenuItem::Separator,
            MenuItem::Heading("Import"),
            action(Command::ImportObj, "Wavefront OBJ…"),
            action(Command::ImportStl, "STL…"),
        ],
        MenuKind::View => {
            let current = view_name(camera);
            let view = |command, label: &'static str| MenuItem::Action {
                command,
                label,
                checked: current == label,
            };
            vec![
                view(Command::ViewFront, "Front"),
                view(Command::ViewBack, "Back"),
                view(Command::ViewRight, "Right"),
                view(Command::ViewLeft, "Left"),
                view(Command::ViewTop, "Top"),
                view(Command::ViewBottom, "Bottom"),
                MenuItem::Separator,
                MenuItem::Action {
                    command: Command::ToggleProjection,
                    label: "Orthographic",
                    checked: camera.orthographic,
                },
                action(Command::ViewPerspective, "Reset perspective"),
                MenuItem::Separator,
                action(Command::FrameSelected, "Frame selection"),
                action(Command::FrameAll, "Frame all"),
            ]
        }
    }
}

pub(crate) fn first_item(kind: MenuKind) -> usize {
    items(kind, forma_core::Camera::default())
        .iter()
        .position(|item| item.command().is_some())
        .unwrap_or(0)
}

/// Next actionable item in `direction`, wrapping and skipping headings.
pub(crate) fn step(items: &[MenuItem], index: usize, direction: isize) -> usize {
    let count = items.len() as isize;
    if count == 0 {
        return 0;
    }
    let mut next = index as isize;
    for _ in 0..count {
        next = (next + direction).rem_euclid(count);
        if items[next as usize].command().is_some() {
            return next as usize;
        }
    }
    index
}

pub(super) fn overlay(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    let menu = s.panels.menu.as_ref().unwrap();
    let anchor = menu.anchor;
    let selected = menu.index;
    let kind = menu.kind;
    let items = items(kind, s.scene.camera);
    // Rail menus open beside the rail; others drop below their trigger.
    let (position, corner) = match menu.from {
        Anchor::ToolRail => (
            point(anchor.right() + px(SP_2 + 2.), anchor.top() - px(SP_1)),
            Corner::TopLeft,
        ),
        Anchor::Outliner | Anchor::ViewHud => (
            point(anchor.right(), anchor.bottom() + px(SP_1_5)),
            Corner::TopRight,
        ),
    };
    let mut list = popover(t).w(px(MENU_WIDTH)).gap(px(1.));
    for (index, item) in items.into_iter().enumerate() {
        list = match item {
            MenuItem::Heading(label) => list.child(group_label(t, label)),
            MenuItem::Separator => list.child(
                div()
                    .my(px(SP_1))
                    .mx(px(SP_1))
                    .h(px(1.))
                    .bg(rgb(t.hairline)),
            ),
            MenuItem::Action {
                command,
                label,
                checked,
            } => {
                let active = index == selected;
                let info = command_info(command);
                list.child(
                    row()
                        .id(("menu-item", index))
                        .h(px(H_MD))
                        .px(px(SP_2))
                        .gap(px(SP_2))
                        .rounded(px(R_SM + 1.))
                        .cursor_pointer()
                        .bg(if active {
                            hex(t.control_hover)
                        } else {
                            clear()
                        })
                        .child(match command_icon(command) {
                            Some(glyph) => {
                                icon(glyph, ICON_MD, if active { t.text } else { t.text_muted })
                                    .into_any_element()
                            }
                            None => div().w(px(ICON_MD)).into_any_element(),
                        })
                        .child(div().flex_1().text_color(rgb(t.text)).child(label))
                        .when(checked, |d| {
                            d.child(icon(Icon::Check, ICON_SM, t.accent_text))
                        })
                        .when_some(info.shortcut, |d, shortcut| d.child(keycaps(t, shortcut)))
                        .on_hover(cx.listener(move |s, hovered, _, cx| {
                            if *hovered
                                && let Some(menu) = s.panels.menu.as_mut()
                                && menu.index != index
                            {
                                menu.index = index;
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |s, _, w, cx| {
                            cx.stop_propagation();
                            s.panels.close_menu();
                            s.execute(command, w, cx);
                        })),
                )
            }
        };
    }
    backdrop(
        t,
        "menu-backdrop",
        false,
        |s, _, cx| {
            s.panels.close_menu();
            cx.notify();
        },
        cx,
    )
    .child(
        anchored()
            .position(position)
            .anchor(corner)
            .snap_to_window_with_margin(px(SP_2))
            .child(pop_in(contain(list.id("menu")), ("menu", kind as usize))),
    )
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{MenuKind, first_item, items, step};
    use crate::app::Command;
    use forma_core::Primitive;

    #[test]
    fn keyboard_steps_skip_headings_and_wrap() {
        let items = items(MenuKind::Add, forma_core::Camera::default());
        let first = first_item(MenuKind::Add);
        assert_eq!(items[first].command(), Some(Command::Add(Primitive::Cube)));
        let last = step(&items, first, -1);
        assert_eq!(items[last].command(), Some(Command::ImportStl));
        assert_eq!(step(&items, last, 1), first);
        let plane = items
            .iter()
            .position(|item| item.command() == Some(Command::Add(Primitive::Plane)))
            .unwrap();
        assert_eq!(
            items[step(&items, plane, 1)].command(),
            Some(Command::ImportObj)
        );
    }
}
