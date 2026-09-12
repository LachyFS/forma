use forma_render::RenderMode;
use glam::Vec2;

const HALF_EXTENT: Vec2 = Vec2::new(234.0, 192.0);
pub(crate) const CARD_HALF_SIZE: Vec2 = Vec2::new(84.0, 26.0);
const DEAD_ZONE: f32 = 30.0;

#[derive(Clone, Copy)]
pub(crate) struct PieChoice {
    pub mode: RenderMode,
    pub offset: Vec2,
    pub key: &'static str,
}

pub(crate) const CHOICES: [PieChoice; 4] = [
    PieChoice {
        mode: RenderMode::Wireframe,
        offset: Vec2::new(-142.0, 0.0),
        key: "4",
    },
    PieChoice {
        mode: RenderMode::Solid,
        offset: Vec2::new(142.0, 0.0),
        key: "6",
    },
    PieChoice {
        mode: RenderMode::MaterialPreview,
        offset: Vec2::new(0.0, 103.0),
        key: "2",
    },
    PieChoice {
        mode: RenderMode::Rendered,
        offset: Vec2::new(0.0, -103.0),
        key: "8",
    },
];

/// Pointer interaction is independent of rendering and scene state. Opening or
/// hovering the pie must not restart viewport accumulation.
pub(crate) struct ShadingPie {
    pub center: Vec2,
    pub hovered: Option<RenderMode>,
    pub trigger_held: bool,
    pub scale: f32,
    origin: Vec2,
}

impl ShadingPie {
    pub fn new(pointer: Vec2, viewport_min: Vec2, viewport_max: Vec2) -> Self {
        let min = viewport_min.min(viewport_max);
        let max = viewport_min.max(viewport_max);
        let size = max - min;
        let scale = (size / (HALF_EXTENT * 2.0)).min_element().clamp(0.0, 1.0);
        let padding = HALF_EXTENT * scale;
        let lower = min + padding;
        let upper = max - padding;
        let clamp_axis = |value: f32, lower: f32, upper: f32| {
            if lower <= upper {
                value.clamp(lower, upper)
            } else {
                (lower + upper) * 0.5
            }
        };
        Self {
            center: Vec2::new(
                clamp_axis(pointer.x, lower.x, upper.x),
                clamp_axis(pointer.y, lower.y, upper.y),
            ),
            hovered: None,
            trigger_held: true,
            scale,
            origin: pointer,
        }
    }

    /// A held Z gesture starts at the real cursor, even when the visible menu
    /// has moved inward to fit. After a tap, ordinary hover uses the drawn menu.
    pub fn pointer_moved(&mut self, pointer: Vec2) -> bool {
        let origin = if self.trigger_held {
            self.origin
        } else {
            self.center
        };
        let delta = pointer - origin;
        let deadzone_squared = (DEAD_ZONE * self.scale).powi(2);
        let hovered = if delta.length_squared() <= deadzone_squared
            || pointer.distance_squared(self.center) <= deadzone_squared
        {
            None
        } else {
            // A visible card takes precedence over a gesture direction. Test
            // the deadzone first so clamping cannot turn tiny motion into a pick.
            self.mode_at(pointer).or_else(|| Some(direction(delta)))
        };
        let changed = self.hovered != hovered;
        self.hovered = hovered;
        changed
    }

    /// Only the first Z release can commit a held gesture. A neutral release
    /// leaves the pie open for pointer or keyboard selection.
    pub fn release_trigger(&mut self) -> Option<RenderMode> {
        if !self.trigger_held {
            return None;
        }
        self.trigger_held = false;
        self.hovered
    }

    pub fn mode_at(&self, pointer: Vec2) -> Option<RenderMode> {
        CHOICES
            .into_iter()
            .find_map(|PieChoice { mode, offset, .. }| {
                let distance = (pointer - (self.center + offset * self.scale)).abs();
                distance
                    .cmple(CARD_HALF_SIZE * self.scale)
                    .all()
                    .then_some(mode)
            })
    }

    pub fn mode_for_key(key: &str) -> Option<RenderMode> {
        let key = match key {
            "left" => "4",
            "right" => "6",
            "down" => "2",
            "up" => "8",
            _ => key,
        };
        CHOICES
            .into_iter()
            .find(|choice| choice.key == key)
            .map(|choice| choice.mode)
    }
}

