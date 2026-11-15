//! Parametric CAD parts: an ordered, editable feature history stored as plain
//! serializable data.
//!
//! A [`PartData`] block never contains kernel types. `forma-cad` regenerates
//! it into a B-rep solid and caches the tessellation in an ordinary
//! [`crate::MeshData`] block referenced by the part's object, so rendering,
//! picking and mesh exports work unchanged. The part's `stamp` identifies its
//! current content; `evaluated` records which stamp the cached tessellation
//! was produced from, so stale caches are detected after edits, undo and load.
use crate::{Mesh, Sketch};
use anyhow::{Context, Result, ensure};
use glam::{EulerRot, Mat4, Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Most features in one part, counting features nested in combine tools.
pub const MAX_PART_FEATURES: usize = 4096;
/// Deepest nesting of combine tools inside combine tools.
pub const MAX_TOOL_DEPTH: usize = 8;
/// Largest STEP document embedded as a base body.
pub const MAX_PART_STEP_BYTES: usize = 32 * 1024 * 1024;
/// Largest linear dimension or offset accepted by a feature, in metres.
pub const MAX_PART_LENGTH: f32 = 1.0e6;
/// Smallest dimension accepted by a feature, in metres.
pub const MIN_PART_LENGTH: f32 = 1.0e-6;
/// Most profile selection points or edge references in one feature.
pub const MAX_FEATURE_REFERENCES: usize = 4096;

/// A parametric part data block.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PartData {
    pub id: u64,
    pub name: String,
    /// Ordered history; each feature modifies the body produced by the ones before it.
    #[serde(default)]
    pub features: Vec<Feature>,
    /// Timeline marker: only the first `rollback` features are evaluated.
    /// `None` evaluates the whole history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback: Option<usize>,
    #[serde(default)]
    pub quality: TessellationQuality,
    /// Next unused feature id. Feature ids are stable for the part's lifetime.
    #[serde(default = "first_feature_id")]
    pub next_feature_id: u64,
    /// Content identity of `features`, `rollback` and `quality`; see [`PartData::restamp`].
    #[serde(default)]
    pub stamp: u64,
    /// The regeneration that produced the cached tessellation, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluated: Option<PartEvaluation>,
}

fn first_feature_id() -> u64 {
    1
}

/// Outcome of the regeneration behind a part's cached tessellation.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PartEvaluation {
    /// The part stamp that was evaluated.
    pub stamp: u64,
    /// Features that failed or were skipped, in history order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<FeatureIssue>,
}

/// A feature the kernel could not apply. The history continues from the body
/// before that feature, as in a CAD timeline with a failed feature.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureIssue {
    /// Feature id; 0 for problems that are not tied to one feature.
    pub feature: u64,
    pub message: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub suggestion: String,
}

/// One step of a part history.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Feature {
    /// Stable within its part (see [`PartData::next_feature_id`]).
    pub id: u64,
    pub name: String,
    /// Suppressed features are kept in the history but not evaluated.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub suppressed: bool,
    /// Scene object the feature was created from (a sketch, a mesh, a tool
    /// part), for provenance only. It may no longer exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<u64>,
    pub kind: FeatureKind,
}

impl Feature {
    pub fn new(id: u64, name: impl Into<String>, kind: FeatureKind) -> Self {
        Self {
            id,
            name: name.into(),
            suppressed: false,
            source: None,
            kind,
        }
    }
}

/// Feature parameters. Every variant is a plain struct so a property inspector
/// can edit one field at a time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FeatureKind {
    Primitive(PrimitiveFeature),
    Extrude(ExtrudeFeature),
    Revolve(RevolveFeature),
    Combine(CombineFeature),
    PushPull(PushPullFeature),
    Chamfer(ChamferFeature),
    BaseBody(BaseBodyFeature),
}

