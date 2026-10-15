//! Pure input math for modal modelling tools, independent of window dispatch.
use glam::{Vec2, Vec3};

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

/// Numeric input accepts unit suffixes without treating them as tool shortcuts.
/// Axis keys remain reserved for constraints during modal operations.
pub fn numeric_input_char(c: char) -> bool {
    c.is_ascii_digit() || ".+-*/() ".contains(c) || "eEmMcCiInNfFtTdDgGrRaA".contains(c)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NumericContext {
    Scalar,
    Distance,
    Angle,
}

#[derive(Clone, Copy, Debug)]
struct Quantity {
    value: f64,
    // Dimensions are carried through multiplication and division so expressions
    // such as `2cm * 3cm` cannot accidentally become a linear distance.
    length: i16,
    angle: i16,
}

impl Quantity {
    fn scalar(value: f64) -> Self {
        Self {
            value,
            length: 0,
            angle: 0,
        }
    }
    fn finite(self) -> Option<Self> {
        self.value.is_finite().then_some(self)
    }
}

struct NumericParser<'a> {
    bytes: &'a [u8],
    cursor: usize,
    context: NumericContext,
    has_units: bool,
}

impl NumericParser<'_> {
    fn skip_space(&mut self) {
        while self
            .bytes
            .get(self.cursor)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.cursor += 1;
        }
    }

    fn consume(&mut self, c: u8) -> bool {
        self.skip_space();
        if self.bytes.get(self.cursor) == Some(&c) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    fn expression(&mut self, depth: usize) -> Option<Quantity> {
        let mut value = self.product(depth)?;
        loop {
            let subtract = if self.consume(b'+') {
                false
            } else if self.consume(b'-') {
                true
            } else {
                break;
            };
            let rhs = self.product(depth)?;
            if (value.length, value.angle) != (rhs.length, rhs.angle) {
                return None;
            }
            value.value += if subtract { -rhs.value } else { rhs.value };
            value = value.finite()?;
        }
        Some(value)
    }

    fn product(&mut self, depth: usize) -> Option<Quantity> {
        let mut value = self.primary(depth)?;
        loop {
            let divide = if self.consume(b'*') {
                false
            } else if self.consume(b'/') {
                true
            } else {
                break;
            };
            let rhs = self.primary(depth)?;
            if divide {
                value.value /= rhs.value;
                value.length = value.length.checked_sub(rhs.length)?;
                value.angle = value.angle.checked_sub(rhs.angle)?;
            } else {
                value.value *= rhs.value;
                value.length = value.length.checked_add(rhs.length)?;
                value.angle = value.angle.checked_add(rhs.angle)?;
            }
            value = value.finite()?;
        }
        Some(value)
    }

    fn primary(&mut self, depth: usize) -> Option<Quantity> {
        if depth > 32 {
            return None;
        }
        self.skip_space();
        let mut sign = 1.;
        loop {
            if self.consume(b'-') {
                sign = -sign;
            } else if !self.consume(b'+') {
                break;
            }
        }
        let mut value = if self.consume(b'(') {
            let value = self.expression(depth + 1)?;
            if !self.consume(b')') {
                return None;
            }
            value
        } else {
            self.skip_space();
            let start = self.cursor;
            while self.bytes.get(self.cursor).is_some_and(u8::is_ascii_digit) {
                self.cursor += 1;
            }
            if self.bytes.get(self.cursor) == Some(&b'.') {
                self.cursor += 1;
                while self.bytes.get(self.cursor).is_some_and(u8::is_ascii_digit) {
                    self.cursor += 1;
                }
            }
            if self
                .bytes
                .get(self.cursor)
                .is_some_and(|c| *c == b'e' || *c == b'E')
            {
                self.cursor += 1;
                if self
                    .bytes
                    .get(self.cursor)
                    .is_some_and(|c| *c == b'+' || *c == b'-')
                {
                    self.cursor += 1;
                }
                let exponent_start = self.cursor;
                while self.bytes.get(self.cursor).is_some_and(u8::is_ascii_digit) {
                    self.cursor += 1;
                }
                if exponent_start == self.cursor {
                    return None;
                }
            }
            Quantity::scalar(
                std::str::from_utf8(&self.bytes[start..self.cursor])
                    .ok()?
                    .parse()
                    .ok()?,
            )
        };
        self.skip_space();
        let unit_start = self.cursor;
        while self
            .bytes
            .get(self.cursor)
            .is_some_and(u8::is_ascii_alphabetic)
        {
            self.cursor += 1;
        }
        if unit_start != self.cursor {
            let unit = std::str::from_utf8(&self.bytes[unit_start..self.cursor])
                .ok()?
                .to_ascii_lowercase();
            let (factor, dimension) = unit_info(&unit)?;
            if dimension != self.context || value.length != 0 || value.angle != 0 {
                return None;
            }
            self.has_units = true;
            value.value *= factor;
            value.length = i16::from(dimension == NumericContext::Distance);
            value.angle = i16::from(dimension == NumericContext::Angle);
        }
        value.value *= sign;
        value.finite()
    }
}

