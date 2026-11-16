//! The feature selected in the timeline, shown where it acts. The panel and
//! timeline name the feature; the canvas shows it: a sketch feature's
//! profile with a faint ghost of the body it sweeps, a primitive's tool body
//! as an outline with a faint fill, and an arrow handle on the feature's
//! main value with that value hanging off its tip. Dragging the arrow edits
//! the value live (the part regenerates as it moves) and records one undo
//! step on release, like scrubbing the value in the inspector.
//!
//! Ghosts come from the feature's parameters, not the kernel result, so they
//! follow a drag at once and show a tool body even where the result hides
//! it (a cut's tool, the part of a rounding cylinder outside the block).
use super::geometry::{Lathe, Prism};
use super::overlay::{
    AreaPaint, ArrowPaint, LabelPaint, LabelStyle, LathePaint, PrismPaint, ToolOverlay,
};
use super::{AxisDrag, DRAG_PIXELS, HANDLE_PIXELS, PICK_PIXELS, Regions, segment_distance};
use crate::app::Studio;
use crate::timeline::FeatureField;
use forma_core::{ExtrudeExtent, Feature, FeatureKind, Mat4, PrimitiveShape, Vec2, Vec3};
use gpui::Context;
use std::rc::Rc;

/// What the selected feature looks like on the canvas, in world space.
/// Built from the feature's parameters and the part's transform.
#[derive(Default)]
pub(crate) struct FeatureGhost {
    /// The regions a sketch feature sweeps: outer loops and holes.
    pub profile: Vec<Vec<Vec<Vec3>>>,
    /// A swept body between two offsets along the prism's normal.
    pub prism: Option<(Rc<Prism>, f32, f32)>,
    /// A revolved body and its sweep.
    pub lathe: Option<(Rc<Lathe>, f32)>,
    pub handle: Option<FeatureHandle>,
}

/// An arrow on a feature's main value. The value maps linearly to a world
/// distance along `axis` from `origin`: the arrow stands at
/// `origin + axis * value * per_unit`, pointing away from `origin`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FeatureHandle {
    pub field: FeatureField,
    pub value: f32,
    pub origin: Vec3,
    /// Unit world direction.
    pub axis: Vec3,
    /// World distance along `axis` per unit of value (signed).
    pub per_unit: f32,
    /// A short prefix naming the value on its chip ("r" for a radius).
    pub prefix: &'static str,
}

impl FeatureHandle {
    pub(crate) fn base(&self) -> Vec3 {
        self.origin + self.axis * (self.value * self.per_unit)
    }

    /// The world direction the arrow points: away from its origin.
    fn pointing(&self) -> Vec3 {
        if self.value * self.per_unit < 0.0 {
            -self.axis
        } else {
            self.axis
        }
    }

    /// The value for a world offset along the axis.
    fn value_at(&self, offset: f32) -> f32 {
        offset / self.per_unit
    }
}

/// A drag of the feature handle: live edits, one undo step on release.
pub(crate) struct FeatureDrag {
    pub object: u64,
    pub feature: u64,
    pub handle: FeatureHandle,
    pub axis: AxisDrag,
    pub before: Box<forma_core::Scene>,
    pub moved: bool,
}

/// Cache key of a feature ghost: object, feature, part stamp, transform.
type GhostKey = (u64, u64, u64, Mat4);

#[derive(Default)]
pub(crate) struct FeatureState {
    pub drag: Option<FeatureDrag>,
    /// The pointer rests on the handle.
    pub hot: bool,
    ghost: std::cell::RefCell<Option<(GhostKey, Rc<FeatureGhost>)>>,
}

/// Segments of a primitive's circular cross-section.
const CIRCLE_SEGMENTS: usize = 64;

