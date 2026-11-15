//! Sketch associativity: features remember the sketch object they were made
//! from (`Feature::source`) and follow its edits.
//!
//! A linked feature embeds its own copy of the sketch, with the plane in
//! part space, so the part regenerates without the scene. When the source
//! sketch changes, the copy is replaced:
//!
//! - **Aligned links** — the copy sits where the source sketch is, mapped
//!   through the world transforms (`part⁻¹ · sketch`). The new copy is the
//!   source mapped the same way, so a re-planed sketch carries the feature
//!   with it. Uniform scale differences scale the curves.
//! - **Preserved links** — the part or the sketch moved since the link was
//!   made. Their world relationship no longer means anything, so the copy
//!   keeps its plane and takes the source's curves in plane coordinates:
//!   moving a part never drags its profile back to where the sketch lies.
//!
//! Region picks (interior points) are remapped by overlap so each pick keeps
//! choosing "the same" region: a pick stays bit-identical while its region
//! still best matches the old one, and otherwise moves inside the new region
//! that overlaps the old one most (a circle drawn inside a picked plate makes
//! the plate a ring rather than stealing the pick).
//!
//! Propagation is part of the sketch edit that caused it — no separate undo
//! step — so undo and redo always restore a sketch together with the parts
//! that follow it.
use forma_core::{
    Feature, FeatureKind, Mat4, PartData, Profile, SKETCH_TOLERANCE, Scene, Sketch, SketchCurve,
    SketchEntity, SketchPlane, Vec2, Vec3,
};

/// How a linked copy followed its source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Link {
    /// Mapped through world transforms, scaling curves by this factor.
    Aligned(f32),
    /// Plane kept; curves copied in plane coordinates.
    Preserved,
}

const PLANE_TOLERANCE: f32 = 1.0e-4;

/// Whether two planes describe the same frame (origin and axes).
pub fn planes_match(a: &SketchPlane, b: &SketchPlane) -> bool {
    let scale = a
        .origin
        .abs()
        .max_element()
        .max(b.origin.abs().max_element())
        .max(1.0);
    a.origin.distance(b.origin) <= PLANE_TOLERANCE * scale
        && a.x_axis.dot(b.x_axis) >= 1.0 - 1.0e-5
        && a.y_axis.dot(b.y_axis) >= 1.0 - 1.0e-5
}

/// `sketch`'s plane mapped by an affine `matrix`, and the uniform in-plane
/// scale it applies to sketch coordinates. `None` when the matrix shears or
/// squashes the plane (arcs would stop being circular) or collapses it.
pub fn map_plane(plane: &SketchPlane, matrix: Mat4) -> Option<(SketchPlane, f32)> {
    if !matrix.is_finite() {
        return None;
    }
    let origin = matrix.transform_point3(plane.origin);
    let x = matrix.transform_vector3(plane.x_axis);
    let y = matrix.transform_vector3(plane.y_axis);
    let (sx, sy) = (x.length(), y.length());
    if sx < 1.0e-9 || sy < 1.0e-9 {
        return None;
    }
    let similar = (sx - sy).abs() <= 1.0e-4 * sx.max(sy) && x.dot(y).abs() <= 1.0e-4 * sx * sy;
    if !similar {
        return None;
    }
    let mapped = SketchPlane {
        origin,
        x_axis: x / sx,
        y_axis: y / sy,
    };
    mapped.is_valid().then_some((mapped, (sx + sy) * 0.5))
}

/// A curve scaled about the plane origin.
pub fn scale_curve(curve: SketchCurve, scale: f32) -> SketchCurve {
    match curve {
        SketchCurve::Line { start, end } => SketchCurve::Line {
            start: start * scale,
            end: end * scale,
        },
        SketchCurve::Arc {
            center,
            radius,
            start_angle,
            sweep,
        } => SketchCurve::Arc {
            center: center * scale,
            radius: radius * scale,
            start_angle,
            sweep,
        },
        SketchCurve::Circle { center, radius } => SketchCurve::Circle {
            center: center * scale,
            radius: radius * scale,
        },
    }
}

fn scale_entities(entities: &[SketchEntity], scale: f32) -> Vec<SketchEntity> {
    if (scale - 1.0).abs() <= 1.0e-6 {
        return entities.to_vec();
    }
    entities
        .iter()
        .map(|entity| SketchEntity {
            curve: scale_curve(entity.curve, scale),
            construction: entity.construction,
        })
        .collect()
}

