//! Color theme picker: search, preview on hover or arrows, commit on click
//! or Return, cancel on Escape or outside click.
use super::super::assets::{Icon, icon};
use super::super::components::*;
use super::super::tokens::*;
use super::{
    animate_in, backdrop, contain, float_border, float_shadow, hover_fill, selected_fill,
    selection_bar,
};
use crate::app::Studio;
use crate::theme::{Colors, Theme};
use gpui::{prelude::*, *};

/// A miniature workspace drawn in the theme's own tokens.
fn swatch(theme: Theme) -> Div {
    let t = theme.colors();
    col()
        .w(px(84.))
        .h(px(52.))
        .flex_shrink_0()
        .rounded(px(R_MD))
        .border_1()
        .border_color(rgb(t.border))
        .bg(rgb(t.surface))
        .overflow_hidden()
        .child(
            row()
                .h(px(8.))
                .px(px(4.))
                .gap(px(2.))
                .border_b_1()
                .border_color(rgb(t.hairline))
                .child(div().size(px(3.)).rounded_full().bg(rgb(t.accent)))
                .child(
                    div()
                        .w(px(14.))
                        .h(px(2.))
                        .rounded_full()
                        .bg(rgb(t.text_faint)),
                ),
        )
        .child(
            row()
                .flex_1()
                .items_stretch()
                .child(
                    div()
                        .flex_1()
                        .bg(rgb(t.sunken))
                        .relative()
                        .child(
                            div()
                                .absolute()
                                .left(px(4.))
                                .top(px(4.))
                                .w(px(6.))
                                .h(px(18.))
                                .rounded(px(2.))
                                .bg(rgba(t.hud)),
                        )
                        .child(div().absolute().left(px(18.)).top(px(10.)).child(icon(
                            Icon::Logo,
                            20.,
                            t.accent,
                        ))),
                )
                .child(
                    col()
                        .w(px(28.))
                        .p(px(3.))
                        .gap(px(3.))
                        .border_l_1()
                        .border_color(rgb(t.hairline))
                        .child(
                            div()
                                .w_full()
                                .h(px(4.))
                                .rounded(px(1.))
                                .bg(rgb(t.accent_soft)),
                        )
                        .child(div().w_full().h(px(3.)).rounded(px(1.)).bg(rgb(t.field)))
                        .child(div().w(px(12.)).h(px(3.)).rounded(px(1.)).bg(rgb(t.field))),
                ),
        )
}

pub(super) fn overlay(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    let picker = s.theme_picker.as_ref().unwrap();
    let themes = picker.matches();
    let selected = picker.index;
    let query = picker.query.clone();
    let list = col()
        .id("theme-list")
        .min_h(px(0.))
        .overflow_y_scroll()
        .p(px(SP_1_5))
        .gap(px(2.))
        .when(themes.is_empty(), |d| {
            d.child(
                col()
                    .items_center()
                    .py(px(SP_6))
                    .gap(px(SP_1_5))
                    .child(icon(Icon::Theme, ICON_LG, t.text_faint))
                    .child(div().text_color(rgb(t.text)).child("No matching themes"))
                    .child(caption(t, "Try light, dark, pink or green")),
            )
        })
        .children(themes.into_iter().enumerate().map(|(index, theme)| {
            let active = index == selected;
            row()
                .id(SharedString::from(format!("theme-{}", theme.id())))
                .relative()
                .h(px(68.))
                .flex_shrink_0()
                .px(px(SP_2 + 2.))
                .gap(px(SP_3 + 2.))
                .rounded(px(R_MD + 1.))
                .bg(if active { selected_fill(t) } else { clear() })
                .when(!active, |d| d.hover(move |d| d.bg(hover_fill(t))))
                .when(active, |d| d.child(selection_bar(t.accent, 18.)))
                .cursor_pointer()
                .child(swatch(theme))
                .child(
                    col()
                        .flex_1()
                        .min_w(px(0.))
                        .gap(px(3.))
                        .child(
                            div()
                                .text_size(px(TEXT_LG))
                                .font_weight(WEIGHT_MEDIUM)
                                .text_color(rgb(t.text))
                                .child(theme.name()),
                        )
                        .child(caption(t, theme.description())),
                )
                .when(theme == s.theme, |d| {
                    d.child(badge(t, "Current", BadgeTone::Accent))
                })
                .on_hover(cx.listener(move |s, hovered, _, cx| {
                    if *hovered
                        && let Some(picker) = &mut s.theme_picker
                        && picker.index != index
                    {
                        picker.index = index;
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |s, _, _, cx| s.apply_theme(theme, cx)))
        }));
    let count = picker.matches().len();
    backdrop(t, "theme-overlay", true, |s, _, cx| s.cancel_theme(cx), cx)
        .flex()
        .flex_col()
        .items_center()
        .pt(px(64.))
        .child(animate_in(
            contain(
                dialog(t)
                    .id("theme-picker")
                    .w(px(520.))
                    .max_w(relative(0.92))
                    .border_color(float_border(t))
                    .shadow(float_shadow(t))
                    .max_h(relative(0.86))
                    .child(dialog_header(t, Some(Icon::Theme), "Color theme", None))
                    .child(
                        row()
                            .flex_shrink_0()
                            .mx(px(SP_4))
                            .mb(px(SP_2))
                            .px(px(SP_2 + 2.))
                            .h(px(H_LG))
                            .gap(px(SP_2))
                            .rounded(px(R_MD))
                            .bg(rgb(t.field))
                            .border_1()
                            .border_color(rgb(t.focus))
                            .child(icon(Icon::Search, ICON_SM, t.text_faint))
                            .child(
                                row()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .when(query.is_empty(), |d| {
                                        d.child(
                                            div().w(px(1.5)).h(px(15.)).mr(px(1.)).bg(rgb(t.focus)),
                                        )
                                        .child(
                                            div()
                                                .text_color(rgb(t.text_faint))
                                                .child("Search themes"),
                                        )
                                    })
                                    .when(!query.is_empty(), |d| {
                                        d.child(query).child(
                                            div().w(px(1.5)).h(px(15.)).ml(px(1.)).bg(rgb(t.focus)),
                                        )
                                    }),
                            )
                            .child(
                                caption(
                                    t,
                                    format!(
                                        "{count} {}",
                                        if count == 1 { "theme" } else { "themes" }
                                    ),
                                )
                                .text_color(rgb(t.text_faint)),
                            ),
                    )
                    .child(list)
                    .child(
                        dialog_footer(t)
                            .gap(px(SP_4))
                            .child(hint(t, "↑ ↓", "Preview"))
                            .child(hint(t, "Return", "Keep"))
                            .child(spacer())
                            .child(hint(t, "Esc", "Cancel")),
                    ),
            ),
            "theme-picker-enter",
        ))
        .into_any_element()
}