impl FeatureKind {
    /// Short human-readable type, for timelines and messages.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Primitive(feature) => feature.shape.label(),
            Self::Extrude(_) => "Extrude",
            Self::Revolve(_) => "Revolve",
            Self::Combine(feature) => match feature.operation {
                FeatureOperation::Join | FeatureOperation::NewBody => "Join",
                FeatureOperation::Cut => "Cut",
                FeatureOperation::Intersect => "Intersect",
            },
            Self::PushPull(_) => "Push/pull",
            Self::Chamfer(_) => "Chamfer",
            Self::BaseBody(feature) => match feature.body {
                BaseBody::Mesh { .. } => "Mesh body",
                BaseBody::Step { .. } => "STEP body",
            },
        }
    }

    /// How the feature's result combines with the body before it.
    pub fn operation(&self) -> FeatureOperation {
        match self {
            Self::Primitive(feature) => feature.operation,
            Self::Extrude(feature) => feature.operation,
            Self::Revolve(feature) => feature.operation,
            Self::Combine(feature) => feature.operation,
            Self::BaseBody(feature) => feature.operation,
            Self::PushPull(_) | Self::Chamfer(_) => FeatureOperation::Join,
        }
    }
}

/// How a feature's new solid combines with the part's current body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureOperation {
    /// Union with the current body (or start the body when it is empty).
    #[default]
    Join,
    /// Subtract from the current body.
    Cut,
    /// Keep only the overlap with the current body.
    Intersect,
    /// Add as a separate lump without a boolean.
    NewBody,
}

impl FeatureOperation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Join => "Join",
            Self::Cut => "Cut",
            Self::Intersect => "Intersect",
            Self::NewBody => "New body",
        }
    }
}

/// Position and orientation of a primitive in part space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub origin: Vec3,
    /// Euler XYZ angles in radians, matching object transforms.
    #[serde(default)]
    pub rotation: Vec3,
}

impl Placement {
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_rotation_translation(
            Quat::from_euler(
                EulerRot::XYZ,
                self.rotation.x,
                self.rotation.y,
                self.rotation.z,
            ),
            self.origin,
        )
    }

    fn is_valid(&self) -> bool {
        self.origin.is_finite()
            && self.origin.abs().max_element() <= MAX_PART_LENGTH
            && self.rotation.is_finite()
            && self.rotation.abs().max_element() <= 1.0e3
    }
}

/// A parametric primitive. Shapes are centered on the placement origin with
/// their axis along local +Y, matching Forma's mesh primitives.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrimitiveFeature {
    pub shape: PrimitiveShape,
    #[serde(default)]
    pub placement: Placement,
    #[serde(default)]
    pub operation: FeatureOperation,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PrimitiveShape {
    Box {
        size: Vec3,
    },
    Cylinder {
        radius: f32,
        height: f32,
    },
    Sphere {
        radius: f32,
    },
    /// A cone or frustum; `top_radius` may be zero.
    Cone {
        bottom_radius: f32,
        top_radius: f32,
        height: f32,
    },
    Torus {
        major_radius: f32,
        minor_radius: f32,
    },
}

impl PrimitiveShape {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Box { .. } => "Box",
            Self::Cylinder { .. } => "Cylinder",
            Self::Sphere { .. } => "Sphere",
            Self::Cone { .. } => "Cone",
            Self::Torus { .. } => "Torus",
        }
    }

    fn validate(&self) -> Result<()> {
        let length =
            |value: f32| value.is_finite() && (MIN_PART_LENGTH..=MAX_PART_LENGTH).contains(&value);
        match *self {
            Self::Box { size } => ensure!(
                length(size.x) && length(size.y) && length(size.z),
                "Box sizes must be between 1 µm and 1000 km"
            ),
            Self::Cylinder { radius, height } => ensure!(
                length(radius) && length(height),
                "Cylinder radius and height must be between 1 µm and 1000 km"
            ),
            Self::Sphere { radius } => ensure!(
                length(radius),
                "Sphere radius must be between 1 µm and 1000 km"
            ),
            Self::Cone {
                bottom_radius,
                top_radius,
                height,
            } => ensure!(
                length(bottom_radius)
                    && top_radius.is_finite()
                    && (0.0..=MAX_PART_LENGTH).contains(&top_radius)
                    && length(height),
                "Cone radii and height must be positive and below 1000 km"
            ),
            Self::Torus {
                major_radius,
                minor_radius,
            } => ensure!(
                length(major_radius) && length(minor_radius) && minor_radius < major_radius,
                "Torus radii must be positive and the tube must be thinner than the ring"
            ),
        }
        Ok(())
    }
}

