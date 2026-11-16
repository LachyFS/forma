//! Dropdown menus (Add, Views): anchored to their trigger, keyboard
//! navigable, and dismissed by Escape or any press outside. Typing while the
//! Add menu is open filters it.
use super::super::assets::{Icon, icon};
use super::super::commands::{
    MESH_DETAIL, PART_DETAIL, SKETCH_DETAIL, command_icon, command_info, command_keywords,
};
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
use std::ops::Range;

const ADD_WIDTH: f32 = 280.;
const VIEW_WIDTH: f32 = 224.;
/// Space between a menu and the edge of the control that opened it.
const MENU_GAP: f32 = SP_2;
/// Longest filter the Add menu accepts; anything longer matches nothing.
const QUERY_MAX: usize = 40;

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
            MenuItem::Heading("Import", Some("From a file")),
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

fn query_words(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect()
}

/// Whether every query word occurs in the item's label, its section, or the
/// words its command answers to ("part cyl" finds the cylinder part).
fn item_matches(label: &str, section: &str, command: Command, words: &[String]) -> bool {
    let haystack = format!(
        "{label} {section} {} {}",
        command_info(command).label,
        command_keywords(command)
    )
    .to_ascii_lowercase();
    words.iter().all(|word| haystack.contains(word.as_str()))
}

