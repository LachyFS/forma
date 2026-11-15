//! Design tokens: one scale for spacing, radii, control heights, type and
//! motion. Components use these names instead of literal values so density and
//! rhythm stay consistent across the shell.
use gpui::{Animation, BoxShadow, FontFeatures, FontWeight, Hsla, Styled, point, px, rgb, rgba};
use std::{sync::LazyLock, time::Duration};

use crate::theme::Colors;

/// 4-pixel spacing scale. `SP_0_5` is the hairline gap inside dense groups.
pub(crate) const SP_0_5: f32 = 2.;
pub(crate) const SP_1: f32 = 4.;
pub(crate) const SP_1_5: f32 = 6.;
pub(crate) const SP_2: f32 = 8.;
pub(crate) const SP_3: f32 = 12.;
pub(crate) const SP_4: f32 = 16.;
pub(crate) const SP_5: f32 = 20.;
pub(crate) const SP_6: f32 = 24.;

/// Corner radii: keycaps and ticks, controls, cards and clusters, dialogs.
pub(crate) const R_XS: f32 = 3.;
pub(crate) const R_SM: f32 = 5.;
pub(crate) const R_MD: f32 = 7.;
pub(crate) const R_LG: f32 = 10.;
pub(crate) const R_XL: f32 = 12.;

/// Control heights. Inspector values and inline buttons use `H_SM`; HUD and
/// title bar buttons `H_MD`; search and dialog actions `H_LG`.
pub(crate) const H_XS: f32 = 20.;
pub(crate) const H_SM: f32 = 24.;
pub(crate) const H_MD: f32 = 28.;
pub(crate) const H_LG: f32 = 32.;

/// Property rows: a fixed label column and a value column.
pub(crate) const ROW_H: f32 = 28.;
pub(crate) const LABEL_W: f32 = 84.;

/// Type scale (Inter). `TEXT_MD` is the body size of the shell.
pub(crate) const TEXT_XS: f32 = 10.;
pub(crate) const TEXT_SM: f32 = 11.;
pub(crate) const TEXT_MD: f32 = 12.;
pub(crate) const TEXT_LG: f32 = 13.;
pub(crate) const TEXT_XL: f32 = 15.;

/// Icon sizes on the 24-unit Lucide grid.
pub(crate) const ICON_SM: f32 = 14.;
pub(crate) const ICON_MD: f32 = 16.;
pub(crate) const ICON_LG: f32 = 18.;

pub(crate) const FONT_UI: &str = "Inter";
pub(crate) const WEIGHT_MEDIUM: FontWeight = FontWeight::MEDIUM;
pub(crate) const WEIGHT_SEMIBOLD: FontWeight = FontWeight::SEMIBOLD;

/// Inter's tabular figures keep columns of numbers aligned while values change.
static TABULAR: LazyLock<FontFeatures> =
    LazyLock::new(|| FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)])));

pub(crate) trait TypeExt: Styled + Sized {
    /// Stretch children across the cross axis (GPUI has no shorthand).
    fn items_stretch(mut self) -> Self {
        self.style().align_items = Some(gpui::AlignItems::Stretch);
        self
    }

    /// Tabular numerals for values, counters and measurements.
    fn tabular(mut self) -> Self {
        self.text_style()
            .get_or_insert_with(Default::default)
            .font_features = Some(TABULAR.clone());
        self
    }
}
impl<T: Styled> TypeExt for T {}

/// Motion: overlays fade and settle quickly enough never to delay input.
pub(crate) const MOTION_FAST: Duration = Duration::from_millis(110);
pub(crate) const MOTION_BASE: Duration = Duration::from_millis(150);

pub(crate) fn enter() -> Animation {
    Animation::new(MOTION_BASE).with_easing(gpui::ease_out_quint())
}
pub(crate) fn enter_fast() -> Animation {
    Animation::new(MOTION_FAST).with_easing(gpui::ease_out_quint())
}

pub(crate) fn hex(color: u32) -> Hsla {
    rgb(color).into()
}
pub(crate) fn hex_alpha(color: u32) -> Hsla {
    rgba(color).into()
}
/// `color` with `alpha` (0–1), for ad-hoc tints of opaque tokens.
pub(crate) fn tint(color: u32, alpha: f32) -> Hsla {
    let mut color = hex(color);
    color.a = alpha;
    color
}

/// Elevation for floating surfaces. Light themes use a softer, tighter shadow.
pub(crate) fn elevation(t: Colors, level: u8) -> Vec<BoxShadow> {
    let shadow = hex_alpha(t.shadow);
    let (y, blur, ambient) = match level {
        0 => (1., 2., 0.),
        1 => (4., 12., 1.),
        2 => (10., 28., 1.),
        _ => (18., 48., 1.),
    };
    let mut shadows = vec![BoxShadow {
        color: shadow,
        offset: point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(0.),
    }];
    if ambient > 0. {
        let mut near = shadow;
        near.a *= 0.6;
        shadows.push(BoxShadow {
            color: near,
            offset: point(px(0.), px(1.)),
            blur_radius: px(2.),
            spread_radius: px(0.),
        });
    }
    shadows
}