/// Profiles of an embedded sketch swept along the sketch normal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExtrudeFeature {
    /// The consumed sketch, embedded so the part regenerates on its own. Its
    /// plane is in part space.
    pub sketch: Sketch,
    /// Interior points (sketch coordinates) picking regions of the sketch's
    /// arrangement (see `Sketch::profiles`); each point picks the region that
    /// contains it, and adjacent picks merge into one solid. Empty picks the
    /// outermost regions with their holes. Points survive sketch edits better
    /// than region indices.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<Vec2>,
    /// Signed distance along the sketch normal (the opposite way when negative).
    pub distance: f32,
    #[serde(default)]
    pub extent: ExtrudeExtent,
    /// Reverse the direction.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flip: bool,
    #[serde(default)]
    pub operation: FeatureOperation,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ExtrudeExtent {
    /// From the sketch plane by `distance`.
    #[default]
    OneSide,
    /// `|distance|` in total, centered on the sketch plane.
    Symmetric,
    /// `distance` forward and `back_distance` backward.
    TwoSides { back_distance: f32 },
}

/// Profiles of an embedded sketch revolved around an axis in the sketch plane.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RevolveFeature {
    pub sketch: Sketch,
    /// Region picks, as for [`ExtrudeFeature::profiles`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<Vec2>,
    /// A point on the axis, in sketch coordinates.
    pub axis_origin: Vec2,
    /// Axis direction in sketch coordinates.
    pub axis_direction: Vec2,
    /// Signed sweep in radians, at most one full turn.
    pub angle: f32,
    #[serde(default)]
    pub operation: FeatureOperation,
}

/// Booleans with tool bodies. Tools keep their own histories, so the combined
/// part stays editable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CombineFeature {
    pub operation: FeatureOperation,
    pub tools: Vec<ToolBody>,
}

/// A body defined by its own feature history, placed in the part.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolBody {
    pub name: String,
    /// Tool space to part space. May include non-uniform scale.
    pub transform: Mat4,
    pub features: Vec<Feature>,
}

/// A persistent reference to a face, stored geometrically and resolved at
/// regeneration time, because kernel face ids do not survive edits. The face
/// whose surface passes nearest `point` with a matching normal is chosen.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FaceRef {
    /// A point on the face in part space (for example the picked point).
    pub point: Vec3,
    /// Outward surface normal at `point`.
    pub normal: Vec3,
}

impl FaceRef {
    fn is_valid(&self) -> bool {
        self.point.is_finite()
            && self.point.abs().max_element() <= MAX_PART_LENGTH
            && self.normal.is_finite()
            && (self.normal.length() - 1.0).abs() < 1.0e-3
    }
}

/// A persistent reference to a straight edge by its end points in part space.
/// Resolution accepts the edge in either direction, or an edge collinear with
/// and covering the same segment.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeRef {
    pub start: Vec3,
    pub end: Vec3,
}

/// Offset a planar face along its normal: positive pulls material out,
/// negative pushes into the body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PushPullFeature {
    pub face: FaceRef,
    pub distance: f32,
}

/// Equal-distance chamfer of straight edges between planar faces.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChamferFeature {
    pub edges: Vec<EdgeRef>,
    pub distance: f32,
}

/// A body without parametric history: a converted mesh or an imported STEP file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BaseBodyFeature {
    pub body: BaseBody,
    #[serde(default)]
    pub operation: FeatureOperation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "format", rename_all = "snake_case")]
pub enum BaseBody {
    /// A closed, manifold polygon mesh in part space. Planar polygons become
    /// B-rep faces (coplanar neighbours merge); others are triangulated.
    Mesh { mesh: Mesh },
    /// An embedded STEP (ISO 10303-21) document, rendered and exported as is.
    /// truck cannot yet run booleans on imported STEP geometry.
    Step { text: String },
}

/// Tessellation density relative to the part's size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TessellationQuality {
    Coarse,
    #[default]
    Medium,
    Fine,
}

