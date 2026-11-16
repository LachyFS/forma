//! Reusable controls. Every component takes the semantic `Colors` and uses
//! the token scales, so a new panel looks native by construction.
use super::assets::{Icon, icon};
use super::commands::command_info;
use super::tokens::*;
use super::{ActiveFieldLayout, PanelSection, PanelState};
use crate::app::{Command, Field, Studio};
use crate::theme::Colors;
use gpui::{prelude::*, *};

pub(crate) fn row() -> Div {
    div().flex().flex_row().items_center()
}
pub(crate) fn col() -> Div {
    div().flex().flex_col()
}
pub(crate) fn spacer() -> Div {
    div().flex_1().min_w(px(0.))
}
/// Fully transparent fill for the resting state of ghost controls.
pub(crate) fn clear() -> Hsla {
    gpui::transparent_black()
}

/// Resting edge of an input. Light themes put fields barely darker than
/// their panel, so a 1 px border keeps values reading as fields; dark themes
/// separate them by fill alone.
pub(crate) fn field_edge(t: Colors) -> Hsla {
    if t.dark { clear() } else { hex(t.border) }
}

/// Thin vertical rule between groups in a toolbar.
pub(crate) fn vrule(t: Colors, height: f32) -> Div {
    div()
        .w(px(1.))
        .h(px(height))
        .flex_shrink_0()
        .bg(rgb(t.border))
}
/// Full-width hairline between regions.
pub(crate) fn hrule(t: Colors) -> Div {
    div().h(px(1.)).w_full().flex_shrink_0().bg(rgb(t.hairline))
}

/// Height of the shadow along a scrolled list's edge.
pub(crate) const SCROLL_SHADOW: f32 = 8.;

/// The shadow's ink at the edge it hugs, and the same ink fully clear.
pub(crate) fn scroll_shadow_inks(t: Colors) -> (Hsla, Hsla) {
    let mut ink = hex_alpha(t.shadow);
    ink.a = ink.a.min(if t.dark { 0.4 } else { 0.12 });
    let mut clear = ink;
    clear.a = 0.;
    (ink, clear)
}

/// A short shadow line along a scrolled list's edge while content continues
/// past it. It adds depth instead of a wash, so the row beside it keeps its
/// contrast and never reads as disabled. `bottom` puts it on the lower edge.
/// Position it absolutely; it takes no input.
pub(crate) fn scroll_shadow(t: Colors, bottom: bool) -> Div {
    let (ink, clear) = scroll_shadow_inks(t);
    div()
        .h(px(SCROLL_SHADOW))
        .flex_shrink_0()
        .bg(linear_gradient(
            if bottom { 180. } else { 0. },
            linear_color_stop(clear, 0.),
            linear_color_stop(ink, 1.),
        ))
}

/// Shorten `text` to fit `width` at the UI font and `size`, ending in "…".
/// GPUI 0.2 caches a no-wrap text's first, unconstrained measurement, so
/// `text_ellipsis` only works when the text's width is known up front;
/// text in content-sized rows (the status bar) is fitted here instead.
pub(crate) fn fit_text(cx: &App, text: SharedString, size: f32, width: f32) -> SharedString {
    let face = font(FONT_UI);
    let mut runs = vec![TextRun {
        len: text.len(),
        font: face.clone(),
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    cx.text_system()
        .line_wrapper(face, px(size))
        .truncate_line(text, px(width), "…", &mut runs)
}

/// Secondary text, for counts and context next to a primary label.
pub(crate) fn caption(t: Colors, text: impl Into<SharedString>) -> Div {
    div()
        .flex_shrink_0()
        .text_size(px(TEXT_SM))
        .text_color(rgb(t.text_muted))
        .child(text.into())
}

/// Quiet group label inside menus, popovers and the palette.
pub(crate) fn group_label(t: Colors, text: impl Into<SharedString>) -> Div {
    row()
        .h(px(H_SM))
        .px(px(SP_2))
        .text_size(px(TEXT_SM))
        .font_weight(WEIGHT_MEDIUM)
        .text_color(rgb(t.text_faint))
        .child(text.into())
}

// ---------------------------------------------------------------------------
// Keycaps and tooltips.

/// One physical key.
pub(crate) fn keycap(t: Colors, key: &str) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .min_w(px(17.))
        .h(px(17.))
        .px(px(4.))
        .flex_shrink_0()
        .rounded(px(R_XS + 1.))
        .bg(rgb(t.control))
        .border_1()
        .border_color(rgb(t.border))
        .border_b_2()
        .text_size(px(TEXT_XS))
        .font_weight(WEIGHT_MEDIUM)
        .text_color(rgb(t.text_muted))
        .child(SharedString::from(key.to_owned()))
}

fn keycap_token_is_text(token: &str) -> bool {
    token == "/"
        || token == "·"
        || (token.len() >= 2 && token.chars().all(|c| c.is_ascii_lowercase()))
}

/// A shortcut such as `"⇧ ⌘ Z"`, `"Ctrl+Shift+Z"`, `"Z then 4"` or
/// `"` / . / F"`. Keys become caps; connecting words stay plain text.
pub(crate) fn keycaps(t: Colors, shortcut: &str) -> Div {
    let mut keys = row().gap(px(3.)).flex_shrink_0();
    for token in shortcut.split_whitespace() {
        if keycap_token_is_text(token) {
            keys = keys.child(
                div()
                    .text_size(px(TEXT_SM))
                    .text_color(rgb(t.text_faint))
                    .child(SharedString::from(token.to_owned())),
            );
        } else if token.len() > 1 && token.contains('+') {
            for key in token.split('+').filter(|key| !key.is_empty()) {
                keys = keys.child(keycap(t, key));
            }
        } else {
            keys = keys.child(keycap(t, token));
        }
    }
    keys
}

/// Hover help: a label, optional explanation and the shortcut as keycaps.
pub(crate) struct Tooltip {
    pub label: SharedString,
    pub detail: Option<SharedString>,
    pub shortcut: Option<&'static str>,
    pub colors: Colors,
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = self.colors;
        col()
            .max_w(px(300.))
            .px(px(SP_2))
            .py(px(SP_1 + 1.))
            .gap(px(2.))
            .rounded(px(R_SM + 1.))
            .bg(rgb(t.overlay))
            .border_1()
            .border_color(rgb(t.border))
            .shadow(elevation(t, 1))
            .font_family(FONT_UI)
            .text_size(px(TEXT_SM))
            .text_color(rgb(t.text))
            .child(
                row()
                    .gap(px(SP_3))
                    .child(div().child(self.label.clone()))
                    .when_some(self.shortcut, |d, shortcut| {
                        d.child(spacer()).child(keycaps(t, shortcut))
                    }),
            )
            .when_some(self.detail.clone(), |d, detail| {
                d.child(
                    div()
                        .text_color(rgb(t.text_muted))
                        .line_height(px(15.))
                        .child(detail),
                )
            })
    }
}

