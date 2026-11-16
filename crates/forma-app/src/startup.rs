//! The scene Forma opens with (and New project creates): polygon meshes and
//! a parametric CAD part sharing one lit studio plinth, so the first thing a
//! new user sees is what makes Forma different.
//!
//! The part is a machined angle bracket built the way a user would build it
//! in Draw mode: an L profile sketched on the right plane and extruded, a
//! gusset sketched on the same plane and joined, mounting holes sketched on
//! the base and upright faces and cut through, and chamfered corners. Its
//! history therefore fills the timeline with real, editable features. The
//! part arrives without a tessellation; the CAD worker regenerates it on
//! launch like any part loaded from a file.
use forma_core::{
    ChamferFeature, EdgeRef, ExtrudeExtent, ExtrudeFeature, FeatureKind, FeatureOperation,
    Material, Mesh, PartData, Primitive, Scene, Sketch, SketchCurve, SketchEntity, SketchPlane,
    Vec2, Vec3,
};
use std::collections::HashMap;

/// Name of the part selected on launch, so its timeline shows immediately.
pub(crate) const HERO_PART: &str = "Bracket";

/// Base flange depth (world Z), upright height, and plate thickness, in metres.
const DEPTH: f32 = 1.5;
const HEIGHT: f32 = 1.15;
const THICKNESS: f32 = 0.22;
/// Bracket width along world X.
const WIDTH: f32 = 1.9;

fn line(start: Vec2, end: Vec2) -> SketchEntity {
    SketchCurve::Line { start, end }.into()
}

fn circle(center: Vec2, radius: f32) -> SketchEntity {
    SketchCurve::Circle { center, radius }.into()
}

fn polygon(points: &[Vec2]) -> Vec<SketchEntity> {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(&start, &end)| line(start, end))
        .collect()
}

fn extrude(
    sketch: Sketch,
    distance: f32,
    extent: ExtrudeExtent,
    flip: bool,
    operation: FeatureOperation,
) -> FeatureKind {
    FeatureKind::Extrude(ExtrudeFeature {
        sketch,
        profiles: Vec::new(),
        distance,
        extent,
        flip,
        operation,
    })
}

/// The bracket's feature history, in part space (origin at the centre of
/// the base's underside; the upright stands at the back, −Z).
pub(crate) fn bracket() -> PartData {
    let p = Vec2::new;
    let back = DEPTH * 0.5;
    let front = -back;
    let mut part = PartData::new(0, HERO_PART);

    // Right plane: sketch x is world −Z, sketch y is world Y.
    let mut profile = Sketch::new(SketchPlane::right());
    profile.entities = polygon(&[
        p(front, 0.0),
        p(back, 0.0),
        p(back, HEIGHT),
        p(back - THICKNESS, HEIGHT),
        p(back - THICKNESS, THICKNESS),
        p(front, THICKNESS),
    ]);
    part.push_feature(
        "Profile",
        extrude(
            profile,
            WIDTH,
            ExtrudeExtent::Symmetric,
            false,
            FeatureOperation::Join,
        ),
    );

    // The gusset's corners sit inside the plates, so the join never meets a
    // coplanar face.
    let mut gusset = Sketch::new(SketchPlane::right());
    gusset.entities = polygon(&[p(back - 0.15, 0.15), p(back - 0.15, 0.8), p(-0.05, 0.15)]);
    part.push_feature(
        "Gusset",
        extrude(
            gusset,
            0.16,
            ExtrudeExtent::Symmetric,
            false,
            FeatureOperation::Join,
        ),
    );

    // Ground plane on the base's top face: sketch y is world −Z.
    let mut base_holes = Sketch::new(SketchPlane {
        origin: Vec3::new(0.0, THICKNESS, 0.0),
        ..SketchPlane::ground()
    });
    base_holes.entities = vec![circle(p(-0.58, -0.3), 0.11), circle(p(0.58, -0.3), 0.11)];
    part.push_feature(
        "Base holes",
        extrude(
            base_holes,
            0.5,
            ExtrudeExtent::OneSide,
            true,
            FeatureOperation::Cut,
        ),
    );

    // Front-facing plane on the upright's inner face.
    let mut upright_holes = Sketch::new(SketchPlane {
        origin: Vec3::new(0.0, 0.0, -(back - THICKNESS)),
        ..SketchPlane::front()
    });
    upright_holes.entities = vec![circle(p(-0.58, 0.74), 0.11), circle(p(0.58, 0.74), 0.11)];
    part.push_feature(
        "Upright holes",
        extrude(
            upright_holes,
            THICKNESS + 0.08,
            ExtrudeExtent::OneSide,
            true,
            FeatureOperation::Cut,
        ),
    );

    // The base's front corners and the upright's top corners.
    let x = WIDTH * 0.5;
    let mut edges = Vec::new();
    for side in [-x, x] {
        edges.push(EdgeRef {
            start: Vec3::new(side, 0.0, back),
            end: Vec3::new(side, THICKNESS, back),
        });
        edges.push(EdgeRef {
            start: Vec3::new(side, HEIGHT, -back),
            end: Vec3::new(side, HEIGHT, -(back - THICKNESS)),
        });
    }
    part.push_feature(
        "Chamfer",
        FeatureKind::Chamfer(ChamferFeature {
            edges,
            distance: 0.14,
        }),
    );
    part
}