impl TessellationQuality {
    /// Chord deviation as a fraction of the part's bounding-box diagonal.
    pub fn chord_fraction(self) -> f64 {
        match self {
            Self::Coarse => 4.0e-3,
            Self::Medium => 1.2e-3,
            Self::Fine => 3.0e-4,
        }
    }

    /// Largest angle in radians a facet may turn through on a curved face.
    pub fn max_angle(self) -> f64 {
        match self {
            Self::Coarse => 22.0_f64.to_radians(),
            Self::Medium => 12.0_f64.to_radians(),
            Self::Fine => 6.0_f64.to_radians(),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Coarse => "Coarse",
            Self::Medium => "Medium",
            Self::Fine => "Fine",
        }
    }
}

impl PartData {
    /// An empty part. Callers normally push a first feature.
    pub fn new(id: u64, name: impl Into<String>) -> Self {
        let mut part = Self {
            id,
            name: name.into(),
            features: Vec::new(),
            rollback: None,
            quality: TessellationQuality::default(),
            next_feature_id: 1,
            stamp: 0,
            evaluated: None,
        };
        part.restamp();
        part
    }

    /// Append a feature, returning its id. Restamps the part.
    pub fn push_feature(&mut self, name: impl Into<String>, kind: FeatureKind) -> u64 {
        let id = self.allocate_feature_id();
        self.features.push(Feature::new(id, name, kind));
        self.restamp();
        id
    }

    /// A fresh feature id, for callers assembling features themselves.
    pub fn allocate_feature_id(&mut self) -> u64 {
        let highest = all_features(&self.features)
            .map(|feature| feature.id)
            .max()
            .unwrap_or(0);
        let id = self.next_feature_id.max(highest.saturating_add(1)).max(1);
        self.next_feature_id = id.saturating_add(1);
        id
    }

    pub fn feature(&self, id: u64) -> Option<&Feature> {
        self.features.iter().find(|feature| feature.id == id)
    }

    /// Edit one feature in place, then restamp. Returns false when it does not exist.
    pub fn edit_feature(&mut self, id: u64, edit: impl FnOnce(&mut Feature)) -> bool {
        let Some(feature) = self.features.iter_mut().find(|feature| feature.id == id) else {
            return false;
        };
        edit(feature);
        self.restamp();
        true
    }

    /// Features evaluated at regeneration: before the rollback marker and not suppressed.
    pub fn active_features(&self) -> impl Iterator<Item = &Feature> {
        let end = self
            .rollback
            .unwrap_or(self.features.len())
            .min(self.features.len());
        self.features[..end]
            .iter()
            .filter(|feature| !feature.suppressed)
    }

