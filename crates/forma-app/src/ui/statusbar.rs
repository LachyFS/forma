//! Status bar: contextual input hints on the left, the latest status message,
//! then selection, render progress, device and update state on the right.
use super::STATUSBAR_HEIGHT;
use super::assets::{Icon, icon};
use super::components::*;
use super::tokens::*;
use crate::app::{Command, EditMode, Studio};
use crate::platform_shortcut;
use crate::theme::Colors;
use forma_render::RenderMode;
use gpui::{prelude::*, *};

/// Input hints for what the pointer and keyboard do right now.
pub(crate) fn input_hints(s: &Studio) -> Vec<(&'static str, &'static str)> {
    if let Some(hints) = s.draw_input_hints() {
        return hints;
    }
    if let Some(hints) = s.cad_tool_hints() {
        return hints;
    }
    if let Some(drag) = &s.transform_drag {
        if drag.mesh_tool.is_some() {
            return vec![
                ("Return", "Apply"),
                ("Esc", "Cancel"),
                ("0–9", "Exact value"),
                ("Ctrl", "Snap"),
            ];
        }
        return vec![
            ("Return", "Apply"),
            ("Esc", "Cancel"),
            ("X Y Z", "Axis"),
            ("⇧ Z", "Plane"),
            ("Ctrl", "Snap"),
            ("⇧", "Precision"),
        ];
    }
    if s.box_select.is_some() {
        return vec![
            ("Drag", "Box select"),
            ("⇧", "Add"),
            ("Ctrl", "Subtract"),
            ("Esc", "Cancel"),
        ];
    }
    if s.active_field.is_some() {
        return vec![
            ("Return", "Apply"),
            ("Esc", "Cancel"),
            ("Tab", "Next field"),
            ("⇧ Tab", "Previous"),
        ];
    }
    if let Some(hints) = s.timeline_input_hints() {
        return hints;
    }
    if s.edit_mode == EditMode::Object {
        vec![
            ("Click", "Select"),
            ("⇧ Click", "Extend"),
            ("B", "Box"),
            ("G R S", "Move · Rotate · Scale"),
            ("Tab", "Edit"),
            ("Z", "Shading"),
            (platform_shortcut("⌘ K", "Ctrl K"), "Commands"),
        ]
    } else {
        vec![
            ("1 2 3", "Vertex · Edge · Face"),
            ("Click", "Select"),
            ("L", "Linked"),
            ("E", "Extrude"),
            ("I", "Inset"),
            ("G R S", "Transform"),
            ("Tab", "Object"),
        ]
    }
}

impl Studio {
    /// The mesh component selection in the edit mode's own words, e.g.
    /// "768 faces selected". The status message and the selection readout
    /// use the same text, so the status bar shows it once.
    pub(crate) fn component_summary(&self) -> String {
        let count = self.component_elements().len();
        let (one, many) = match self.edit_mode {
            EditMode::Vertex => ("vertex", "vertices"),
            EditMode::Edge => ("edge", "edges"),
            _ => ("face", "faces"),
        };
        match count {
            0 => format!("No {one} selected"),
            1 => format!("1 {one} selected"),
            _ => format!("{count} {many} selected"),
        }
    }
}

/// What is selected. Object facts (face counts, features) belong to the
/// inspector, so a single mesh or part adds nothing here.
fn selection_summary(s: &Studio) -> Option<String> {
    if let Some(summary) = s.cad_tool_summary() {
        return Some(summary);
    }
    if s.edit_mode.is_component() {
        return Some(s.component_summary());
    }
    if let Some(summary) = s.draw_summary() {
        return Some(summary);
    }
    let objects = s.selected_ids().len();
    match objects {
        0 => None,
        1 => {
            let sketch = s.scene.object_sketch(s.selected?)?;
            Some(format!("{} curves", sketch.entities.len()))
        }
        count => Some(format!("{count} objects selected")),
    }
}

/// The active part's last regeneration time and its breakdown.
fn regeneration(s: &Studio) -> Option<(String, String)> {
    let (_, part) = s.timeline_part()?;
    if part.is_pending() {
        return None;
    }
    super::feature_inspector::regeneration_timing(s, part.id)
}

/// Approximate drawn width of a hint pair, for whole-pair truncation.
fn hint_width(keys: &str, label: &str) -> f32 {
    let caps: f32 = keys
        .split_whitespace()
        .map(|key| 10. + key.chars().count() as f32 * 6.5)
        .sum();
    caps + label.chars().count() as f32 * 6. + 18.
}

fn segment_text(t: Colors, text: impl Into<SharedString>) -> Div {
    div()
        .flex_shrink_0()
        .whitespace_nowrap()
        .tabular()
        .text_color(rgb(t.text_muted))
        .child(text.into())
}