/// The ghost of `feature` in a part whose world transform is `matrix`.
fn build_ghost(object: u64, feature: &Feature, matrix: Mat4) -> FeatureGhost {
    let mut ghost = FeatureGhost::default();
    match &feature.kind {
        FeatureKind::Extrude(extrude) => {
            let Some(regions) = Regions::build(object, &extrude.sketch, matrix) else {
                return ghost;
            };
            let picked = picked_regions(&regions, &extrude.profiles);
            if picked.is_empty() {
                return ghost;
            }
            ghost.profile = picked.iter().map(|&i| regions.world[i].clone()).collect();
            let polygons: Vec<Vec<Vec<Vec2>>> = picked
                .iter()
                .map(|&i| regions.polygons[i].clone())
                .collect();
            let prism = Rc::new(Prism::from_regions(
                &extrude.sketch.plane,
                matrix,
                &polygons,
            ));
            let scale = regions.normal_scale();
            let sign = if extrude.flip { -1.0 } else { 1.0 };
            let distance = extrude.distance * sign;
            let (start, end) = match extrude.extent {
                ExtrudeExtent::OneSide => (0.0, distance),
                ExtrudeExtent::Symmetric => {
                    (-extrude.distance.abs() * 0.5, extrude.distance.abs() * 0.5)
                }
                ExtrudeExtent::TwoSides { back_distance } => (-back_distance * sign, distance),
            };
            let (start, end) = (start.min(end) * scale, start.max(end) * scale);
            // The arrow rides the cap the distance moves: the far one, or
            // the forward one of a symmetric extrusion.
            let per_unit = match extrude.extent {
                ExtrudeExtent::Symmetric => scale * 0.5 * extrude.distance.signum(),
                _ => scale,
            };
            ghost.handle = Some(FeatureHandle {
                field: FeatureField::Distance,
                value: extrude.distance,
                origin: prism.interior_point(),
                axis: prism.normal * sign,
                per_unit,
                prefix: "",
            });
            ghost.prism = Some((prism, start, end));
        }
        FeatureKind::Revolve(revolve) => {
            let Some(regions) = Regions::build(object, &revolve.sketch, matrix) else {
                return ghost;
            };
            let picked = picked_regions(&regions, &revolve.profiles);
            if picked.is_empty() {
                return ghost;
            }
            ghost.profile = picked.iter().map(|&i| regions.world[i].clone()).collect();
            let polygons: Vec<Vec<Vec<Vec2>>> = picked
                .iter()
                .map(|&i| regions.polygons[i].clone())
                .collect();
            let origin = regions.to_world(revolve.axis_origin);
            if let Some(direction) =
                (regions.to_world(revolve.axis_origin + revolve.axis_direction) - origin)
                    .try_normalize()
            {
                ghost.lathe = Some((
                    Rc::new(Lathe {
                        loops: ghost.profile.iter().flatten().cloned().collect(),
                        prism: Prism::from_regions(&revolve.sketch.plane, matrix, &polygons),
                        axis_origin: origin,
                        axis_direction: direction,
                    }),
                    revolve.angle,
                ));
            }
        }
        FeatureKind::Primitive(primitive) => {
            let placed = matrix * primitive.placement.matrix();
            let up = placed.transform_vector3(Vec3::Y);
            let scale = placed.transform_vector3(Vec3::X).length();
            let center = placed.transform_point3(Vec3::ZERO);
            match primitive.shape {
                PrimitiveShape::Cylinder { radius, height } => {
                    let points = (0..CIRCLE_SEGMENTS)
                        .map(|k| {
                            let a = k as f32 / CIRCLE_SEGMENTS as f32 * std::f32::consts::TAU;
                            placed.transform_point3(Vec3::new(
                                radius * a.cos(),
                                -height * 0.5,
                                -radius * a.sin(),
                            ))
                        })
                        .collect();
                    let prism = Rc::new(Prism::from_convex(points, up));
                    ghost.prism = Some((prism, 0.0, height * up.length()));
                    // The radius handle stands on the silhouette, set in
                    // `facing_handle` once the view is known.
                    ghost.handle = Some(FeatureHandle {
                        field: FeatureField::Radius,
                        value: radius,
                        origin: center,
                        axis: placed.transform_vector3(Vec3::X).normalize_or_zero(),
                        per_unit: scale,
                        prefix: "r ",
                    });
                }
                PrimitiveShape::Box { size } => {
                    let half = size * 0.5;
                    let points = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                        .map(|(x, z)| {
                            placed.transform_point3(Vec3::new(half.x * x, -half.y, half.z * z))
                        })
                        .to_vec();
                    let prism = Rc::new(Prism::from_convex(points, up));
                    ghost.prism = Some((prism, 0.0, size.y * up.length()));
                    // The box grows both ways about its centre, so its top
                    // moves half as far as the height changes.
                    ghost.handle = Some(FeatureHandle {
                        field: FeatureField::Size(1),
                        value: size.y,
                        origin: center,
                        axis: up.normalize_or_zero(),
                        per_unit: up.length() * 0.5,
                        prefix: "h ",
                    });
                }
                _ => {}
            }
        }
        _ => {}
    }
    ghost
}

/// The regions a feature's picks choose, as the kernel chooses them: each
/// pick's region, or the outermost regions when there are no picks.
fn picked_regions(regions: &Regions, picks: &[Vec2]) -> Vec<usize> {
    let mut picked: Vec<usize> = if picks.is_empty() {
        regions.defaults.clone()
    } else {
        picks.iter().filter_map(|p| regions.region_at(*p)).collect()
    };
    picked.sort_unstable();
    picked.dedup();
    picked
}

