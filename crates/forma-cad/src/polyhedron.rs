//! Polyhedral solids: every face planar, every edge straight.
//!
//! This path avoids truck's coplanar-prone booleans where the answer is a
//! matter of moving planes: converting meshes to parts, and push/pull of a
//! face whose neighbours are planar (vertices slide along the intersections
//! of their incident planes, so the topology is unchanged).
use crate::KernelError;
use crate::body::{Body, face_normal, guard};
use crate::geom::{dvec, newell, point, project_onto_planes};
use forma_core::Mesh;
use glam::DVec3;
use std::collections::{BTreeMap, HashMap};
use truck_modeling::{
    BoundedCurve, Edge, Face, ParametricCurve, Shell, Solid, Vertex, Wire, builder,
};

/// Faces as loops of point indices; the first loop is the outer boundary,
/// counter-clockwise seen from outside, the rest are holes.
#[derive(Clone, Debug, Default)]
pub(crate) struct Polyhedron {
    pub points: Vec<DVec3>,
    pub faces: Vec<Vec<Vec<usize>>>,
}

impl Polyhedron {
    fn size(&self) -> f64 {
        let mut min = DVec3::splat(f64::INFINITY);
        let mut max = DVec3::splat(f64::NEG_INFINITY);
        for p in &self.points {
            min = min.min(*p);
            max = max.max(*p);
        }
        (max - min).length().max(1.0e-9)
    }

    fn face_plane(&self, face: usize) -> (DVec3, f64) {
        let outer: Vec<DVec3> = self.faces[face][0]
            .iter()
            .map(|&i| self.points[i])
            .collect();
        let normal = newell(&outer).normalize_or_zero();
        let centroid = outer.iter().sum::<DVec3>() / outer.len().max(1) as f64;
        (normal, normal.dot(centroid))
    }

    /// Signed volume by the divergence theorem over fan triangles.
    pub fn volume(&self) -> f64 {
        let origin = self.points.first().copied().unwrap_or_default();
        let mut volume = 0.0;
        for face in &self.faces {
            for lp in face {
                for k in 1..lp.len().saturating_sub(1) {
                    let (a, b, c) = (
                        self.points[lp[0]] - origin,
                        self.points[lp[k]] - origin,
                        self.points[lp[k + 1]] - origin,
                    );
                    volume += a.dot(b.cross(c)) / 6.0;
                }
            }
        }
        volume
    }
}

/// Build a truck solid, sharing every edge between its two faces.
pub(crate) fn build_solid(poly: &Polyhedron) -> Result<Body, KernelError> {
    guard(|| {
        let vertices: Vec<Vertex> = poly
            .points
            .iter()
            .map(|p| builder::vertex(point(*p)))
            .collect();
        let mut edges: HashMap<(usize, usize), Edge> = HashMap::new();
        let mut faces = Vec::with_capacity(poly.faces.len());
        for (index, loops) in poly.faces.iter().enumerate() {
            let mut wires = Vec::with_capacity(loops.len());
            for lp in loops {
                let mut wire = Wire::new();
                for k in 0..lp.len() {
                    let (a, b) = (lp[k], lp[(k + 1) % lp.len()]);
                    if a == b {
                        continue;
                    }
                    let key = (a.min(b), a.max(b));
                    let edge = edges
                        .entry(key)
                        .or_insert_with(|| builder::line(&vertices[key.0], &vertices[key.1]))
                        .clone();
                    wire.push_back(if a == key.0 { edge } else { edge.inverse() });
                }
                wires.push(wire);
            }
            let face = builder::try_attach_plane(&wires).map_err(|error| {
                KernelError::new(
                    format!("Face {index} is not planar or not closed ({error:?})"),
                    "Triangulate non-planar polygons before converting",
                )
            })?;
            // try_attach_plane orients by winding; make outward = counter-clockwise.
            let (expected, _) = poly.face_plane(index);
            let n = face_normal(&face);
            let face: Face = if crate::geom::dvec_v(n).dot(expected) < 0.0 {
                face.inverse()
            } else {
                face
            };
            faces.push(face);
        }
        let solid = Solid::try_new(vec![Shell::from(faces)]).map_err(|error| {
            KernelError::new(
                format!("The faces do not form a closed solid ({error:?})"),
                "Make sure the mesh is closed and manifold",
            )
        })?;
        Ok(Body::solid(solid, f64::INFINITY))
    })
}

