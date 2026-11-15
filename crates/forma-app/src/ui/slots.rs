//! Reserved viewport slots. Features that need persistent controls over the
//! viewport fill these hooks instead of adding toolbars to the shell.
//!
//! Both slots are positioned, styled and isolated from viewport input by the
//! shell (`hud::viewport_area`): return content only, sized to fit, and use
//! `components::hud` for the floating surface so it matches the other
//! clusters. Returning `None` hides the slot entirely.
//!
//! - [`viewport_dock`] sits bottom-center, below any active-operation readout.
//!   The Draw toolbar, CAD tool HUDs and the feature timeline share it (see
//!   its priority list). Keep it to one row of `tokens::H_MD` controls; it may
//!   be as wide as the viewport allows.
//! - [`adjust_last_operation`] sits bottom-left. Show it after an operation
//!   completes to tweak its parameters (extrude distance, bevel width,
//!   primitive size) and hide it when the selection or tool changes. Keep it
//!   at most ~260 px wide; build rows with `components::numeric_row`.
use crate::app::Studio;
use crate::theme::Colors;
use gpui::{AnyElement, Context};

/// Bottom-center dock for the feature timeline and mode toolbars. One
/// occupant at a time, by priority:
///
/// 1. the Draw toolbar while a sketch is open;
/// 2. an active CAD tool's HUD (`cad_tools`; add its branch here, e.g.
///    `if let Some(hud) = studio.cad_tool_hud(t, cx) { return Some(hud); }`);
/// 3. the feature timeline whenever a part object is active.
pub(crate) fn viewport_dock(
    studio: &Studio,
    t: Colors,
    cx: &mut Context<Studio>,
) -> Option<AnyElement> {
    if let Some(toolbar) = studio.draw_toolbar(t, cx) {
        return Some(toolbar);
    }
    // A running modelling tool's HUD sits between the Draw toolbar and the
    // timeline.
    if let Some(hud) = studio.cad_tool_hud(t, cx) {
        return Some(hud);
    }
    super::timeline::timeline(studio, t, cx)
}

/// Bottom-left "adjust last operation" panel.
pub(crate) fn adjust_last_operation(
    studio: &Studio,
    t: Colors,
    cx: &mut Context<Studio>,
) -> Option<AnyElement> {
    studio.adjust_panel(t, cx)
}
