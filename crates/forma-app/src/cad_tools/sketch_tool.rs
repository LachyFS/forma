//! Extrude and Revolve: sweep picked sketch regions into a new part, or join,
//! cut or intersect them with the part whose face the sketch lies on.
//!
//! The tool is a session: click regions (Shift adds), drag the arrow (or a
//! region) along the sketch normal, type an exact value, toggle options in
//! the HUD, then Return or Done. A ghost follows every pointer move; on
//! release the candidate feature is applied to the live scene so the kernel's
//! real result replaces the ghost before the user commits.
use super::geometry::{self, Lathe, Prism};
use super::{AxisDrag, DRAG_PIXELS, PICK_PIXELS, PendingGhost, Preview, Session, Tool};
use crate::app::{EditMode, Studio};
use forma_core::{
    ExtrudeExtent, ExtrudeFeature, FeatureKind, FeatureOperation, Mat4, PartData, Profile, Ray,
    RevolveFeature, SKETCH_TOLERANCE, Scene, Sketch, SketchCurve, SketchPlane, Vec2, Vec3,
};
use gpui::{Context, KeyDownEvent, MouseDownEvent, Window};
use std::{collections::BTreeSet, rc::Rc};

/// Smallest extrusion the tool commits (world units).
const MIN_DISTANCE: f32 = 1.0e-5;
/// Revolve angles snap to these steps unless Ctrl is held.
const ANGLE_STEP: f32 = std::f32::consts::PI / 12.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SketchToolKind {
    Extrude,
    Revolve,
}

impl SketchToolKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Extrude => "Extrude",
            Self::Revolve => "Revolve",
        }
    }
}

/// What the swept regions do: become a new part, or combine with the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    NewPart,
    Join,
    Cut,
    Intersect,
}

impl Operation {
    pub(crate) const ALL: [Self; 4] = [Self::NewPart, Self::Join, Self::Cut, Self::Intersect];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::NewPart => "New",
            Self::Join => "Join",
            Self::Cut => "Cut",
            Self::Intersect => "Intersect",
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::NewPart => "N",
            Self::Join => "J",
            Self::Cut => "K",
            Self::Intersect => "I",
        }
    }

    /// The feature operation: a new part starts its body with a join.
    pub(crate) fn feature(self) -> FeatureOperation {
        match self {
            Self::NewPart | Self::Join => FeatureOperation::Join,
            Self::Cut => FeatureOperation::Cut,
            Self::Intersect => FeatureOperation::Intersect,
        }
    }

    pub(crate) fn from_feature(operation: FeatureOperation) -> Self {
        match operation {
            FeatureOperation::Join | FeatureOperation::NewBody => Self::Join,
            FeatureOperation::Cut => Self::Cut,
            FeatureOperation::Intersect => Self::Intersect,
        }
    }

    fn next(self) -> Self {
        let index = Self::ALL.iter().position(|o| *o == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }
}

/// The closed regions of one sketch object, ready for picking and sweeping.
/// Built with the same planar arrangement Draw shades (`Sketch::profiles`)
/// and cached per sketch revision.
#[derive(Debug)]
pub(crate) struct Regions {
    pub object: u64,
    pub name: String,
    /// The sketch in its object's local space.
    pub sketch: Sketch,
    /// The object's world transform.
    pub matrix: Mat4,
    /// Each profile's outer loop and holes, in sketch coordinates.
    pub polygons: Vec<Vec<Vec<Vec2>>>,
    /// The same polygons in world space, for painting.
    pub world: Vec<Vec<Vec<Vec3>>>,
    /// A point strictly inside each profile (sketch coordinates).
    pub samples: Vec<Option<Vec2>>,
    /// The outermost regions: the default pick.
    pub defaults: Vec<usize>,
    /// Straight curves that can serve as revolve axes: (entity, start, end).
    pub lines: Vec<(usize, Vec2, Vec2)>,
    inverse: Mat4,
}

impl Regions {
    pub(crate) fn build(object: u64, name: &str, sketch: &Sketch, matrix: Mat4) -> Option<Self> {
        if !matrix.is_finite() || matrix.determinant().abs() < 1.0e-12 {
            return None;
        }
        let profiles: Vec<Profile> = sketch.profiles(SKETCH_TOLERANCE);
        let world_point = |p: Vec2| matrix.transform_point3(sketch.plane.to_world(p));
        let polygons: Vec<Vec<Vec<Vec2>>> = profiles
            .iter()
            .map(|profile| {
                std::iter::once(&profile.outer)
                    .chain(&profile.holes)
                    .map(|l| {
                        let (min, max) = l.bounds();
                        l.polygon(((max - min).max_element() * 1.0e-3).max(1.0e-7))
                    })
                    .collect()
            })
            .collect();
        let world = polygons
            .iter()
            .map(|loops| {
                loops
                    .iter()
                    .map(|l| l.iter().map(|p| world_point(*p)).collect())
                    .collect()
            })
            .collect();
        let samples = profiles
            .iter()
            .map(forma_cad::regions::interior_point)
            .collect();
        let defaults = forma_cad::regions::outermost_regions(&profiles);
        let lines = sketch
            .entities
            .iter()
            .enumerate()
            .filter_map(|(i, e)| match e.curve {
                SketchCurve::Line { start, end } => Some((i, start, end)),
                _ => None,
            })
            .collect();
        Some(Self {
            object,
            name: name.to_owned(),
            sketch: sketch.clone(),
            matrix,
            polygons,
            world,
            samples,
            defaults,
            lines,
            inverse: matrix.inverse(),
        })
    }

    /// The sketch point under a world ray, its world position and distance.
    pub(crate) fn ray_point(&self, ray: Ray) -> Option<(Vec2, Vec3, f32)> {
        let local = Ray {
            origin: self.inverse.transform_point3(ray.origin),
            direction: self.inverse.transform_vector3(ray.direction),
        };
        let point = self.sketch.plane.intersect(local)?;
        let world = self.to_world(point);
        Some((point, world, world.distance(ray.origin)))
    }

    /// The region containing a sketch point.
    pub(crate) fn region_at(&self, point: Vec2) -> Option<usize> {
        self.polygons.iter().position(|loops| {
            loops
                .iter()
                .filter(|polygon| point_in_polygon(point, polygon))
                .count()
                % 2
                == 1
        })
    }

    pub(crate) fn to_world(&self, point: Vec2) -> Vec3 {
        self.matrix.transform_point3(self.sketch.plane.to_world(point))
    }

    /// Unit sketch normal in world space.
    pub(crate) fn normal(&self) -> Vec3 {
        self.matrix
            .transform_vector3(self.sketch.plane.normal())
            .normalize_or_zero()
    }

    /// World length of one sketch unit along the normal (object scale).
    pub(crate) fn normal_scale(&self) -> f32 {
        self.matrix
            .transform_vector3(self.sketch.plane.normal())
            .length()
            .max(1.0e-9)
    }

    /// The sketch plane in world space.
    pub(crate) fn world_plane(&self) -> SketchPlane {
        self.sketch
            .plane
            .transformed(self.matrix)
            .unwrap_or(self.sketch.plane)
    }
}

fn point_in_polygon(point: Vec2, polygon: &[Vec2]) -> bool {
    let mut inside = false;
    let n = polygon.len();
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + 1) % n]);
        if (a.y > point.y) != (b.y > point.y)
            && point.x < a.x + (point.y - a.y) * (b.x - a.x) / (b.y - a.y)
        {
            inside = !inside;
        }
    }
    inside
}