/// Tooltip builder for `.tooltip(...)`.
pub(crate) fn tip(
    t: Colors,
    label: impl Into<SharedString>,
    shortcut: Option<&'static str>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let label = label.into();
    move |_, cx| {
        let label = label.clone();
        cx.new(|_| Tooltip {
            label,
            detail: None,
            shortcut,
            colors: t,
        })
        .into()
    }
}

/// Tooltip with a second explanatory line.
pub(crate) fn tip_detail(
    t: Colors,
    label: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    shortcut: Option<&'static str>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let label = label.into();
    let detail = detail.into();
    move |_, cx| {
        let (label, detail) = (label.clone(), detail.clone());
        cx.new(|_| Tooltip {
            label,
            detail: Some(detail),
            shortcut,
            colors: t,
        })
        .into()
    }
}

/// Tooltip for a command, from the shared command metadata.
pub(crate) fn command_tip(
    t: Colors,
    command: Command,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let info = command_info(command);
    move |_, cx| {
        cx.new(|_| Tooltip {
            label: info.label.into(),
            detail: info.detail.map(SharedString::from),
            shortcut: info.shortcut,
            colors: t,
        })
        .into()
    }
}

// ---------------------------------------------------------------------------
// Buttons.

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tone {
    /// Transparent until hovered. Toolbars, rows and HUDs.
    Ghost,
    /// Filled neutral. Secondary actions in panels and dialogs.
    Secondary,
    /// The one emphasised action in a region.
    Primary,
    /// The on state of a toggle or the current choice.
    Selected,
    /// Accent outline on no fill: a commit action in a region that defers
    /// to the screen's one filled primary elsewhere.
    Outline,
}