/// Read a solid back as a polyhedron when all of its faces are planar and all
/// of its edges straight (within `tolerance`). Faces keep truck's face order.
pub(crate) fn extract(solid: &Solid, tolerance: f64) -> Option<Polyhedron> {
    let mut index: HashMap<truck_topology::VertexID<truck_modeling::Point3>, usize> =
        HashMap::new();
    let mut poly = Polyhedron::default();
    for face in solid.face_iter() {
        let mut loops = Vec::new();
        let mut samples = Vec::new();
        for wire in face.boundaries() {
            let mut lp = Vec::new();
            for edge in wire.edge_iter() {
                let curve = edge.oriented_curve();
                let (t0, t1) = curve.range_tuple();
                let (a, b) = (dvec(curve.subs(t0)), dvec(curve.subs(t1)));
                for k in 1..4 {
                    let p = dvec(curve.subs(t0 + (t1 - t0) * k as f64 / 4.0));
                    let along = (b - a).normalize_or_zero();
                    let off = (p - a) - along * (p - a).dot(along);
                    if off.length() > tolerance || !p.is_finite() {
                        return None;
                    }
                    samples.push(p);
                }
                let vertex = edge.front();
                let next = poly.points.len();
                let id = *index.entry(vertex.id()).or_insert(next);
                if id == next {
                    poly.points.push(dvec(vertex.point()));
                }
                lp.push(id);
            }
            if lp.len() < 3 {
                return None;
            }
            loops.push(lp);
        }
        let outer: Vec<DVec3> = loops[0].iter().map(|&i| poly.points[i]).collect();
        let normal = newell(&outer).normalize_or_zero();
        if normal == DVec3::ZERO {
            return None;
        }
        let offset = normal.dot(outer[0]);
        let planar = loops
            .iter()
            .flatten()
            .map(|&i| poly.points[i])
            .chain(samples)
            .all(|p| (normal.dot(p) - offset).abs() <= tolerance);
        if !planar {
            return None;
        }
        poly.faces.push(loops);
    }
    (!poly.faces.is_empty()).then_some(poly)
}