/// The part a sketch can join or cut.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Target {
    pub object: u64,
    pub name: String,
    /// Outward normal of the face the sketch lies on (world), when it lies on
    /// one: the extrusion's direction against it decides Join or Cut.
    pub outward: Option<Vec3>,
    /// Offsets of the target's faces parallel to the sketch, along the
    /// sketch normal: drags snap to them ("up to face", "through").
    pub heights: Vec<f32>,
}

/// The revolve axis in sketch coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RevolveAxis {
    pub origin: Vec2,
    pub direction: Vec2,
    pub source: AxisSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AxisSource {
    Line(usize),
    SketchX,
    SketchY,
}

impl AxisSource {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Line(_) => "Line",
            Self::SketchX => "X axis",
            Self::SketchY => "Y axis",
        }
    }
}

/// An angle drag around the revolve axis.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AngleDrag {
    foot: Vec3,
    axis: Vec3,
    plane_point: Vec3,
    last_raw: f32,
    accumulated: f32,
    start_value: f32,
    press: Vec2,
    pub started: bool,
    edge_on: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ToolDrag {
    Distance(AxisDrag),
    Angle(AngleDrag),
}

impl ToolDrag {
    pub(crate) fn started(&self) -> bool {
        match self {
            Self::Distance(drag) => drag.started,
            Self::Angle(drag) => drag.started,
        }
    }

    fn press(&self) -> Vec2 {
        match self {
            Self::Distance(drag) => drag.press,
            Self::Angle(drag) => drag.press,
        }
    }

    fn start(&mut self) {
        match self {
            Self::Distance(drag) => drag.started = true,
            Self::Angle(drag) => drag.started = true,
        }
    }
}

/// A press on a region that turns into a distance drag once it moves.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RegionPress {
    screen: Vec2,
    world: Vec3,
}

pub(crate) struct SketchTool {
    pub kind: SketchToolKind,
    pub regions: Rc<Regions>,
    pub selected: BTreeSet<usize>,
    pub hover: Option<usize>,
    /// A straight sketch curve under the pointer (Revolve axis candidate).
    pub hover_line: Option<usize>,
    /// The pointer rests on the arrow handle.
    pub handle_hot: bool,
    /// Signed world distance along the sketch normal (total when symmetric).
    pub distance: f32,
    pub symmetric: bool,
    /// Signed revolve sweep in radians.
    pub angle: f32,
    pub axis: Option<RevolveAxis>,
    pub manual_operation: Option<Operation>,
    pub target: Option<Target>,
    pub numeric: String,
    pub drag: Option<ToolDrag>,
    press: Option<RegionPress>,
    pub prism: Rc<Prism>,
    pub lathe: Option<Rc<Lathe>>,
    pub session: Session,
    /// The drag snapped to a target face height.
    pub snapped_face: bool,
    pub error: Option<String>,
}

impl SketchTool {
    /// The operation the commit will perform.
    pub(crate) fn operation(&self) -> Operation {
        if let Some(operation) = self.manual_operation
            && (operation == Operation::NewPart || self.target.is_some())
        {
            return operation;
        }
        match (&self.target, self.kind) {
            (
                Some(Target {
                    outward: Some(outward),
                    ..
                }),
                SketchToolKind::Extrude,
            ) => {
                if self.symmetric {
                    Operation::Join
                } else {
                    let direction = self.regions.normal() * self.distance.signum();
                    Operation::from_feature(geometry::infer_operation(direction, *outward))
                }
            }
            _ => Operation::NewPart,
        }
    }

    /// Whether the operation was chosen by inference rather than the user.
    pub(crate) fn inferred(&self) -> bool {
        self.manual_operation.is_none() && self.target.as_ref().is_some_and(|t| t.outward.is_some())
    }

    /// The value the numeric entry shows and edits: distance or angle.
    pub(crate) fn value(&self) -> f32 {
        match self.kind {
            SketchToolKind::Extrude => self.distance,
            SketchToolKind::Revolve => self.angle,
        }
    }

    /// The typed value, if the entry parses: `Err` marks invalid text.
    pub(crate) fn typed(&self) -> Result<Option<f32>, ()> {
        let text = self.numeric.trim();
        if text.is_empty() {
            return Ok(None);
        }
        match self.kind {
            SketchToolKind::Extrude => crate::transform::distance_value(text),
            SketchToolKind::Revolve => crate::transform::angle_value(text),
        }
        .map(Some)
        .ok_or(())
    }

    /// Sweep offsets of the ghost along the normal: (start, end).
    pub(crate) fn offsets(&self) -> (f32, f32) {
        if self.symmetric {
            let half = self.distance.abs() * 0.5;
            (-half, half)
        } else {
            (0.0_f32.min(self.distance), 0.0_f32.max(self.distance))
        }
    }

    /// Distance of the arrow handle from the sketch plane.
    pub(crate) fn handle_offset(&self) -> f32 {
        if self.symmetric {
            self.distance.abs() * 0.5
        } else {
            self.distance
        }
    }

    /// The point the arrow handle stands on: the picked regions' centroid.
    pub(crate) fn anchor(&self) -> Vec3 {
        self.prism.centroid()
    }

    /// The revolve axis in world space: (origin, unit direction).
    pub(crate) fn world_axis(&self) -> Option<(Vec3, Vec3)> {
        let axis = self.axis?;
        let origin = self.regions.to_world(axis.origin);
        let direction = (self.regions.to_world(axis.origin + axis.direction) - origin)
            .try_normalize()?;
        Some((origin, direction))
    }

    pub(crate) fn set_sketch_axis(&mut self, y: bool) {
        self.axis = Some(RevolveAxis {
            origin: Vec2::ZERO,
            direction: if y { Vec2::Y } else { Vec2::X },
            source: if y {
                AxisSource::SketchY
            } else {
                AxisSource::SketchX
            },
        });
    }

    /// Why the revolve cannot run yet, if it cannot.
    pub(crate) fn revolve_problem(&self) -> Option<&'static str> {
        let axis = self.axis?;
        let normal = axis.direction.perp().normalize_or_zero();
        let mut sides = (false, false);
        for &index in &self.selected {
            for polygon in &self.regions.polygons[index] {
                for p in polygon {
                    let side = (*p - axis.origin).dot(normal);
                    let tolerance = 1.0e-5;
                    sides.0 |= side > tolerance;
                    sides.1 |= side < -tolerance;
                }
            }
        }
        (sides.0 && sides.1).then_some("The profile crosses the axis · Pick an axis beside it")
    }

    /// Key identifying the candidate feature, for reusing a live preview.
    fn preview_key(&self) -> String {
        format!(
            "{:?}|{:?}|{}|{}|{}|{:?}|{:?}",
            self.kind,
            self.selected,
            self.distance,
            self.symmetric,
            self.angle,
            self.axis.map(|a| (a.origin, a.direction)),
            self.operation()
        )
    }

    /// Rebuilds the cached ghost geometry for the current picks and axis.
    pub(crate) fn rebuild_ghost(&mut self) {
        let regions = &self.regions;
        let picked: Vec<Vec<Vec<Vec2>>> = self
            .selected
            .iter()
            .filter_map(|&i| regions.polygons.get(i).cloned())
            .collect();
        self.prism = Rc::new(Prism::from_regions(
            &regions.sketch.plane,
            regions.matrix,
            &picked,
        ));
        self.lathe = self.world_axis().map(|(origin, direction)| {
            Rc::new(Lathe {
                loops: self
                    .selected
                    .iter()
                    .flat_map(|&i| regions.world[i].iter().cloned())
                    .collect(),
                prism: (*self.prism).clone(),
                axis_origin: origin,
                axis_direction: direction,
            })
        });
    }

    fn sketch_lines_near(&self, studio: &Studio, screen: Vec2) -> Option<usize> {
        self.regions
            .lines
            .iter()
            .filter_map(|&(index, a, b)| {
                let a = studio.world_to_screen(self.regions.to_world(a))?;
                let b = studio.world_to_screen(self.regions.to_world(b))?;
                let d = super::segment_distance(screen, a, b);
                (d <= PICK_PIXELS).then_some((d, index))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, i)| i)
    }

    /// Screen base and tip of the extrusion arrow.
    pub(crate) fn handle_screen(&self, studio: &Studio) -> Option<(Vec2, Vec2)> {
        if self.selected.is_empty() || self.kind != SketchToolKind::Extrude {
            return None;
        }
        let normal = self.regions.normal();
        let base = self.anchor() + normal * self.handle_offset();
        let screen = studio.world_to_screen(base)?;
        let ahead = studio.world_to_screen(base + normal * studio.world_per_pixel(base) * 10.0);
        let sign = if self.distance < 0.0 && !self.symmetric {
            -1.0
        } else {
            1.0
        };
        let direction = ahead
            .map(|p| (p - screen) * sign)
            .and_then(|d| d.try_normalize())
            .unwrap_or(Vec2::NEG_Y);
        Some((screen, screen + direction * super::HANDLE_PIXELS))
    }

    /// Screen position of the revolve angle handle.
    pub(crate) fn angle_handle_screen(&self, studio: &Studio) -> Option<Vec2> {
        let (origin, direction) = self.world_axis()?;
        let point = geometry::rotate_about(self.anchor(), origin, direction, self.angle);
        studio.world_to_screen(point)
    }
}

