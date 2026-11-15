//! Editable feature parameters. Every scalar the inspector shows for a
//! feature is one [`FeatureField`], read and written here as plain data so
//! the inspector, scrubbing, typing and tests share one set of rules.
//!
//! Values are in display units: metres for lengths and degrees for angles.
use forma_core::{
    ExtrudeExtent, Feature, FeatureKind, MAX_PART_LENGTH, MIN_PART_LENGTH, PrimitiveShape,
};

/// One editable value of the selected feature.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FeatureField {
    /// The feature's name (text).
    Name,
    /// Box size along an axis.
    Size(usize),
    /// Cylinder, sphere or cone (bottom) radius.
    Radius,
    /// Cone top radius; zero makes a point.
    TopRadius,
    Height,
    /// Torus ring radius.
    MajorRadius,
    /// Torus tube radius.
    MinorRadius,
    /// Primitive placement origin along an axis.
    Origin(usize),
    /// Primitive placement rotation about an axis, in degrees.
    Rotation(usize),
    /// Extrude, push/pull or chamfer distance.
    Distance,
    /// Second extrude distance, for two-sided extrusions.
    BackDistance,
    /// Revolve sweep in degrees.
    Angle,
}

/// How a field's text is parsed and shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Text,
    Length,
    Angle,
}

impl FeatureField {
    /// Tab order inside the inspector's feature section.
    pub const ORDER: [FeatureField; 18] = [
        FeatureField::Name,
        FeatureField::Size(0),
        FeatureField::Size(1),
        FeatureField::Size(2),
        FeatureField::Radius,
        FeatureField::TopRadius,
        FeatureField::Height,
        FeatureField::MajorRadius,
        FeatureField::MinorRadius,
        FeatureField::Distance,
        FeatureField::BackDistance,
        FeatureField::Angle,
        FeatureField::Origin(0),
        FeatureField::Origin(1),
        FeatureField::Origin(2),
        FeatureField::Rotation(0),
        FeatureField::Rotation(1),
        FeatureField::Rotation(2),
    ];

    pub fn unit(self) -> Unit {
        match self {
            Self::Name => Unit::Text,
            Self::Rotation(_) | Self::Angle => Unit::Angle,
            _ => Unit::Length,
        }
    }

    /// Display units per pointer pixel when scrubbing, and typed decimals.
    pub fn scrub_step(self) -> (f32, usize) {
        match self.unit() {
            Unit::Text => (0., 0),
            Unit::Length => (0.005, 3),
            Unit::Angle => (0.5, 1),
        }
    }

    /// Status-line help while the field is being typed into.
    pub fn entry_hint(self) -> &'static str {
        match self.unit() {
            Unit::Text => "Feature name · Enter apply · Esc cancel",
            Unit::Length => "Length · m / cm / mm / in / ft · Enter apply · Esc cancel",
            Unit::Angle => {
                "Angle · Degrees by default, or use deg / rad · Enter apply · Esc cancel"
            }
        }
    }

    /// Parse typed text into a display value.
    pub fn parse(self, text: &str) -> Option<f32> {
        match self.unit() {
            Unit::Text => None,
            Unit::Length => crate::transform::distance_value(text),
            Unit::Angle => crate::transform::angle_value(text).map(f32::to_degrees),
        }
    }

    /// Example input for an error message.
    pub fn example(self) -> &'static str {
        match self.unit() {
            Unit::Text => "Use a name between 1 and 128 bytes",
            Unit::Length => "Enter a distance expression, e.g. 25mm, 1/8in or 1m+2cm",
            Unit::Angle => "Enter an angle expression, e.g. 90, 45deg or 1.57rad",
        }
    }
}

/// Whether `feature` has an editable value for `field`.
pub fn applies(feature: &Feature, field: FeatureField) -> bool {
    if field == FeatureField::Name {
        return true;
    }
    value(feature, field).is_some()
}