    /// Stable 64-bit identity of everything that affects regeneration
    /// (features, rollback and quality). Feature and tool names and feature
    /// provenance are excluded (see [`FeatureGeometry`]), so renaming or
    /// relinking never invalidates the cached tessellation. Computed with FNV-1a over canonical JSON, so it is equal
    /// across processes and Forma versions that serialize identically.
    pub fn content_hash(&self) -> u64 {
        #[derive(Serialize)]
        struct Content<'a> {
            features: FeaturesGeometry<'a>,
            rollback: Option<usize>,
            quality: TessellationQuality,
        }
        let mut hasher = Fnv1a::default();
        // Serializing plain data into a writer cannot fail.
        let _ = serde_json::to_writer(
            &mut hasher,
            &Content {
                features: FeaturesGeometry(&self.features),
                rollback: self.rollback,
                quality: self.quality,
            },
        );
        // Zero is reserved for "never stamped".
        hasher.0.max(1)
    }

    /// Record the current content identity. Call after editing `features`,
    /// `rollback` or `quality` directly; the editing helpers do it for you.
    pub fn restamp(&mut self) {
        self.stamp = self.content_hash();
    }

    /// Whether the cached tessellation was produced from the current content.
    pub fn is_evaluated(&self) -> bool {
        self.evaluated
            .as_ref()
            .is_some_and(|evaluated| evaluated.stamp == self.stamp)
    }

    /// Issues from the latest regeneration of the current content.
    pub fn issues(&self) -> &[FeatureIssue] {
        match &self.evaluated {
            Some(evaluated) if evaluated.stamp == self.stamp => &evaluated.issues,
            _ => &[],
        }
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(valid_name(&self.name), "Invalid part name");
        ensure!(
            self.next_feature_id > 0 && self.next_feature_id < u64::MAX,
            "Invalid next feature id in part {}",
            self.name
        );
        if let Some(rollback) = self.rollback {
            ensure!(
                rollback <= self.features.len(),
                "Rollback marker is past the end of part {}",
                self.name
            );
        }
        let mut count = 0;
        let mut ids = BTreeSet::new();
        validate_features(&self.features, 0, &mut count, &mut ids)
            .with_context(|| format!("Invalid feature in part {}", self.name))?;
        ensure!(
            ids.last().is_none_or(|&id| id < self.next_feature_id),
            "Feature ids in part {} must be below its next feature id",
            self.name
        );
        if let Some(evaluated) = &self.evaluated {
            ensure!(
                evaluated.issues.len() <= MAX_PART_FEATURES + 16,
                "Too many feature issues in part {}",
                self.name
            );
            for issue in &evaluated.issues {
                ensure!(
                    issue.message.len() <= 4096 && issue.suggestion.len() <= 4096,
                    "Feature issue text is too long in part {}",
                    self.name
                );
            }
        }
        Ok(())
    }

    /// Rough memory held by this block, for undo history budgets.
    pub fn approximate_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.name.len()
            + all_features(&self.features)
                .map(|feature| {
                    std::mem::size_of::<Feature>()
                        + feature.name.len()
                        + match &feature.kind {
                            FeatureKind::Extrude(extrude) => {
                                extrude.sketch.entities.len() * 40 + extrude.profiles.len() * 8
                            }
                            FeatureKind::Revolve(revolve) => {
                                revolve.sketch.entities.len() * 40 + revolve.profiles.len() * 8
                            }
                            FeatureKind::Chamfer(chamfer) => chamfer.edges.len() * 24,
                            FeatureKind::BaseBody(base) => match &base.body {
                                BaseBody::Mesh { mesh } => {
                                    mesh.positions.len() * 12
                                        + mesh.faces.iter().map(|f| f.len() * 4 + 24).sum::<usize>()
                                }
                                BaseBody::Step { text } => text.len(),
                            },
                            _ => 0,
                        }
                })
                .sum::<usize>()
    }
}

/// The parts of a feature that affect its geometry: id, suppression and
/// parameters (recursively through combine tools), but not names or
/// provenance. Content stamps and kernel caches hash this view, so renaming
/// a feature or relinking its source never forces a regeneration.
pub struct FeatureGeometry<'a>(pub &'a Feature);

/// [`FeatureGeometry`] of each feature in a history.
pub struct FeaturesGeometry<'a>(pub &'a [Feature]);

struct KindGeometry<'a>(&'a FeatureKind);

struct ToolGeometry<'a>(&'a ToolBody);

impl Serialize for FeaturesGeometry<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for feature in self.0 {
            seq.serialize_element(&FeatureGeometry(feature))?;
        }
        seq.end()
    }
}

impl Serialize for FeatureGeometry<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("Feature", 3)?;
        state.serialize_field("id", &self.0.id)?;
        state.serialize_field("suppressed", &self.0.suppressed)?;
        state.serialize_field("kind", &KindGeometry(&self.0.kind))?;
        state.end()
    }
}

impl Serialize for KindGeometry<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        match self.0 {
            FeatureKind::Combine(combine) => {
                let tools: Vec<ToolGeometry<'_>> = combine.tools.iter().map(ToolGeometry).collect();
                let mut state = serializer.serialize_struct("Combine", 3)?;
                state.serialize_field("type", "combine")?;
                state.serialize_field("operation", &combine.operation)?;
                state.serialize_field("tools", &tools)?;
                state.end()
            }
            other => other.serialize(serializer),
        }
    }
}

impl Serialize for ToolGeometry<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("ToolBody", 2)?;
        state.serialize_field("transform", &self.0.transform)?;
        state.serialize_field("features", &FeaturesGeometry(&self.0.features))?;
        state.end()
    }
}