/// The sketch, its region picks and a length expressed in another space
/// (`relative` maps the sketch object's space into it). Rejects uneven
/// scales, which would distort the profile.
fn sketch_in_space(sketch: &Sketch, picks: &[Vec2], relative: Mat4) -> Result<(Sketch, Vec<Vec2>, f32), String> {
    let x = relative.transform_vector3(sketch.plane.x_axis);
    let y = relative.transform_vector3(sketch.plane.y_axis);
    let n = relative.transform_vector3(sketch.plane.normal());
    let (sx, sy, sn) = (x.length(), y.length(), n.length());
    let uneven = |a: f32, b: f32| (a - b).abs() > a.max(b) * 1.0e-4;
    if uneven(sx, sy) || uneven(sx, sn) || x.dot(y).abs() > sx * sy * 1.0e-4 {
        return Err("The part is scaled unevenly · Apply its scale first".into());
    }
    let plane = sketch
        .plane
        .transformed(relative)
        .ok_or("The part's transform collapses the sketch plane")?;
    let scale = |p: Vec2| p * sx;
    let mut moved = Sketch::new(plane);
    moved.entities = sketch
        .entities
        .iter()
        .map(|entity| {
            let mut entity = *entity;
            entity.curve = match entity.curve {
                SketchCurve::Line { start, end } => SketchCurve::Line {
                    start: scale(start),
                    end: scale(end),
                },
                SketchCurve::Arc {
                    center,
                    radius,
                    start_angle,
                    sweep,
                } => SketchCurve::Arc {
                    center: scale(center),
                    radius: radius * sx,
                    start_angle,
                    sweep,
                },
                SketchCurve::Circle { center, radius } => SketchCurve::Circle {
                    center: scale(center),
                    radius: radius * sx,
                },
            };
            entity
        })
        .collect();
    Ok((moved, picks.iter().map(|p| scale(*p)).collect(), sx))
}

/// Where a built candidate landed in the scene.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Applied {
    pub object: u64,
    pub part: u64,
    pub feature: u64,
}

impl SketchTool {
    /// Adds the candidate feature to `scene`: a new part, or a feature on the
    /// target part.
    pub(crate) fn build_into(&self, scene: &mut Scene) -> Result<Applied, String> {
        let regions = &self.regions;
        let picks: Vec<Vec2> = self
            .selected
            .iter()
            .filter_map(|&i| regions.samples.get(i).copied().flatten())
            .collect();
        if picks.is_empty() {
            return Err("Pick a closed region of the sketch".into());
        }
        let operation = self.operation();
        let (local_distance, extent) = match self.kind {
            SketchToolKind::Extrude => {
                if self.distance.abs() < MIN_DISTANCE {
                    return Err("Drag the arrow or type a distance".into());
                }
                let extent = if self.symmetric {
                    ExtrudeExtent::Symmetric
                } else {
                    ExtrudeExtent::OneSide
                };
                (self.distance / regions.normal_scale(), extent)
            }
            SketchToolKind::Revolve => {
                if self.axis.is_none() {
                    return Err("Pick an axis · Click a line, or press X or Y".into());
                }
                if let Some(problem) = self.revolve_problem() {
                    return Err(problem.into());
                }
                if self.angle.abs() < 1.0e-3 {
                    return Err("Drag the angle handle or type an angle".into());
                }
                (0.0, ExtrudeExtent::OneSide)
            }
        };
        let name = match (self.kind, operation) {
            (SketchToolKind::Extrude, Operation::Cut) => "Cut",
            (SketchToolKind::Extrude, Operation::Intersect) => "Intersect",
            (SketchToolKind::Extrude, _) => "Extrude",
            (SketchToolKind::Revolve, Operation::Cut) => "Revolve cut",
            (SketchToolKind::Revolve, _) => "Revolve",
        };
        let kind_for = |sketch: Sketch, picks: Vec<Vec2>, length_scale: f32, op: FeatureOperation| {
            match self.kind {
                SketchToolKind::Extrude => FeatureKind::Extrude(ExtrudeFeature {
                    sketch,
                    profiles: picks,
                    distance: local_distance * length_scale,
                    extent,
                    flip: false,
                    operation: op,
                }),
                SketchToolKind::Revolve => {
                    let axis = self.axis.unwrap();
                    FeatureKind::Revolve(RevolveFeature {
                        sketch,
                        profiles: picks,
                        axis_origin: axis.origin * length_scale,
                        axis_direction: axis.direction,
                        angle: self.angle.clamp(-std::f32::consts::TAU, std::f32::consts::TAU),
                        operation: op,
                    })
                }
            }
        };
        if operation == Operation::NewPart {
            let source = scene
                .object(regions.object)
                .cloned()
                .ok_or("The sketch no longer exists")?;
            let mut part = PartData::new(0, name);
            let feature = part.push_feature(
                name,
                kind_for(regions.sketch.clone(), picks, 1.0, operation.feature()),
            );
            part.edit_feature(feature, |f| f.source = Some(regions.object));
            part.validate().map_err(|e| format!("{e:#}"))?;
            let suffix = match self.kind {
                SketchToolKind::Extrude => "extrusion",
                SketchToolKind::Revolve => "revolution",
            };
            let object = scene
                .add_part(format!("{} {suffix}", source.name), part)
                .map_err(|e| format!("{e:#}"))?;
            // Part space is the sketch object's space, where its plane lives.
            let created = scene.object_mut(object).unwrap();
            created.transform = source.transform;
            created.parent = source.parent;
            created.parent_inverse = source.parent_inverse;
            created.collections = source.collections.clone();
            let part = scene.object(object).and_then(|o| o.part_id()).unwrap();
            return Ok(Applied {
                object,
                part,
                feature,
            });
        }
        let target = self
            .target
            .as_ref()
            .ok_or("Join, Cut and Intersect need a part")?;
        let part_matrix = scene
            .world_transform(target.object)
            .ok_or("The target part no longer exists")?;
        let relative = part_matrix.inverse() * regions.matrix;
        let (sketch, picks, scale) = sketch_in_space(&regions.sketch, &picks, relative)?;
        let part = scene
            .object_part_mut(target.object)
            .ok_or("The target is no longer a part")?;
        let feature = part.push_feature(name, kind_for(sketch, picks, scale, operation.feature()));
        part.edit_feature(feature, |f| f.source = Some(regions.object));
        part.validate().map_err(|e| format!("{e:#}"))?;
        let part = part.id;
        Ok(Applied {
            object: target.object,
            part,
            feature,
        })
    }
}