fn direction(delta: Vec2) -> RenderMode {
    if delta.x.abs() >= delta.y.abs() {
        if delta.x < 0.0 {
            RenderMode::Wireframe
        } else {
            RenderMode::Solid
        }
    } else if delta.y < 0.0 {
        RenderMode::Rendered
    } else {
        RenderMode::MaterialPreview
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn centered() -> ShadingPie {
        ShadingPie::new(
            Vec2::new(500.0, 400.0),
            Vec2::ZERO,
            Vec2::new(1000.0, 800.0),
        )
    }

    #[test]
    fn held_gestures_follow_blender_shading_directions() {
        for (offset, expected) in [
            (Vec2::new(-250.0, 0.0), RenderMode::Wireframe),
            (Vec2::new(250.0, 0.0), RenderMode::Solid),
            (Vec2::new(0.0, 250.0), RenderMode::MaterialPreview),
            (Vec2::new(0.0, -250.0), RenderMode::Rendered),
        ] {
            let mut pie = centered();
            assert!(pie.pointer_moved(pie.center + offset));
            assert_eq!(pie.release_trigger(), Some(expected));
            assert_eq!(pie.release_trigger(), None);
        }
    }

    #[test]
    fn neutral_tap_latches_and_future_releases_do_not_select_hover() {
        let mut pie = centered();
        assert!(!pie.pointer_moved(pie.center + Vec2::new(30.0, 0.0)));
        assert_eq!(pie.release_trigger(), None);
        assert!(!pie.trigger_held);
        pie.pointer_moved(pie.center + Vec2::new(142.0, 0.0));
        assert_eq!(pie.hovered, Some(RenderMode::Solid));
        assert_eq!(pie.release_trigger(), None);
    }

    #[test]
    fn returning_to_deadzone_cancels_the_flick() {
        let mut pie = centered();
        pie.pointer_moved(pie.center + Vec2::new(0.0, -100.0));
        assert!(pie.pointer_moved(pie.center + Vec2::new(3.0, 4.0)));
        assert_eq!(pie.release_trigger(), None);
    }

    #[test]
    fn clamped_menu_does_not_pick_without_a_gesture() {
        for pointer in [
            Vec2::ZERO,
            Vec2::new(1000.0, 0.0),
            Vec2::new(0.0, 800.0),
            Vec2::new(1000.0, 800.0),
        ] {
            let mut pie = ShadingPie::new(pointer, Vec2::ZERO, Vec2::new(1000.0, 800.0));
            assert!(!pie.pointer_moved(pointer));
            assert_eq!(pie.release_trigger(), None);
            assert!((pie.center - HALF_EXTENT).cmpge(Vec2::ZERO).all());
            assert!(
                (pie.center + HALF_EXTENT)
                    .cmple(Vec2::new(1000.0, 800.0))
                    .all()
            );
        }
    }

    #[test]
    fn clamped_gesture_uses_cursor_origin_but_visible_cards_win() {
        let pointer = Vec2::new(0.0, 400.0);
        let mut pie = ShadingPie::new(pointer, Vec2::ZERO, Vec2::new(1000.0, 800.0));
        pie.pointer_moved(pointer + Vec2::new(0.0, -80.0));
        assert_eq!(pie.hovered, Some(RenderMode::Rendered));
        pie.pointer_moved(pie.center + Vec2::new(-142.0, 0.0));
        assert_eq!(pie.hovered, Some(RenderMode::Wireframe));
    }

    #[test]
    fn visible_center_remains_neutral_when_the_menu_is_clamped() {
        let mut pie = ShadingPie::new(Vec2::ZERO, Vec2::ZERO, Vec2::new(1000.0, 800.0));
        pie.pointer_moved(pie.center + Vec2::new(2.0, 1.0));
        assert_eq!(pie.release_trigger(), None);
    }

    #[test]
    fn invocation_outside_viewport_preserves_real_gesture_origin() {
        let pointer = Vec2::new(20.0, 400.0);
        let mut pie = ShadingPie::new(pointer, Vec2::new(230.0, 100.0), Vec2::new(1000.0, 800.0));
        pie.pointer_moved(pointer + Vec2::ONE);
        assert_eq!(pie.release_trigger(), None);
    }

    #[test]
    fn clamping_over_a_card_preserves_deadzone_and_latched_hover() {
        let pointer = Vec2::new(60.0, 400.0);
        let mut pie = ShadingPie::new(pointer, Vec2::ZERO, Vec2::new(1000.0, 800.0));
        assert_eq!(pie.mode_at(pointer), Some(RenderMode::Wireframe));
        pie.pointer_moved(pointer + Vec2::new(2.0, 0.0));
        assert_eq!(pie.release_trigger(), None);
        pie.pointer_moved(pointer + Vec2::new(3.0, 0.0));
        assert_eq!(pie.hovered, Some(RenderMode::Wireframe));
        pie.pointer_moved(pie.center);
        assert_eq!(pie.hovered, None);
    }

    #[test]
    fn small_viewports_scale_the_whole_menu_into_bounds() {
        let min = Vec2::new(200.0, 80.0);
        let max = Vec2::new(434.0, 436.0);
        let pie = ShadingPie::new(max, min, max);
        assert_eq!(pie.scale, 0.5);
        assert!((pie.center - HALF_EXTENT * pie.scale).cmpge(min).all());
        assert!((pie.center + HALF_EXTENT * pie.scale).cmple(max).all());
        assert_eq!(
            pie.mode_at(pie.center + Vec2::new(142.0, 0.0) * pie.scale),
            Some(RenderMode::Solid)
        );
        let empty = ShadingPie::new(Vec2::ZERO, min, min);
        assert_eq!(empty.center, min);
        assert_eq!(empty.scale, 0.0);
    }

    #[test]
    fn number_keys_match_the_pie_cardinal_positions() {
        assert_eq!(ShadingPie::mode_for_key("4"), Some(RenderMode::Wireframe));
        assert_eq!(ShadingPie::mode_for_key("6"), Some(RenderMode::Solid));
        assert_eq!(
            ShadingPie::mode_for_key("2"),
            Some(RenderMode::MaterialPreview)
        );
        assert_eq!(ShadingPie::mode_for_key("8"), Some(RenderMode::Rendered));
        assert_eq!(ShadingPie::mode_for_key("z"), None);
    }
}
