//! Conversions that need no kernel: checks the application can run on its own
//! thread before handing work to the worker, and face references from picks.
use forma_core::{
    BaseBody, BaseBodyFeature, FaceRef, FeatureKind, FeatureOperation, Mesh, PartData,
};
use glam::Vec3;
use std::collections::HashMap;

/// Whether a mesh bounds a solid: every edge is shared by exactly two faces
/// that traverse it in opposite directions. Returns a user-facing reason.
pub fn check_closed_manifold(mesh: &Mesh) -> Result<(), String> {
    mesh.validate()
        .map_err(|error| format!("The mesh is invalid: {error}"))?;
    let mut directed: HashMap<(u32, u32), u32> = HashMap::new();
    for face in &mesh.faces {
        for k in 0..face.len() {
            let edge = (face[k], face[(k + 1) % face.len()]);
            *directed.entry(edge).or_default() += 1;
        }
    }
    let mut open = 0;
    let mut non_manifold = 0;
    let mut flipped = 0;
    for (&(a, b), &count) in &directed {
        let twin = directed.get(&(b, a)).copied().unwrap_or(0);
        if count > 1 {
            // Two faces running the same way: either non-manifold or flipped.
            if twin == 0 {
                flipped += 1;
            } else {
                non_manifold += 1;
            }
        } else if twin == 0 {
            open += 1;
        } else if twin > 1 {
            non_manifold += 1;
        }
    }
    if open > 0 {
        return Err(format!(
            "The mesh is open ({open} boundary edges) · Close its holes to make a solid part"
        ));
    }
    if non_manifold > 0 {
        return Err(format!(
            "The mesh is non-manifold ({non_manifold} edges shared by more than two faces)"
        ));
    }
    if flipped > 0 {
        return Err(format!(
            "The mesh has inconsistent normals ({flipped} edges) · Recalculate normals first"
        ));
    }
    Ok(())
}

/// A persistent face reference from a viewport pick on a part's cached
/// tessellation: `polygon` is the picked mesh face and `point` the hit
/// position, both in part (object local) space. Planar face groups use the
/// group's exact plane normal; curved ones the picked triangle's normal.
pub fn face_ref_from_pick(mesh: &Mesh, polygon: usize, point: Vec3) -> Option<FaceRef> {
    let group = mesh.face_group(polygon)?;
    let members: Vec<usize> = if mesh.has_groups() {
        (0..mesh.faces.len())
            .filter(|&f| mesh.groups[f] == group)
            .collect()
    } else {
        vec![polygon]
    };
    let picked = mesh.face_normal(polygon);
    let mut sum = Vec3::ZERO;
    let mut planar = true;
    for &f in &members {
        let n = mesh.face_normal(f);
        planar &= n.dot(picked) > 1.0 - 1.0e-4;
        let face = &mesh.faces[f];
        let a = mesh.positions[face[0] as usize];
        let area: Vec3 = face
            .windows(2)
            .skip(1)
            .map(|w| (mesh.positions[w[0] as usize] - a).cross(mesh.positions[w[1] as usize] - a))
            .sum();
        sum += area;
    }
    let normal = if planar { sum } else { picked }.try_normalize()?;
    point.is_finite().then_some(FaceRef { point, normal })
}

/// A new part whose history is a single base body made from `mesh` (in part
/// space). Check [`check_closed_manifold`] first for an immediate message;
/// the worker reports any remaining problems as a feature issue.
pub fn part_from_mesh(name: &str, mesh: &Mesh, source: Option<u64>) -> PartData {
    let mut mesh = mesh.clone();
    mesh.groups.clear();
    let mut part = PartData::new(0, name);
    let id = part.push_feature(
        "Mesh body",
        FeatureKind::BaseBody(BaseBodyFeature {
            body: BaseBody::Mesh { mesh },
            operation: FeatureOperation::Join,
        }),
    );
    part.edit_feature(id, |feature| feature.source = source);
    part
}

#[cfg(test)]
mod tests {
    use super::*;
    use forma_core::Primitive;

    #[test]
    fn closedness_checks_explain_the_problem() {
        let cube = Mesh::primitive(Primitive::Cube);
        assert!(check_closed_manifold(&cube).is_ok());
        let plane = Mesh::primitive(Primitive::Plane);
        assert!(check_closed_manifold(&plane).unwrap_err().contains("open"));
        let mut flipped = cube.clone();
        flipped.faces[0].reverse();
        assert!(
            check_closed_manifold(&flipped)
                .unwrap_err()
                .contains("normals")
        );
    }

    #[test]
    fn picks_on_planar_groups_use_the_group_plane() {
        let mut mesh = Mesh::primitive(Primitive::Plane);
        mesh.faces = vec![vec![0, 1, 2], vec![0, 2, 3]];
        mesh.groups = vec![4, 4];
        let reference = face_ref_from_pick(&mesh, 1, Vec3::new(0.5, 0.0, -0.5)).unwrap();
        assert!(reference.normal.distance(Vec3::Y) < 1.0e-6);
        assert_eq!(reference.point, Vec3::new(0.5, 0.0, -0.5));
        assert!(face_ref_from_pick(&mesh, 9, Vec3::ZERO).is_none());
    }
}
