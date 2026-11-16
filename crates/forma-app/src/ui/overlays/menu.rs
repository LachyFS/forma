//! Dropdown menus (Add, Views): anchored to their trigger, keyboard
//! navigable, and dismissed by Escape or any press outside.
use super::super::assets::{Icon, icon};
use super::super::commands::{MESH_DETAIL, PART_DETAIL, SKETCH_DETAIL, command_icon, command_info};
use super::super::components::*;
use super::super::tokens::*;
use super::super::{Anchor, MenuKind, hud::view_name};
use super::{backdrop, contain, float_border, float_shadow, heading, pop_in, selected_fill};
use crate::app::{Command, Studio};
use crate::cad::{CadCommand, PartShape};
use crate::sketch::DrawCommand;
use crate::theme::Colors;
use forma_core::Primitive;
use gpui::{prelude::*, *};

const ADD_WIDTH: f32 = 264.;
const VIEW_WIDTH: f32 = 224.;
/// Space between a menu and the edge of the control that opened it.
const MENU_GAP: f32 = SP_2;

#[derive(Clone, Copy)]
pub(crate) enum MenuItem {
    /// A section title and a short note on what the section makes.
    Heading(&'static str, Option<&'static str>),
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
            MenuItem::Heading("Mesh", Some(MESH_DETAIL)),
            action(Command::Add(Primitive::Cube), "Cube"),
            action(Command::Add(Primitive::Sphere), "Sphere"),
            action(Command::Add(Primitive::Cylinder), "Cylinder"),
            action(Command::Add(Primitive::Torus), "Torus"),
            action(Command::Add(Primitive::Plane), "Plane"),
            MenuItem::Separator,
            MenuItem::Heading("Part", Some(PART_DETAIL)),
            action(Command::Cad(CadCommand::AddPart(PartShape::Box)), "Box"),
            action(
                Command::Cad(CadCommand::AddPart(PartShape::Cylinder)),
                "Cylinder",
            ),
            action(
                Command::Cad(CadCommand::AddPart(PartShape::Sphere)),
                "Sphere",
            ),
            MenuItem::Separator,
            MenuItem::Heading("Sketch", Some(SKETCH_DETAIL)),
            action(Command::Draw(DrawCommand::NewSketch), "New sketch"),
            MenuItem::Separator,
            MenuItem::Heading("Import", None),
            action(Command::ImportObj, "Wavefront OBJ…"),
            action(Command::ImportStl, "STL…"),
            action(Command::Cad(CadCommand::ImportStep), "STEP…"),
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

fn is_named_view(command: Command) -> bool {
    matches!(
        command,
        Command::ViewFront
            | Command::ViewBack
            | Command::ViewRight
            | Command::ViewLeft
            | Command::ViewTop
            | Command::ViewBottom
    )
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
    // Rail menus open beside the rail, level with their button; others drop
    // below their trigger. `RAIL_EDGE` is the rail's padding and border
    // around the button, so the gap is measured from the rail itself.
    const RAIL_EDGE: f32 = 4.;
    let (position, corner) = match menu.from {
        Anchor::ToolRail => (
            point(anchor.right() + px(RAIL_EDGE + MENU_GAP), anchor.top()),
            Corner::TopLeft,
        ),
        Anchor::Outliner | Anchor::ViewHud => (
            point(anchor.right(), anchor.bottom() + px(MENU_GAP)),
            Corner::TopRight,
        ),
    };
    let mut list = popover(t)
        .w(px(if kind == MenuKind::Add {
            ADD_WIDTH
        } else {
            VIEW_WIDTH
        }))
        .border_color(float_border(t))
        .shadow(float_shadow(t))
        .gap(px(1.));
    for (index, item) in items.into_iter().enumerate() {
        list = match item {
            MenuItem::Heading(label, detail) => list.child(
                heading(t, label)
                    .h(px(H_MD - 2.))
                    .gap(px(SP_2))
                    .child(spacer())
                    .when_some(detail, |d, detail| {
                        d.child(
                            div()
                                .font_weight(FontWeight::NORMAL)
                                .text_color(rgb(t.text_faint))
                                .child(detail),
                        )
                    }),
            ),
            MenuItem::Separator => list.child(
                div()
                    .my(px(SP_1))
                    .mx(px(SP_2))
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
                        .gap(px(SP_2 + 2.))
                        .rounded(px(R_SM + 1.))
                        .cursor_pointer()
                        .bg(if active { selected_fill(t) } else { clear() })
                        // Named views share one glyph; repeating it adds noise.
                        .child(
                            match command_icon(command)
                                .filter(|_| kind != MenuKind::View || !is_named_view(command))
                            {
                                Some(glyph) => {
                                    icon(glyph, ICON_MD, if active { t.text } else { t.text_muted })
                                        .into_any_element()
                                }
                                None => div().w(px(ICON_MD)).flex_shrink_0().into_any_element(),
                            },
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .text_size(px(TEXT_LG))
                                .text_color(rgb(t.text))
                                .child(label),
                        )
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
    use crate::cad::{CadCommand, PartShape};
    use forma_core::Primitive;

    #[test]
    fn keyboard_steps_skip_headings_and_wrap() {
        let items = items(MenuKind::Add, forma_core::Camera::default());
        let first = first_item(MenuKind::Add);
        assert_eq!(items[first].command(), Some(Command::Add(Primitive::Cube)));
        let last = step(&items, first, -1);
        assert_eq!(
            items[last].command(),
            Some(Command::Cad(CadCommand::ImportStep))
        );
        assert_eq!(step(&items, last, 1), first);
        let plane = items
            .iter()
            .position(|item| item.command() == Some(Command::Add(Primitive::Plane)))
            .unwrap();
        assert_eq!(
            items[step(&items, plane, 1)].command(),
            Some(Command::Cad(CadCommand::AddPart(PartShape::Box)))
        );
    }
}