/// A quad-only box of half-extent 1 whose edges and corners are rounded with
/// `radius`. Lattice lines are spaced by angle across the rounds and evenly
/// across the flats, so highlights run smoothly around every edge.
pub(crate) fn rounded_box(radius: f32, round_segments: u32, flat_segments: u32) -> Mesh {
    let inner = 1.0 - radius;
    let mut coordinates = Vec::new();
    for k in 0..round_segments {
        let angle = std::f32::consts::FRAC_PI_2 * k as f32 / round_segments as f32;
        coordinates.push(-(inner + radius * angle.cos()));
    }
    for k in 0..flat_segments {
        coordinates.push(-inner + 2.0 * inner * k as f32 / flat_segments as f32);
    }
    for k in 0..=round_segments {
        let angle = std::f32::consts::FRAC_PI_2 * k as f32 / round_segments as f32;
        coordinates.push(inner + radius * angle.sin());
    }
    let n = coordinates.len() - 1;
    let mut mesh = Mesh {
        positions: Vec::new(),
        faces: Vec::new(),
        groups: Vec::new(),
    };
    let mut index = HashMap::new();
    let mut vertex = |lattice: [usize; 3], mesh: &mut Mesh| -> u32 {
        *index.entry(lattice).or_insert_with(|| {
            let p = Vec3::from_array(lattice.map(|l| coordinates[l]));
            let core = p.clamp(Vec3::splat(-inner), Vec3::splat(inner));
            mesh.positions
                .push(core + (p - core).normalize_or_zero() * radius);
            mesh.positions.len() as u32 - 1
        })
    };
    for axis in 0..3 {
        let (b, c) = ((axis + 1) % 3, (axis + 2) % 3);
        for side in [0, n] {
            for i in 0..n {
                for j in 0..n {
                    let mut quad = [(0, 0), (1, 0), (1, 1), (0, 1)].map(|(di, dj)| {
                        let mut lattice = [0; 3];
                        lattice[axis] = side;
                        lattice[b] = i + di;
                        lattice[c] = j + dj;
                        vertex(lattice, &mut mesh)
                    });
                    // b × c points along +axis, so this order faces outward on
                    // the far side and must flip on the near side.
                    if side == 0 {
                        quad.reverse();
                    }
                    mesh.faces.push(quad.to_vec());
                }
            }
        }
    }
    mesh
}

/// The startup studio: mesh objects and the bracket part on a plinth under
/// a warm area light.
pub(crate) fn studio_scene() -> Scene {
    let mut scene = Scene::empty();

    let plinth = scene.add(Primitive::Cube);
    if let Some(object) = scene.object_mut(plinth) {
        object.name = "Studio plinth".into();
        object.transform.translation = Vec3::new(0.0, -0.15, 0.0);
        object.transform.scale = Vec3::new(3.25, 0.15, 2.3);
    }
    if let Some(material) = scene.object_material_mut(plinth) {
        // Neutral graphite: reads as a stage in dark and light themes alike.
        material.base_color = Vec3::new(0.17, 0.173, 0.18);
        material.roughness = 0.6;
    }

    let bracket = scene
        .add_part(HERO_PART, bracket())
        .expect("the startup bracket is a valid part");
    if let Some(object) = scene.object_mut(bracket) {
        object.transform.translation = Vec3::new(-0.25, 0.0, 1.0);
        object.transform.rotation.y = 24f32.to_radians();
    }
    if let Some(material) = scene.object_material_mut(bracket) {
        *material = Material {
            // Bead-blasted aluminium. A fully metallic finish mirrors the dark
            // studio horizon and turns every vertical face near black.
            base_color: Vec3::new(0.78, 0.8, 0.82),
            metallic: 0.55,
            roughness: 0.42,
            ..Material::default()
        };
    }

    let torus = scene.add(Primitive::Torus);
    if let Some(object) = scene.object_mut(torus) {
        object.name = "Torus".into();
        object.transform.translation = Vec3::new(0.35, 1.12, -1.6);
        object.transform.rotation = Vec3::new(70f32.to_radians(), 0.0, -14f32.to_radians());
    }
    if let Some(material) = scene.object_material_mut(torus) {
        *material = Material {
            // Slate blue, away from the jade accent, so a selection on it reads.
            base_color: Vec3::new(0.1, 0.14, 0.26),
            metallic: 0.35,
            roughness: 0.3,
            ..Material::default()
        };
    }

    let sphere = scene.add(Primitive::Sphere);
    if let Some(object) = scene.object_mut(sphere) {
        object.transform.translation = Vec3::new(2.0, 0.6, -0.15);
        object.transform.scale = Vec3::splat(0.6);
    }
    if let Some(material) = scene.object_material_mut(sphere) {
        *material = Material {
            base_color: Vec3::new(0.72, 0.50, 0.30),
            metallic: 0.72,
            roughness: 0.19,
            ..Material::default()
        };
    }

    let cube = scene.add(Primitive::Cube);
    if let Some(mesh) = scene.object_mesh_mut(cube) {
        *mesh = rounded_box(0.32, 10, 6);
    }
    if let Some(object) = scene.object_mut(cube) {
        object.name = "Rounded cube".into();
        object.transform.translation = Vec3::new(-2.45, 0.59, 0.75);
        object.transform.scale = Vec3::splat(0.8);
        object.transform.rotation.y = 12f32.to_radians();
    }
    if let Some(material) = scene.object_material_mut(cube) {
        material.base_color = Vec3::new(0.61, 0.65, 0.71);
        material.roughness = 0.28;
    }

    let light = scene.add(Primitive::Plane);
    if let Some(object) = scene.object_mut(light) {
        object.name = "Key · area light".into();
        object.transform.translation = Vec3::new(-2.0, 5.0, 2.0);
        object.transform.rotation.x = std::f32::consts::PI;
        object.transform.scale = Vec3::new(1.6, 1.0, 1.25);
    }
    if let Some(material) = scene.object_material_mut(light) {
        material.base_color = Vec3::ONE;
        material.emission = Vec3::new(12.0, 10.5, 9.0);
    }
    // A neutral studio world, so renders keep the materials' own colors.
    scene.world.color = Vec3::new(0.6, 0.61, 0.63);
    scene.world.strength = 0.4;
    // A little closer than the default view, so the objects fill the frame
    // above the timeline.
    scene.camera.target = Vec3::new(0.0, 0.4, 0.2);
    scene.camera.distance = 8.1;
    scene
}