thread_local! {
    static TOOL_PANEL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Build a running tool's inspector panel. The floating tool bar is the one
/// place a tool commits, so the panel keeps its parameters but drops its own
/// Cancel and Done (Return and Esc still work), and any other primary button
/// in it draws as a quieter `Tone::Outline`.
pub(crate) fn in_tool_panel<R>(build: impl FnOnce() -> R) -> R {
    let before = TOOL_PANEL.with(|panel| panel.replace(true));
    let built = build();
    TOOL_PANEL.with(|panel| panel.set(before));
    built
}

fn in_tool_panel_region() -> bool {
    TOOL_PANEL.with(|panel| panel.get())
}

/// `tone` as this region draws it.
fn resolved(tone: Tone) -> Tone {
    if tone == Tone::Primary && in_tool_panel_region() {
        Tone::Outline
    } else {
        tone
    }
}

/// Run `command` on click, without letting the click reach the viewport.
pub(crate) fn on_command<E: StatefulInteractiveElement>(
    element: E,
    command: Command,
    cx: &mut Context<Studio>,
) -> E {
    element.on_click(cx.listener(move |s, _, window, cx| {
        cx.stop_propagation();
        s.execute(command, window, cx);
    }))
}

/// Unstyled-behaviour button base with tone, height and optional content.
pub(crate) fn button_base(
    t: Colors,
    id: impl Into<ElementId>,
    tone: Tone,
    height: f32,
) -> Stateful<Div> {
    let tone = resolved(tone);
    let (fill, ink, hover, pressed, border) = match tone {
        Tone::Ghost => (
            clear(),
            t.text_muted,
            hex(t.control_hover),
            hex(t.control_active),
            None,
        ),
        Tone::Secondary => (
            hex(t.control),
            t.text,
            hex(t.control_hover),
            hex(t.control_active),
            Some(t.border),
        ),
        Tone::Primary => (
            hex(t.accent),
            t.on_accent,
            hex(t.accent_hover),
            hex(t.accent_hover),
            None,
        ),
        Tone::Selected => (
            hex(t.accent_soft),
            t.accent_text,
            hex(t.accent_soft_hover),
            hex(t.accent_soft_hover),
            None,
        ),
        Tone::Outline => (
            clear(),
            t.accent_text,
            hex(t.accent_soft),
            hex(t.accent_soft_hover),
            Some(t.accent_border),
        ),
    };
    row()
        .id(id.into())
        .h(px(height))
        .flex_shrink_0()
        .justify_center()
        .gap(px(SP_1_5))
        .rounded(px(R_SM + 1.))
        .bg(fill)
        .when_some(border, |d, border| d.border_1().border_color(rgb(border)))
        .text_color(rgb(ink))
        .cursor_pointer()
        .hover(move |d| {
            let d = d.bg(hover);
            if tone == Tone::Ghost {
                d.text_color(rgb(t.text))
            } else {
                d
            }
        })
        .active(move |d| d.bg(pressed))
}

/// Text button, optionally with a leading icon.
pub(crate) fn button(
    t: Colors,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    glyph: Option<Icon>,
    tone: Tone,
) -> Stateful<Div> {
    let ink = match resolved(tone) {
        Tone::Ghost => t.text_muted,
        Tone::Secondary => t.text_muted,
        Tone::Primary => t.on_accent,
        Tone::Selected | Tone::Outline => t.accent_text,
    };
    button_base(t, id, tone, H_SM)
        .px(px(SP_2 + 1.))
        .text_size(px(TEXT_MD))
        .font_weight(WEIGHT_MEDIUM)
        .when_some(glyph, |d, glyph| d.child(icon(glyph, ICON_SM, ink)))
        .child(label.into())
}

/// Text button that runs a command and explains itself on hover.
pub(crate) fn command_button(
    t: Colors,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    glyph: Option<Icon>,
    command: Command,
    tone: Tone,
    cx: &mut Context<Studio>,
) -> Stateful<Div> {
    use crate::cad_tools::ToolCommand;
    let commit = matches!(
        command,
        Command::CadTool(ToolCommand::Done | ToolCommand::Cancel)
    );
    on_command(
        button(t, id, label, glyph, tone).tooltip(command_tip(t, command)),
        command,
        cx,
    )
    .when(commit && in_tool_panel_region(), |button| button.hidden())
}

/// Square icon-only button. `active` shows it as the current choice.
pub(crate) fn icon_button(
    t: Colors,
    id: impl Into<ElementId>,
    glyph: Icon,
    active: bool,
    side: f32,
) -> Stateful<Div> {
    let ink = if active { t.accent_text } else { t.text_muted };
    button_base(
        t,
        id,
        if active { Tone::Selected } else { Tone::Ghost },
        side,
    )
    .w(px(side))
    .child(icon(
        glyph,
        if side >= H_MD { ICON_MD } else { ICON_SM },
        ink,
    ))
}

/// Icon button that runs a command, with a tooltip and keycaps.
pub(crate) fn command_icon(
    t: Colors,
    id: impl Into<ElementId>,
    glyph: Icon,
    command: Command,
    active: bool,
    side: f32,
    cx: &mut Context<Studio>,
) -> Stateful<Div> {
    on_command(
        icon_button(t, id, glyph, active, side).tooltip(command_tip(t, command)),
        command,
        cx,
    )
}

/// Disabled look for controls that are visible but not yet available.
pub(crate) fn disabled<E: Styled>(element: E) -> E {
    element.opacity(0.42)
}

// ---------------------------------------------------------------------------
// Segmented control, toggle, badge.

/// Track for mutually exclusive segments.
pub(crate) fn segmented(t: Colors) -> Div {
    row()
        .p(px(2.))
        .gap(px(2.))
        .flex_shrink_0()
        .rounded(px(R_SM + 2.))
        .bg(rgb(t.sunken))
        .border_1()
        .border_color(rgb(if t.dark { t.hairline } else { t.border }))
}

/// One segment. The selected segment takes a soft accent fill and accent
/// ink, so the choice reads at a glance in every theme rather than as one
/// more field-coloured key.
pub(crate) fn segment(
    t: Colors,
    id: impl Into<ElementId>,
    active: bool,
    height: f32,
) -> Stateful<Div> {
    row()
        .id(id.into())
        .h(px(height))
        .px(px(SP_2))
        .gap(px(SP_1 + 1.))
        .justify_center()
        .rounded(px(R_SM))
        .text_size(px(TEXT_MD))
        .font_weight(WEIGHT_MEDIUM)
        .cursor_pointer()
        .when(active, |d| {
            d.bg(tint(t.accent, 0.16)).text_color(rgb(t.accent_text))
        })
        .when(!active, |d| {
            d.text_color(rgb(t.text_muted))
                .hover(move |d| d.text_color(rgb(t.text)).bg(rgb(t.control_hover)))
        })
}

/// Switch: a pill track with a sliding knob.
pub(crate) fn switch(t: Colors, on: bool) -> Div {
    row()
        .w(px(26.))
        .h(px(15.))
        .px(px(2.))
        .flex_shrink_0()
        .rounded_full()
        .bg(rgb(if on { t.accent } else { t.control_active }))
        .when(on, |d| d.justify_end())
        .child(div().size(px(11.)).rounded_full().bg(rgb(if on {
            t.on_accent
        } else {
            t.text_muted
        })))
}

/// Labelled switch row that runs `command`.
pub(crate) fn switch_row(
    t: Colors,
    id: impl Into<ElementId>,
    label: &'static str,
    detail: Option<&'static str>,
    on: bool,
    command: Command,
    cx: &mut Context<Studio>,
) -> Stateful<Div> {
    on_command(
        row()
            .id(id.into())
            .min_h(px(ROW_H))
            .py(px(SP_1))
            .gap(px(SP_3))
            .cursor_pointer()
            .rounded(px(R_SM))
            .child(
                col()
                    .flex_1()
                    .min_w(px(0.))
                    .gap(px(1.))
                    .child(div().text_color(rgb(t.text)).child(label))
                    .when_some(detail, |d, detail| {
                        d.child(
                            div()
                                .text_size(px(TEXT_SM))
                                .text_color(rgb(t.text_muted))
                                .child(detail),
                        )
                    }),
            )
            .child(switch(t, on))
            .tooltip(command_tip(t, command)),
        command,
        cx,
    )
}

#[derive(Clone, Copy)]
pub(crate) enum BadgeTone {
    Neutral,
    Accent,
    Danger,
}

/// Compact status or type tag.
pub(crate) fn badge(t: Colors, text: impl Into<SharedString>, tone: BadgeTone) -> Div {
    let (ink, fill) = match tone {
        BadgeTone::Neutral => (t.text_muted, hex(t.control)),
        BadgeTone::Accent => (t.accent_text, hex(t.accent_soft)),
        BadgeTone::Danger => (t.danger, tint(t.danger, 0.14)),
    };
    row()
        .h(px(18.))
        .px(px(SP_1_5))
        .flex_shrink_0()
        .rounded(px(R_XS + 1.))
        .bg(fill)
        .text_size(px(TEXT_SM))
        .font_weight(WEIGHT_MEDIUM)
        .text_color(rgb(ink))
        .child(text.into())
}

/// Status dot for health and progress.
pub(crate) fn dot(color: u32, size: f32) -> Div {
    div()
        .size(px(size))
        .flex_shrink_0()
        .rounded_full()
        .bg(rgb(color))
}

// ---------------------------------------------------------------------------
// Tabs.

/// Underlined tab. Tabs sit in a row with a hairline beneath.
pub(crate) fn tab(
    t: Colors,
    id: impl Into<ElementId>,
    label: &'static str,
    active: bool,
) -> Stateful<Div> {
    col()
        .id(id.into())
        .h_full()
        .justify_center()
        .px(px(SP_1))
        .relative()
        .cursor_pointer()
        .text_size(px(TEXT_MD))
        .font_weight(WEIGHT_MEDIUM)
        .text_color(rgb(if active { t.text } else { t.text_muted }))
        .when(!active, |d| d.hover(move |d| d.text_color(rgb(t.text))))
        .child(label)
        .child(
            div()
                .absolute()
                .bottom_0()
                .left(px(SP_1))
                .right(px(SP_1))
                .h(px(2.))
                .rounded_full()
                .bg(if active { hex(t.accent) } else { clear() }),
        )
}

// ---------------------------------------------------------------------------
// Panels: sections and property rows.

/// Collapsible section with a quiet header. Content is indented on the
/// panel's padding grid; sections are separated by hairlines.
pub(crate) fn section(
    t: Colors,
    s: &Studio,
    panel: PanelSection,
    title: &'static str,
    detail: Option<SharedString>,
    contents: impl IntoElement,
    cx: &mut Context<Studio>,
) -> Div {
    let open = s.section_open(panel);
    col()
        .flex_shrink_0()
        .border_b_1()
        .border_color(rgb(t.hairline))
        .child(
            row()
                .id(SharedString::from(format!("section-{}", panel as usize)))
                .h(px(32.))
                .flex_shrink_0()
                .px(px(SP_3))
                .gap(px(SP_1_5))
                .cursor_pointer()
                .group("section-header")
                .child(
                    div()
                        .flex_1()
                        .text_size(px(TEXT_MD))
                        .font_weight(WEIGHT_SEMIBOLD)
                        .text_color(rgb(t.text))
                        .child(title),
                )
                .when_some(detail, |d, detail| d.child(caption(t, detail).tabular()))
                .child(
                    div()
                        .flex()
                        .text_color(rgb(t.text_faint))
                        .group_hover("section-header", move |d| d.text_color(rgb(t.text)))
                        .child(
                            svg()
                                .path(if open {
                                    Icon::ChevronDown.path()
                                } else {
                                    Icon::ChevronRight.path()
                                })
                                .size(px(ICON_SM))
                                .text_color(rgb(t.text_faint)),
                        ),
                )
                .tooltip(tip(t, if open { "Collapse" } else { "Expand" }, None))
                .on_click(cx.listener(move |s, _, _, cx| {
                    cx.stop_propagation();
                    s.toggle_section(panel, cx);
                })),
        )
        .when(open, |d| {
            d.child(
                col()
                    .px(px(SP_3))
                    .pb(px(SP_3))
                    .gap(px(SP_1))
                    .child(contents),
            )
        })
}

/// Label column plus a value. Labels are sentence case and left aligned.
pub(crate) fn property_row(
    t: Colors,
    label: impl Into<SharedString>,
    value: impl IntoElement,
) -> Div {
    row()
        .min_h(px(ROW_H))
        .gap(px(SP_2))
        .child(
            div()
                .w(px(LABEL_W))
                .flex_shrink_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_color(rgb(t.text_muted))
                .child(label.into()),
        )
        .child(row().flex_1().min_w(px(0.)).gap(px(SP_1)).child(value))
}

/// A unit suffix shown after a label, such as "m" or "°".
pub(crate) fn unit_label(t: Colors, label: &'static str, unit: &'static str) -> Div {
    row()
        .gap(px(SP_1))
        .min_w(px(0.))
        .child(label)
        .when(!unit.is_empty(), |d| {
            d.child(div().text_color(rgb(t.text_faint)).child(unit))
        })
}

// ---------------------------------------------------------------------------
// Numeric fields.

/// How a field labels its axis or channel.
#[derive(Clone, Copy)]
pub(crate) enum FieldMark {
    None,
    /// Axis-coloured tick: X, Y, Z (or R, G, B).
    Axis(usize),
    /// A short text prefix such as "U" or "V".
    Text(&'static str),
}

/// Characters an axis field shows before it rounds its display: five keep
/// the value clear of the axis tick at the default sidebar width.
const AXIS_FIELD_CHARS: usize = 5;

/// Round a numeric display string to at most `max` characters, keeping its
/// integer digits. Non-numeric text and values that cannot shrink are kept.
pub(crate) fn compact_number(text: String, max: usize) -> String {
    if text.chars().count() <= max {
        return text;
    }
    let Ok(value) = text.parse::<f64>() else {
        return text;
    };
    // `trunc` keeps the sign of small negatives: -0.4 formats as "-0".
    let integer = format!("{:.0}", value.trunc()).len();
    if integer + 2 > max {
        return format!("{value:.0}");
    }
    let decimals = max - integer - 1;
    let rounded = format!("{value:.decimals$}");
    let trimmed = rounded.trim_end_matches('0').trim_end_matches('.');
    if trimmed == "-0" {
        "0".into()
    } else {
        trimmed.into()
    }
}

/// Typeset a number for display: a hyphen before a digit becomes a true
/// minus sign (U+2212). Hyphens inside words ("X-ray") are kept.
pub(crate) fn typeset_minus(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    let mut previous: Option<char> = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let numeric_next = chars
            .peek()
            .is_some_and(|next| next.is_ascii_digit() || *next == '.');
        let word_before = previous.is_some_and(|p| p.is_alphanumeric() || p == '.');
        if c == '-' && numeric_next && !word_before {
            out.push('\u{2212}');
        } else {
            out.push(c);
        }
        previous = Some(c);
    }
    out
}

/// Display text for a row of related values (X/Y/Z, R/G/B): one shared
/// number of decimals, about three significant digits for the row's largest
/// value, at most `max` characters for its widest, and trailing zeros
/// trimmed only where every value can lose them. "1.33, 1.3, 1.329" becomes
/// "1.33, 1.30, 1.33". Text that is not a plain number is kept as it is.
pub(crate) fn row_numbers(texts: &[String], max: usize) -> Vec<String> {
    let values: Vec<Option<f64>> = texts
        .iter()
        .map(|text| text.parse::<f64>().ok().filter(|v| v.is_finite()))
        .collect();
    let numbers: Vec<f64> = values.iter().flatten().copied().collect();
    let largest = numbers.iter().fold(0f64, |m, v| m.max(v.abs()));
    if numbers.is_empty() || (largest > 0. && largest < 1.0e-3) {
        return texts
            .iter()
            .map(|text| compact_number(text.clone(), max))
            .collect();
    }
    let integer_len = |v: f64| format!("{:.0}", v.trunc()).len();
    // Decimals the widest value can show within `max` characters.
    let room = numbers
        .iter()
        .map(|&v| max.saturating_sub(integer_len(v) + 1))
        .min()
        .unwrap_or(0);
    let significant = if largest < 1.0e-12 {
        0
    } else {
        (3 - (largest.log10().floor() as i32 + 1)).clamp(0, 4) as usize
    };
    let format = |v: f64, decimals: usize| {
        let text = format!("{v:.decimals$}");
        // "-0.00" is zero, not a negative number.
        if text
            .trim_start_matches('-')
            .chars()
            .all(|c| c == '0' || c == '.')
        {
            text.trim_start_matches('-').to_owned()
        } else {
            text
        }
    };
    let mut decimals = significant.min(room);
    while decimals > 0 && numbers.iter().all(|&v| format(v, decimals).ends_with('0')) {
        decimals -= 1;
    }
    texts
        .iter()
        .zip(&values)
        .map(|(text, value)| match value {
            Some(v) => format(*v, decimals),
            None => text.clone(),
        })
        .collect()
}

/// Editable value well. Drag horizontally to scrub (Shift for fine steps);
/// click to type an exact expression. Values use tabular numerals.
pub(crate) fn numeric_field(
    t: Colors,
    s: &Studio,
    id: impl Into<ElementId>,
    f: Field,
    mark: FieldMark,
    tooltip: &'static str,
    cx: &mut Context<Studio>,
) -> Stateful<Div> {
    numeric_field_showing(t, s, id, f, mark, tooltip, None, cx)
}

/// [`numeric_field`] showing `display` at rest instead of its own value text,
/// so a row of fields can share one precision. Editing shows the exact value.
#[allow(clippy::too_many_arguments)]
pub(crate) fn numeric_field_showing(
    t: Colors,
    s: &Studio,
    id: impl Into<ElementId>,
    f: Field,
    mark: FieldMark,
    tooltip: &'static str,
    display: Option<String>,
    cx: &mut Context<Studio>,
) -> Stateful<Div> {
    let active = s.field_is_active(f);
    let selected = s.field_selection_active(f);
    let scrubbing = s.panels.scrubbing_field() == Some(f);
    let name = f.is_text();
    let value = match (display, mark) {
        (_, _) if active || name => s.field_value(f),
        (Some(display), _) => typeset_minus(&display),
        // Dense axis fields round their display; the exact value is edited.
        (None, FieldMark::Axis(_)) => {
            typeset_minus(&compact_number(s.field_value(f), AXIS_FIELD_CHARS))
        }
        (None, _) => typeset_minus(&s.field_value(f)),
    };
    let mut field = row()
        .id(id.into())
        .relative()
        .h(px(H_SM))
        .min_w(px(0.))
        .flex_shrink_0()
        .px(px(SP_2))
        .gap(px(SP_1 + 1.))
        .rounded(px(R_SM))
        .bg(rgb(if active { t.sunken } else { t.field }))
        .border_1()
        .border_color(if active {
            hex(t.focus)
        } else if scrubbing {
            hex(t.accent_border)
        } else {
            field_edge(t)
        })
        .tabular()
        .when(!active, |d| d.hover(move |d| d.bg(rgb(t.field_hover))))
        .cursor(if active || name {
            CursorStyle::IBeam
        } else {
            CursorStyle::ResizeLeftRight
        });
    if !tooltip.is_empty() && !active {
        field = field.tooltip(tip_detail(
            t,
            tooltip,
            if name {
                "Click to rename"
            } else {
                "Drag to adjust · Shift for fine steps · Click to type a value or an expression such as 25mm, 1/8in or (2+3)*10cm"
            },
            None,
        ));
    }
    field = match mark {
        FieldMark::None => field,
        // A tick on the field's leading edge names the axis by color.
        FieldMark::Axis(axis) => field.pl(px(SP_2 + 2.)).child(
            div()
                .absolute()
                .left(px(0.))
                .top(px(5.))
                .w(px(2.))
                .h(px(12.))
                .rounded_full()
                .bg(rgb(t.axis[axis.min(2)])),
        ),
        FieldMark::Text(text) => field.child(
            div()
                .flex_shrink_0()
                .text_size(px(TEXT_SM))
                .text_color(rgb(t.text_faint))
                .child(text),
        ),
    };
    // Clip editing text instead of using an ellipsis: GPUI 0.2.2's ellipsis
    // layout mutates run lengths across measurements and can split the caret.
    let text = row()
        .flex_1()
        .min_w(px(0.))
        .overflow_hidden()
        .whitespace_nowrap()
        .when(!name, |d| d.justify_end())
        .child(
            div()
                .when(selected, |d| d.bg(rgb(t.accent_soft)).rounded(px(2.)))
                .text_color(rgb(if active {
                    t.text
                } else if value == "—" {
                    t.text_faint
                } else {
                    t.text
                }))
                .child(value),
        )
        .when(active, |d| {
            d.child(div().w(px(1.5)).h(px(14.)).flex_shrink_0().bg(rgb(t.focus)))
        });
    field = field.child(text);
    if active && PanelState::is_inspector_field(f) {
        let bounds = s.panels.active_field_bounds.clone();
        field = field.child(
            canvas(
                move |layout, _, _| {
                    bounds.set(Some(ActiveFieldLayout {
                        field: f,
                        bounds: layout,
                    }))
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0()
            .size_full(),
        );
    }
    if active {
        field.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    } else if name {
        field.on_click(cx.listener(move |s, _, w, cx| {
            cx.stop_propagation();
            s.begin_field(f, w, cx);
        }))
    } else {
        field.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |s, event: &MouseDownEvent, w, cx| {
                cx.stop_propagation();
                s.begin_scrub(f, event, w, cx);
            }),
        )
    }
}

/// One row with three axis fields, e.g. location X/Y/Z.
#[allow(clippy::too_many_arguments)]
pub(crate) fn vector_row(
    t: Colors,
    s: &Studio,
    label: &'static str,
    unit: &'static str,
    id: &'static str,
    field_of: impl Fn(usize) -> Field,
    tooltips: [&'static str; 3],
    stacked: bool,
    cx: &mut Context<Studio>,
) -> Div {
    // The three values share one precision, so a row never mixes
    // "1.33", "1.3" and "1.329".
    let texts: Vec<String> = (0..3).map(|axis| s.field_value(field_of(axis))).collect();
    let mut shown = row_numbers(&texts, AXIS_FIELD_CHARS).into_iter();
    let fields = row()
        .flex_1()
        .min_w(px(0.))
        .gap(px(SP_1))
        .children((0..3).map(|axis| {
            numeric_field_showing(
                t,
                s,
                SharedString::from(format!("{id}-{axis}")),
                field_of(axis),
                FieldMark::Axis(axis),
                tooltips[axis],
                shown.next(),
                cx,
            )
            .flex_1()
            .flex_basis(px(0.))
        }));
    if stacked {
        col()
            .gap(px(SP_1))
            .pb(px(SP_1))
            .child(
                div()
                    .text_color(rgb(t.text_muted))
                    .child(unit_label(t, label, unit)),
            )
            .child(fields)
    } else {
        row()
            .min_h(px(ROW_H))
            .gap(px(SP_2))
            .child(
                div()
                    .w(px(LABEL_W))
                    .flex_shrink_0()
                    .text_color(rgb(t.text_muted))
                    .child(unit_label(t, label, unit)),
            )
            .child(fields)
    }
}

/// Property row whose value is one numeric field.
pub(crate) fn numeric_row(
    t: Colors,
    s: &Studio,
    id: &'static str,
    label: &'static str,
    unit: &'static str,
    f: Field,
    cx: &mut Context<Studio>,
) -> Div {
    row()
        .min_h(px(ROW_H))
        .gap(px(SP_2))
        .child(
            div()
                .w(px(LABEL_W))
                .flex_shrink_0()
                .text_color(rgb(t.text_muted))
                .child(unit_label(t, label, unit)),
        )
        .child(
            numeric_field(t, s, id, f, FieldMark::None, label, cx)
                .flex_1()
                .min_w(px(0.)),
        )
}

/// A read-only fact (a count, a derived value): label and plain value with
/// no well, so it never reads as an input. Hover explains where it comes from.
/// A fact in the value column, left aligned like every other value. `locked`
/// marks one the user cannot change here: muted, with a padlock after it.
pub(crate) fn fact_row(
    t: Colors,
    label: &'static str,
    value: impl Into<SharedString>,
    reason: &'static str,
    locked: bool,
) -> Stateful<Div> {
    row()
        .id(label)
        .min_h(px(ROW_H))
        .gap(px(SP_2))
        .tooltip(tip(t, reason, None))
        .child(
            div()
                .w(px(LABEL_W))
                .flex_shrink_0()
                .text_color(rgb(t.text_muted))
                .child(label),
        )
        .child(
            row()
                .flex_1()
                .min_w(px(0.))
                .gap(px(SP_1_5))
                .child(
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .text_color(rgb(if locked { t.text_muted } else { t.text }))
                        .child(value.into()),
                )
                .when(locked, |d| d.child(icon(Icon::Lock, 12., t.text_faint))),
        )
}

pub(crate) fn readonly_row(
    t: Colors,
    label: &'static str,
    unit: &'static str,
    value: String,
    reason: &'static str,
) -> Stateful<Div> {
    row()
        .id(label)
        .min_h(px(ROW_H))
        .gap(px(SP_2))
        .tooltip(tip(t, reason, None))
        .child(
            div()
                .w(px(LABEL_W))
                .flex_shrink_0()
                .text_color(rgb(t.text_muted))
                .child(unit_label(t, label, unit)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .px(px(SP_2))
                .text_right()
                .truncate()
                .tabular()
                .text_color(rgb(t.text))
                .child(value),
        )
}

/// An input that is temporarily unavailable: the field's shape, dimmed,
/// with the reason on hover. For values that are never editable, use
/// [`readonly_row`] or [`stat_tiles`].
pub(crate) fn disabled_field_row(
    t: Colors,
    label: &'static str,
    unit: &'static str,
    value: String,
    reason: &'static str,
) -> Stateful<Div> {
    row()
        .id(label)
        .min_h(px(ROW_H))
        .gap(px(SP_2))
        .tooltip(tip(t, reason, None))
        .child(
            div()
                .w(px(LABEL_W))
                .flex_shrink_0()
                .text_color(rgb(t.text_faint))
                .child(unit_label(t, label, unit)),
        )
        .child(
            row()
                .flex_1()
                .min_w(px(0.))
                .h(px(H_SM))
                .px(px(SP_2))
                .justify_end()
                .rounded(px(R_SM))
                .border_1()
                .border_color(rgb(t.hairline))
                .text_color(rgb(t.text_faint))
                .tabular()
                .child(value),
        )
}

/// One summary statistic: a small caption over its value, in an inset tile.
/// Tiles never look like fields, so derived data never invites editing.
pub(crate) fn stat_tile(t: Colors, label: &'static str, value: impl Into<SharedString>) -> Div {
    col()
        .flex_1()
        .min_w(px(0.))
        .px(px(SP_2 + 2.))
        .py(px(SP_1_5))
        .gap(px(1.))
        .rounded(px(R_SM))
        .bg(rgb(t.raised))
        .child(caption(t, label).truncate())
        .child(
            div()
                .tabular()
                .font_weight(WEIGHT_MEDIUM)
                .truncate()
                .child(value.into()),
        )
}

/// A row of [`stat_tile`]s sharing the width equally.
pub(crate) fn stat_tiles(tiles: impl IntoIterator<Item = Div>) -> Div {
    row().gap(px(SP_1)).children(tiles)
}

// ---------------------------------------------------------------------------
// Empty state and floating clusters.

/// Centered guidance for an empty panel: icon, title, body and actions.
pub(crate) fn empty_state(t: Colors, glyph: Icon, title: &'static str, body: &'static str) -> Div {
    col()
        .items_center()
        .px(px(SP_5))
        .py(px(SP_6))
        .gap(px(SP_2))
        .text_center()
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(36.))
                .rounded(px(R_LG))
                .bg(rgb(t.control))
                .border_1()
                .border_color(rgb(t.hairline))
                .child(icon(glyph, ICON_LG, t.text_muted)),
        )
        .child(
            div()
                .mt(px(SP_1))
                .text_size(px(TEXT_LG))
                .font_weight(WEIGHT_MEDIUM)
                .text_color(rgb(t.text))
                .child(title),
        )
        .child(
            div()
                .max_w(px(220.))
                .line_height(px(17.))
                .text_color(rgb(t.text_muted))
                .child(body),
        )
}

/// Notification card: a tinted icon, title, optional body and actions.
pub(crate) fn toast(
    t: Colors,
    glyph: Icon,
    ink: u32,
    title: impl Into<SharedString>,
    body: Option<SharedString>,
) -> Div {
    row()
        .items_start()
        .w(px(300.))
        .p(px(SP_3))
        .gap(px(SP_2 + 2.))
        .rounded(px(R_LG))
        .bg(rgb(t.overlay))
        .border_1()
        .border_color(rgb(t.border))
        .shadow(elevation(t, 2))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(28.))
                .flex_shrink_0()
                .rounded(px(R_MD))
                .bg(tint(ink, 0.14))
                .child(icon(glyph, ICON_MD, ink)),
        )
        .child(
            col()
                .flex_1()
                .min_w(px(0.))
                .gap(px(2.))
                .child(
                    div()
                        .font_weight(WEIGHT_SEMIBOLD)
                        .text_color(rgb(t.text))
                        .child(title.into()),
                )
                .when_some(body, |d, body| {
                    d.child(
                        div()
                            .text_size(px(TEXT_SM))
                            .line_height(px(16.))
                            .text_color(rgb(t.text_muted))
                            .child(body),
                    )
                }),
        )
}

