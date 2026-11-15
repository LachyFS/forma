//! Update status and preferences.
use super::super::assets::{Icon, icon};
use super::super::components::*;
use super::super::tokens::*;
use super::{animate_in, backdrop, contain};
use crate::app::{Command, Studio};
use crate::theme::Colors;
use gpui::{prelude::*, *};

pub(super) fn overlay(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> AnyElement {
    use crate::updater::State;
    let locked = s.updates.restart_pending;
    let state = &s.updates.state;
    let (status_icon, status_ink) = match state {
        State::Ready(_) | State::Available(_) => (Icon::Update, t.accent_text),
        State::Downloading(_) => (Icon::Update, t.text_muted),
        _ => (Icon::Success, t.success),
    };
    let mut actions = dialog_footer(t);
    if !locked {
        actions = actions.child(command_button(
            t,
            "update-check",
            "Check now",
            Some(Icon::Update),
            Command::CheckForUpdates,
            Tone::Ghost,
            cx,
        ));
        actions = actions.child(spacer());
        if matches!(state, State::Ready(_) | State::Available(_)) {
            actions = actions.child(command_button(
                t,
                "update-skip",
                "Skip this version",
                None,
                Command::SkipUpdate,
                Tone::Ghost,
                cx,
            ));
        }
        actions = actions.child(command_button(
            t,
            "update-later",
            "Later",
            None,
            Command::ToggleUpdates,
            Tone::Secondary,
            cx,
        ));
        if matches!(state, State::Available(_)) {
            actions = actions.child(command_button(
                t,
                "update-download",
                "Download",
                None,
                Command::DownloadUpdate,
                Tone::Primary,
                cx,
            ));
        }
        if matches!(state, State::Ready(_)) {
            actions = actions.child(command_button(
                t,
                "update-restart",
                "Restart to update",
                None,
                Command::RestartUpdate,
                Tone::Primary,
                cx,
            ));
        }
    }
    let body = col()
        .px(px(SP_4))
        .pb(px(SP_4))
        .gap(px(SP_3))
        .child(
            row()
                .p(px(SP_3))
                .gap(px(SP_2 + 2.))
                .rounded(px(R_MD))
                .bg(rgb(t.raised))
                .child(icon(status_icon, ICON_MD, status_ink))
                .child(div().flex_1().text_color(rgb(t.text)).child(if locked {
                    "Saving and preparing to restart…".into()
                } else {
                    state.label()
                })),
        )
        .when(!locked, |d| {
            d.child(switch_row(
                t,
                "update-auto-check",
                "Check automatically",
                Some("Look for new releases in the background"),
                s.updates.preferences.automatic_checks,
                Command::ToggleUpdateChecks,
                cx,
            ))
            .child(switch_row(
                t,
                "update-auto-download",
                "Download automatically",
                Some("Updates install only when you choose to restart"),
                s.updates.preferences.automatic_downloads,
                Command::ToggleUpdateDownloads,
                cx,
            ))
            .child(
                caption(t, "Your projects and preferences are kept across updates.")
                    .text_color(rgb(t.text_faint)),
            )
        });
    backdrop(t, "updates-backdrop", true, |_, _, _| {}, cx)
        .flex()
        .items_center()
        .justify_center()
        .child(animate_in(
            contain(
                dialog(t)
                    .id("updates")
                    .w(px(480.))
                    .max_w(relative(0.92))
                    .child(dialog_header(
                        t,
                        Some(Icon::Update),
                        "Forma updates",
                        Some(format!("Version {}", env!("CARGO_PKG_VERSION")).into()),
                    ))
                    .child(body)
                    .when(!locked, |d| d.child(actions)),
            ),
            "updates-enter",
        ))
        .into_any_element()
}