impl Studio {
    /// Starts Extrude or Revolve on the sketch under the pointer or selected,
    /// or on `object` when given.
    pub(crate) fn start_sketch_tool(
        &mut self,
        kind: SketchToolKind,
        object: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        let context = match object {
            Some(id) => Some((id, None)),
            None => self.sketch_context(),
        };
        let Some((id, profile)) = context else {
            self.status = format!(
                "{} · Select a sketch, or point at one of its regions",
                kind.label()
            );
            cx.notify();
            return;
        };
        let regions = self
            .region_sets()
            .iter()
            .find(|r| r.object == id)
            .cloned()
            .or_else(|| {
                let sketch = self.scene.object_sketch(id)?;
                let matrix = self.scene.world_transform(id)?;
                let name = self.scene.object(id)?.name.clone();
                Regions::build(id, &name, sketch, matrix).map(Rc::new)
            });
        let Some(regions) = regions.filter(|r| !r.polygons.is_empty()) else {
            self.status = "The sketch has no closed regions · Close its outline first".into();
            cx.notify();
            return;
        };
        if self.edit_mode != EditMode::Object {
            self.edit_mode = EditMode::Object;
            self.clear_components();
        }
        self.navigation = None;
        let selected: BTreeSet<usize> = match profile {
            Some(profile) => [profile].into_iter().collect(),
            None if regions.defaults.is_empty() => [0].into_iter().collect(),
            None => regions.defaults.iter().copied().collect(),
        };
        let axis = regions
            .sketch
            .entities
            .iter()
            .enumerate()
            .find_map(|(i, e)| match (e.construction, e.curve) {
                (true, SketchCurve::Line { start, end }) => Some(RevolveAxis {
                    origin: start,
                    direction: (end - start).normalize_or_zero(),
                    source: AxisSource::Line(i),
                }),
                _ => None,
            });
        let mut tool = SketchTool {
            kind,
            regions,
            selected,
            hover: None,
            hover_line: None,
            handle_hot: false,
            distance: 0.0,
            symmetric: false,
            angle: std::f32::consts::TAU,
            axis,
            manual_operation: None,
            target: None,
            numeric: String::new(),
            drag: None,
            press: None,
            prism: Rc::default(),
            lathe: None,
            session: Session::new(&self.scene),
            snapped_face: false,
            error: None,
        };
        tool.rebuild_ghost();
        tool.target = self.infer_target(&tool);
        self.cad_tool.region_hover = None;
        self.cad_tool.tool = Some(Tool::Sketch(Box::new(tool)));
        self.refresh_sketch_status();
        cx.notify();
    }

    /// The part the picked regions can combine with: the part whose planar
    /// face the sketch lies on, else a part the sweep would pass through.
    fn infer_target(&self, tool: &SketchTool) -> Option<Target> {
        let scene = &tool.session.before;
        let regions = &tool.regions;
        let plane = regions.world_plane();
        let normal = regions.normal();
        let samples: Vec<Vec3> = tool
            .selected
            .iter()
            .filter_map(|&i| regions.samples[i].map(|p| regions.to_world(p)))
            .collect();
        let parts: Vec<(u64, Mat4, &forma_core::Mesh)> = scene
            .objects
            .iter()
            .filter(|o| o.is_part() && scene.is_effectively_visible(o.id))
            .filter_map(|o| Some((o.id, scene.world_transform(o.id)?, scene.object_mesh(o.id)?)))
            .collect();
        let target = |object: u64, outward: Option<Vec3>| -> Option<Target> {
            let (_, matrix, mesh) = parts.iter().find(|(id, _, _)| *id == object)?;
            Some(Target {
                object,
                name: scene.object(object)?.name.clone(),
                outward,
                heights: geometry::parallel_heights(mesh, *matrix, plane.origin, normal),
            })
        };
        if let Some(face) = geometry::find_coplanar_face(&plane, &samples, &parts) {
            return target(face.object, Some(face.outward));
        }
        // A part straddling the sweep line through a picked region.
        let hit = parts.iter().find(|(_, matrix, mesh)| {
            let (lo, hi) = mesh.positions.iter().fold(
                (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
                |(lo, hi), p| {
                    let w = matrix.transform_point3(*p);
                    (lo.min(w), hi.max(w))
                },
            );
            samples.iter().any(|s| line_hits_box(*s, normal, lo, hi))
        })?;
        target(hit.0, None)
    }

    /// The sketch tool changed in a way that affects the candidate: rebuild
    /// derived state, and preview through the kernel once things settle.
    pub(crate) fn sketch_tool_changed(&mut self, ghost: bool, cx: &mut Context<Self>) {
        let target = match self.cad_tool.tool.as_mut() {
            Some(Tool::Sketch(tool)) => {
                if ghost {
                    tool.rebuild_ghost();
                }
                tool.error = None;
                Some(())
            }
            _ => None,
        };
        if target.is_some() && ghost {
            let tool = match self.cad_tool.tool.as_ref() {
                Some(Tool::Sketch(tool)) => tool,
                _ => unreachable!(),
            };
            let target = self.infer_target(tool);
            if let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_mut() {
                if target.is_none() {
                    tool.manual_operation = tool
                        .manual_operation
                        .filter(|o| *o == Operation::NewPart);
                }
                tool.target = target;
            }
        }
        self.refresh_sketch_status();
        self.schedule_kernel_preview(cx);
        cx.notify();
    }

    fn refresh_sketch_status(&mut self) {
        let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_ref() else {
            return;
        };
        let regions = tool.selected.len();
        let picked = format!(
            "{regions} region{}",
            if regions == 1 { "" } else { "s" }
        );
        self.status = match tool.kind {
            SketchToolKind::Extrude if tool.distance == 0.0 => format!(
                "Extrude · {picked} · Drag the arrow or a region, or type a distance · Return to apply"
            ),
            SketchToolKind::Extrude => format!(
                "Extrude {} · {picked} · {} · Return to apply · Esc cancel",
                crate::sketch::overlay::format_length(tool.distance),
                tool.operation().label()
            ),
            SketchToolKind::Revolve if tool.axis.is_none() => {
                format!("Revolve · {picked} · Click a line for the axis, or press X or Y")
            }
            SketchToolKind::Revolve => format!(
                "Revolve {} · {picked} · Drag the handle or type an angle · Return to apply",
                crate::sketch::overlay::format_angle(tool.angle)
            ),
        };
    }

    pub(crate) fn sketch_tool_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        let mods = event.keystroke.modifiers;
        let character = event
            .keystroke
            .key_char
            .as_deref()
            .and_then(|text| text.chars().next());
        let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_mut() else {
            return false;
        };
        match key {
            "escape" => {
                if tool.drag.as_ref().is_some_and(|d| d.started()) {
                    self.cancel_sketch_drag(cx);
                } else if !tool.numeric.is_empty() {
                    tool.numeric.clear();
                    self.sketch_tool_changed(false, cx);
                } else {
                    self.cancel_cad_tool(cx);
                }
                true
            }
            "enter" => {
                if !event.is_held {
                    self.commit_sketch_tool(cx);
                }
                true
            }
            "backspace" | "delete" => {
                if super::erase_numeric(&mut tool.numeric, key, mods.alt) {
                    self.apply_typed_value();
                    self.sketch_tool_changed(false, cx);
                }
                true
            }
            "tab" => {
                if tool.target.is_some() {
                    let next = if mods.shift {
                        Operation::ALL
                            .into_iter()
                            .rev()
                            .skip_while(|o| *o != tool.operation())
                            .nth(1)
                            .unwrap_or(Operation::Intersect)
                    } else {
                        tool.operation().next()
                    };
                    tool.manual_operation = Some(next);
                    self.sketch_tool_changed(false, cx);
                } else {
                    self.status = "Only a new part is possible here · Sketch on a part's face to join or cut".into();
                }
                true
            }
            _ if super::type_numeric(&mut tool.numeric, character) => {
                self.apply_typed_value();
                self.sketch_tool_changed(false, cx);
                true
            }
            _ if mods.alt => false,
            _ => {
                let command = match (key, mods.shift) {
                    ("f", false) => Some(super::ToolCommand::Flip),
                    ("s", false) if tool.kind == SketchToolKind::Extrude => {
                        Some(super::ToolCommand::ToggleSymmetric)
                    }
                    ("r", _) => Some(if tool.kind == SketchToolKind::Revolve {
                        super::ToolCommand::Extrude
                    } else {
                        super::ToolCommand::Revolve
                    }),
                    ("e", false) => Some(super::ToolCommand::Extrude),
                    ("x", false) if tool.kind == SketchToolKind::Revolve => {
                        Some(super::ToolCommand::SketchAxis(false))
                    }
                    ("y", false) if tool.kind == SketchToolKind::Revolve => {
                        Some(super::ToolCommand::SketchAxis(true))
                    }
                    ("n", false) => Some(super::ToolCommand::SetOperation(Operation::NewPart)),
                    ("j", false) => Some(super::ToolCommand::SetOperation(Operation::Join)),
                    ("k", false) => Some(super::ToolCommand::SetOperation(Operation::Cut)),
                    ("i", false) => Some(super::ToolCommand::SetOperation(Operation::Intersect)),
                    _ => None,
                };
                match command {
                    Some(command) => {
                        self.execute_cad_tool(command, window, cx);
                        true
                    }
                    None => false,
                }
            }
        }
    }