/// Move face `face` along its outward normal by `distance`. Every vertex of
/// the face moves to the nearest point on the moved plane and all of its other
/// incident planes, so neighbouring faces stay planar and keep their planes.
pub(crate) fn move_face(
    poly: &Polyhedron,
    face: usize,
    distance: f64,
) -> Result<Polyhedron, KernelError> {
    let size = poly.size();
    let planes: Vec<(DVec3, f64)> = (0..poly.faces.len()).map(|f| poly.face_plane(f)).collect();
    let mut incident: Vec<Vec<usize>> = vec![Vec::new(); poly.points.len()];
    for (f, loops) in poly.faces.iter().enumerate() {
        for &v in loops.iter().flatten() {
            if !incident[v].contains(&f) {
                incident[v].push(f);
            }
        }
    }
    let (normal, offset) = planes[face];
    let mut moved = poly.clone();
    let mut changed = Vec::new();
    for &v in poly.faces[face].iter().flatten() {
        if changed.contains(&v) {
            continue;
        }
        let mut constraints = vec![(normal, offset + distance)];
        // Group incident planes that coincide (coplanar neighbours).
        for &f in &incident[v] {
            if f == face {
                continue;
            }
            let (n, d) = planes[f];
            if constraints
                .iter()
                .any(|(m, e)| m.dot(n) > 1.0 - 1.0e-9 && (e - d).abs() < size * 1.0e-9)
            {
                continue;
            }
            constraints.push((n, d));
        }
        let target = project_onto_planes(poly.points[v], &constraints).ok_or_else(|| {
            KernelError::new(
                "Push/pull cannot move this face: a neighbouring face is parallel to it",
                "Push/pull faces whose neighbours meet them at an angle",
            )
        })?;
        moved.points[v] = target;
        changed.push(v);
    }
    // Reject moves that collapse or invert geometry.
    for (f, loops) in moved.faces.iter().enumerate() {
        let before = newell(&loops[0].iter().map(|&i| poly.points[i]).collect::<Vec<_>>());
        let after = newell(
            &loops[0]
                .iter()
                .map(|&i| moved.points[i])
                .collect::<Vec<_>>(),
        );
        if after.length() < size * size * 1.0e-12 || before.dot(after) <= 0.0 {
            return Err(KernelError::new(
                format!("Push/pull of {distance:.4} m would collapse a neighbouring face ({f})"),
                "Use a smaller distance, or cut through with a sketch extrusion instead",
            ));
        }
        for lp in loops {
            for k in 0..lp.len() {
                let (a, b) = (lp[k], lp[(k + 1) % lp.len()]);
                let (old, new) = (
                    poly.points[b] - poly.points[a],
                    moved.points[b] - moved.points[a],
                );
                if new.length() < size * 1.0e-9 || old.dot(new) <= 0.0 {
                    return Err(KernelError::new(
                        format!("Push/pull of {distance:.4} m passes through other geometry"),
                        "Use a smaller distance",
                    ));
                }
            }
        }
    }
    let volume = moved.volume();
    if volume <= 0.0 || (volume - poly.volume()) * distance.signum() <= 0.0 {
        return Err(KernelError::new(
            "Push/pull would turn the body inside out",
            "Use a smaller distance",
        ));
    }
    Ok(moved)
}

