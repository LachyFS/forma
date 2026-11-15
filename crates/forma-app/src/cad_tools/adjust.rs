//! "Adjust last operation": after a tool commits, its key parameters stay
//! editable in a small bottom-left panel, like Blender's redo panel. Edits
//! amend the committed feature (or redo the mesh push) in place, merged into
//! the same undo step, and regenerate. The panel goes away on the next
//! unrelated action: a command, a selection click, another tool, or any
//! change to the object it refers to.
use super::PendingGhost;
use super::edit::ChamferEdge;
use super::geometry::Prism;
use super::push_pull::{ActiveFaces, FacePick};
use super::sketch_tool::{Applied, Operation, SketchTool, SketchToolKind};
use crate::app::{EditMode, Studio};
use forma_core::{FeatureKind, Mesh, Scene, Transform, Vec3};
use gpui::Context;
use std::rc::Rc;

/// Parameters the panel edits through numeric fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdjustParam {
    Distance,
    Angle,
}

impl AdjustParam {
    /// The inspector field editing the same value.
    pub(crate) fn feature_field(self) -> crate::timeline::FeatureField {
        match self {
            Self::Distance => crate::timeline::FeatureField::Distance,
            Self::Angle => crate::timeline::FeatureField::Angle,
        }
    }

    /// Display units per pointer pixel when scrubbing, and decimals shown.
    pub(crate) fn scrub_step(self) -> (f32, usize) {
        match self {
            Self::Distance => (0.005, 3),
            Self::Angle => (0.5, 1),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum LastKind {
    Extrude {
        symmetric: bool,
        operation: Operation,
        prism: Rc<Prism>,
    },
    Revolve {
        operation: Operation,
    },
    PushPull {
        features: Vec<u64>,
        prisms: Vec<Rc<Prism>>,
    },
    Chamfer {
        edges: Vec<ChamferEdge>,
    },
    MeshPushPull {
        before: Mesh,
        face: usize,
        /// Object-space offset per world unit of distance.
        direction: Vec3,
    },
}

/// The operation the panel adjusts.
#[derive(Clone, Debug)]
pub(crate) struct LastOperation {
    pub kind: LastKind,
    pub object: u64,
    pub part: Option<u64>,
    /// The feature amended (the first, for multi-face push/pull).
    pub feature: u64,
    /// The part content after the commit or latest amendment.
    pub stamp: u64,
    /// World distance or angle in radians.
    pub value: f32,
    /// Feature units per world unit.
    pub scale: f32,
    transform: Transform,
    mesh_marker: Option<(usize, usize)>,
    edit_mode: EditMode,
}

impl LastOperation {
    fn base(
        scene: &Scene,
        object: u64,
        edit_mode: EditMode,
    ) -> (Transform, Option<(usize, usize)>) {
        let transform = scene
            .object(object)
            .map(|o| o.transform)
            .unwrap_or_default();
        let marker = scene
            .object(object)
            .filter(|o| !o.is_part())
            .and_then(|_| scene.object_mesh(object))
            .map(|m| (m.positions.len(), m.faces.len()));
        let _ = edit_mode;
        (transform, marker)
    }

    pub(crate) fn sketch(tool: &SketchTool, applied: Applied, stamp: u64, scene: &Scene) -> Self {
        let (transform, mesh_marker) = Self::base(scene, applied.object, EditMode::Object);
        let operation = tool.operation();
        let (kind, value, scale) = match tool.kind {
            SketchToolKind::Extrude => {
                let part_matrix = scene.world_transform(applied.object).unwrap_or_default();
                let world_normal = part_matrix
                    .inverse()
                    .transform_vector3(tool.regions.normal())
                    .length()
                    .max(1.0e-9);
                (
                    LastKind::Extrude {
                        symmetric: tool.symmetric,
                        operation,
                        prism: tool.prism.clone(),
                    },
                    tool.distance,
                    world_normal,
                )
            }
            SketchToolKind::Revolve => (LastKind::Revolve { operation }, tool.angle, 1.0),
        };
        Self {
            kind,
            object: applied.object,
            part: Some(applied.part),
            feature: applied.feature,
            stamp,
            value,
            scale,
            transform,
            mesh_marker,
            edit_mode: EditMode::Object,
        }
    }

    pub(crate) fn push_pull(
        active: &ActiveFaces,
        first: &FacePick,
        features: Vec<u64>,
        mesh: Option<(Mesh, usize, Vec3)>,
        scene: &Scene,
        edit_mode: EditMode,
    ) -> Option<Self> {
        let (transform, mesh_marker) = Self::base(scene, first.object, edit_mode);
        let part = scene.object(first.object).and_then(|o| o.part_id());
        let stamp = part.and_then(|p| scene.part(p)).map_or(0, |p| p.stamp);
        let kind = match mesh {
            Some((before, face, direction)) => LastKind::MeshPushPull {
                before,
                face,
                direction,
            },
            None => LastKind::PushPull {
                prisms: active.faces.iter().map(|f| f.prism.clone()).collect(),
                features: features.clone(),
            },
        };
        Some(Self {
            kind,
            object: first.object,
            part,
            feature: features.first().copied().unwrap_or(0),
            stamp,
            value: active.distance,
            scale: 1.0 / first.scale,
            transform,
            mesh_marker,
            edit_mode,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn chamfer(
        object: u64,
        part: u64,
        feature: u64,
        stamp: u64,
        distance: f32,
        scale: f32,
        edges: Vec<ChamferEdge>,
        scene: &Scene,
        edit_mode: EditMode,
    ) -> Self {
        let (transform, mesh_marker) = Self::base(scene, object, edit_mode);
        Self {
            kind: LastKind::Chamfer { edges },
            object,
            part: Some(part),
            feature,
            stamp,
            value: distance,
            scale: 1.0 / scale,
            transform,
            mesh_marker,
            edit_mode,
        }
    }

    pub(crate) fn label(&self) -> &'static str {
        match &self.kind {
            LastKind::Extrude { operation, .. } => match operation {
                Operation::Cut => "Cut",
                Operation::Intersect => "Intersect",
                _ => "Extrude",
            },
            LastKind::Revolve { .. } => "Revolve",
            LastKind::PushPull { .. } | LastKind::MeshPushPull { .. } => "Push/Pull",
            LastKind::Chamfer { .. } => "Chamfer",
        }
    }

    pub(crate) fn edge_count(&self) -> usize {
        match &self.kind {
            LastKind::Chamfer { edges } => edges.len(),
            _ => 0,
        }
    }

    /// The boolean of a sketch feature on an existing part, if adjustable.
    pub(crate) fn operation(&self) -> Option<Operation> {
        match &self.kind {
            LastKind::Extrude { operation, .. } | LastKind::Revolve { operation } => {
                Some(*operation)
            }
            _ => None,
        }
    }

    pub(crate) fn symmetric(&self) -> Option<bool> {
        match &self.kind {
            LastKind::Extrude { symmetric, .. } => Some(*symmetric),
            _ => None,
        }
    }

    pub(crate) fn param(&self) -> AdjustParam {
        match self.kind {
            LastKind::Revolve { .. } => AdjustParam::Angle,
            _ => AdjustParam::Distance,
        }
    }
}

impl Studio {
    /// The last operation, while nothing has happened since that would make
    /// adjusting it surprising.
    pub(crate) fn adjustable_operation(&self) -> Option<&LastOperation> {
        let last = self.cad_tool.last.as_ref()?;
        if self.is_drawing()
            || self.cad_tool.active()
            || self.transform_drag.is_some()
            || self.selected != Some(last.object)
            || self.edit_mode != last.edit_mode
        {
            return None;
        }
        let object = self.scene.object(last.object)?;
        if object.transform != last.transform {
            return None;
        }
        match last.part {
            Some(part) => {
                let part = self.scene.part(part)?;
                (part.stamp == last.stamp && part.feature(last.feature).is_some()).then_some(last)
            }
            None => {
                let mesh = self.scene.object_mesh(last.object)?;
                (Some((mesh.positions.len(), mesh.faces.len())) == last.mesh_marker).then_some(last)
            }
        }
    }

    /// The adjustable value of a panel field, in display units. Part
    /// features are read through the inspector's `timeline::params`, so the
    /// panel and the inspector always show the same value.
    pub(crate) fn adjust_value(&self, param: AdjustParam) -> Option<f32> {
        let last = self.adjustable_operation()?;
        if let Some(feature) = last
            .part
            .and_then(|part| self.scene.part(part))
            .and_then(|part| part.feature(last.feature))
        {
            let value = crate::timeline::params::value(feature, param.feature_field())?;
            return Some(match param {
                AdjustParam::Distance => value / last.scale,
                AdjustParam::Angle => value,
            });
        }
        Some(match param {
            AdjustParam::Distance => last.value,
            AdjustParam::Angle => last.value.to_degrees(),
        })
    }

    /// Commits a typed or scrubbed panel value, amending the operation.
    pub(crate) fn commit_adjust(&mut self, param: AdjustParam, text: &str, cx: &mut Context<Self>) {
        let parsed = match param {
            AdjustParam::Distance => crate::transform::distance_value(text),
            AdjustParam::Angle => crate::transform::angle_value(text),
        };
        let Some(value) = parsed else {
            self.status = match param {
                AdjustParam::Distance => "Enter a distance expression, e.g. 25mm, 1/8in or 1m+2cm",
                AdjustParam::Angle => "Enter an angle expression, e.g. 90, 45deg or 1.57rad",
            }
            .into();
            cx.notify();
            return;
        };
        self.active_field = None;
        if self.adjustable_operation().is_none() {
            self.status = "Nothing to adjust".into();
            cx.notify();
            return;
        }
        let value = match param {
            AdjustParam::Distance => value,
            AdjustParam::Angle => value.clamp(-std::f32::consts::TAU, std::f32::consts::TAU),
        };
        self.amend_last(|last, kind| amend_value(last, kind, value), cx);
    }

    pub(crate) fn adjust_operation(&mut self, operation: Operation, cx: &mut Context<Self>) {
        if self.adjustable_operation().is_none() || operation == Operation::NewPart {
            return;
        }
        self.amend_last(
            |last, kind| {
                match kind {
                    FeatureKind::Extrude(e) => e.operation = operation.feature(),
                    FeatureKind::Revolve(r) => r.operation = operation.feature(),
                    _ => return Err("This operation has no boolean".into()),
                }
                match &mut last.kind {
                    LastKind::Extrude { operation: o, .. } | LastKind::Revolve { operation: o } => {
                        *o = operation
                    }
                    _ => {}
                }
                Ok(())
            },
            cx,
        );
    }

    pub(crate) fn adjust_symmetric(&mut self, cx: &mut Context<Self>) {
        if self.adjustable_operation().is_none() {
            return;
        }
        self.amend_last(
            |last, kind| {
                let FeatureKind::Extrude(extrude) = kind else {
                    return Err("Only extrusions can be symmetric".into());
                };
                let LastKind::Extrude { symmetric, .. } = &mut last.kind else {
                    return Err("Only extrusions can be symmetric".into());
                };
                *symmetric = !*symmetric;
                let mode = if *symmetric {
                    crate::timeline::ExtentMode::Symmetric
                } else {
                    crate::timeline::ExtentMode::OneSide
                };
                extrude.extent = mode.apply(extrude.extent, extrude.distance);
                Ok(())
            },
            cx,
        );
    }

    pub(crate) fn adjust_flip(&mut self, cx: &mut Context<Self>) {
        let Some(last) = self.adjustable_operation() else {
            return;
        };
        let value = -last.value;
        self.amend_last(|last, kind| amend_value(last, kind, value), cx);
    }

    /// Applies `edit` to the last operation's feature (or redoes its mesh
    /// push) without a new undo step, then regenerates.
    fn amend_last(
        &mut self,
        edit: impl Fn(&mut LastOperation, &mut FeatureKind) -> Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        let Some(mut last) = self.cad_tool.last.clone() else {
            return;
        };
        let result = match (&last.kind, last.part) {
            (LastKind::MeshPushPull { .. }, _) => {
                let mut scratch = FeatureKind::PushPull(forma_core::PushPullFeature {
                    face: forma_core::FaceRef {
                        point: Vec3::ZERO,
                        normal: Vec3::Y,
                    },
                    distance: last.value,
                });
                edit(&mut last, &mut scratch).and_then(|()| {
                    let LastKind::MeshPushPull {
                        before,
                        face,
                        direction,
                    } = &last.kind
                    else {
                        unreachable!()
                    };
                    let mut mesh = before.clone();
                    super::push_pull::push_mesh_face(&mut mesh, *face, *direction * last.value)
                        .map_err(|e| format!("{e:#}"))?;
                    let target = self
                        .scene
                        .object_mesh_mut(last.object)
                        .ok_or("The mesh no longer exists")?;
                    *target = mesh;
                    last.mesh_marker = Some((target.positions.len(), target.faces.len()));
                    Ok(())
                })
            }
            (_, Some(part_id)) => {
                let features = match &last.kind {
                    LastKind::PushPull { features, .. } => features.clone(),
                    _ => vec![last.feature],
                };
                let mut candidate = self.scene.part(part_id).cloned();
                let outcome = candidate
                    .as_mut()
                    .ok_or_else(|| "The part no longer exists".to_owned())
                    .and_then(|part| {
                        for id in &features {
                            let mut error = None;
                            part.edit_feature(*id, |feature| {
                                if let Err(e) = edit(&mut last, &mut feature.kind) {
                                    error = Some(e);
                                }
                            });
                            if let Some(error) = error {
                                return Err(error);
                            }
                        }
                        part.validate().map_err(|e| format!("{e:#}"))
                    });
                outcome.map(|()| {
                    let part = candidate.unwrap();
                    last.stamp = part.stamp;
                    *self.scene.part_mut(part_id).unwrap() = part;
                })
            }
            _ => Err("Nothing to adjust".into()),
        };
        match result {
            Ok(()) => {
                self.dirty = true;
                self.cad.announce = true;
                self.refresh_pending_for(&last);
                self.status = format!(
                    "{} {} · {}",
                    last.label(),
                    match last.param() {
                        AdjustParam::Distance => crate::sketch::overlay::format_length(last.value),
                        AdjustParam::Angle => super::format_sweep(last.value),
                    },
                    if last.part.is_some() {
                        "Regenerating…"
                    } else {
                        "Adjusted"
                    }
                );
                self.cad_tool.last = Some(last);
                self.invalidate(true, cx);
            }
            Err(message) => {
                self.status = message;
                cx.notify();
            }
        }
    }

    /// Replaces the pending ghost of an amended operation, so scrubbing the
    /// panel previews instantly while the kernel catches up.
    fn refresh_pending_for(&mut self, last: &LastOperation) {
        let Some(part) = last.part else {
            return;
        };
        self.cad_tool.pending.retain(|ghost| ghost.part != part);
        let t = self.theme_colors();
        let value = last.value;
        let span = (0.0_f32.min(value), 0.0_f32.max(value));
        match &last.kind {
            LastKind::Extrude {
                symmetric,
                operation,
                prism,
            } => {
                let (start, end) = if *symmetric {
                    (-value.abs() * 0.5, value.abs() * 0.5)
                } else {
                    span
                };
                self.cad_tool.pending.push(PendingGhost {
                    part,
                    stamp: last.stamp,
                    prism: Some((prism.clone(), start, end)),
                    quads: Vec::new(),
                    color: super::operation_color(&t, *operation),
                });
            }
            LastKind::PushPull { prisms, .. } => {
                for prism in prisms {
                    self.cad_tool.pending.push(PendingGhost {
                        part,
                        stamp: last.stamp,
                        prism: Some((prism.clone(), span.0, span.1)),
                        quads: Vec::new(),
                        color: if value >= 0.0 { t.accent } else { t.danger },
                    });
                }
            }
            LastKind::Chamfer { edges } => self.cad_tool.pending.push(PendingGhost {
                part,
                stamp: last.stamp,
                prism: None,
                quads: edges.iter().map(|e| e.quad(value)).collect(),
                color: t.accent,
            }),
            _ => {}
        }
    }
}

/// Sets the adjusted distance (world units) or angle (radians) on a
/// feature through the inspector's parameter rules (`timeline::params`).
fn amend_value(last: &mut LastOperation, kind: &mut FeatureKind, value: f32) -> Result<(), String> {
    if matches!(last.kind, LastKind::MeshPushPull { .. }) {
        // A mesh push has no feature: only the distance is checked.
        if value.abs() < 1.0e-5 {
            return Err("The distance must not be zero".into());
        }
        last.value = value;
        return Ok(());
    }
    let (field, display) = match kind {
        FeatureKind::Revolve(_) => (AdjustParam::Angle, value.to_degrees()),
        FeatureKind::Extrude(_) | FeatureKind::PushPull(_) | FeatureKind::Chamfer(_) => {
            (AdjustParam::Distance, value * last.scale)
        }
        _ => return Err("Nothing to adjust".into()),
    };
    let mut feature = forma_core::Feature::new(1, "Adjust", kind.clone());
    crate::timeline::params::set(&mut feature, field.feature_field(), display)?;
    *kind = feature.kind;
    last.value = value;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use forma_core::{ChamferFeature, EdgeRef, RevolveFeature, Sketch, Vec2};

    fn last(kind: LastKind, scale: f32) -> LastOperation {
        LastOperation {
            kind,
            object: 1,
            part: Some(2),
            feature: 3,
            stamp: 0,
            value: 0.0,
            scale,
            transform: Transform::default(),
            mesh_marker: None,
            edit_mode: EditMode::Object,
        }
    }

    #[test]
    fn amendments_follow_the_inspector_rules_in_feature_units() {
        // A chamfer on a part scaled 2× in the world: 0.1 m in the world is
        // 0.05 part units; zero and negative distances are refused.
        let mut chamfer = FeatureKind::Chamfer(ChamferFeature {
            edges: vec![EdgeRef {
                start: Vec3::ZERO,
                end: Vec3::X,
            }],
            distance: 0.02,
        });
        let mut op = last(LastKind::Chamfer { edges: Vec::new() }, 0.5);
        amend_value(&mut op, &mut chamfer, 0.1).unwrap();
        assert!(matches!(&chamfer, FeatureKind::Chamfer(c) if (c.distance - 0.05).abs() < 1.0e-6));
        assert_eq!(op.value, 0.1);
        assert!(amend_value(&mut op, &mut chamfer, -0.1).is_err());
        assert!(matches!(&chamfer, FeatureKind::Chamfer(c) if (c.distance - 0.05).abs() < 1.0e-6));
        // Revolve angles are radians in, degrees through the inspector field.
        let mut revolve = FeatureKind::Revolve(RevolveFeature {
            sketch: Sketch::default(),
            profiles: Vec::new(),
            axis_origin: Vec2::ZERO,
            axis_direction: Vec2::Y,
            angle: 1.0,
            operation: forma_core::FeatureOperation::Join,
        });
        let mut op = last(
            LastKind::Revolve {
                operation: Operation::NewPart,
            },
            1.0,
        );
        amend_value(&mut op, &mut revolve, 90f32.to_radians()).unwrap();
        assert!(
            matches!(&revolve, FeatureKind::Revolve(r) if (r.angle - std::f32::consts::FRAC_PI_2).abs() < 1.0e-5)
        );
        assert!(
            amend_value(&mut op, &mut revolve, 7.0).is_err(),
            "more than a turn"
        );
        assert_eq!(op.param(), AdjustParam::Angle);
    }
}