    /// Applies a valid typed value on the current side.
    fn apply_typed_value(&mut self) {
        let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_mut() else {
            return;
        };
        if let Ok(Some(value)) = tool.typed() {
            match tool.kind {
                SketchToolKind::Extrude => {
                    tool.distance = super::typed_on_side(value, tool.distance)
                }
                SketchToolKind::Revolve => tool.angle = super::typed_on_side(value, tool.angle),
            }
            tool.snapped_face = false;
        }
    }

    pub(crate) fn sketch_tool_press(
        &mut self,
        position: Vec2,
        event: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_ref() else {
            return false;
        };
        // The arrow (or the revolve angle handle) first.
        if tool.kind == SketchToolKind::Extrude
            && let Some((base, tip)) = tool.handle_screen(self)
            && super::segment_distance(position, base, tip) <= PICK_PIXELS
        {
            let normal = tool.regions.normal();
            let drag = AxisDrag::new(self, tool.anchor(), normal, tool.handle_offset(), position);
            if let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_mut() {
                tool.drag = Some(ToolDrag::Distance(drag));
                tool.numeric.clear();
            }
            return true;
        }
        if tool.kind == SketchToolKind::Revolve
            && let Some(handle) = tool.angle_handle_screen(self)
            && handle.distance(position) <= PICK_PIXELS + 3.0
            && let Some(drag) = self.angle_drag(tool, position)
        {
            if let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_mut() {
                tool.drag = Some(ToolDrag::Angle(drag));
                tool.numeric.clear();
            }
            return true;
        }
        if tool.kind == SketchToolKind::Revolve
            && let Some(line) = tool.sketch_lines_near(self, position)
        {
            let (_, start, end) = *tool.regions.lines.iter().find(|l| l.0 == line).unwrap();
            if let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_mut() {
                tool.axis = Some(RevolveAxis {
                    origin: start,
                    direction: (end - start).normalize_or_zero(),
                    source: AxisSource::Line(line),
                });
            }
            self.sketch_tool_changed(true, cx);
            return true;
        }
        let ray = self.tool_ray(position);
        let hit = ray.and_then(|ray| {
            let (point, world, _) = tool.regions.ray_point(ray)?;
            Some((tool.regions.region_at(point)?, world))
        });
        let Some((profile, world)) = hit else {
            // Empty space keeps the picks, so a stray click loses nothing.
            return true;
        };
        let shift = event.modifiers.shift;
        if let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_mut() {
            if shift {
                if !tool.selected.remove(&profile) {
                    tool.selected.insert(profile);
                }
            } else if !tool.selected.contains(&profile) {
                tool.selected = [profile].into_iter().collect();
            }
            tool.press = Some(RegionPress {
                screen: position,
                world,
            });
        }
        self.sketch_tool_changed(true, cx);
        true
    }

    fn angle_drag(&self, tool: &SketchTool, position: Vec2) -> Option<AngleDrag> {
        let (origin, axis) = tool.world_axis()?;
        let anchor = tool.anchor();
        let foot = origin + axis * (anchor - origin).dot(axis);
        let ray = self.tool_ray(position)?;
        let edge_on = ray.direction.normalize_or_zero().dot(axis).abs() < 0.08;
        let mut drag = AngleDrag {
            foot,
            axis,
            plane_point: anchor,
            last_raw: 0.0,
            accumulated: 0.0,
            start_value: tool.angle,
            press: position,
            started: false,
            edge_on,
        };
        drag.last_raw = drag.raw(ray).unwrap_or(0.0);
        Some(drag)
    }

    pub(crate) fn cancel_sketch_drag(&mut self, cx: &mut Context<Self>) {
        if let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_mut()
            && let Some(drag) = tool.drag.take()
        {
            match drag {
                ToolDrag::Distance(drag) => {
                    tool.distance = if tool.symmetric {
                        drag.start_value * 2.0
                    } else {
                        drag.start_value
                    };
                }
                ToolDrag::Angle(drag) => tool.angle = drag.start_value,
            }
            tool.snapped_face = false;
            self.sketch_tool_changed(false, cx);
        }
    }