fn unit_info(unit: &str) -> Option<(f64, NumericContext)> {
    Some(match unit {
        "mm" => (0.001, NumericContext::Distance),
        "cm" => (0.01, NumericContext::Distance),
        "m" => (1., NumericContext::Distance),
        "in" => (0.0254, NumericContext::Distance),
        "ft" => (0.3048, NumericContext::Distance),
        "deg" => (std::f64::consts::PI / 180., NumericContext::Angle),
        "rad" => (1., NumericContext::Angle),
        _ => return None,
    })
}

fn finite_f32(value: f64) -> Option<f32> {
    let result = value as f32;
    (result.is_finite() && (value == 0. || result != 0.)).then_some(result)
}

fn parse_numeric(input: &str, context: NumericContext) -> Option<f32> {
    if input.len() > 4096 {
        return None;
    }
    // CAD-style fractional entry: a sole trailing unit qualifies the entire
    // unitless expression, so `1/8in` is an eighth of an inch. Expressions
    // containing multiple unit suffixes use dimensional arithmetic below.
    let trimmed = input.trim_end();
    let suffix_start = trimmed
        .trim_end_matches(|c: char| c.is_ascii_alphabetic())
        .len();
    if context != NumericContext::Scalar && suffix_start < trimmed.len() {
        let suffix = trimmed[suffix_start..].to_ascii_lowercase();
        if let Some((factor, dimension)) = unit_info(&suffix) {
            let prefix = &trimmed[..suffix_start];
            let mut scalar = NumericParser {
                bytes: prefix.as_bytes(),
                cursor: 0,
                context: NumericContext::Scalar,
                has_units: false,
            };
            if let Some(value) = scalar.expression(0) {
                scalar.skip_space();
                if scalar.cursor == prefix.len() && !scalar.has_units {
                    return (dimension == context)
                        .then(|| finite_f32(value.value * factor))
                        .flatten();
                }
            }
        }
    }
    let mut parser = NumericParser {
        bytes: input.as_bytes(),
        cursor: 0,
        context,
        has_units: false,
    };
    let mut result = parser.expression(0)?;
    parser.skip_space();
    if parser.cursor != input.len() {
        return None;
    }
    if parser.has_units {
        let expected = match context {
            NumericContext::Scalar => (0, 0),
            NumericContext::Distance => (1, 0),
            NumericContext::Angle => (0, 1),
        };
        if (result.length, result.angle) != expected {
            return None;
        }
    } else if context == NumericContext::Angle {
        // Bare rotation entries retain the application's degree convention.
        result.value = result.value.to_radians();
    }
    finite_f32(result.value)
}

/// Exact dimensionless arithmetic, including parentheses and scientific notation.
/// Explicit units are rejected in scalar fields such as scale and roughness.
pub fn numeric_value(input: &str) -> Option<f32> {
    parse_numeric(input, NumericContext::Scalar)
}

/// Exact distance in metres. Bare expressions are metres; explicit terms must
/// use compatible distance units, e.g. `1/8in`, `(2 + 3)cm`, `1ft + 6in`.
pub fn distance_value(input: &str) -> Option<f32> {
    parse_numeric(input, NumericContext::Distance)
}