/// Translucent viewport cluster. It swallows presses and wheel events so the
/// viewport never selects or zooms through a control, but lets pointer moves
/// through so an orbit continues when the cursor crosses it.
pub(crate) fn hud(t: Colors) -> Div {
    row()
        .p(px(3.))
        .gap(px(2.))
        .flex_shrink_0()
        .rounded(px(R_LG))
        .bg(rgba(t.hud))
        .border_1()
        .border_color(rgba(t.hud_border))
        .shadow(elevation(t, 1))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
}

/// Floating card for menus and popovers.
pub(crate) fn popover(t: Colors) -> Div {
    col()
        .p(px(SP_1))
        .rounded(px(R_LG))
        .bg(rgb(t.overlay))
        .border_1()
        .border_color(rgb(t.border))
        .shadow(elevation(t, 2))
        .overflow_hidden()
}

/// Modal dialog card.
pub(crate) fn dialog(t: Colors) -> Div {
    col()
        .rounded(px(R_XL))
        .bg(rgb(t.overlay))
        .border_1()
        .border_color(rgb(t.border))
        .shadow(elevation(t, 3))
        .overflow_hidden()
}

/// Dialog title row with a close affordance.
pub(crate) fn dialog_header(
    t: Colors,
    glyph: Option<Icon>,
    title: impl Into<SharedString>,
    subtitle: Option<SharedString>,
) -> Div {
    row()
        .flex_shrink_0()
        .px(px(SP_4))
        .pt(px(SP_4))
        .pb(px(SP_3))
        .gap(px(SP_2 + 2.))
        .items_start()
        .when_some(glyph, |d, glyph| {
            d.child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(28.))
                    .rounded(px(R_MD))
                    .bg(rgb(t.accent_soft))
                    .child(icon(glyph, ICON_MD, t.accent_text)),
            )
        })
        .child(
            col()
                .flex_1()
                .min_w(px(0.))
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(TEXT_XL))
                        .font_weight(WEIGHT_SEMIBOLD)
                        .text_color(rgb(t.text))
                        .child(title.into()),
                )
                .when_some(subtitle, |d, subtitle| {
                    d.child(
                        div()
                            .text_size(px(TEXT_SM))
                            .line_height(px(16.))
                            .text_color(rgb(t.text_muted))
                            .child(subtitle),
                    )
                }),
        )
        .child(keycaps(t, "Esc"))
}