    pub(crate) fn sketch_tool_move(
        &mut self,
        position: Vec2,
        held: bool,
        free: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_mut() else {
            return false;
        };
        if !held {
            // A release outside the window never arrived.
            tool.press = None;
            if tool.drag.take().is_some() {
                self.apply_kernel_preview(cx);
            }
        }
        let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_mut() else {
            return false;
        };
        // A press on a region becomes a drag once it travels.
        if let Some(press) = tool.press
            && held
            && press.screen.distance(position) > DRAG_PIXELS
        {
            tool.press = None;
            let normal = tool.regions.normal();
            let mut drag = AxisDrag::new(self, press.world, normal, 0.0, press.screen);
            let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_mut() else {
                return false;
            };
            drag.grab -= tool.handle_offset();
            drag.start_value = tool.handle_offset();
            drag.started = true;
            tool.drag = Some(ToolDrag::Distance(drag));
            tool.numeric.clear();
            self.clear_sketch_preview(cx);
        }
        let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_mut() else {
            return false;
        };
        if let Some(drag) = tool.drag.as_mut()
            && held
        {
            if !drag.started() {
                if drag.press().distance(position) <= DRAG_PIXELS {
                    return true;
                }
                drag.start();
                self.clear_sketch_preview(cx);
            }
            self.update_sketch_drag(position, free);
            self.refresh_sketch_status();
            return true;
        }
        // Hover: handle, then axis candidates, then regions.
        let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_ref() else {
            return false;
        };
        let handle_hot = match tool.kind {
            SketchToolKind::Extrude => tool
                .handle_screen(self)
                .is_some_and(|(a, b)| super::segment_distance(position, a, b) <= PICK_PIXELS),
            SketchToolKind::Revolve => tool
                .angle_handle_screen(self)
                .is_some_and(|h| h.distance(position) <= PICK_PIXELS + 3.0),
        };
        let hover_line = (tool.kind == SketchToolKind::Revolve && !handle_hot)
            .then(|| tool.sketch_lines_near(self, position))
            .flatten();
        let hover = (!handle_hot && hover_line.is_none())
            .then(|| {
                let ray = self.tool_ray(position)?;
                let (point, _, _) = tool.regions.ray_point(ray)?;
                tool.regions.region_at(point)
            })
            .flatten();
        if let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_mut()
            && (tool.hover, tool.hover_line, tool.handle_hot) != (hover, hover_line, handle_hot)
        {
            tool.hover = hover;
            tool.hover_line = hover_line;
            tool.handle_hot = handle_hot;
            cx.notify();
        }
        false
    }

    fn update_sketch_drag(&mut self, position: Vec2, free: bool) {
        let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_ref() else {
            return;
        };
        match tool.drag {
            Some(ToolDrag::Distance(drag)) => {
                let Some(raw) = drag.value(self, position) else {
                    return;
                };
                let anchor = tool.anchor();
                let normal = tool.regions.normal();
                let per_pixel = self.world_per_pixel(anchor + normal * tool.handle_offset());
                // Pixels per world unit along the axis on screen.
                let along = self
                    .world_to_screen(anchor)
                    .zip(self.world_to_screen(anchor + normal * per_pixel * 20.0))
                    .map(|(a, b)| a.distance(b) / (per_pixel * 20.0))
                    .filter(|p| *p > 1.0e-3)
                    .unwrap_or(1.0 / per_pixel.max(1.0e-9));
                let symmetric = tool.symmetric;
                let mut value = if symmetric { raw * 2.0 } else { raw };
                let mut snapped_face = false;
                if !free {
                    let step = geometry::snap_step(along);
                    let targets: &[f32] = if symmetric {
                        &[]
                    } else {
                        tool.target.as_ref().map_or(&[], |t| t.heights.as_slice())
                    };
                    let (snapped, face) = geometry::snap_value(value, step, targets, 8.0 / along);
                    value = snapped;
                    snapped_face = face.is_some();
                }
                if let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_mut() {
                    tool.distance = value;
                    tool.snapped_face = snapped_face;
                }
            }
            Some(ToolDrag::Angle(mut drag)) => {
                let Some(ray) = self.tool_ray(position) else {
                    return;
                };
                let raw = if drag.edge_on {
                    Some((position.x - drag.press.x) * 0.5f32.to_radians())
                } else {
                    drag.raw(ray)
                };
                let Some(raw) = raw else {
                    return;
                };
                let mut angle = if drag.edge_on {
                    drag.start_value + raw
                } else {
                    let mut delta = raw - drag.last_raw;
                    let tau = std::f32::consts::TAU;
                    delta = (delta + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI;
                    drag.accumulated += delta;
                    drag.last_raw = raw;
                    drag.start_value + drag.accumulated
                };
                angle = angle.clamp(-std::f32::consts::TAU, std::f32::consts::TAU);
                if !free {
                    angle = geometry::snap_angle(angle, ANGLE_STEP);
                }
                if let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_mut() {
                    tool.angle = angle;
                    tool.drag = Some(ToolDrag::Angle(drag));
                }
            }
            None => {}
        }
    }

    pub(crate) fn sketch_tool_release(&mut self, _position: Vec2, cx: &mut Context<Self>) {
        let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_mut() else {
            return;
        };
        tool.press = None;
        if tool.drag.take().is_some_and(|d| d.started()) {
            // The real kernel result replaces the ghost now.
            self.apply_sketch_preview(cx);
            self.refresh_sketch_status();
        }
        cx.notify();
    }

    /// Restores the live scene to the session's start, so a drag shows only
    /// the ghost (not a stale kernel result beside it).
    fn clear_sketch_preview(&mut self, cx: &mut Context<Self>) {
        self.cad_tool.preview_token += 1;
        let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_mut() else {
            return;
        };
        if tool.session.preview.take().is_some() {
            self.scene = tool.session.before.clone();
            self.invalidate(true, cx);
        }
    }

    /// Applies the candidate feature to a copy of the scene the session
    /// started from, so the kernel regenerates the real result.
    pub(crate) fn apply_sketch_preview(&mut self, cx: &mut Context<Self>) {
        let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_ref() else {
            return;
        };
        if tool.drag.as_ref().is_some_and(|d| d.started()) {
            return;
        }
        let key = tool.preview_key();
        if tool.session.preview.as_ref().is_some_and(|p| p.key == key) {
            return;
        }
        let mut scene = tool.session.before.clone();
        let result = tool.build_into(&mut scene);
        let had_preview = tool.session.preview.is_some();
        let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_mut() else {
            return;
        };
        match result {
            Ok(applied) => {
                tool.session.preview = Some(Preview {
                    key,
                    object: applied.object,
                    part: applied.part,
                    feature: applied.feature,
                });
                tool.error = None;
                keep_evaluated(&self.scene, &mut scene);
                self.scene = scene;
                self.invalidate(true, cx);
            }
            Err(message) => {
                // Nothing to preview yet (no distance or axis): not an error.
                let quiet = message.starts_with("Drag") || message.starts_with("Pick an axis");
                tool.error = (!quiet).then_some(message);
                if had_preview {
                    tool.session.preview = None;
                    self.scene = tool.session.before.clone();
                    self.invalidate(true, cx);
                }
            }
        }
        cx.notify();
    }

    /// Commits the sketch tool as one undo step: the new part or feature,
    /// with the consumed sketch hidden.
    pub(crate) fn commit_sketch_tool(&mut self, cx: &mut Context<Self>) {
        self.apply_typed_value();
        let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_ref() else {
            return;
        };
        if tool.typed().is_err() {
            self.status = format!("Invalid value · {}", tool.numeric);
            cx.notify();
            return;
        }
        let key = tool.preview_key();
        let reuse = tool
            .session
            .preview
            .as_ref()
            .filter(|p| p.key == key)
            .cloned();
        let (applied, mut scene) = match reuse {
            Some(preview) => (
                Applied {
                    object: preview.object,
                    part: preview.part,
                    feature: preview.feature,
                },
                std::mem::take(&mut self.scene),
            ),
            None => {
                let mut scene = tool.session.before.clone();
                match tool.build_into(&mut scene) {
                    Ok(applied) => {
                        keep_evaluated(&self.scene, &mut scene);
                        (applied, scene)
                    }
                    Err(message) => {
                        self.status = message.clone();
                        if let Some(Tool::Sketch(tool)) = self.cad_tool.tool.as_mut() {
                            tool.error = Some(message);
                        }
                        cx.notify();
                        return;
                    }
                }
            }
        };
        let Some(Tool::Sketch(tool)) = self.cad_tool.tool.take() else {
            unreachable!()
        };
        self.cad_tool.preview_token += 1;
        let mut tool = *tool;
        let before = std::mem::replace(&mut tool.session.before, Scene::empty());
        let sketch = tool.regions.object;
        if let Some(object) = scene.object_mut(sketch) {
            object.visible = false;
        }
        let operation = tool.operation();
        self.scene = scene;
        self.history.checkpoint_owned(before);
        self.select_objects([applied.object].into_iter().collect(), Some(applied.object));
        self.dirty = true;
        self.cad.announce = true;
        let stamp = self.scene.part(applied.part).map(|p| p.stamp).unwrap_or(0);
        let color = super::operation_color(&self.theme_colors(), operation);
        let (start, end) = tool.offsets();
        if tool.kind == SketchToolKind::Extrude {
            self.cad_tool.pending.push(PendingGhost {
                part: applied.part,
                stamp,
                prism: Some((tool.prism.clone(), start, end)),
                quads: Vec::new(),
                color,
            });
        }
        self.cad_tool.last = Some(super::LastOperation::sketch(
            &tool, applied, stamp, &self.scene,
        ));
        self.status = match tool.kind {
            SketchToolKind::Extrude => format!(
                "{} {} · {} · Regenerating…",
                if operation == Operation::NewPart {
                    "Extruded"
                } else {
                    operation.label()
                },
                crate::sketch::overlay::format_length(tool.distance),
                self.scene
                    .object(applied.object)
                    .map(|o| o.name.clone())
                    .unwrap_or_default()
            ),
            SketchToolKind::Revolve => format!(
                "Revolved {} · Regenerating…",
                crate::sketch::overlay::format_angle(tool.angle)
            ),
        };
        self.invalidate(true, cx);
    }
}

/// Carries finished tessellations of unchanged parts from `live` into a
/// freshly rebuilt `scene`, so re-applying a preview does not regenerate
/// parts the kernel already finished for the same content.
fn keep_evaluated(live: &Scene, scene: &mut Scene) {
    let finished: Vec<(u64, u64)> = scene
        .parts
        .iter()
        .filter(|part| !part.is_evaluated())
        .filter_map(|part| {
            let current = live.part(part.id)?;
            (current.stamp == part.stamp && current.is_evaluated()).then_some((part.id, part.stamp))
        })
        .collect();
    for (part_id, stamp) in finished {
        let Some(user) = live
            .objects
            .iter()
            .find(|o| o.part_id() == Some(part_id))
            .map(|o| o.id)
        else {
            continue;
        };
        let (Some(mesh), Some(evaluated)) = (
            live.object_mesh(user).cloned(),
            live.part(part_id).and_then(|p| p.evaluated.clone()),
        ) else {
            continue;
        };
        let _ = scene.set_part_tessellation(part_id, stamp, Some(mesh), evaluated.issues);
    }
}

impl AngleDrag {
    /// The pointer's angle around the axis from the anchor direction.
    fn raw(&self, ray: Ray) -> Option<f32> {
        let denominator = ray.direction.dot(self.axis);
        if denominator.abs() < 1.0e-6 {
            return None;
        }
        let t = (self.plane_point - ray.origin).dot(self.axis) / denominator;
        if !t.is_finite() || t < 0.0 {
            return None;
        }
        let q = ray.origin + ray.direction * t - self.foot;
        let reference = (self.plane_point - self.foot).try_normalize()?;
        let v = q - self.axis * q.dot(self.axis);
        Some(self.axis.dot(reference.cross(v)).atan2(reference.dot(v)))
    }
}

/// Whether the infinite line through `point` along `direction` meets the box.
fn line_hits_box(point: Vec3, direction: Vec3, lo: Vec3, hi: Vec3) -> bool {
    let mut near = f32::NEG_INFINITY;
    let mut far = f32::INFINITY;
    for axis in 0..3 {
        if direction[axis].abs() < 1.0e-9 {
            if point[axis] < lo[axis] || point[axis] > hi[axis] {
                return false;
            }
        } else {
            let a = (lo[axis] - point[axis]) / direction[axis];
            let b = (hi[axis] - point[axis]) / direction[axis];
            near = near.max(a.min(b));
            far = far.min(a.max(b));
        }
    }
    near <= far
}

#[cfg(test)]
mod tests {
    use super::*;
    use forma_core::{Primitive, SketchEntity};