pub(super) fn statusbar(
    t: Colors,
    s: &Studio,
    window: &Window,
    cx: &mut Context<Studio>,
) -> AnyElement {
    use crate::updater::State;
    let wide = f32::from(window.viewport_size().width) >= 1280.;
    let health = if s.render_error.is_some() {
        t.danger
    } else if s.frame.is_none() {
        t.warning
    } else {
        t.success
    };
    // Keep whole hint pairs that fit beside the right-hand readouts, which
    // are estimated from their text at the 11 px status size.
    let width = f32::from(window.viewport_size().width);
    let text = |chars: usize| chars as f32 * 6.2;
    let message = text(s.status.chars().count()).min(if wide { 420. } else { 260. }) + 24.;
    // A status message that repeats the selection readout is shown once.
    let summary = selection_summary(s).filter(|summary| *summary != s.status);
    let selection = summary
        .as_ref()
        .map_or(0., |summary| text(summary.len()) + 25.);
    let regenerated = regeneration(s);
    let regeneration_width = regenerated
        .as_ref()
        .map_or(0., |(total, _)| text(total.len()) + 30.);
    let device_width = if wide { 260. } else { 150. };
    let device = text(s.device_name.chars().count()).min(device_width) + 30.;
    let rendered = if s.settings.mode == RenderMode::Rendered {
        130.
    } else {
        0.
    };
    let mut budget = width - message - selection - regeneration_width - device - rendered - 190.;
    let hints: Vec<_> = input_hints(s)
        .into_iter()
        .take_while(|(keys, label)| {
            budget -= hint_width(keys, label);
            budget >= 0.
        })
        .collect();
    let update = match &s.updates.state {
        State::Ready(_) => Some(("Update ready", BadgeTone::Accent)),
        State::Downloading(_) => Some(("Downloading update…", BadgeTone::Neutral)),
        State::Available(_) => Some(("Update available", BadgeTone::Accent)),
        _ => None,
    };
    let rendered = s.settings.mode == RenderMode::Rendered;
    let message_width = if wide { 420. } else { 260. };
    // The dot and its gap sit inside the message's width.
    let status = fit_text(cx, s.status.clone().into(), TEXT_SM, message_width - 12.);
    let device_name = fit_text(
        cx,
        s.device_name.clone().into(),
        TEXT_SM,
        device_width - 16.,
    );
    row()
        .h(px(STATUSBAR_HEIGHT))
        .flex_shrink_0()
        .pl(px(SP_3))
        .pr(px(SP_1))
        .gap(px(SP_3))
        .bg(rgb(t.surface))
        .border_t_1()
        .border_color(rgb(t.hairline))
        .text_size(px(TEXT_SM))
        .text_color(rgb(t.text_muted))
        // Hints shrink first; each pair is kept whole.
        .child(
            row()
                .flex_1()
                .min_w(px(0.))
                .gap(px(SP_3))
                .overflow_hidden()
                .children(hints.into_iter().map(|(keys, label)| hint(t, keys, label))),
        )
        .when(s.render_error.is_some(), |d| {
            d.child(badge(t, "Renderer error", BadgeTone::Danger))
        })
        .child(
            row()
                .id("status-message")
                .max_w(px(message_width))
                .min_w(px(0.))
                .gap(px(SP_1_5))
                .child(dot(health, 6.))
                .child(
                    div()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_color(rgb(if s.render_error.is_some() {
                            t.danger
                        } else {
                            t.text
                        }))
                        .child(status),
                )
                .tooltip(tip(t, s.status.clone(), None)),
        )
        .child(vrule(t, 12.))
        .when_some(summary, |d, summary| {
            d.child(segment_text(t, summary)).child(vrule(t, 12.))
        })
        .when(rendered, |d| {
            d.child(
                row()
                    .gap(px(SP_1_5))
                    .flex_shrink_0()
                    .child(
                        div()
                            .w(px(36.))
                            .h(px(3.))
                            .rounded_full()
                            .bg(rgb(t.field))
                            .child(
                                div().h_full().rounded_full().bg(rgb(t.accent)).w(relative(
                                    (s.samples as f32 / s.settings.max_samples.max(1) as f32)
                                        .clamp(0., 1.),
                                )),
                            ),
                    )
                    .child(segment_text(
                        t,
                        format!("{}/{} spp", s.samples, s.settings.max_samples),
                    )),
            )
        })
        // Moved here from the timeline strip: a part's regeneration time.
        .when_some(regenerated, |d, (total, detail)| {
            d.child(
                row()
                    .id("regeneration-time")
                    .gap(px(SP_1))
                    .flex_shrink_0()
                    .child(icon(Icon::History, 12., t.text_faint))
                    .child(segment_text(t, total.clone()))
                    .tooltip(tip_detail(
                        t,
                        format!("Part regenerated in {total}"),
                        detail,
                        None,
                    )),
            )
        })
        .child(
            row()
                .id("render-time")
                .gap(px(SP_1))
                .flex_shrink_0()
                .child(icon(Icon::Timer, 12., t.text_faint))
                .child(segment_text(t, format!("{:.1} ms", s.render_ms)))
                .tooltip(tip(t, "Last frame render time", None)),
        )
        .child(
            row()
                .id("render-device")
                .gap(px(SP_1))
                .flex_shrink_0()
                .max_w(px(device_width))
                .child(icon(Icon::Device, 12., t.text_faint))
                .child(
                    div()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(device_name),
                )
                .tooltip(tip(t, s.device_name.clone(), None)),
        )
        .child(match update {
            Some((label, tone)) => on_command(
                row()
                    .id("footer-updates")
                    .flex_shrink_0()
                    .cursor_pointer()
                    .child(badge(t, label, tone))
                    .tooltip(command_tip(t, Command::ToggleUpdates)),
                Command::ToggleUpdates,
                cx,
            )
            .into_any_element(),
            None => command_icon(
                t,
                "footer-updates",
                Icon::Update,
                Command::ToggleUpdates,
                s.updates.open,
                H_XS,
                cx,
            )
            .into_any_element(),
        })
        .child(command_icon(
            t,
            "footer-help",
            Icon::Keyboard,
            Command::ToggleHelp,
            s.help_open,
            H_XS,
            cx,
        ))
        .into_any_element()
}