/// Exact angle in radians. Bare expressions are degrees; `deg` and `rad` can
/// be mixed in an explicitly dimensioned expression, e.g. `90deg + 1rad`.
pub fn angle_value(input: &str) -> Option<f32> {
    parse_numeric(input, NumericContext::Angle)
}

/// Compact inspector text. Editing uses the original value separately, so
/// rounding here never changes a transform and tiny distances remain visible.
pub fn format_precision_value(value: f32) -> String {
    if !value.is_finite() {
        return "—".into();
    }
    if value == 0. {
        return "0".into();
    }
    let trim = |text: &str| text.trim_end_matches('0').trim_end_matches('.').to_owned();
    if value.abs() < 0.0001 || value.abs() >= 1_000_000. {
        let scientific = format!("{value:.4e}");
        let (mantissa, exponent) = scientific.split_once('e').unwrap();
        format!("{}e{exponent}", trim(mantissa))
    } else {
        trim(&format!("{value:.4}"))
    }
}

/// Local mesh bounding-box dimensions after the object's own scale. Rotation,
/// parenting and the parent's transform do not affect these inspector values.
pub fn local_dimensions(positions: &[Vec3], scale: Vec3) -> Option<Vec3> {
    if positions.is_empty() || !scale.is_finite() || positions.iter().any(|p| !p.is_finite()) {
        return None;
    }
    let min = positions
        .iter()
        .copied()
        .fold(Vec3::splat(f32::INFINITY), Vec3::min);
    let max = positions
        .iter()
        .copied()
        .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
    let dimensions = (max - min) * scale.abs();
    dimensions.is_finite().then_some(dimensions)
}