/// Every feature, depth first, including features nested in combine tools.
pub fn all_features(features: &[Feature]) -> Box<dyn Iterator<Item = &Feature> + '_> {
    Box::new(features.iter().flat_map(|feature| {
        let nested: Box<dyn Iterator<Item = &Feature>> = match &feature.kind {
            FeatureKind::Combine(combine) => Box::new(
                combine
                    .tools
                    .iter()
                    .flat_map(|tool| all_features(&tool.features)),
            ),
            _ => Box::new(std::iter::empty()),
        };
        std::iter::once(feature).chain(nested)
    }))
}

fn validate_features(
    features: &[Feature],
    depth: usize,
    count: &mut usize,
    ids: &mut BTreeSet<u64>,
) -> Result<()> {
    ensure!(
        depth <= MAX_TOOL_DEPTH,
        "Combine tools are nested too deeply"
    );
    for feature in features {
        *count += 1;
        ensure!(
            *count <= MAX_PART_FEATURES,
            "Part exceeds {MAX_PART_FEATURES} features"
        );
        ensure!(
            feature.id > 0 && ids.insert(feature.id),
            "Feature ids must be nonzero and unique"
        );
        ensure!(valid_name(&feature.name), "Invalid feature name");
        validate_kind(&feature.kind, depth, count, ids)
            .with_context(|| format!("Feature {}", feature.name))?;
    }
    Ok(())
}

fn validate_kind(
    kind: &FeatureKind,
    depth: usize,
    count: &mut usize,
    ids: &mut BTreeSet<u64>,
) -> Result<()> {
    let distance = |value: f32| {
        value.is_finite() && value.abs() >= MIN_PART_LENGTH && value.abs() <= MAX_PART_LENGTH
    };
    let points = |points: &[Vec2]| {
        points.len() <= MAX_FEATURE_REFERENCES
            && points
                .iter()
                .all(|p| p.is_finite() && p.abs().max_element() <= crate::MAX_SKETCH_COORDINATE)
    };
    match kind {
        FeatureKind::Primitive(feature) => {
            feature.shape.validate()?;
            ensure!(feature.placement.is_valid(), "Invalid primitive placement");
        }
        FeatureKind::Extrude(feature) => {
            feature.sketch.validate()?;
            ensure!(points(&feature.profiles), "Invalid profile selection");
            ensure!(
                distance(feature.distance),
                "Extrude distance must be nonzero and below 1000 km"
            );
            if let ExtrudeExtent::TwoSides { back_distance } = feature.extent {
                ensure!(
                    back_distance.is_finite() && back_distance.abs() <= MAX_PART_LENGTH,
                    "Invalid second extrude distance"
                );
            }
        }
        FeatureKind::Revolve(feature) => {
            feature.sketch.validate()?;
            ensure!(points(&feature.profiles), "Invalid profile selection");
            ensure!(
                feature.axis_origin.is_finite()
                    && feature.axis_origin.abs().max_element() <= crate::MAX_SKETCH_COORDINATE
                    && feature.axis_direction.is_finite()
                    && feature.axis_direction.length() > 1.0e-6,
                "Invalid revolve axis"
            );
            ensure!(
                feature.angle.is_finite()
                    && feature.angle.abs() > 1.0e-4
                    && feature.angle.abs() <= std::f32::consts::TAU + 1.0e-4,
                "Revolve angle must be nonzero and at most one turn"
            );
        }
        FeatureKind::Combine(feature) => {
            ensure!(
                !feature.tools.is_empty() && feature.tools.len() <= 256,
                "Combine needs one to 256 tool bodies"
            );
            for tool in &feature.tools {
                ensure!(valid_name(&tool.name), "Invalid tool name");
                ensure!(
                    tool.transform.is_finite() && tool.transform.determinant().abs() >= 1.0e-12,
                    "Tool transform must be finite and invertible"
                );
                ensure!(!tool.features.is_empty(), "Tool body has no features");
                validate_features(&tool.features, depth + 1, count, ids)?;
            }
        }
        FeatureKind::PushPull(feature) => {
            ensure!(feature.face.is_valid(), "Invalid face reference");
            ensure!(
                distance(feature.distance),
                "Push/pull distance must be nonzero and below 1000 km"
            );
        }
        FeatureKind::Chamfer(feature) => {
            ensure!(
                !feature.edges.is_empty() && feature.edges.len() <= MAX_FEATURE_REFERENCES,
                "Chamfer needs one to {MAX_FEATURE_REFERENCES} edges"
            );
            ensure!(
                feature.edges.iter().all(|edge| {
                    edge.start.is_finite()
                        && edge.end.is_finite()
                        && edge.start.abs().max_element() <= MAX_PART_LENGTH
                        && edge.end.abs().max_element() <= MAX_PART_LENGTH
                        && edge.start.distance(edge.end) >= MIN_PART_LENGTH
                }),
                "Invalid chamfer edge reference"
            );
            ensure!(
                feature.distance.is_finite()
                    && (MIN_PART_LENGTH..=MAX_PART_LENGTH).contains(&feature.distance),
                "Chamfer distance must be positive and below 1000 km"
            );
        }
        FeatureKind::BaseBody(feature) => match &feature.body {
            BaseBody::Mesh { mesh } => {
                mesh.validate().context("Invalid base mesh")?;
                ensure!(
                    mesh.faces.len() <= 200_000,
                    "Base meshes are limited to 200,000 faces"
                );
            }
            BaseBody::Step { text } => ensure!(
                !text.is_empty() && text.len() <= MAX_PART_STEP_BYTES,
                "Embedded STEP data must be between 1 byte and 32 MiB"
            ),
        },
    }
    Ok(())
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 512 && !name.chars().any(char::is_control)
}