    fn rectangle(min: Vec2, max: Vec2) -> Vec<SketchEntity> {
        let c = [min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)];
        (0..4)
            .map(|i| {
                SketchCurve::Line {
                    start: c[i],
                    end: c[(i + 1) % 4],
                }
                .into()
            })
            .collect()
    }

    fn ring_sketch(plane: SketchPlane) -> Sketch {
        let mut sketch = Sketch::new(plane);
        sketch.entities = rectangle(Vec2::new(-1.0, -0.6), Vec2::new(1.0, 0.6));
        sketch.entities.push(
            SketchCurve::Circle {
                center: Vec2::ZERO,
                radius: 0.3,
            }
            .into(),
        );
        sketch
    }

    fn tool_for(scene: &Scene, object: u64, kind: SketchToolKind) -> SketchTool {
        let sketch = scene.object_sketch(object).unwrap();
        let matrix = scene.world_transform(object).unwrap();
        let regions = Rc::new(Regions::build(object, "Sketch", sketch, matrix).unwrap());
        let mut tool = SketchTool {
            kind,
            selected: regions.defaults.iter().copied().collect(),
            regions,
            hover: None,
            hover_line: None,
            handle_hot: false,
            distance: 0.5,
            symmetric: false,
            angle: std::f32::consts::TAU,
            axis: None,
            manual_operation: None,
            target: None,
            numeric: String::new(),
            drag: None,
            press: None,
            prism: Rc::default(),
            lathe: None,
            session: Session::new(scene),
            snapped_face: false,
            error: None,
        };
        tool.rebuild_ghost();
        tool
    }

    #[test]
    fn regions_pick_the_ring_and_its_disc_separately() {
        let mut scene = Scene::empty();
        let id = scene
            .add_sketch("Sketch", ring_sketch(SketchPlane::ground()))
            .unwrap();
        let tool = tool_for(&scene, id, SketchToolKind::Extrude);
        let regions = &tool.regions;
        assert_eq!(regions.polygons.len(), 2);
        // The ring (with the circle as a hole) is the outermost default.
        assert_eq!(regions.defaults.len(), 1);
        let ring = regions.defaults[0];
        assert_eq!(regions.polygons[ring].len(), 2);
        assert_eq!(regions.region_at(Vec2::new(0.7, 0.0)), Some(ring));
        let disc = regions.region_at(Vec2::ZERO).unwrap();
        assert_ne!(disc, ring);
        assert_eq!(regions.region_at(Vec2::new(3.0, 0.0)), None);
        // The ghost of the ring is the rectangle minus the disc.
        let area = 2.0 * 1.2 - std::f32::consts::PI * 0.09;
        assert!((tool.prism.area() - area).abs() < 2.0e-3, "{}", tool.prism.area());
        assert!(tool.prism.normal.distance(Vec3::Y) < 1.0e-6);
    }