/// The object selected when the startup scene opens.
pub(crate) fn hero(scene: &Scene) -> Option<u64> {
    scene
        .objects
        .iter()
        .find(|object| object.is_part() && object.name == HERO_PART)
        .or_else(|| scene.objects.first())
        .map(|object| object.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_startup_scene_mixes_meshes_and_a_parametric_part() {
        let scene = studio_scene();
        scene.validate().unwrap();
        let hero = hero(&scene).unwrap();
        let object = scene.object(hero).unwrap();
        assert!(object.is_part());
        assert_eq!(object.name, HERO_PART);
        assert!(scene.objects.iter().any(|o| o.data.is_editable_mesh()));
        let part = scene.object_part(hero).unwrap();
        // Sketch-based features make a meaningful timeline.
        assert!(part.features.len() >= 5);
        assert!(
            part.features
                .iter()
                .filter(|f| crate::timeline::feature_sketch(f).is_some())
                .count()
                >= 4
        );
    }

    #[test]
    fn the_rounded_box_is_a_closed_outward_quad_mesh() {
        let mesh = rounded_box(0.32, 6, 6);
        assert!(mesh.faces.iter().all(|face| face.len() == 4));
        // Closed: every edge is shared by exactly two quads.
        let mut uses = HashMap::<(u32, u32), u32>::new();
        for face in &mesh.faces {
            for k in 0..4 {
                let (a, b) = (face[k], face[(k + 1) % 4]);
                *uses.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        assert!(uses.values().all(|&count| count == 2));
        // Outward: each face normal points away from the centre, and the
        // flats stay flat at the full half-extent.
        for face in 0..mesh.faces.len() {
            let center = mesh.faces[face]
                .iter()
                .map(|&i| mesh.positions[i as usize])
                .sum::<Vec3>()
                / 4.0;
            assert!(mesh.face_normal(face).dot(center) > 0.0);
        }
        let extent = mesh
            .positions
            .iter()
            .fold(Vec3::ZERO, |m, p| m.max(p.abs()));
        assert!((extent - Vec3::ONE).abs().max_element() < 1e-5);
    }

    #[test]
    fn the_startup_bracket_regenerates_cleanly() {
        let part = bracket();
        part.validate().unwrap();
        let result = forma_cad::regenerate(
            &forma_cad::RegenRequest {
                part_id: 1,
                part: part.clone(),
                skip: Vec::new(),
            },
            &mut forma_cad::RegenCache::default(),
            &mut |_| {},
        );
        // No warnings either: a fallback result would flag "Check" on launch.
        assert!(result.issues.is_empty(), "{:?}", result.issues);
        let mesh = result.mesh.expect("tessellation");
        mesh.validate().unwrap();
        assert!(mesh.has_groups());
        let (min, max) = mesh.positions.iter().fold(
            (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
            |(min, max), &p| (min.min(p), max.max(p)),
        );
        assert!(
            (max - min - Vec3::new(WIDTH, HEIGHT, DEPTH))
                .abs()
                .max_element()
                < 1.0e-3
        );
        assert!(min.y.abs() < 1.0e-4, "the bracket rests on the plinth");
        // Every feature changed the body: the L prism's 8 faces, the
        // gusset's 3, a wall per hole and a face per chamfered edge.
        assert!(
            result.faces.len() >= 8 + 3 + 4 + 4,
            "{} faces",
            result.faces.len()
        );
    }
}
