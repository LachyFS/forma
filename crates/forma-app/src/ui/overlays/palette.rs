//! Command palette: search every editor command, grouped by area, with
//! recent commands first and shortcuts shown as keycaps.
use super::super::assets::{Icon, icon};
use super::super::commands::{command_group, command_icon};
use super::super::components::*;
use super::super::tokens::*;
use super::{animate_in, backdrop, contain};
use crate::app::{Command, Studio};
use crate::theme::Colors;
use gpui::{prelude::*, *};

/// Rows visible at once; the window follows the keyboard selection.
const VISIBLE: usize = 10;
const ROW: f32 = 32.;

pub(super) fn overlay(
    t: Colors,
    s: &Studio,
    window: &Window,
    cx: &mut Context<Studio>,
) -> AnyElement {
    let commands = s.palette_items();
    let recent = s.palette_recent_count();
    let selected = s.palette_index.min(commands.len().saturating_sub(1));
    let start = selected.saturating_sub(VISIBLE - 1);
    let top = (f32::from(window.viewport_size().height) * 0.12).clamp(56., 140.);
    let exports = s.palette_exports_only;

    let mut list = col().p(px(SP_1_5)).gap(px(1.));
    let mut previous_group = None;
    for (index, (label, shortcut, command)) in commands
        .iter()
        .copied()
        .enumerate()
        .skip(start)
        .take(VISIBLE)
    {
        let group = if exports {
            "Export"
        } else if index < recent {
            "Recent"
        } else {
            command_group(command)
        };
        if previous_group != Some(group) {
            list = list
                .child(group_label(t, group).when(previous_group.is_some(), |d| d.mt(px(SP_1))));
            previous_group = Some(group);
        }
        let active = index == selected;
        list = list.child(
            row()
                .id(("palette-command", index))
                .h(px(ROW))
                .flex_shrink_0()
                .px(px(SP_2 + 2.))
                .gap(px(SP_2 + 2.))
                .rounded(px(R_MD))
                .cursor_pointer()
                .bg(if active { hex(t.accent_soft) } else { clear() })
                .when(!active, |d| d.hover(move |d| d.bg(rgb(t.control_hover))))
                .child(match command_icon(command) {
                    Some(glyph) => icon(
                        glyph,
                        ICON_MD,
                        if active { t.accent_text } else { t.text_muted },
                    )
                    .into_any_element(),
                    None => div().w(px(ICON_MD)).into_any_element(),
                })
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_color(rgb(t.text))
                        .child(label),
                )
                .when(!shortcut.is_empty(), |d| d.child(keycaps(t, shortcut)))
                .on_click(cx.listener(move |s, _, w, cx| {
                    s.palette_open = false;
                    s.panels.remember_command(command);
                    s.execute(command, w, cx);
                })),
        );
    }
    if commands.is_empty() {
        list = list.child(
            col()
                .items_center()
                .py(px(SP_6))
                .gap(px(SP_1_5))
                .child(icon(Icon::Search, ICON_LG, t.text_faint))
                .child(div().text_color(rgb(t.text)).child(if exports {
                    "No matching export formats"
                } else {
                    "No matching commands"
                }))
                .child(caption(t, "Try a shorter or different word")),
        );
    }
    let query = s.palette_query.clone();
    let search = row()
        .h(px(48.))
        .flex_shrink_0()
        .px(px(SP_4))
        .gap(px(SP_2 + 2.))
        .border_b_1()
        .border_color(rgb(t.hairline))
        .child(icon(
            if exports { Icon::Export } else { Icon::Search },
            ICON_LG,
            t.text_muted,
        ))
        .child(
            row()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(px(TEXT_XL))
                .when(query.is_empty(), |d| {
                    d.child(div().w(px(1.5)).h(px(18.)).mr(px(1.)).bg(rgb(t.focus)))
                        .child(div().text_color(rgb(t.text_faint)).child(if exports {
                            "Export as…"
                        } else {
                            "Search commands"
                        }))
                })
                .when(!query.is_empty(), |d| {
                    d.child(div().text_color(rgb(t.text)).child(query))
                        .child(div().w(px(1.5)).h(px(18.)).ml(px(1.)).bg(rgb(t.focus)))
                }),
        )
        .child(
            caption(
                t,
                format!(
                    "{} {}",
                    commands.len(),
                    if commands.len() == 1 {
                        "result"
                    } else {
                        "results"
                    }
                ),
            )
            .tabular()
            .text_color(rgb(t.text_faint)),
        );
    let footer = row()
        .h(px(36.))
        .flex_shrink_0()
        .px(px(SP_4))
        .gap(px(SP_4))
        .border_t_1()
        .border_color(rgb(t.hairline))
        .bg(rgb(t.raised))
        .child(hint(t, "↑ ↓", "Navigate"))
        .child(hint(t, "Return", "Run"))
        .child(spacer())
        .child(hint(t, "Esc", "Close"));
    backdrop(
        t,
        "command-palette-backdrop",
        true,
        |s, w, cx| s.execute(Command::TogglePalette, w, cx),
        cx,
    )
    .flex()
    .flex_col()
    .items_center()
    .pt(px(top))
    .child(animate_in(
        contain(
            dialog(t)
                .id("command-menu")
                .w(px(580.))
                .max_w(relative(0.9))
                .child(search)
                .child(list)
                .child(footer),
        ),
        "command-menu-enter",
    ))
    .into_any_element()
}