/// Footer row for dialog actions, right aligned.
pub(crate) fn dialog_footer(t: Colors) -> Div {
    row()
        .flex_shrink_0()
        .px(px(SP_4))
        .py(px(SP_3))
        .gap(px(SP_2))
        .border_t_1()
        .border_color(rgb(t.hairline))
        .bg(rgb(t.raised))
}

/// Keyboard hint pair for footers: keycaps then a label.
pub(crate) fn hint(t: Colors, keys: &str, label: &'static str) -> Div {
    row()
        .gap(px(SP_1 + 1.))
        .flex_shrink_0()
        .child(keycaps(t, keys))
        .child(
            div()
                .text_size(px(TEXT_SM))
                .text_color(rgb(t.text_muted))
                .child(label),
        )
}

#[cfg(test)]
mod tests {
    use super::keycap_token_is_text;

    #[test]
    fn compact_numbers_keep_integer_digits_and_round_decimals() {
        use super::compact_number;
        let compact = |text: &str| compact_number(text.into(), 6);
        assert_eq!(compact("69.9009"), "69.901");
        assert_eq!(compact("-13.178"), "-13.18");
        assert_eq!(compact("0.56"), "0.56");
        assert_eq!(compact("-0.123456"), "-0.123");
        assert_eq!(compact("1234.5678"), "1234.6");
        assert_eq!(compact("12345.678"), "12346");
        assert_eq!(compact("1234567"), "1234567");
        assert_eq!(compact("2.0000001"), "2");
        assert_eq!(compact("—"), "—");
    }