/// 64-bit FNV-1a, used for stable content stamps.
struct Fnv1a(u64);

impl Default for Fnv1a {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl std::io::Write for Fnv1a {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        for &byte in bytes {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SketchCurve, SketchPlane};

    fn box_feature() -> FeatureKind {
        FeatureKind::Primitive(PrimitiveFeature {
            shape: PrimitiveShape::Box {
                size: Vec3::new(1.0, 2.0, 3.0),
            },
            placement: Placement::default(),
            operation: FeatureOperation::Join,
        })
    }

    #[test]
    fn stamps_follow_content_and_ignore_names() {
        let mut part = PartData::new(7, "Bracket");
        let empty = part.stamp;
        let id = part.push_feature("Box", box_feature());
        assert_ne!(part.stamp, empty);
        let stamp = part.stamp;
        part.name = "Renamed".into();
        assert_eq!(part.content_hash(), stamp);
        assert!(part.edit_feature(id, |feature| feature.suppressed = true));
        assert_ne!(part.stamp, stamp);
        assert_eq!(part.active_features().count(), 0);
        part.edit_feature(id, |feature| feature.suppressed = false);
        assert_eq!(part.stamp, stamp, "identical content restores the stamp");
        part.evaluated = Some(PartEvaluation {
            stamp,
            issues: Vec::new(),
        });
        assert!(part.is_evaluated());
        part.rollback = Some(0);
        part.restamp();
        assert!(!part.is_evaluated());
        assert_eq!(part.active_features().count(), 0);
    }

    #[test]
    fn names_and_provenance_never_change_the_stamp() {
        // Review case: renaming a feature or relinking its sketch changed the
        // hash without a restamp, so reopened files regenerated the part.
        let mut part = PartData::new(1, "Part");
        let id = part.push_feature("Box", box_feature());
        let tool = ToolBody {
            name: "Tool".into(),
            transform: Mat4::IDENTITY,
            features: vec![Feature::new(50, "Inner", box_feature())],
        };
        part.features.push(Feature::new(
            40,
            "Join",
            FeatureKind::Combine(CombineFeature {
                operation: FeatureOperation::Join,
                tools: vec![tool],
            }),
        ));
        part.next_feature_id = 100;
        part.restamp();
        let stamp = part.stamp;
        part.features[0].name = "Renamed".into();
        part.features[0].source = Some(77);
        part.name = "Other".into();
        if let FeatureKind::Combine(combine) = &mut part.features[1].kind {
            combine.tools[0].name = "Renamed tool".into();
            combine.tools[0].features[0].name = "Renamed inner".into();
            combine.tools[0].features[0].source = Some(5);
        }
        assert_eq!(part.content_hash(), stamp);
        // Geometry still counts, including inside tools.
        if let FeatureKind::Combine(combine) = &mut part.features[1].kind {
            combine.tools[0].transform = Mat4::from_translation(Vec3::X);
        }
        assert_ne!(part.content_hash(), stamp);
        if let FeatureKind::Combine(combine) = &mut part.features[1].kind {
            combine.tools[0].transform = Mat4::IDENTITY;
        }
        assert_eq!(part.content_hash(), stamp);
        part.edit_feature(id, |feature| feature.suppressed = true);
        assert_ne!(part.stamp, stamp);
    }