/// `source` (in its object's space) mapped into another space by `matrix`.
pub fn map_sketch(source: &Sketch, matrix: Mat4) -> Option<(Sketch, f32)> {
    let (plane, scale) = map_plane(&source.plane, matrix)?;
    Some((
        Sketch {
            plane,
            entities: scale_entities(&source.entities, scale),
        },
        scale,
    ))
}

/// The copy a linked feature should embed after its source changed.
/// `mapping` takes the source object's space to the feature's space.
pub fn relinked(embedded: &Sketch, source: &Sketch, mapping: Mat4) -> (Sketch, Link) {
    if let Some((mapped, scale)) = map_sketch(source, mapping)
        && planes_match(&mapped.plane, &embedded.plane)
    {
        // Snap to the embedded plane so float noise never changes the stamp.
        return (
            Sketch {
                plane: embedded.plane,
                entities: mapped.entities,
            },
            Link::Aligned(scale),
        );
    }
    (
        Sketch {
            plane: embedded.plane,
            entities: source.entities.clone(),
        },
        Link::Preserved,
    )
}

/// Points sampled inside a profile, for overlap estimates.
fn samples(profile: &Profile) -> Vec<Vec2> {
    const GRID: usize = 18;
    let (min, max) = profile.outer.bounds();
    let size = max - min;
    let mut points = Vec::new();
    for j in 0..GRID {
        for i in 0..GRID {
            // Offset the grid slightly so symmetric shapes do not sample
            // exactly on their own boundaries.
            let t = Vec2::new(
                (i as f32 + 0.5 + 0.013) / GRID as f32,
                (j as f32 + 0.5 + 0.029) / GRID as f32,
            );
            let point = min + size * t;
            if profile.contains(point) {
                points.push(point);
            }
        }
    }
    points
}

/// Remap region picks from `old` to `new` (both in the same 2D frame).
/// Empty picks (the default "outermost regions") stay empty.
pub fn remap_picks(old: &Sketch, new: &Sketch, picks: &[Vec2]) -> Vec<Vec2> {
    if picks.is_empty() || old.entities == new.entities {
        return picks.to_vec();
    }
    let before = old.profiles(SKETCH_TOLERANCE);
    let after = new.profiles(SKETCH_TOLERANCE);
    let mut result: Vec<Vec2> = Vec::with_capacity(picks.len());
    let mut used: Vec<Option<usize>> = Vec::new();
    for &pick in picks {
        let region_now = after.iter().position(|profile| profile.contains(pick));
        let Some(old_region) = before.iter().find(|profile| profile.contains(pick)) else {
            // Already dangling: keep it, so the feature reports it.
            result.push(pick);
            used.push(region_now);
            continue;
        };
        let points = samples(old_region);
        let best = if points.is_empty() {
            None
        } else {
            let old_area = old_region.area().abs().max(f32::EPSILON);
            after
                .iter()
                .enumerate()
                .filter_map(|(index, candidate)| {
                    let inside = points.iter().filter(|p| candidate.contains(**p)).count();
                    if inside == 0 {
                        return None;
                    }
                    let overlap = old_area * inside as f32 / points.len() as f32;
                    let union = old_area + candidate.area().abs() - overlap;
                    Some((index, overlap / union.max(f32::EPSILON)))
                })
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .filter(|(_, iou)| *iou >= 0.05)
                .map(|(index, _)| index)
        };
        let (point, region) = match (best, region_now) {
            (Some(best), Some(now)) if best == now => (pick, Some(now)),
            (Some(best), _) => match forma_cad::regions::interior_point(&after[best]) {
                Some(point) => (point, Some(best)),
                None => (pick, region_now),
            },
            (None, _) => (pick, region_now),
        };
        // Two picks landing in one region would only repeat it.
        if region.is_some() && used.contains(&region) {
            continue;
        }
        result.push(point);
        used.push(region);
    }
    result
}

/// The transform from a sketch object's space into a part object's space.
pub fn sketch_to_part(scene: &Scene, sketch_object: u64, part_object: u64) -> Option<Mat4> {
    let part = scene.world_transform(part_object)?;
    let sketch = scene.world_transform(sketch_object)?;
    if part.determinant().abs() < 1.0e-12 {
        return None;
    }
    let mapping = part.inverse() * sketch;
    mapping.is_finite().then_some(mapping)
}

