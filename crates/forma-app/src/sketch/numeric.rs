//! Typed dimensions while drawing. Each tool step exposes up to two fields
//! (length and angle, width and height, ...); typing fills the active field
//! and Tab moves to the next. A filled field locks that dimension while the
//! others keep following the pointer. Values use the app's expression parser:
//! `25mm`, `(2+3)*10cm`, `1ft+6in`, `45` (degrees) or `0.5rad`.
use super::tools::{Dimension, Locks};

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Numeric {
    pub fields: [String; 2],
    pub active: usize,
}

impl Numeric {
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.fields.iter().all(String::is_empty)
    }

    pub(crate) fn active_text(&self) -> &str {
        &self.fields[self.active.min(1)]
    }

    pub(crate) fn push(&mut self, text: &str) {
        let field = &mut self.fields[self.active.min(1)];
        if field.len() + text.len() <= 128 {
            field.push_str(text);
        }
    }

    pub(crate) fn erase(&mut self, erase: crate::text_entry::Erase) -> bool {
        let mut replace = false;
        crate::text_entry::erase(&mut self.fields[self.active.min(1)], &mut replace, erase)
    }

    /// Moves to the next (or previous) of `count` fields.
    pub(crate) fn advance(&mut self, count: usize, backwards: bool) {
        if count > 1 {
            self.active = if backwards {
                (self.active + count - 1) % count
            } else {
                (self.active + 1) % count
            };
        }
    }

    /// The parsed value of a field for its dimension, or `None` when empty.
    /// `Err` marks unparseable text.
    pub(crate) fn value(&self, index: usize, dimension: Dimension) -> Result<Option<f32>, ()> {
        let text = self.fields[index].trim();
        if text.is_empty() {
            return Ok(None);
        }
        let parsed = if dimension.is_angle() {
            crate::transform::angle_value(text)
        } else {
            crate::transform::distance_value(text)
        };
        parsed.map(Some).ok_or(())
    }

    /// Locks for the step's dimensions; invalid fields stay unlocked and are
    /// reported in the second value.
    pub(crate) fn locks(&self, dimensions: &[Dimension]) -> (Locks, bool) {
        let mut locks = [None, None];
        let mut valid = true;
        for (index, dimension) in dimensions.iter().enumerate().take(2) {
            match self.value(index, *dimension) {
                Ok(value) => locks[index] = value,
                Err(()) => valid = false,
            }
        }
        (locks, valid)
    }
}

/// Characters that start a number (others only continue one, such as unit
/// letters, which would otherwise be tool shortcuts).
pub(crate) fn starts_number(c: char) -> bool {
    c.is_ascii_digit() || ".-(".contains(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_parse_units_and_expressions() {
        let mut numeric = Numeric::default();
        numeric.push("25mm");
        numeric.advance(2, false);
        numeric.push("45");
        let (locks, valid) = numeric.locks(&[Dimension::Length, Dimension::Angle]);
        assert!(valid);
        assert!((locks[0].unwrap() - 0.025).abs() < 1.0e-6);
        assert!((locks[1].unwrap() - 45f32.to_radians()).abs() < 1.0e-6);
        numeric.clear();
        numeric.push("1ft+6in");
        let (locks, _) = numeric.locks(&[Dimension::Width]);
        assert!((locks[0].unwrap() - 0.4572).abs() < 1.0e-5);
        numeric.push("+");
        let (locks, valid) = numeric.locks(&[Dimension::Width]);
        assert!(!valid && locks[0].is_none());
        assert!(numeric.erase(crate::text_entry::Erase::Backspace));
        assert_eq!(numeric.active_text(), "1ft+6in");
    }
}