    #[test]
    fn new_parts_carry_the_sketch_picks_distance_and_source() {
        let mut scene = Scene::empty();
        let id = scene
            .add_sketch("Sketch", ring_sketch(SketchPlane::ground()))
            .unwrap();
        scene.object_mut(id).unwrap().transform.translation = Vec3::new(0.0, 0.0, 2.0);
        let tool = tool_for(&scene, id, SketchToolKind::Extrude);
        assert_eq!(tool.operation(), Operation::NewPart);
        let applied = tool.build_into(&mut scene).unwrap();
        let object = scene.object(applied.object).unwrap();
        assert_eq!(object.transform.translation, Vec3::new(0.0, 0.0, 2.0));
        let part = scene.part(applied.part).unwrap();
        assert_eq!(part.features.len(), 1);
        assert_eq!(part.features[0].source, Some(id));
        let FeatureKind::Extrude(extrude) = &part.features[0].kind else {
            panic!("not an extrusion");
        };
        assert_eq!(extrude.profiles.len(), 1);
        assert!((extrude.distance - 0.5).abs() < 1.0e-6);
        assert_eq!(extrude.operation, FeatureOperation::Join);
        assert_eq!(extrude.extent, ExtrudeExtent::OneSide);
        part.validate().unwrap();
    }

    #[test]
    fn sketches_on_part_faces_join_outward_and_cut_inward() {
        let mut scene = Scene::empty();
        // A part whose tessellation is a grouped cube, lifted so its top is at 2.
        let mut part = PartData::new(0, "Block");
        part.push_feature(
            "Box",
            FeatureKind::Primitive(forma_core::PrimitiveFeature {
                shape: forma_core::PrimitiveShape::Box {
                    size: Vec3::splat(2.0),
                },
                placement: Default::default(),
                operation: FeatureOperation::Join,
            }),
        );
        let block = scene.add_part("Block", part).unwrap();
        scene.object_mut(block).unwrap().transform.translation = Vec3::new(0.0, 1.0, 0.0);
        let cube = forma_core::Mesh::primitive(Primitive::Cube);
        let mesh = forma_core::Mesh {
            groups: (0..cube.faces.len() as u32).collect(),
            ..cube
        };
        let part_id = scene.object(block).unwrap().part_id().unwrap();
        let stamp = scene.part(part_id).unwrap().stamp;
        scene
            .set_part_tessellation(part_id, stamp, Some(mesh), Vec::new())
            .unwrap();
        let mut small = Sketch::new(SketchPlane {
            origin: Vec3::new(0.0, 2.0, 0.0),
            ..SketchPlane::ground()
        });
        small.entities = rectangle(Vec2::splat(-0.2), Vec2::splat(0.2));
        let id = scene.add_sketch("Sketch", small).unwrap();
        let mut tool = tool_for(&scene, id, SketchToolKind::Extrude);
        let studio_target = {
            // `infer_target` needs a Studio; exercise its pieces directly.
            let plane = tool.regions.world_plane();
            let samples = [tool.regions.to_world(tool.regions.samples[0].unwrap())];
            let mesh = scene.object_mesh(block).unwrap();
            let parts = [(block, scene.world_transform(block).unwrap(), mesh)];
            let face = geometry::find_coplanar_face(&plane, &samples, &parts).unwrap();
            Target {
                object: block,
                name: "Block".into(),
                outward: Some(face.outward),
                heights: geometry::parallel_heights(
                    mesh,
                    scene.world_transform(block).unwrap(),
                    plane.origin,
                    tool.regions.normal(),
                ),
            }
        };
        // The bottom face is 2 below the sketch: a "through" snap target.
        assert!(studio_target.heights.iter().any(|h| (h + 2.0).abs() < 1.0e-4));
        tool.target = Some(studio_target);
        tool.distance = 0.3;
        assert_eq!(tool.operation(), Operation::Join);
        assert!(tool.inferred());
        tool.distance = -0.3;
        assert_eq!(tool.operation(), Operation::Cut);
        let applied = tool.build_into(&mut scene).unwrap();
        assert_eq!(applied.object, block);
        let part = scene.object_part(block).unwrap();
        assert_eq!(part.features.len(), 2);
        let FeatureKind::Extrude(cut) = &part.features[1].kind else {
            panic!("not an extrusion");
        };
        assert_eq!(cut.operation, FeatureOperation::Cut);
        // In part space the sketch sits at the top face (y = 1 locally).
        assert!((cut.sketch.plane.origin.y - 1.0).abs() < 1.0e-5);
        assert!((cut.distance + 0.3).abs() < 1.0e-6);
        // A manual choice overrides inference.
        tool.manual_operation = Some(Operation::Intersect);
        assert_eq!(tool.operation(), Operation::Intersect);
    }

    #[test]
    fn revolve_needs_an_axis_beside_the_profile() {
        let mut scene = Scene::empty();
        let mut sketch = Sketch::new(SketchPlane::front());
        sketch.entities = rectangle(Vec2::new(0.2, 0.0), Vec2::new(0.6, 1.0));
        let id = scene.add_sketch("Profile", sketch).unwrap();
        let mut tool = tool_for(&scene, id, SketchToolKind::Revolve);
        assert!(tool.build_into(&mut scene.clone()).is_err());
        tool.set_sketch_axis(true);
        tool.rebuild_ghost();
        assert!(tool.revolve_problem().is_none());
        assert!(tool.lathe.is_some());
        let applied = tool.build_into(&mut scene).unwrap();
        let part = scene.part(applied.part).unwrap();
        let FeatureKind::Revolve(revolve) = &part.features[0].kind else {
            panic!("not a revolve");
        };
        assert_eq!(revolve.axis_direction, Vec2::Y);
        assert!((revolve.angle - std::f32::consts::TAU).abs() < 1.0e-6);
        // An axis through the middle of the profile is refused before the kernel.
        tool.axis = Some(RevolveAxis {
            origin: Vec2::new(0.4, 0.0),
            direction: Vec2::Y,
            source: AxisSource::SketchY,
        });
        assert!(tool.revolve_problem().is_some());
    }

    #[test]
    fn symmetric_extrusions_centre_on_the_plane() {
        let mut scene = Scene::empty();
        let id = scene
            .add_sketch("Sketch", ring_sketch(SketchPlane::ground()))
            .unwrap();
        let mut tool = tool_for(&scene, id, SketchToolKind::Extrude);
        tool.distance = -0.8;
        assert_eq!(tool.offsets(), (-0.8, 0.0));
        tool.symmetric = true;
        assert_eq!(tool.offsets(), (-0.4, 0.4));
        assert_eq!(tool.handle_offset(), 0.4);
        let applied = tool.build_into(&mut scene).unwrap();
        let FeatureKind::Extrude(extrude) = &scene.part(applied.part).unwrap().features[0].kind
        else {
            panic!()
        };
        assert_eq!(extrude.extent, ExtrudeExtent::Symmetric);
    }

    #[test]
    fn sketches_move_into_uniformly_scaled_part_space() {
        let sketch = ring_sketch(SketchPlane::ground());
        let scale = Mat4::from_scale(Vec3::splat(2.0));
        let (moved, picks, factor) = sketch_in_space(&sketch, &[Vec2::new(0.5, 0.0)], scale).unwrap();
        assert_eq!(factor, 2.0);
        assert_eq!(picks[0], Vec2::new(1.0, 0.0));
        assert!(matches!(moved.entities[4].curve, SketchCurve::Circle { radius, .. } if (radius - 0.6).abs() < 1.0e-6));
        let uneven = Mat4::from_scale(Vec3::new(1.0, 2.0, 1.0));
        assert!(sketch_in_space(&sketch, &[], uneven).is_err());
        assert!(line_hits_box(Vec3::ZERO, Vec3::Y, Vec3::new(-1.0, 3.0, -1.0), Vec3::ONE * 5.0));
        assert!(!line_hits_box(Vec3::new(9.0, 0.0, 0.0), Vec3::Y, Vec3::splat(-1.0), Vec3::ONE));
    }
}