/// Update one sketch-based feature from its source. Returns whether the
/// feature changed.
pub fn follow_source(feature: &mut Feature, source: &Sketch, mapping: Mat4) -> bool {
    // The embedded copy, its picks and a revolve axis are all in feature
    // coordinates already; only the source needs mapping.
    let (sketch, profiles) = match &mut feature.kind {
        FeatureKind::Extrude(extrude) => (&mut extrude.sketch, &mut extrude.profiles),
        FeatureKind::Revolve(revolve) => (&mut revolve.sketch, &mut revolve.profiles),
        _ => return false,
    };
    let (copy, _) = relinked(sketch, source, mapping);
    if copy == *sketch {
        return false;
    }
    *profiles = remap_picks(sketch, &copy, profiles);
    *sketch = copy;
    true
}

/// Apply `follow` to every sketch feature linked to `sketch_id` in
/// `features` (recursing into combine tools, whose space adds their
/// transform). Returns the ids of features that changed.
fn follow_in(
    features: &mut [Feature],
    sketch_id: u64,
    source: &Sketch,
    mapping: Mat4,
    changed: &mut Vec<u64>,
) {
    for feature in features {
        if let FeatureKind::Combine(combine) = &mut feature.kind {
            for tool in &mut combine.tools {
                if tool.transform.determinant().abs() < 1.0e-12 {
                    continue;
                }
                let inner = tool.transform.inverse() * mapping;
                follow_in(&mut tool.features, sketch_id, source, inner, changed);
            }
            continue;
        }
        if feature.source == Some(sketch_id) && follow_source(feature, source, mapping) {
            changed.push(feature.id);
        }
    }
}

/// Bring every part linked to the sketch object `sketch_id` up to date.
/// Returns `(part id, changed feature ids)` for parts that changed; those
/// parts are validated and restamped. Parts whose update would be invalid
/// are left untouched.
pub fn propagate(scene: &mut Scene, sketch_id: u64) -> Vec<(u64, Vec<u64>)> {
    let Some(source) = scene.object_sketch(sketch_id).cloned() else {
        return Vec::new();
    };
    // Part space is the space of the part's (first) object.
    let mut owners: Vec<(u64, u64)> = Vec::new();
    for object in &scene.objects {
        if let Some(part) = object.part_id()
            && !owners.iter().any(|(id, _)| *id == part)
        {
            owners.push((part, object.id));
        }
    }
    let mut updated = Vec::new();
    for (part_id, object) in owners {
        let linked = scene.part(part_id).is_some_and(|part| {
            forma_core::all_features(&part.features).any(|f| f.source == Some(sketch_id))
        });
        if !linked {
            continue;
        }
        let Some(mapping) = sketch_to_part(scene, sketch_id, object) else {
            continue;
        };
        let mut candidate: PartData = scene.part(part_id).unwrap().clone();
        let mut changed = Vec::new();
        follow_in(
            &mut candidate.features,
            sketch_id,
            &source,
            mapping,
            &mut changed,
        );
        if changed.is_empty() {
            continue;
        }
        candidate.restamp();
        if candidate.validate().is_err() {
            continue;
        }
        *scene.part_mut(part_id).unwrap() = candidate;
        updated.push((part_id, changed));
    }
    updated
}

/// A hidden sketch object rebuilt from a feature's embedded copy, placed so
/// its world plane coincides with the feature's, for editing a feature whose
/// source sketch was deleted. Returns the new object's id.
pub fn materialize(
    scene: &mut Scene,
    part_object: u64,
    sketch: &Sketch,
    name: &str,
) -> anyhow::Result<u64> {
    let owner = scene
        .object(part_object)
        .ok_or_else(|| anyhow::anyhow!("The part no longer exists"))?
        .clone();
    let id = scene.add_sketch(name, sketch.clone())?;
    let object = scene.object_mut(id).unwrap();
    // The part's own placement makes the sketch's space equal part space.
    object.transform = owner.transform;
    object.parent = owner.parent;
    object.parent_inverse = owner.parent_inverse;
    object.collections = owner.collections.clone();
    object.visible = false;
    Ok(id)
}

/// Re-place a hidden source sketch so it lies on its feature, keeping its
/// curves. Used before editing a consumed sketch whose part has moved, so
/// Draw opens on the part instead of where the sketch was left behind.
pub fn align_to_feature(
    scene: &mut Scene,
    sketch_object: u64,
    part_object: u64,
    plane: SketchPlane,
) {
    let Some(owner) = scene.object(part_object).cloned() else {
        return;
    };
    let Some(object) = scene.object_mut(sketch_object) else {
        return;
    };
    object.transform = owner.transform;
    object.parent = owner.parent;
    object.parent_inverse = owner.parent_inverse;
    if let Some(sketch) = scene.object_sketch_mut(sketch_object) {
        sketch.plane = plane;
    }
}

