//! Reserved viewport slots. Features that need persistent controls over the
//! viewport fill these hooks instead of adding toolbars to the shell.
//!
//! Both slots are positioned, styled and isolated from viewport input by the
//! shell (`hud::viewport_area`): return content only, sized to fit, and use
//! `components::hud` for the floating surface so it matches the other
//! clusters. Returning `None` hides the slot entirely.
//!
//! - [`viewport_dock`] sits bottom-center, below any active-operation readout.
//!   The CAD feature timeline and the Draw toolbar belong here. Keep it to one
//!   row of `tokens::H_MD` controls; it may be as wide as the viewport allows.
//! - [`adjust_last_operation`] sits bottom-left. Show it after an operation
//!   completes to tweak its parameters (extrude distance, bevel width,
//!   primitive size) and hide it when the selection or tool changes. Keep it
//!   at most ~260 px wide; build rows with `components::numeric_row`.
use crate::app::Studio;
use crate::theme::Colors;
use gpui::{AnyElement, Context};

/// Bottom-center dock for the feature timeline and mode toolbars.
pub(crate) fn viewport_dock(
    studio: &Studio,
    t: Colors,
    cx: &mut Context<Studio>,
) -> Option<AnyElement> {
    studio.draw_toolbar(t, cx)
}

/// Bottom-left "adjust last operation" panel.
pub(crate) fn adjust_last_operation(
    _studio: &Studio,
    _t: Colors,
    _cx: &mut Context<Studio>,
) -> Option<AnyElement> {
    None
}