/// Return the signed scale needed to reach an exact positive local dimension.
/// No mutation occurs on failure, including planar/empty geometry and scales
/// that cannot be represented by a valid scene transform.
pub fn scale_for_dimension(
    positions: &[Vec3],
    scale: Vec3,
    axis: usize,
    dimension: f32,
) -> Result<f32, &'static str> {
    if axis > 2 || !dimension.is_finite() || dimension <= 0. {
        return Err("Dimension must be a positive distance");
    }
    let extent =
        local_dimensions(positions, Vec3::ONE).ok_or("The mesh has no finite dimensions")?[axis];
    if extent <= 0. {
        return Err("This axis has no thickness; edit the mesh to add geometry first");
    }
    let magnitude = dimension / extent;
    if !magnitude.is_finite()
        || !(1e-5..=1e6).contains(&magnitude)
        || !scale[axis].is_finite()
        || scale[axis] == 0.
    {
        return Err("Dimension would exceed the supported object scale range (0.00001 to 1000000)");
    }
    Ok(magnitude.copysign(scale[axis]))
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
        for (input, value) in [
            ("1/8", 0.125),
            ("2+3*4", 14.),
            ("-2*-3", 6.),
            (".25", 0.25),
            (" 2 * (3 + 4) / 7 ", 2.),
            ("-(2+3)*-2", 10.),
            ("1e-3 + 2E-3", 0.003),
            ("(1 + 2) / (4 - 2)", 1.5),
        ] {
            assert_eq!(numeric_value(input), Some(value));
        }
        for input in [
            "", "-", "2/", "1/0", "2..3", "2a", "1e999", "1e", "1e-", "(2+3", "2+3)", "2(3)",
            "1 2", "NaN", "inf", "2cm", "90deg", "1e-50",
        ] {
            assert_eq!(numeric_value(input), None);
        }
    }
    #[test]
    fn distances_convert_units_and_preserve_dimensional_arithmetic() {
        for (input, expected) in [
            ("25mm", 0.025),
            ("2.5 cm", 0.025),
            ("1m/8", 0.125),
            ("(2+3)cm", 0.05),
            ("2 * (3cm + 4mm)", 0.068),
            ("1ft + 6in", 0.4572),
            ("1/8in", 0.003175),
            ("2+3cm", 0.05),
            ("2E2MM", 0.2),
            ("1m / 2m * 4cm", 0.02),
            ("-2.5e-2m", -0.025),
            ("1 / 8", 0.125),
        ] {
            assert!(
                (distance_value(input).unwrap() - expected).abs() < 1e-6,
                "{input}"
            );
        }
        for input in [
            "2rad",
            "90deg",
            "1m + 2",
            "2cm * 3cm",
            "1m / 2m",
            "2mcm",
            "1m / (2cm)",
            "(2cm)m",
        ] {
            assert_eq!(distance_value(input), None, "{input}");
        }
    }

    #[test]
    fn angles_default_to_degrees_but_allow_explicit_radians() {
        for (input, expected) in [
            ("90", std::f32::consts::FRAC_PI_2),
            ("(180 / 2)deg", std::f32::consts::FRAC_PI_2),
            ("1.57079632679rad", std::f32::consts::FRAC_PI_2),
            ("180deg + 1rad", std::f32::consts::PI + 1.),
            ("-2 * 45deg", -std::f32::consts::FRAC_PI_2),
        ] {
            assert!(
                (angle_value(input).unwrap() - expected).abs() < 1e-6,
                "{input}"
            );
        }
        for input in ["1m", "1in", "90deg + 2", "2rad * 3rad"] {
            assert_eq!(angle_value(input), None, "{input}");
        }
    }

    #[test]
    fn numeric_input_supports_every_documented_expression_and_reserves_axes() {
        assert!(
            "(2.5e-3m + 4cm) / 2 90deg 1RAD 6in 1ft"
                .chars()
                .all(numeric_input_char)
        );
        for c in "xyzXYZ;\n".chars() {
            assert!(!numeric_input_char(c));
        }
        assert_eq!(
            numeric_value(&format!("{}1{}", "(".repeat(34), ")".repeat(34))),
            None
        );
    }

    #[test]
    fn inspector_precision_does_not_hide_tiny_distances_or_signs() {
        assert_eq!(format_precision_value(69.900856), "69.9009");
        assert_eq!(format_precision_value(-13.17803), "-13.178");
        assert_eq!(format_precision_value(0.025), "0.025");
        assert_eq!(format_precision_value(2.), "2");
        assert_eq!(format_precision_value(-0.), "0");
        for value in [0.00001, -0.00001, 1e-12, -1e-12, 1e8] {
            let display = format_precision_value(value);
            let parsed = numeric_value(&display).unwrap();
            assert_ne!(parsed, 0., "Tiny dimensions must not appear as zero");
            assert_eq!(parsed.is_sign_negative(), value.is_sign_negative());
            assert!((parsed - value).abs() <= value.abs() * 0.0001);
            assert!(display.len() <= 12);
        }
    }

    #[test]
    fn exact_dimensions_preserve_mirrors_and_handle_offset_meshes() {
        let positions = [Vec3::new(10., -3., 2.), Vec3::new(12., 1., 3.)];
        let scale = Vec3::new(-2., 0.5, 3.);
        assert_eq!(
            local_dimensions(&positions, scale),
            Some(Vec3::new(4., 2., 3.))
        );
        let x = scale_for_dimension(&positions, scale, 0, 0.25).unwrap();
        assert_eq!(x, -0.125);
        let updated = Vec3::new(x, scale.y, scale.z);
        assert_eq!(local_dimensions(&positions, updated).unwrap().x, 0.25);
        assert_eq!(scale_for_dimension(&positions, scale, 1, 8.), Ok(2.));
    }

    #[test]
    fn exact_dimensions_reject_flat_invalid_or_unrepresentable_targets() {
        let positions = [Vec3::ZERO, Vec3::new(1., 1., 0.)];
        assert!(scale_for_dimension(&positions, Vec3::ONE, 2, 1.).is_err());
        for dimension in [0., -1., f32::NAN, f32::INFINITY, 1e-8, 1e8] {
            assert!(scale_for_dimension(&positions, Vec3::ONE, 0, dimension).is_err());
        }
        assert!(scale_for_dimension(&positions, Vec3::ONE, 3, 1.).is_err());
        assert!(scale_for_dimension(&[], Vec3::ONE, 0, 1.).is_err());
        assert_eq!(local_dimensions(&[Vec3::NAN], Vec3::ONE), None);
        assert_eq!(local_dimensions(&[], Vec3::ONE), None);
    }
}