/// Whether a source sketch currently lies on its feature's embedded plane.
pub fn is_aligned(scene: &Scene, sketch_object: u64, part_object: u64, embedded: &Sketch) -> bool {
    let (Some(source), Some(mapping)) = (
        scene.object_sketch(sketch_object),
        sketch_to_part(scene, sketch_object, part_object),
    ) else {
        return false;
    };
    map_plane(&source.plane, mapping).is_some_and(|(plane, scale)| {
        planes_match(&plane, &embedded.plane) && (scale - 1.0).abs() < 1.0e-4
    })
}

/// The world-space normal of a plane under a matrix.
#[cfg(test)]
pub fn world_normal(plane: &SketchPlane, matrix: Mat4) -> Vec3 {
    matrix.transform_vector3(plane.normal()).normalize_or_zero()
}

#[cfg(test)]
mod tests {
    use super::*;
    use forma_core::{ExtrudeExtent, ExtrudeFeature, FeatureOperation, RevolveFeature};
    use glam::Quat;

    fn line(a: (f32, f32), b: (f32, f32)) -> SketchEntity {
        SketchCurve::Line {
            start: Vec2::new(a.0, a.1),
            end: Vec2::new(b.0, b.1),
        }
        .into()
    }

    fn rectangle(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<SketchEntity> {
        vec![
            line((x0, y0), (x1, y0)),
            line((x1, y0), (x1, y1)),
            line((x1, y1), (x0, y1)),
            line((x0, y1), (x0, y0)),
        ]
    }

    fn circle(x: f32, y: f32, radius: f32) -> SketchEntity {
        SketchCurve::Circle {
            center: Vec2::new(x, y),
            radius,
        }
        .into()
    }

    fn extrude(sketch: Sketch, profiles: Vec<Vec2>, source: u64) -> Feature {
        let mut feature = Feature::new(
            7,
            "Extrude",
            FeatureKind::Extrude(ExtrudeFeature {
                sketch,
                profiles,
                distance: 0.5,
                extent: ExtrudeExtent::OneSide,
                flip: false,
                operation: FeatureOperation::Join,
            }),
        );
        feature.source = Some(source);
        feature
    }

    fn embedded(feature: &Feature) -> (&Sketch, &[Vec2]) {
        match &feature.kind {
            FeatureKind::Extrude(extrude) => (&extrude.sketch, &extrude.profiles),
            FeatureKind::Revolve(revolve) => (&revolve.sketch, &revolve.profiles),
            _ => unreachable!(),
        }
    }

    #[test]
    fn planes_map_through_rigid_and_uniformly_scaled_transforms() {
        let plane = SketchPlane::ground();
        let rigid = Mat4::from_rotation_translation(
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            Vec3::new(1.0, 2.0, 3.0),
        );
        let (mapped, scale) = map_plane(&plane, rigid).unwrap();
        assert!((scale - 1.0).abs() < 1.0e-6);
        assert!(mapped.origin.distance(Vec3::new(1.0, 2.0, 3.0)) < 1.0e-6);
        // Ground x (+X) rotated a quarter turn about Z points along +Y.
        assert!(mapped.x_axis.distance(Vec3::Y) < 1.0e-6);
        assert!(world_normal(&plane, rigid).distance(Vec3::NEG_X) < 1.0e-6);
        // A point drawn on the plane lands in the same world place both ways.
        let p = Vec2::new(0.3, -0.7);
        assert!(
            rigid
                .transform_point3(plane.to_world(p))
                .distance(mapped.to_world(p))
                < 1.0e-5
        );
        let (_, doubled) = map_plane(&plane, Mat4::from_scale(Vec3::splat(2.0))).unwrap();
        assert!((doubled - 2.0).abs() < 1.0e-6);
        // Non-uniform scale in the plane would distort arcs: refused.
        assert!(map_plane(&plane, Mat4::from_scale(Vec3::new(2.0, 1.0, 1.0))).is_none());
        // Scaling along the normal only is fine.
        assert!(map_plane(&plane, Mat4::from_scale(Vec3::new(1.0, 3.0, 1.0))).is_some());
    }

    #[test]
    fn aligned_links_map_through_world_transforms_and_scale_curves() {
        let mut source = Sketch::new(SketchPlane::ground());
        source.entities = rectangle(-1.0, -1.0, 1.0, 1.0);
        // The part is twice as large as the sketch object: part space is
        // half the size, so curves shrink to stay put in the world.
        let mapping = Mat4::from_scale(Vec3::splat(0.5));
        let (copy, _) = map_sketch(&source, mapping).unwrap();
        let mut feature = extrude(copy.clone(), vec![Vec2::new(0.1, 0.1)], 9);
        source.entities = rectangle(-2.0, -1.0, 2.0, 1.0);
        assert!(follow_source(&mut feature, &source, mapping));
        let (sketch, picks) = embedded(&feature);
        assert_eq!(sketch.plane, copy.plane);
        assert_eq!(
            sketch.entities[0].curve,
            SketchCurve::Line {
                start: Vec2::new(-1.0, -0.5),
                end: Vec2::new(1.0, -0.5)
            }
        );
        // The pick still lies inside the (scaled) region.
        assert!(sketch.profiles(SKETCH_TOLERANCE)[0].contains(picks[0]));
    }

    #[test]
    fn moved_parts_preserve_their_profile_frame() {
        let mut source = Sketch::new(SketchPlane::ground());
        source.entities = rectangle(0.0, 0.0, 1.0, 1.0);
        let mut feature = extrude(source.clone(), Vec::new(), 3);
        // The part moved 2 m along X after the link was made.
        let moved = Mat4::from_translation(Vec3::new(-2.0, 0.0, 0.0));
        source.entities.push(circle(0.5, 0.5, 0.2));
        assert!(follow_source(&mut feature, &source, moved));
        let (sketch, picks) = embedded(&feature);
        assert_eq!(sketch.plane, SketchPlane::ground(), "plane frame kept");
        assert_eq!(sketch.entities, source.entities);
        assert!(picks.is_empty(), "default picks stay default");
        // An unchanged source is a no-op, so stamps stay stable.
        assert!(!follow_source(&mut feature, &source, moved));
    }

    #[test]
    fn region_picks_survive_unrelated_edits_bit_for_bit() {
        let mut old = Sketch::new(SketchPlane::ground());
        old.entities = rectangle(0.0, 0.0, 1.0, 1.0);
        old.entities.extend(rectangle(2.0, 0.0, 3.0, 1.0));
        let picks = vec![Vec2::new(0.37, 0.61)];
        let mut new = old.clone();
        // Widen the other rectangle: the picked one is untouched.
        new.entities.truncate(4);
        new.entities.extend(rectangle(2.0, 0.0, 3.5, 1.2));
        assert_eq!(remap_picks(&old, &new, &picks), picks);
        // Grow the picked rectangle: the pick is still its best match.
        let mut grown = old.clone();
        grown.entities.splice(0..4, rectangle(0.0, 0.0, 1.4, 1.0));
        assert_eq!(remap_picks(&old, &grown, &picks), picks);
    }

    #[test]
    fn a_hole_drawn_over_a_pick_keeps_the_surrounding_region() {
        let mut old = Sketch::new(SketchPlane::ground());
        old.entities = rectangle(0.0, 0.0, 2.0, 2.0);
        let pick = Vec2::new(1.0, 1.0);
        let mut new = old.clone();
        new.entities.push(circle(1.0, 1.0, 0.4));
        let remapped = remap_picks(&old, &new, &[pick]);
        assert_eq!(remapped.len(), 1);
        let profiles = new.profiles(SKETCH_TOLERANCE);
        let ring = profiles
            .iter()
            .position(|p| !p.holes.is_empty())
            .expect("ring region");
        assert!(
            profiles[ring].contains(remapped[0]),
            "the pick moved into the ring instead of the new disc"
        );
    }

    #[test]
    fn moved_regions_carry_their_pick_and_duplicates_collapse() {
        let mut old = Sketch::new(SketchPlane::ground());
        old.entities = rectangle(0.0, 0.0, 1.0, 1.0);
        let picks = vec![Vec2::new(0.2, 0.2), Vec2::new(0.8, 0.8)];
        let mut new = Sketch::new(SketchPlane::ground());
        // Slide the rectangle so the first pick falls outside it.
        new.entities = rectangle(0.5, 0.0, 1.5, 1.0);
        let remapped = remap_picks(&old, &new, &picks);
        assert_eq!(remapped.len(), 1, "both picks now choose the same region");
        assert!(new.profiles(SKETCH_TOLERANCE)[0].contains(remapped[0]));
    }

    #[test]
    fn propagation_follows_the_sketch_and_restamps_only_linked_parts() {
        let mut scene = Scene::empty();
        let mut sketch = Sketch::new(SketchPlane::ground());
        sketch.entities = rectangle(-0.5, -0.5, 0.5, 0.5);
        let sketch_id = scene.add_sketch("Sketch", sketch.clone()).unwrap();
        let mut part = PartData::new(0, "Linked");
        let feature = part.allocate_feature_id();
        let mut linked = extrude(sketch.clone(), Vec::new(), sketch_id);
        linked.id = feature;
        part.features.push(linked);
        part.restamp();
        let object = scene.add_part("Linked", part).unwrap();
        let mut other = PartData::new(0, "Other");
        other
            .features
            .push(extrude(sketch.clone(), Vec::new(), 999));
        other.next_feature_id = 100;
        other.restamp();
        let other_object = scene.add_part("Other", other).unwrap();
        let other_stamp = scene.object_part(other_object).unwrap().stamp;
        let stamp = scene.object_part(object).unwrap().stamp;

        assert!(
            propagate(&mut scene, sketch_id).is_empty(),
            "already in sync"
        );
        scene
            .object_sketch_mut(sketch_id)
            .unwrap()
            .entities
            .push(circle(0.0, 0.0, 0.2));
        let updated = propagate(&mut scene, sketch_id);
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].1, vec![feature]);
        let part = scene.object_part(object).unwrap();
        assert_ne!(part.stamp, stamp);
        assert_eq!(part.stamp, part.content_hash());
        assert_eq!(embedded(&part.features[0]).0.entities.len(), 5);
        assert_eq!(scene.object_part(other_object).unwrap().stamp, other_stamp);
    }

    #[test]
    fn revolve_copies_follow_scaled_sources_and_keep_their_axis() {
        let mut source = Sketch::new(SketchPlane::front());
        source.entities = rectangle(0.0, 0.0, 1.0, 1.0);
        let mapping = Mat4::from_scale(Vec3::splat(2.0));
        let (copy, _) = map_sketch(&source, mapping).unwrap();
        let mut feature = Feature::new(
            5,
            "Revolve",
            FeatureKind::Revolve(RevolveFeature {
                sketch: copy,
                profiles: Vec::new(),
                axis_origin: Vec2::new(-2.0, 0.0),
                axis_direction: Vec2::Y,
                angle: 1.0,
                operation: FeatureOperation::Join,
            }),
        );
        feature.source = Some(1);
        source.entities = rectangle(0.0, 0.0, 1.0, 2.0);
        let mut moved_axis = feature.clone();
        assert!(follow_source(&mut moved_axis, &source, mapping));
        let FeatureKind::Revolve(revolve) = &moved_axis.kind else {
            unreachable!()
        };
        // The axis is already in feature coordinates; only curves follow,
        // scaled by the part/sketch scale difference.
        assert_eq!(revolve.axis_origin, Vec2::new(-2.0, 0.0));
        assert_eq!(revolve.sketch.entities[1].curve.end(), Vec2::new(2.0, 4.0));
    }

    #[test]
    fn materialized_sketches_land_on_the_feature_plane_in_the_world() {
        let mut scene = Scene::empty();
        let mut part = PartData::new(0, "Part");
        let mut copy = Sketch::new(SketchPlane {
            origin: Vec3::new(0.0, 0.3, 0.0),
            ..SketchPlane::ground()
        });
        copy.entities = rectangle(0.0, 0.0, 1.0, 1.0);
        part.features.push(extrude(copy.clone(), Vec::new(), 404));
        part.next_feature_id = 8;
        part.restamp();
        let object = scene.add_part("Part", part).unwrap();
        scene.object_mut(object).unwrap().transform.translation = Vec3::new(4.0, 0.0, 1.0);
        let sketch = materialize(&mut scene, object, &copy, "Extrude sketch").unwrap();
        assert!(!scene.object(sketch).unwrap().visible);
        assert!(is_aligned(&scene, sketch, object, &copy));
        let world = scene.world_transform(sketch).unwrap();
        let corner = world.transform_point3(copy.plane.to_world(Vec2::new(1.0, 1.0)));
        assert!(
            corner.distance(Vec3::new(5.0, 0.3, 0.0)) < 1.0e-5,
            "{corner}"
        );
        // Moving the part breaks alignment; re-aligning restores it.
        scene.object_mut(object).unwrap().transform.translation.x = -3.0;
        assert!(!is_aligned(&scene, sketch, object, &copy));
        align_to_feature(&mut scene, sketch, object, copy.plane);
        assert!(is_aligned(&scene, sketch, object, &copy));
    }
}