impl Studio {
    /// The selected timeline feature and its part object, when the canvas
    /// shows it: Object mode with no tool running.
    fn context_feature(&self) -> Option<(u64, &Feature)> {
        if self.cad_tool.active() || self.part_editing().is_some() {
            return None;
        }
        let selected = self.selected_feature()?;
        Some((selected.object, selected.feature))
    }

    /// The ghost of the selected feature, cached per part stamp and
    /// transform.
    pub(crate) fn feature_ghost(&self) -> Option<(u64, u64, Rc<FeatureGhost>)> {
        let (object, feature) = self.context_feature()?;
        let part = self.scene.object_part(object)?;
        let matrix = self.scene.world_transform(object)?;
        let key = (object, feature.id, part.stamp, matrix);
        let state = &self.cad_tool.feature;
        if let Some((cached, ghost)) = state.ghost.borrow().as_ref()
            && *cached == key
        {
            return Some((object, feature.id, ghost.clone()));
        }
        let ghost = Rc::new(build_ghost(object, feature, matrix));
        *state.ghost.borrow_mut() = Some((key, ghost.clone()));
        Some((object, feature.id, ghost))
    }

    /// The selected feature's handle for the current view. A radius handle
    /// turns to stand on the cylinder's silhouette, on its right, so it
    /// points across the screen rather than into it.
    fn facing_handle(&self, ghost: &FeatureGhost) -> Option<FeatureHandle> {
        let mut handle = ghost.handle?;
        if handle.field == FeatureField::Radius
            && let Some((prism, start, end)) = &ghost.prism
        {
            let up = prism.normal;
            let camera = self.scene.camera;
            let view = if camera.orthographic {
                -camera.view_matrix().inverse().z_axis.truncate()
            } else {
                (handle.origin - camera.position()).normalize_or_zero()
            };
            if let Some(mut radial) = up.cross(view).try_normalize() {
                let screen_x = camera.view_matrix().inverse().x_axis.truncate();
                if radial.dot(screen_x) < 0.0 {
                    radial = -radial;
                }
                handle.axis = radial;
            }
            // Halfway up the tool body.
            handle.origin = prism.centroid() + up * ((start + end) * 0.5);
        }
        Some(handle)
    }

    /// Screen base and tip of the selected feature's arrow.
    fn feature_arrow(&self, handle: &FeatureHandle) -> Option<(Vec2, Vec2)> {
        let base = handle.base();
        let screen = self.world_to_screen(base)?;
        let ahead =
            self.world_to_screen(base + handle.pointing() * self.world_per_pixel(base) * 10.0)?;
        let direction = (ahead - screen).try_normalize().unwrap_or(Vec2::NEG_Y);
        Some((screen, screen + direction * HANDLE_PIXELS))
    }

    /// The selected feature on the canvas: profile, ghost body and handle.
    pub(crate) fn feature_context_overlay(&self, overlay: &mut ToolOverlay) {
        let t = overlay.colors;
        let dragging = self.cad_tool.feature.drag.as_ref();
        // While dragging, the handle the drag started from moves with the
        // value; the ghost is rebuilt from the edited feature every move.
        let Some((_, _, ghost)) = self.feature_ghost() else {
            return;
        };
        for regions in &ghost.profile {
            overlay.areas.push(AreaPaint {
                polygons: regions.clone(),
                color: t.accent,
                fill: 0.16,
                outline: 1.5,
            });
        }
        // A faint body, stronger while its value is being dragged.
        let fill = if dragging.is_some() { 0.7 } else { 0.4 };
        if let Some((prism, start, end)) = &ghost.prism
            && end - start > 1.0e-6
        {
            overlay.prisms.push(PrismPaint {
                prism: prism.clone(),
                start: *start,
                end: *end,
                color: t.accent,
                emphasis_end: true,
                fill,
            });
        }
        if let Some((lathe, angle)) = &ghost.lathe {
            overlay.lathes.push(LathePaint {
                lathe: lathe.clone(),
                angle: *angle,
                color: t.accent,
                fill,
            });
        }
        let Some(handle) = self.facing_handle(&ghost) else {
            return;
        };
        let Some((base, tip)) = self.feature_arrow(&handle) else {
            return;
        };
        let live = dragging.is_some_and(|d| d.moved);
        overlay.arrows.push(ArrowPaint {
            base,
            tip,
            color: t.accent,
            hot: self.cad_tool.feature.hot || live,
        });
        overlay.labels.push(LabelPaint {
            at: tip,
            leader: (tip - base).try_normalize(),
            text: format!(
                "{}{}",
                handle.prefix,
                crate::sketch::overlay::format_length(handle.value)
            ),
            style: if live {
                LabelStyle::Typed
            } else {
                LabelStyle::Measured
            },
        });
    }

