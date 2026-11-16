//! The inspector while drawing: which sketch and plane, what it holds, the
//! drawing tools with their keys, and the actions that leave Draw mode.
//! Nothing is selected in the scene while drawing, so this replaces the
//! object page rather than showing an empty state.
use super::assets::{Icon, icon};
use super::components::*;
use super::tokens::*;
use crate::app::{Command, Studio};
use crate::cad_tools::ToolCommand;
use crate::sketch::hud::tool_icon;
use crate::sketch::{DrawCommand, DrawTool};
use crate::theme::Colors;
use gpui::{prelude::*, *};

/// The Draw mode inspector, or `None` when not drawing.
pub(super) fn panel(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> Option<Div> {
    let session = s.sketch.session.as_ref()?;
    let object = s.session_object();
    let name = object
        .and_then(|id| s.scene.object(id))
        .map_or_else(|| "New sketch".to_owned(), |o| o.name.clone());
    let (curves, profiles, construction) = s.sketch_analysis().map_or((0, 0, 0), |analysis| {
        (
            analysis.entities.len(),
            analysis.profiles.len(),
            analysis.entities.iter().filter(|e| e.construction).count(),
        )
    });
    let header = row()
        .h(px(48.))
        .px(px(SP_3))
        .gap(px(SP_2))
        .flex_shrink_0()
        .border_b_1()
        .border_color(rgb(t.hairline))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(H_MD))
                .flex_shrink_0()
                .rounded(px(R_MD))
                .bg(rgb(t.accent_soft))
                .child(icon(Icon::Sketch, ICON_MD, t.accent_text)),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_size(px(TEXT_LG))
                .font_weight(WEIGHT_MEDIUM)
                .child(name),
        )
        .child(badge(t, "Drawing", BadgeTone::Accent));
    let mut summary = col()
        .gap(px(SP_1))
        .child(readonly_row(
            t,
            "Plane",
            "",
            session.plane_label.clone(),
            "The plane new curves are drawn on",
        ))
        .child(
            stat_tiles([
                stat_tile(t, "Curves", curves.to_string()),
                stat_tile(t, "Profiles", profiles.to_string()),
                stat_tile(t, "Construction", construction.to_string()),
            ])
            .id("draw-panel-stats")
            .tooltip(tip(
                t,
                "Profiles are closed regions Extrude and Revolve can use · Construction curves only guide drawing",
                None,
            )),
        );
    let mut tools = div().grid().grid_cols(2).gap(px(2.));
    for tool in DrawTool::ALL {
        tools = tools.child(tool_row(
            t,
            SharedString::from(format!("draw-panel-{}", tool.key())),
            tool_icon(tool),
            tool.label(),
            tool.key(),
            session.tool == tool,
            Command::Draw(DrawCommand::Tool(tool)),
            cx,
        ));
    }
    tools = tools.child(tool_row(
        t,
        "draw-panel-construction",
        Icon::DrawConstruction,
        "Construction",
        "X",
        session.construction,
        Command::Draw(DrawCommand::ToggleConstruction),
        cx,
    ));
    let extrude = if profiles > 0 {
        command_button(
            t,
            "draw-panel-extrude",
            "Extrude",
            Some(Icon::Extrude),
            Command::CadTool(ToolCommand::Extrude),
            Tone::Secondary,
            cx,
        )
    } else {
        disabled(button(
            t,
            "draw-panel-extrude",
            "Extrude",
            Some(Icon::Extrude),
            Tone::Secondary,
        ))
        .cursor_default()
        .tooltip(tip(t, "Close a region to extrude it", None))
    };
    let actions = row()
        .gap(px(SP_1_5))
        .child(extrude.h(px(H_MD)).flex_1())
        .child(
            command_button(
                t,
                "draw-panel-finish",
                "Finish",
                Some(Icon::Check),
                Command::Draw(DrawCommand::Finish),
                Tone::Primary,
                cx,
            )
            .h(px(H_MD))
            .flex_1(),
        );
    // The way out stays in view however short the panel is.
    summary = summary.child(actions.mt(px(SP_2)));
    Some(
        col().flex_1().min_h(px(0.)).child(header).child(
            col()
                .id("draw-panel-scroll")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(block(t, "Sketch", summary))
                .child(block(t, "Tools", tools)),
        ),
    )
}

/// A titled group on the panel's padding grid, separated by a hairline.
fn block(t: Colors, title: &'static str, contents: impl IntoElement) -> Div {
    col()
        .flex_shrink_0()
        .px(px(SP_3))
        .pb(px(SP_3))
        .border_b_1()
        .border_color(rgb(t.hairline))
        .child(
            row()
                .h(px(32.))
                .text_size(px(TEXT_MD))
                .font_weight(WEIGHT_SEMIBOLD)
                .text_color(rgb(t.text))
                .child(title),
        )
        .child(contents)
}

/// One tool: its icon, name and key. The current tool is highlighted.
#[allow(clippy::too_many_arguments)]
fn tool_row(
    t: Colors,
    id: impl Into<ElementId>,
    glyph: Icon,
    label: &'static str,
    key: &'static str,
    active: bool,
    command: Command,
    cx: &mut Context<Studio>,
) -> Stateful<Div> {
    on_command(
        row()
            .id(id.into())
            .h(px(H_MD))
            .min_w(px(0.))
            .pl(px(SP_2))
            .pr(px(SP_1_5))
            .gap(px(SP_2))
            .rounded(px(R_SM + 1.))
            .cursor_pointer()
            .when(active, |d| d.bg(rgb(t.accent_soft)))
            .when(!active, |d| {
                d.hover(move |d| d.bg(rgb(t.control_hover)))
                    .active(move |d| d.bg(rgb(t.control_active)))
            })
            .child(icon(
                glyph,
                ICON_SM,
                if active { t.accent_text } else { t.text_muted },
            ))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .font_weight(WEIGHT_MEDIUM)
                    .text_color(rgb(if active { t.text } else { t.text_muted }))
                    .child(label),
            )
            .child(keycap(t, key)),
        command,
        cx,
    )
}