/// The current display value of `field`, if the feature has one.
pub fn value(feature: &Feature, field: FeatureField) -> Option<f32> {
    use FeatureField as F;
    match (&feature.kind, field) {
        (FeatureKind::Primitive(primitive), field) => match (primitive.shape, field) {
            (_, F::Origin(axis)) if axis < 3 => Some(primitive.placement.origin[axis]),
            (_, F::Rotation(axis)) if axis < 3 => {
                Some(primitive.placement.rotation[axis].to_degrees())
            }
            (PrimitiveShape::Box { size }, F::Size(axis)) if axis < 3 => Some(size[axis]),
            (PrimitiveShape::Cylinder { radius, .. }, F::Radius) => Some(radius),
            (PrimitiveShape::Cylinder { height, .. }, F::Height) => Some(height),
            (PrimitiveShape::Sphere { radius }, F::Radius) => Some(radius),
            (PrimitiveShape::Cone { bottom_radius, .. }, F::Radius) => Some(bottom_radius),
            (PrimitiveShape::Cone { top_radius, .. }, F::TopRadius) => Some(top_radius),
            (PrimitiveShape::Cone { height, .. }, F::Height) => Some(height),
            (PrimitiveShape::Torus { major_radius, .. }, F::MajorRadius) => Some(major_radius),
            (PrimitiveShape::Torus { minor_radius, .. }, F::MinorRadius) => Some(minor_radius),
            _ => None,
        },
        (FeatureKind::Extrude(extrude), F::Distance) => Some(extrude.distance),
        (FeatureKind::Extrude(extrude), F::BackDistance) => match extrude.extent {
            ExtrudeExtent::TwoSides { back_distance } => Some(back_distance),
            _ => None,
        },
        (FeatureKind::Revolve(revolve), F::Angle) => Some(revolve.angle.to_degrees()),
        (FeatureKind::PushPull(push), F::Distance) => Some(push.distance),
        (FeatureKind::Chamfer(chamfer), F::Distance) => Some(chamfer.distance),
        _ => None,
    }
}

fn length(label: &str, value: f32) -> Result<f32, String> {
    if value.is_finite() && (MIN_PART_LENGTH..=MAX_PART_LENGTH).contains(&value) {
        Ok(value)
    } else {
        Err(format!("{label} must be between 0.001 mm and 1000 km"))
    }
}

fn nonzero(label: &str, value: f32) -> Result<f32, String> {
    if value.is_finite() && (MIN_PART_LENGTH..=MAX_PART_LENGTH).contains(&value.abs()) {
        Ok(value)
    } else {
        Err(format!(
            "{label} must be nonzero and below 1000 km · Negative values reverse it"
        ))
    }
}

/// Set `field` to a display value, validating it first. On error the feature
/// is unchanged and the message says what is allowed.
pub fn set(feature: &mut Feature, field: FeatureField, value: f32) -> Result<(), String> {
    use FeatureField as F;
    if !value.is_finite() {
        return Err("Enter a finite value".into());
    }
    match (&mut feature.kind, field) {
        (FeatureKind::Primitive(primitive), field) => {
            let placement = &mut primitive.placement;
            match (&mut primitive.shape, field) {
                (_, F::Origin(axis)) if axis < 3 => {
                    if value.abs() > MAX_PART_LENGTH {
                        return Err("Position must be within ±1000 km".into());
                    }
                    placement.origin[axis] = value;
                }
                (_, F::Rotation(axis)) if axis < 3 => {
                    // Keep typed angles readable (−90 stays −90) but bounded.
                    let radians = value.to_radians();
                    placement.rotation[axis] = if radians.abs() > std::f32::consts::TAU {
                        radians.rem_euclid(std::f32::consts::TAU)
                    } else {
                        radians
                    };
                }
                (PrimitiveShape::Box { size }, F::Size(axis)) if axis < 3 => {
                    size[axis] = length("Box size", value)?;
                }
                (PrimitiveShape::Cylinder { radius, .. }, F::Radius)
                | (PrimitiveShape::Sphere { radius }, F::Radius)
                | (
                    PrimitiveShape::Cone {
                        bottom_radius: radius,
                        ..
                    },
                    F::Radius,
                ) => *radius = length("Radius", value)?,
                (PrimitiveShape::Cylinder { height, .. }, F::Height)
                | (PrimitiveShape::Cone { height, .. }, F::Height) => {
                    *height = length("Height", value)?
                }
                (PrimitiveShape::Cone { top_radius, .. }, F::TopRadius) => {
                    if !(0.0..=MAX_PART_LENGTH).contains(&value) {
                        return Err("Top radius must be zero or positive".into());
                    }
                    *top_radius = value;
                }
                (
                    PrimitiveShape::Torus {
                        major_radius,
                        minor_radius,
                    },
                    F::MajorRadius,
                ) => {
                    let value = length("Ring radius", value)?;
                    if value <= *minor_radius {
                        return Err("The ring must be wider than the tube".into());
                    }
                    *major_radius = value;
                }
                (
                    PrimitiveShape::Torus {
                        major_radius,
                        minor_radius,
                    },
                    F::MinorRadius,
                ) => {
                    let value = length("Tube radius", value)?;
                    if value >= *major_radius {
                        return Err("The tube must be thinner than the ring".into());
                    }
                    *minor_radius = value;
                }
                _ => return Err("This feature has no such value".into()),
            }
        }
        (FeatureKind::Extrude(extrude), F::Distance) => {
            extrude.distance = nonzero("Distance", value)?;
        }
        (FeatureKind::Extrude(extrude), F::BackDistance) => match &mut extrude.extent {
            ExtrudeExtent::TwoSides { back_distance } => {
                if value.abs() > MAX_PART_LENGTH {
                    return Err("The second distance must be below 1000 km".into());
                }
                *back_distance = value;
            }
            _ => return Err("Only two-sided extrusions have a second distance".into()),
        },
        (FeatureKind::Revolve(revolve), F::Angle) => {
            if value.abs() < 0.01 || value.abs() > 360.0 {
                return Err("The sweep must be nonzero and at most 360°".into());
            }
            revolve.angle = value
                .to_radians()
                .clamp(-std::f32::consts::TAU, std::f32::consts::TAU);
        }
        (FeatureKind::PushPull(push), F::Distance) => {
            push.distance = nonzero("Distance", value)?;
        }
        (FeatureKind::Chamfer(chamfer), F::Distance) => {
            chamfer.distance = length("Chamfer distance", value)?;
        }
        _ => return Err("This feature has no such value".into()),
    }
    Ok(())
}