    #[test]
    fn features_round_trip_through_tagged_json() {
        let mut part = PartData::new(3, "Housing");
        part.push_feature("Box", box_feature());
        let mut sketch = Sketch::new(SketchPlane::ground());
        sketch.entities.push(
            SketchCurve::Circle {
                center: Vec2::ZERO,
                radius: 0.25,
            }
            .into(),
        );
        part.push_feature(
            "Extrude",
            FeatureKind::Extrude(ExtrudeFeature {
                sketch,
                profiles: vec![Vec2::ZERO],
                distance: 0.5,
                extent: ExtrudeExtent::TwoSides { back_distance: 0.1 },
                flip: false,
                operation: FeatureOperation::Cut,
            }),
        );
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains("\"type\":\"extrude\"") && json.contains("\"mode\":\"two_sides\""));
        let back: PartData = serde_json::from_str(&json).unwrap();
        assert_eq!(back, part);
        assert_eq!(back.content_hash(), part.stamp);
        back.validate().unwrap();
    }

    #[test]
    fn validation_rejects_bad_parameters_and_duplicate_ids() {
        let mut part = PartData::new(1, "Part");
        part.push_feature("Box", box_feature());
        part.validate().unwrap();
        let mut bad = part.clone();
        bad.features[0].kind = FeatureKind::Primitive(PrimitiveFeature {
            shape: PrimitiveShape::Box {
                size: Vec3::new(f32::NAN, 1.0, 1.0),
            },
            placement: Placement::default(),
            operation: FeatureOperation::Join,
        });
        assert!(bad.validate().is_err());
        let mut duplicate = part.clone();
        duplicate.features.push(duplicate.features[0].clone());
        assert!(duplicate.validate().is_err());
        let mut rolled = part.clone();
        rolled.rollback = Some(5);
        assert!(rolled.validate().is_err());
        let mut torus = part.clone();
        torus.features[0].kind = FeatureKind::Primitive(PrimitiveFeature {
            shape: PrimitiveShape::Torus {
                major_radius: 0.2,
                minor_radius: 0.5,
            },
            placement: Placement::default(),
            operation: FeatureOperation::Join,
        });
        assert!(torus.validate().is_err());
        let mut deep = part.clone();
        let mut tool = ToolBody {
            name: "Tool".into(),
            transform: Mat4::IDENTITY,
            features: vec![Feature::new(100, "Box", box_feature())],
        };
        for depth in 0..=MAX_TOOL_DEPTH {
            tool = ToolBody {
                name: "Tool".into(),
                transform: Mat4::IDENTITY,
                features: vec![Feature::new(
                    200 + depth as u64,
                    "Join",
                    FeatureKind::Combine(CombineFeature {
                        operation: FeatureOperation::Join,
                        tools: vec![tool],
                    }),
                )],
            };
        }
        deep.features.push(Feature::new(
            50,
            "Join",
            FeatureKind::Combine(CombineFeature {
                operation: FeatureOperation::Join,
                tools: vec![tool],
            }),
        ));
        deep.next_feature_id = 1000;
        assert!(deep.validate().is_err());
    }

    #[test]
    fn nested_tool_features_reserve_ids() {
        let mut part = PartData::new(1, "Part");
        part.features.push(Feature::new(
            1,
            "Join",
            FeatureKind::Combine(CombineFeature {
                operation: FeatureOperation::Join,
                tools: vec![ToolBody {
                    name: "Tool".into(),
                    transform: Mat4::IDENTITY,
                    features: vec![Feature::new(9, "Box", box_feature())],
                }],
            }),
        ));
        assert_eq!(part.allocate_feature_id(), 10);
        assert_eq!(all_features(&part.features).count(), 2);
    }
}