    #[test]
    fn rows_share_one_precision_and_trim_together() {
        use super::row_numbers;
        let row = |texts: &[&str]| {
            row_numbers(&texts.iter().map(|t| t.to_string()).collect::<Vec<_>>(), 5)
        };
        assert_eq!(row(&["1.33", "1.3", "1.329"]), ["1.33", "1.30", "1.33"]);
        assert_eq!(row(&["-0.25", "0", "1"]), ["-0.25", "0.00", "1.00"]);
        assert_eq!(row(&["0", "45", "0"]), ["0", "45", "0"]);
        assert_eq!(row(&["1", "1", "1"]), ["1", "1", "1"]);
        assert_eq!(row(&["2.2", "0.75", "1.5"]), ["2.20", "0.75", "1.50"]);
        assert_eq!(row(&["-12.5", "0.333", "1"]), ["-12.5", "0.3", "1.0"]);
        assert_eq!(row(&["0.25", "0.5", "0.75"]), ["0.25", "0.50", "0.75"]);
        assert_eq!(row(&["-0.0001", "2", "1"]), ["0", "2", "1"]);
        assert_eq!(row(&["1234.6", "1", "1"]), ["1235", "1", "1"]);
        assert_eq!(row(&["—", "1.5", "2"]), ["—", "1.5", "2.0"]);
    }

    #[test]
    fn negative_numbers_use_a_true_minus_sign() {
        use super::typeset_minus;
        assert_eq!(typeset_minus("-0.25"), "\u{2212}0.25");
        assert_eq!(typeset_minus("r -0.3 × 2"), "r \u{2212}0.3 × 2");
        assert_eq!(typeset_minus("-.5 m"), "\u{2212}.5 m");
        assert_eq!(typeset_minus("X-ray 1-2"), "X-ray 1-2");
        assert_eq!(typeset_minus("Re-open"), "Re-open");
        assert_eq!(typeset_minus("1e-5"), "1e-5");
    }

    #[test]
    fn shortcut_words_render_as_text_and_keys_as_caps() {
        for text in ["then", "in", "edit", "mode", "/"] {
            assert!(keycap_token_is_text(text), "{text}");
        }
        for key in ["Z", "⌘", "Ctrl", "Home", "Esc", "`", ".", "F", "1"] {
            assert!(!keycap_token_is_text(key), "{key}");
        }
    }
}