    /// Screen base and tip of the selected feature's arrow, if it has one.
    pub(crate) fn feature_arrow_screen(&self) -> Option<(Vec2, Vec2)> {
        let (_, _, ghost) = self.feature_ghost()?;
        self.feature_arrow(&self.facing_handle(&ghost)?)
    }

    /// Whether `position` grabs the selected feature's arrow.
    fn feature_handle_at(&self, position: Vec2) -> Option<(u64, u64, FeatureHandle)> {
        let (object, feature, ghost) = self.feature_ghost()?;
        let handle = self.facing_handle(&ghost)?;
        let (base, tip) = self.feature_arrow(&handle)?;
        (segment_distance(position, base, tip) <= PICK_PIXELS).then_some((object, feature, handle))
    }

    /// Hover over the selected feature's arrow. Returns true when it is hot.
    pub(crate) fn feature_hover(&mut self, position: Vec2, cx: &mut Context<Self>) -> bool {
        let hot = self.feature_handle_at(position).is_some();
        if hot != self.cad_tool.feature.hot {
            self.cad_tool.feature.hot = hot;
            cx.notify();
        }
        hot
    }

    /// A press on the selected feature's arrow starts a drag of its value.
    pub(crate) fn feature_press(&mut self, position: Vec2, cx: &mut Context<Self>) -> bool {
        let Some((object, feature, handle)) = self.feature_handle_at(position) else {
            return false;
        };
        let offset = handle.value * handle.per_unit;
        let axis = AxisDrag::new(self, handle.origin, handle.axis, offset, position);
        self.cad_tool.feature.drag = Some(FeatureDrag {
            object,
            feature,
            handle,
            axis,
            before: Box::new(self.scene.clone()),
            moved: false,
        });
        self.status = "Drag to change the value · Release to apply · Right-click to cancel".into();
        cx.notify();
        true
    }

    /// Pointer motion while dragging the feature's arrow: the value follows
    /// the pointer along its axis and the part regenerates.
    pub(crate) fn feature_drag_move(&mut self, position: Vec2, cx: &mut Context<Self>) -> bool {
        let Some(drag) = self.cad_tool.feature.drag.as_ref() else {
            return false;
        };
        if !drag.moved && position.distance(drag.axis.press) < DRAG_PIXELS {
            return true;
        }
        let Some(offset) = drag.axis.value(self, position) else {
            return true;
        };
        let (object, feature, handle) = (drag.object, drag.feature, drag.handle);
        // Keep the value's precision to what the inspector shows.
        let (_, decimals) = handle.field.scrub_step();
        let factor = 10f32.powi(decimals as i32);
        let value = (handle.value_at(offset) * factor).round() / factor;
        if let Some(drag) = self.cad_tool.feature.drag.as_mut() {
            drag.moved = true;
        }
        let Some(mut candidate) = self.scene.object_part(object).cloned() else {
            return true;
        };
        let mut outcome = Ok(());
        let mut changed = false;
        candidate.edit_feature(feature, |f| {
            let before = crate::timeline::params::value(f, handle.field);
            outcome = crate::timeline::params::set(f, handle.field, value);
            changed = crate::timeline::params::value(f, handle.field) != before;
        });
        match outcome.and_then(|()| candidate.validate().map_err(|e| format!("{e:#}"))) {
            // Out-of-range values keep the last valid result.
            Err(message) => self.status = message,
            Ok(()) if changed => {
                *self.scene.object_part_mut(object).unwrap() = candidate;
                self.dirty = true;
                self.status = format!(
                    "{}{} · Release to apply",
                    handle.prefix,
                    crate::sketch::overlay::format_length(value)
                );
                self.invalidate(true, cx);
            }
            Ok(()) => {}
        }
        cx.notify();
        true
    }

    /// Ends a feature handle drag: one undo step when it changed the part.
    pub(crate) fn feature_drag_end(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.cad_tool.feature.drag.take() else {
            return;
        };
        if drag.moved && *drag.before != self.scene {
            self.history.checkpoint(&drag.before);
            self.dirty = true;
            self.status = "Value updated · Regenerating…".into();
        } else {
            self.status = "Drag the arrow to change the value".into();
        }
        cx.notify();
    }

    /// Cancels a feature handle drag, restoring the part as it was.
    pub(crate) fn feature_drag_cancel(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(drag) = self.cad_tool.feature.drag.take() else {
            return false;
        };
        if drag.moved {
            self.scene = *drag.before;
            self.invalidate(true, cx);
        }
        self.status = "Cancelled".into();
        cx.notify();
        true
    }
}