/// Convert a closed, manifold polygon mesh into a polyhedron: planar polygons
/// become faces (coplanar neighbours merge into one face, possibly with
/// holes) and non-planar polygons are triangulated. Shared vertices are
/// snapped onto all of their face planes so faces are exactly planar.
pub(crate) fn from_mesh(mesh: &Mesh) -> Result<Polyhedron, KernelError> {
    crate::convert::check_closed_manifold(mesh).map_err(|message| {
        KernelError::new(
            message,
            "Close holes and remove non-manifold edges before converting",
        )
    })?;
    let points: Vec<DVec3> = mesh.positions.iter().map(|p| p.as_dvec3()).collect();
    let mut size_box = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    for p in &points {
        size_box = (size_box.0.min(*p), size_box.1.max(*p));
    }
    let size = (size_box.1 - size_box.0).length().max(1.0e-9);
    let planarity = size * 1.0e-6;
    // Split non-planar polygons into their triangles.
    let triangles = mesh.triangles_with_faces();
    let mut polygons: Vec<Vec<usize>> = Vec::new();
    for (index, face) in mesh.faces.iter().enumerate() {
        let loop_points: Vec<DVec3> = face.iter().map(|&i| points[i as usize]).collect();
        let normal = newell(&loop_points).normalize_or_zero();
        let flat = loop_points
            .iter()
            .all(|p| (normal.dot(*p - loop_points[0])).abs() <= planarity);
        if flat {
            polygons.push(face.iter().map(|&i| i as usize).collect());
        } else {
            for (triangle, _) in triangles.iter().filter(|(_, f)| *f == index) {
                polygons.push(triangle.iter().map(|&i| i as usize).collect());
            }
        }
    }
    let planes: Vec<(DVec3, f64)> = polygons
        .iter()
        .map(|lp| {
            let pts: Vec<DVec3> = lp.iter().map(|&i| points[i]).collect();
            let n = newell(&pts).normalize_or_zero();
            let c = pts.iter().sum::<DVec3>() / pts.len() as f64;
            (n, n.dot(c))
        })
        .collect();
    // Directed edge -> polygon.
    let mut owner: HashMap<(usize, usize), usize> = HashMap::new();
    for (p, lp) in polygons.iter().enumerate() {
        for k in 0..lp.len() {
            owner.insert((lp[k], lp[(k + 1) % lp.len()]), p);
        }
    }
    // Union coplanar neighbours.
    let mut parent: Vec<usize> = (0..polygons.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for (p, lp) in polygons.iter().enumerate() {
        for k in 0..lp.len() {
            let (a, b) = (lp[k], lp[(k + 1) % lp.len()]);
            let Some(&q) = owner.get(&(b, a)) else {
                continue;
            };
            let (np, dp) = planes[p];
            let (nq, dq) = planes[q];
            if np.dot(nq) > 1.0 - 1.0e-8 && (dp - dq).abs() <= planarity {
                let (rp, rq) = (root(&mut parent, p), root(&mut parent, q));
                if rp != rq {
                    parent[rp] = rq;
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for p in 0..polygons.len() {
        let r = root(&mut parent, p);
        groups.entry(r).or_default().push(p);
    }
    let mut faces: Vec<Vec<Vec<usize>>> = Vec::new();
    let mut face_planes: Vec<(DVec3, f64)> = Vec::new();
    for members in groups.values() {
        let merged = if members.len() == 1 {
            None
        } else {
            merge_group(&polygons, members, &owner, &points, planes[members[0]].0)
        };
        match merged {
            Some(loops) => {
                face_planes.push(group_plane(&polygons, members, &points));
                faces.push(loops);
            }
            None => {
                for &p in members {
                    face_planes.push(planes[p]);
                    faces.push(vec![polygons[p].clone()]);
                }
            }
        }
    }
    // Snap every vertex onto the planes of all faces using it.
    let mut incident: Vec<Vec<usize>> = vec![Vec::new(); points.len()];
    for (f, loops) in faces.iter().enumerate() {
        for &v in loops.iter().flatten() {
            if !incident[v].contains(&f) {
                incident[v].push(f);
            }
        }
    }
    let mut snapped = points.clone();
    for (v, faces_here) in incident.iter().enumerate() {
        if faces_here.is_empty() {
            continue;
        }
        let constraints: Vec<(DVec3, f64)> = faces_here.iter().map(|&f| face_planes[f]).collect();
        if let Some(p) = project_onto_planes(points[v], &constraints)
            && p.distance(points[v]) <= size * 1.0e-4
        {
            snapped[v] = p;
        }
    }
    let mut poly = Polyhedron {
        points: snapped,
        faces,
    };
    if poly.volume() < 0.0 {
        // Inside-out mesh: flip every loop rather than refuse it.
        for loops in &mut poly.faces {
            for lp in loops {
                lp.reverse();
            }
        }
    }
    Ok(poly)
}

fn group_plane(polygons: &[Vec<usize>], members: &[usize], points: &[DVec3]) -> (DVec3, f64) {
    let mut normal = DVec3::ZERO;
    let mut centroid = DVec3::ZERO;
    let mut count = 0.0;
    for &p in members {
        let pts: Vec<DVec3> = polygons[p].iter().map(|&i| points[i]).collect();
        normal += newell(&pts);
        centroid += pts.iter().sum::<DVec3>();
        count += pts.len() as f64;
    }
    let normal = normal.normalize_or_zero();
    (normal, normal.dot(centroid / count.max(1.0)))
}

/// Boundary loops of a group of coplanar polygons: the outer loop first,
/// then holes. `None` when the boundary is not a set of simple loops.
fn merge_group(
    polygons: &[Vec<usize>],
    members: &[usize],
    owner: &HashMap<(usize, usize), usize>,
    points: &[DVec3],
    normal: DVec3,
) -> Option<Vec<Vec<usize>>> {
    let inside: std::collections::HashSet<usize> = members.iter().copied().collect();
    let mut next: HashMap<usize, usize> = HashMap::new();
    for &p in members {
        let lp = &polygons[p];
        for k in 0..lp.len() {
            let (a, b) = (lp[k], lp[(k + 1) % lp.len()]);
            let twin_inside = owner.get(&(b, a)).is_some_and(|q| inside.contains(q));
            if !twin_inside && next.insert(a, b).is_some() {
                // Two boundary edges leave one vertex: a pinch. Keep polygons.
                return None;
            }
        }
    }
    let mut loops: Vec<Vec<usize>> = Vec::new();
    let mut visited: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let starts: Vec<usize> = {
        let mut keys: Vec<usize> = next.keys().copied().collect();
        keys.sort_unstable();
        keys
    };
    for start in starts {
        if visited.contains(&start) {
            continue;
        }
        let mut lp = vec![start];
        visited.insert(start);
        let mut current = start;
        loop {
            let n = *next.get(&current)?;
            if n == start {
                break;
            }
            if !visited.insert(n) {
                return None;
            }
            lp.push(n);
            current = n;
        }
        loops.push(lp);
    }
    let area =
        |lp: &Vec<usize>| newell(&lp.iter().map(|&i| points[i]).collect::<Vec<_>>()).dot(normal);
    let outers = loops.iter().filter(|lp| area(lp) > 0.0).count();
    if outers != 1 {
        return None;
    }
    loops.sort_by(|a, b| area(b).total_cmp(&area(a)));
    Some(loops)
}

#[cfg(test)]
mod tests {
    use super::*;
    use forma_core::Primitive;

    #[test]
    fn meshes_become_solids_with_merged_coplanar_faces() {
        let poly = from_mesh(&Mesh::primitive(Primitive::Cube)).unwrap();
        assert_eq!(poly.faces.len(), 6);
        assert!((poly.volume() - 8.0).abs() < 1.0e-9);
        let body = build_solid(&poly).unwrap();
        let volume = body.volume(0.01).unwrap();
        assert!((volume - 8.0).abs() < 1.0e-6, "{volume}");
    }

    #[test]
    fn coplanar_triangles_merge_and_open_meshes_are_refused() {
        // A cube whose top is split into two triangles still has six faces.
        let mut mesh = Mesh::primitive(Primitive::Cube);
        let top = mesh.faces.remove(3);
        mesh.faces.push(vec![top[0], top[1], top[2]]);
        mesh.faces.push(vec![top[0], top[2], top[3]]);
        let poly = from_mesh(&mesh).unwrap();
        assert_eq!(poly.faces.len(), 6);
        mesh.faces.pop();
        assert!(from_mesh(&mesh).is_err());
    }

    #[test]
    fn moving_a_face_slides_its_vertices_along_neighbouring_planes() {
        let poly = from_mesh(&Mesh::primitive(Primitive::Cube)).unwrap();
        let top = (0..poly.faces.len())
            .find(|&f| poly.face_plane(f).0.y > 0.9)
            .unwrap();
        let pulled = move_face(&poly, top, 0.5).unwrap();
        assert!((pulled.volume() - 10.0).abs() < 1.0e-9);
        let pushed = move_face(&poly, top, -1.5).unwrap();
        assert!((pushed.volume() - 2.0).abs() < 1.0e-9);
        assert!(
            move_face(&poly, top, -2.5).is_err(),
            "pushing through the bottom"
        );
        let body = build_solid(&pulled).unwrap();
        assert!((body.volume(0.01).unwrap() - 10.0).abs() < 1.0e-6);
        // Round trip through truck.
        let back = extract(body.as_solid().unwrap(), 1.0e-9).unwrap();
        assert_eq!(back.faces.len(), 6);
        assert!((back.volume() - 10.0).abs() < 1.0e-9);
    }
}