/// Extrusion extent choices offered by the inspector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtentMode {
    OneSide,
    Symmetric,
    TwoSides,
}

impl ExtentMode {
    pub const ALL: [ExtentMode; 3] = [Self::OneSide, Self::Symmetric, Self::TwoSides];

    pub fn of(extent: ExtrudeExtent) -> Self {
        match extent {
            ExtrudeExtent::OneSide => Self::OneSide,
            ExtrudeExtent::Symmetric => Self::Symmetric,
            ExtrudeExtent::TwoSides { .. } => Self::TwoSides,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::OneSide => "One side",
            Self::Symmetric => "Symmetric",
            Self::TwoSides => "Two sides",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Self::OneSide => "From the sketch plane by the distance",
            Self::Symmetric => "The distance in total, centred on the sketch plane",
            Self::TwoSides => "Separate distances forward and backward",
        }
    }

    /// The extent for this mode, keeping the current distance sensible: a
    /// new second side starts at half the first.
    pub fn apply(self, current: ExtrudeExtent, distance: f32) -> ExtrudeExtent {
        match (self, current) {
            (Self::OneSide, _) => ExtrudeExtent::OneSide,
            (Self::Symmetric, _) => ExtrudeExtent::Symmetric,
            (Self::TwoSides, ExtrudeExtent::TwoSides { back_distance }) => {
                ExtrudeExtent::TwoSides { back_distance }
            }
            (Self::TwoSides, _) => ExtrudeExtent::TwoSides {
                back_distance: (distance.abs() * 0.5).max(MIN_PART_LENGTH),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forma_core::{
        ChamferFeature, EdgeRef, ExtrudeFeature, FeatureOperation, Placement, PrimitiveFeature,
        RevolveFeature, Sketch, SketchPlane, Vec2, Vec3,
    };

    fn primitive(shape: PrimitiveShape) -> Feature {
        Feature::new(
            1,
            "Shape",
            FeatureKind::Primitive(PrimitiveFeature {
                shape,
                placement: Placement::default(),
                operation: FeatureOperation::Join,
            }),
        )
    }

    #[test]
    fn primitive_dimensions_round_trip_and_validate() {
        let mut boxed = primitive(PrimitiveShape::Box {
            size: Vec3::new(1.0, 2.0, 3.0),
        });
        assert_eq!(value(&boxed, FeatureField::Size(1)), Some(2.0));
        assert_eq!(value(&boxed, FeatureField::Radius), None);
        set(&mut boxed, FeatureField::Size(2), 0.5).unwrap();
        assert_eq!(value(&boxed, FeatureField::Size(2)), Some(0.5));
        assert!(set(&mut boxed, FeatureField::Size(0), 0.0).is_err());
        assert!(set(&mut boxed, FeatureField::Size(0), -1.0).is_err());
        assert_eq!(value(&boxed, FeatureField::Size(0)), Some(1.0), "unchanged");
        set(&mut boxed, FeatureField::Rotation(1), -90.0).unwrap();
        assert!((value(&boxed, FeatureField::Rotation(1)).unwrap() + 90.0).abs() < 1.0e-4);
        set(&mut boxed, FeatureField::Origin(0), 0.25).unwrap();
        assert_eq!(value(&boxed, FeatureField::Origin(0)), Some(0.25));

        let mut torus = primitive(PrimitiveShape::Torus {
            major_radius: 1.0,
            minor_radius: 0.25,
        });
        assert!(set(&mut torus, FeatureField::MinorRadius, 1.5).is_err());
        assert!(set(&mut torus, FeatureField::MajorRadius, 0.2).is_err());
        set(&mut torus, FeatureField::MinorRadius, 0.5).unwrap();
        let mut cone = primitive(PrimitiveShape::Cone {
            bottom_radius: 1.0,
            top_radius: 0.5,
            height: 2.0,
        });
        set(&mut cone, FeatureField::TopRadius, 0.0).unwrap();
        assert!(set(&mut cone, FeatureField::TopRadius, -0.1).is_err());
        assert!(applies(&cone, FeatureField::Height) && !applies(&cone, FeatureField::Size(0)));
    }

    #[test]
    fn sketch_feature_values_respect_their_extent() {
        let mut extrude = Feature::new(
            2,
            "Extrude",
            FeatureKind::Extrude(ExtrudeFeature {
                sketch: Sketch::new(SketchPlane::ground()),
                profiles: Vec::new(),
                distance: 0.4,
                extent: ExtrudeExtent::OneSide,
                flip: false,
                operation: FeatureOperation::Join,
            }),
        );
        assert!(!applies(&extrude, FeatureField::BackDistance));
        assert!(set(&mut extrude, FeatureField::BackDistance, 0.1).is_err());
        set(&mut extrude, FeatureField::Distance, -0.2).unwrap();
        assert!(set(&mut extrude, FeatureField::Distance, 0.0).is_err());
        if let FeatureKind::Extrude(feature) = &mut extrude.kind {
            feature.extent = ExtentMode::TwoSides.apply(feature.extent, feature.distance);
            assert_eq!(
                feature.extent,
                ExtrudeExtent::TwoSides { back_distance: 0.1 }
            );
        }
        set(&mut extrude, FeatureField::BackDistance, 0.3).unwrap();
        assert_eq!(value(&extrude, FeatureField::BackDistance), Some(0.3));

        let mut revolve = Feature::new(
            3,
            "Revolve",
            FeatureKind::Revolve(RevolveFeature {
                sketch: Sketch::new(SketchPlane::front()),
                profiles: Vec::new(),
                axis_origin: Vec2::ZERO,
                axis_direction: Vec2::Y,
                angle: std::f32::consts::PI,
                operation: FeatureOperation::Join,
            }),
        );
        assert!((value(&revolve, FeatureField::Angle).unwrap() - 180.0).abs() < 1.0e-3);
        set(&mut revolve, FeatureField::Angle, 360.0).unwrap();
        assert!(set(&mut revolve, FeatureField::Angle, 400.0).is_err());
        assert!(set(&mut revolve, FeatureField::Angle, 0.0).is_err());

        let mut chamfer = Feature::new(
            4,
            "Chamfer",
            FeatureKind::Chamfer(ChamferFeature {
                edges: vec![EdgeRef {
                    start: Vec3::ZERO,
                    end: Vec3::X,
                }],
                distance: 0.1,
            }),
        );
        assert!(set(&mut chamfer, FeatureField::Distance, -0.1).is_err());
        set(&mut chamfer, FeatureField::Distance, 0.05).unwrap();
        assert_eq!(value(&chamfer, FeatureField::Distance), Some(0.05));
    }

    #[test]
    fn every_editable_field_scrubs_and_parses() {
        for field in FeatureField::ORDER {
            if field == FeatureField::Name {
                assert_eq!(field.scrub_step().0, 0.0);
                continue;
            }
            assert!(field.scrub_step().0 > 0.0, "{field:?}");
            assert!(field.parse("1").is_some(), "{field:?}");
        }
        assert_eq!(FeatureField::Distance.parse("25mm"), Some(0.025));
        assert!((FeatureField::Angle.parse("1.5707964rad").unwrap() - 90.0).abs() < 1.0e-3);
    }
}
