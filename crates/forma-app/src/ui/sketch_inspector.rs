//! The inspector while drawing: which sketch and plane, what it holds, what
//! still keeps a region from closing, and what is selected. The drawing tools
//! and the way out (Extrude, Finish) live in the Draw toolbar at the bottom of
//! the viewport, so this panel describes the sketch instead of repeating them.
//! Nothing is selected in the scene while drawing, so this replaces the
//! object page rather than showing an empty state.
use super::assets::{Icon, icon};
use super::components::*;
use super::tokens::*;
use crate::app::Studio;
use crate::theme::Colors;
use forma_core::{SketchCurve, SketchEntity};
use gpui::{prelude::*, *};

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// What the sketch holds, counted once per analysis.
#[derive(Default)]
struct Counts {
    curves: usize,
    profiles: usize,
    construction: usize,
    /// Curve ends no other curve meets: each keeps a region from closing.
    open_ends: usize,
}

/// One line saying where the sketch stands and what to do next: a tinted
/// callout while ends are open, a quiet line otherwise.
fn status(t: Colors, counts: &Counts) -> Div {
    if counts.open_ends > 0 {
        return row()
            .items_start()
            .gap(px(SP_2))
            .p(px(SP_2))
            .rounded(px(R_MD))
            .bg(tint(t.warning, 0.09))
            .border_1()
            .border_color(tint(t.warning, 0.24))
            .child(div().pt(px(1.)).child(icon(Icon::Warning, ICON_SM, t.warning)))
            .child(
                col()
                    .flex_1()
                    .min_w(px(0.))
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(TEXT_SM))
                            .line_height(px(16.))
                            .font_weight(WEIGHT_MEDIUM)
                            .text_color(rgb(t.text))
                            .child(format!(
                                "{} keep{} a region from closing",
                                plural(counts.open_ends, "open end", "open ends"),
                                if counts.open_ends == 1 { "s" } else { "" }
                            )),
                    )
                    .child(
                        div()
                            .text_size(px(TEXT_SM))
                            .line_height(px(16.))
                            .text_color(rgb(t.text_muted))
                            .child(
                                "Draw a line to the highlighted ends, or drag an end onto another point to close the loop.",
                            ),
                    ),
            );
    }
    let (ink, text) = if counts.curves == 0 {
        (
            t.text_faint,
            "Pick a tool in the toolbar below and click to place the first point.".to_owned(),
        )
    } else if counts.profiles > 0 {
        (
            t.success,
            format!(
                "{} · ready to extrude",
                plural(counts.profiles, "closed region", "closed regions")
            ),
        )
    } else {
        (
            t.text_faint,
            "No closed region yet · construction curves never close one".to_owned(),
        )
    };
    row()
        .items_start()
        .gap(px(SP_2))
        .pt(px(SP_1))
        .child(div().pt(px(5.)).child(dot(ink, 6.)))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(TEXT_SM))
                .line_height(px(16.))
                .text_color(rgb(t.text_muted))
                .child(text),
        )
}

/// The selected curves: how many, of which kinds, how long, and the keys
/// that act on them.
fn selection(t: Colors, entities: &[SketchEntity], selected: &[usize]) -> Option<Div> {
    let picked: Vec<&SketchEntity> = selected.iter().filter_map(|&i| entities.get(i)).collect();
    if picked.is_empty() {
        return None;
    }
    let mut kinds = [0usize; 3];
    let mut length = 0.;
    for entity in &picked {
        kinds[match entity.curve {
            SketchCurve::Line { .. } => 0,
            SketchCurve::Arc { .. } => 1,
            SketchCurve::Circle { .. } => 2,
        }] += 1;
        length += entity.curve.length();
    }
    let construction = picked.iter().filter(|e| e.construction).count();
    let mut parts: Vec<String> = [("line", "lines"), ("arc", "arcs"), ("circle", "circles")]
        .into_iter()
        .zip(kinds)
        .filter(|(_, n)| *n > 0)
        .map(|((one, many), n)| plural(n, one, many))
        .collect();
    if construction > 0 {
        parts.push(format!("{construction} construction"));
    }
    Some(
        col()
            .gap(px(SP_1))
            .child(
                row()
                    .gap(px(SP_2))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_color(rgb(t.text))
                            .font_weight(WEIGHT_MEDIUM)
                            .child(plural(picked.len(), "curve", "curves")),
                    )
                    .child(caption(t, typeset_minus(&format!("{} m", measure(length)))).tabular()),
            )
            .child(
                div()
                    .text_size(px(TEXT_SM))
                    .text_color(rgb(t.text_muted))
                    .child(parts.join(" · ")),
            )
            .child(
                row()
                    .pt(px(SP_1))
                    .gap(px(SP_3))
                    .child(hint(t, "X", "Construction"))
                    .child(hint(t, "Delete", "Remove")),
            ),
    )
}

/// About three significant digits, as the inspector's rows show values.
fn measure(value: f32) -> String {
    row_numbers(&[crate::transform::format_precision_value(value)], 6)
        .pop()
        .unwrap_or_default()
}

/// The Draw mode inspector, or `None` when not drawing.
pub(super) fn panel(t: Colors, s: &Studio, _cx: &mut Context<Studio>) -> Option<Div> {
    let session = s.sketch.session.as_ref()?;
    let object = s.session_object();
    let name = object
        .and_then(|id| s.scene.object(id))
        .map_or_else(|| "New sketch".to_owned(), |o| o.name.clone());
    let analysis = s.sketch_analysis();
    let counts = analysis
        .as_ref()
        .map_or_else(Counts::default, |analysis| Counts {
            curves: analysis.entities.len(),
            profiles: analysis.profiles.len(),
            construction: analysis.entities.iter().filter(|e| e.construction).count(),
            open_ends: analysis
                .endpoints
                .iter()
                .filter(|(_, count)| *count == 1)
                .count(),
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
    let open_tile = {
        let tile = stat_tile(t, "Open ends", counts.open_ends.to_string());
        if counts.open_ends > 0 {
            tile.text_color(rgb(t.warning))
        } else {
            tile
        }
    };
    let summary = col()
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
                stat_tile(t, "Curves", counts.curves.to_string()),
                stat_tile(t, "Profiles", counts.profiles.to_string()),
                open_tile,
            ])
            .id("draw-panel-stats")
            .tooltip(tip(
                t,
                "Profiles are closed regions Extrude and Revolve can use · Open ends are curve ends no other curve meets",
                None,
            )),
        )
        .when(counts.construction > 0, |d| {
            d.child(readonly_row(
                t,
                "Construction",
                "",
                counts.construction.to_string(),
                "Construction curves guide drawing and never form profiles",
            ))
        })
        .child(status(t, &counts).mt(px(SP_1)));
    let selected: Vec<usize> = session.selection.iter().copied().collect();
    let picked = analysis
        .as_ref()
        .and_then(|analysis| selection(t, &analysis.entities, &selected));
    Some(
        col().flex_1().min_h(px(0.)).child(header).child(
            col()
                .id("draw-panel-scroll")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(block(t, "Sketch", summary))
                .when_some(picked, |d, picked| d.child(block(t, "Selection", picked))),
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