/// Byte ranges of `label` that a query word matched, for highlighting.
fn label_matches(label: &str, words: &[String]) -> Vec<Range<usize>> {
    let lower = label.to_ascii_lowercase();
    let mut ranges: Vec<Range<usize>> = words
        .iter()
        .filter_map(|word| {
            lower
                .find(word.as_str())
                .map(|start| start..start + word.len())
        })
        .collect();
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

/// The menu as drawn: with a query, only matching actions, under the
/// headings of the sections they belong to.
pub(crate) fn visible_items(
    kind: MenuKind,
    camera: forma_core::Camera,
    query: &str,
) -> Vec<MenuItem> {
    let all = items(kind, camera);
    let words = query_words(query);
    if words.is_empty() {
        return all;
    }
    let mut sections: Vec<(Option<MenuItem>, Vec<MenuItem>)> = vec![(None, Vec::new())];
    let mut section = "";
    for item in all {
        match item {
            MenuItem::Heading(title, _) => {
                section = title;
                sections.push((Some(item), Vec::new()));
            }
            MenuItem::Separator => {}
            MenuItem::Action { command, label, .. } => {
                if item_matches(label, section, command, &words) {
                    sections.last_mut().unwrap().1.push(item);
                }
            }
        }
    }
    let mut visible = Vec::new();
    for (heading, actions) in sections.into_iter().filter(|(_, a)| !a.is_empty()) {
        if !visible.is_empty() {
            visible.push(MenuItem::Separator);
        }
        visible.extend(heading);
        visible.extend(actions);
    }
    visible
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

fn first_action(items: &[MenuItem]) -> usize {
    items
        .iter()
        .position(|item| item.command().is_some())
        .unwrap_or(0)
}

pub(crate) fn first_item(kind: MenuKind) -> usize {
    first_action(&items(kind, forma_core::Camera::default()))
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

/// Keyboard handling for an open dropdown. Returns whether it consumed the
/// key; every key is consumed while a menu is open. In the Add menu, typing
/// filters, Backspace edits the filter and Escape clears it before closing.
pub(crate) fn handle_key(
    s: &mut Studio,
    event: &KeyDownEvent,
    window: &mut Window,
    cx: &mut Context<Studio>,
) -> bool {
    let Some(menu) = s.panels.menu.as_mut() else {
        return false;
    };
    let items = visible_items(menu.kind, s.scene.camera, &menu.query);
    let keystroke = &event.keystroke;
    let filterable = menu.kind == MenuKind::Add;
    let mut refilter = false;
    match keystroke.key.as_str() {
        "escape" if !menu.query.is_empty() => {
            menu.query.clear();
            refilter = true;
        }
        "escape" => s.panels.menu = None,
        "up" => menu.index = step(&items, menu.index, -1),
        "down" => menu.index = step(&items, menu.index, 1),
        "enter" => {
            if let Some(command) = items.get(menu.index).and_then(|item| item.command()) {
                s.panels.menu = None;
                s.execute(command, window, cx);
            }
        }
        "backspace" if filterable => {
            menu.query.pop();
            refilter = true;
        }
        _ => {
            if filterable
                && !keystroke.modifiers.platform
                && !keystroke.modifiers.control
                && let Some(text) = &keystroke.key_char
                && !text.chars().any(char::is_control)
                && menu.query.len() + text.len() <= QUERY_MAX
                && !(menu.query.is_empty() && text.trim().is_empty())
            {
                menu.query.push_str(text);
                refilter = true;
            }
        }
    }
    if refilter && let Some(menu) = s.panels.menu.as_mut() {
        menu.index = first_action(&visible_items(menu.kind, s.scene.camera, &menu.query));
    }
    cx.notify();
    true
}

/// The Add menu's filter line: what was typed, or how to start.
fn filter_field(t: Colors, query: &str) -> Div {
    let caret = || div().w(px(1.5)).h(px(15.)).bg(rgb(t.focus));
    row()
        .flex_shrink_0()
        .h(px(H_LG + 2.))
        .px(px(SP_3))
        .gap(px(SP_2))
        .border_b_1()
        .border_color(rgb(t.hairline))
        .child(icon(Icon::Search, ICON_SM, t.text_muted))
        .child(
            row()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(px(TEXT_LG))
                .when(query.is_empty(), |d| {
                    d.child(caret().mr(px(1.)))
                        .child(div().text_color(rgb(t.text_faint)).child("Type to filter"))
                })
                .when(!query.is_empty(), |d| {
                    d.child(
                        div()
                            .text_color(rgb(t.text))
                            .child(SharedString::from(query.to_owned())),
                    )
                    .child(caret().ml(px(1.)))
                }),
        )
}

/// Keys that drive the Add menu, in the shared footer style.
fn footer(t: Colors) -> Div {
    row()
        .flex_shrink_0()
        .h(px(H_MD + 2.))
        .px(px(SP_3))
        .gap(px(SP_3))
        .border_t_1()
        .border_color(rgb(t.hairline))
        .bg(rgb(t.raised))
        .child(hint(t, "↑ ↓", "Move"))
        .child(hint(t, "Return", "Add"))
        .child(spacer())
        .child(hint(t, "Esc", "Close"))
}

pub(super) fn overlay(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    let menu = s.panels.menu.as_ref().unwrap();
    let anchor = menu.anchor;
    let selected = menu.index;
    let kind = menu.kind;
    let add = kind == MenuKind::Add;
    let query = menu.query.clone();
    let words = query_words(&query);
    let items = visible_items(kind, s.scene.camera, &query);
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
    let mut list = col().p(px(SP_1)).gap(px(1.));
    if items.is_empty() {
        list = list.child(
            col()
                .items_center()
                .py(px(SP_4))
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(TEXT_LG))
                        .text_color(rgb(t.text))
                        .child(format!("No matches for \u{201c}{query}\u{201d}")),
                )
                .child(caption(t, "Backspace to edit, Esc to clear")),
        );
    }
    for (index, item) in items.into_iter().enumerate() {
        list = match item {
            // The section's name leads; its description is secondary but
            // still body text, so it keeps the muted ink.
            MenuItem::Heading(label, detail) => list.child(
                heading(t, label)
                    .h(px(H_MD - 2.))
                    .gap(px(SP_2))
                    .text_color(rgb(t.text))
                    .child(spacer())
                    .when_some(detail, |d, detail| {
                        d.child(
                            div()
                                .font_weight(FontWeight::NORMAL)
                                .text_color(rgb(t.text_muted))
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
                let highlight = HighlightStyle {
                    color: Some(hex(t.accent_text)),
                    font_weight: Some(WEIGHT_SEMIBOLD),
                    ..Default::default()
                };
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
                                .child(
                                    StyledText::new(label).with_highlights(
                                        label_matches(label, &words)
                                            .into_iter()
                                            .map(|range| (range, highlight)),
                                    ),
                                ),
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
    let card = popover(t)
        .p(px(0.))
        .w(px(if add { ADD_WIDTH } else { VIEW_WIDTH }))
        .border_color(float_border(t))
        .shadow(float_shadow(t))
        .when(add, |d| d.child(filter_field(t, &query)))
        .child(list)
        .when(add, |d| d.child(footer(t)));
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
            .child(pop_in(contain(card.id("menu")), ("menu", kind as usize))),
    )
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{MenuItem, MenuKind, first_item, items, label_matches, step, visible_items};
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

    #[test]
    fn typing_filters_by_label_and_section_and_keeps_headings() {
        let camera = forma_core::Camera::default();
        let commands = |query: &str| -> Vec<Command> {
            visible_items(MenuKind::Add, camera, query)
                .iter()
                .filter_map(MenuItem::command)
                .collect()
        };
        assert_eq!(
            commands("cyl"),
            [
                Command::Add(Primitive::Cylinder),
                Command::Cad(CadCommand::AddPart(PartShape::Cylinder)),
            ]
        );
        // A section name narrows to that section.
        assert_eq!(
            commands("part cyl"),
            [Command::Cad(CadCommand::AddPart(PartShape::Cylinder))]
        );
        assert!(commands("no such thing").is_empty());
        // Matches keep their section heading, one separator between sections.
        let shown = visible_items(MenuKind::Add, camera, "sphere");
        assert!(matches!(shown[0], MenuItem::Heading("Mesh", _)));
        assert!(matches!(shown[2], MenuItem::Separator));
        assert!(matches!(shown[3], MenuItem::Heading("Part", _)));
        assert_eq!(shown.len(), 5);
        // Matched words highlight separately; touching ones join.
        let ranges = label_matches("Cylinder", &["der".into(), "cy".into()]);
        assert_eq!(ranges, [0..2, 5..8]);
        let joined = label_matches("Cylinder", &["lin".into(), "cy".into()]);
        assert_eq!((joined.len(), joined[0].clone()), (1, 0..5));
    }
}
