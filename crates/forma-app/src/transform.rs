//! Pure input math for modal modelling tools, independent of window dispatch.
use glam::Vec2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Orientation {
    #[default]
    Global,
    Local,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Constraint {
    pub axis: Option<usize>,
    pub plane: bool,
    pub orientation: Orientation,
}

impl Constraint {
    pub fn cycle(&mut self, axis: usize, plane: bool) {
        if self.axis != Some(axis) || self.plane != plane {
            *self = Self {
                axis: Some(axis),
                plane,
                orientation: Orientation::Global,
            };
        } else if self.orientation == Orientation::Global {
            self.orientation = Orientation::Local;
        } else {
            *self = Self::default();
        }
    }

    pub fn label(self) -> String {
        self.axis
            .map(|axis| {
                format!(
                    "{:?} {}{}",
                    self.orientation,
                    if self.plane { "plane excluding " } else { "" },
                    ["X", "Y", "Z"][axis]
                )
            })
            .unwrap_or_else(|| "Free".into())
    }
}

/// Integrating precision motion prevents the selection jumping when Shift is
/// pressed or released halfway through a drag.
#[derive(Clone, Copy, Debug)]
pub struct PointerMotion {
    pub start: Vec2,
    pub previous: Vec2,
    pub effective: Vec2,
    pub angle: f32,
}

impl PointerMotion {
    pub fn new(start: Vec2) -> Self {
        Self {
            start,
            previous: start,
            effective: start,
            angle: 0.,
        }
    }
    pub fn update(&mut self, pointer: Vec2, pivot: Vec2, precise: bool) {
        let previous = self.effective - pivot;
        self.effective += (pointer - self.previous) * if precise { 0.1 } else { 1. };
        self.previous = pointer;
        let next = self.effective - pivot;
        if previous.length() > 4. && next.length() > 4. {
            self.angle += -previous.perp_dot(next).atan2(previous.dot(next));
        }
    }
    pub fn scale(self, pivot: Vec2) -> f32 {
        let initial = self.start - pivot;
        if initial.length() < 8. {
            ((self.effective.x - self.start.x - self.effective.y + self.start.y) * 0.007).exp()
        } else {
            let current = self.effective - pivot;
            current.length() / initial.length() * if current.dot(initial) < 0. { -1. } else { 1. }
        }
    }
}

pub fn snap(value: f32, increment: f32) -> f32 {
    (value / increment).round() * increment
}

/// Small arithmetic parser for exact transforms (e.g. `1/8`, `2*3`, `-0.25`).
/// Invalid or unfinished expressions never silently revert to pointer input.
pub fn numeric_value(input: &str) -> Option<f32> {
    fn expression(bytes: &[u8], index: &mut usize, precedence: u8) -> Option<f64> {
        let mut value = if precedence == 2 {
            let negative = bytes.get(*index) == Some(&b'-');
            if negative || bytes.get(*index) == Some(&b'+') {
                *index += 1;
            }
            let start = *index;
            while bytes
                .get(*index)
                .is_some_and(|c| c.is_ascii_digit() || *c == b'.')
            {
                *index += 1;
            }
            let number = std::str::from_utf8(&bytes[start..*index])
                .ok()?
                .parse::<f64>()
                .ok()?;
            if negative { -number } else { number }
        } else {
            expression(bytes, index, precedence + 1)?
        };
        while let Some(&operator) = bytes.get(*index) {
            if !(if precedence == 0 {
                b"+-".contains(&operator)
            } else if precedence == 1 {
                b"*/".contains(&operator)
            } else {
                false
            }) {
                break;
            }
            *index += 1;
            let rhs = expression(bytes, index, precedence + 1)?;
            value = match operator {
                b'+' => value + rhs,
                b'-' => value - rhs,
                b'*' => value * rhs,
                b'/' => value / rhs,
                _ => return None,
            };
        }
        Some(value)
    }
    let mut index = 0;
    let value = expression(input.as_bytes(), &mut index, 0)? as f32;
    (index == input.len() && value.is_finite()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn axis_cycles_global_local_free_and_plane_restarts_cycle() {
        let mut c = Constraint::default();
        c.cycle(0, false);
        assert_eq!(c.axis, Some(0));
        c.cycle(0, false);
        assert_eq!(c.orientation, Orientation::Local);
        c.cycle(0, false);
        assert_eq!(c.axis, None);
        c.cycle(2, true);
        assert!(c.plane);
        c.cycle(1, false);
        assert!(!c.plane);
        assert_eq!(c.orientation, Orientation::Global);
    }
    #[test]
    fn precision_motion_is_continuous_and_rotation_tracks_pivot() {
        let mut p = PointerMotion::new(Vec2::new(100., 0.));
        p.update(Vec2::new(120., 0.), Vec2::ZERO, false);
        p.update(Vec2::new(120., 0.), Vec2::ZERO, true);
        assert_eq!(p.effective, Vec2::new(120., 0.));
        p.update(Vec2::new(130., 0.), Vec2::ZERO, true);
        assert_eq!(p.effective, Vec2::new(121., 0.));
        p.update(Vec2::new(130., 0.), Vec2::ZERO, false);
        assert_eq!(p.effective, Vec2::new(121., 0.));
        let mut p = PointerMotion::new(Vec2::X * 100.);
        p.update(Vec2::Y * -100., Vec2::ZERO, false);
        assert!((p.angle - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert!((p.scale(Vec2::ZERO).abs() - 1.).abs() < 1e-6);
    }
    #[test]
    fn arithmetic_obeys_precedence_and_rejects_invalid_input() {
        for (input, value) in [("1/8", 0.125), ("2+3*4", 14.), ("-2*-3", 6.), (".25", 0.25)] {
            assert_eq!(numeric_value(input), Some(value));
        }
        for input in ["", "-", "2/", "1/0", "2..3", "2a", "1e999"] {
            assert_eq!(numeric_value(input), None);
        }
    }
}
