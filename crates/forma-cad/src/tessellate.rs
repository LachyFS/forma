//! B-rep → Forma mesh.
//!
//! Display tessellation keeps every B-rep face's triangles in its own vertex
//! set, so shading creases at real edges and stays smooth inside curved
//! faces (Forma's renderer averages normals over shared vertices). Tangent
//! continuous neighbours, such as the three patches truck uses for a
//! cylinder side, share a face group and their vertices, hiding the seams.
//! `Mesh::groups` then lets picking resolve a polygon to its face and lets
//! overlays draw only B-rep edges.
use crate::KernelError;
use crate::body::Body;
use crate::geom::dvec;
use crate::protocol::{FaceInfo, SurfaceKind};
use forma_core::{Mesh, TessellationQuality};
use glam::{DVec3, Vec3};
use std::collections::HashMap;
use truck_polymesh::PolygonMesh;

/// Triangle budget for one part; coarser tolerances are tried beyond it.
const MAX_TRIANGLES: usize = 1_500_000;
/// Neighbouring faces whose normals agree within this angle along their
/// shared edge are drawn as one smooth face.
const SMOOTH_COS: f64 = 0.999_4; // ≈ 2°

pub(crate) struct Tessellation {
    pub mesh: Mesh,
    pub faces: Vec<FaceInfo>,
    /// B-rep faces the kernel could not tessellate.
    pub missing_faces: usize,
}

/// Per B-rep face triangles in part space.
struct FaceTriangles {
    positions: Vec<DVec3>,
    /// Exact surface normals per position, when truck provided finite ones.
    normals: Option<Vec<DVec3>>,
    triangles: Vec<[usize; 3]>,
}

fn face_triangles(poly: &PolygonMesh, degenerate_area: f64) -> FaceTriangles {
    let positions: Vec<DVec3> = poly.positions().iter().map(|p| dvec(*p)).collect();
    let mut normals = vec![DVec3::ZERO; positions.len()];
    let mut have_normals = !poly.normals().is_empty();
    let mut triangles = Vec::new();
    for triangle in poly.faces().triangle_iter() {
        let [a, b, c] = triangle.map(|v| v.pos);
        if a == b || b == c || a == c {
            continue;
        }
        let area = (positions[b] - positions[a])
            .cross(positions[c] - positions[a])
            .length();
        // Also rejects NaN areas.
        if area.partial_cmp(&degenerate_area) != Some(std::cmp::Ordering::Greater) {
            continue;
        }
        for v in triangle {
            match v.nor.and_then(|n| poly.normals().get(n)) {
                Some(n) if n.x.is_finite() && n.y.is_finite() && n.z.is_finite() => {
                    normals[v.pos] = DVec3::new(n.x, n.y, n.z);
                }
                _ => have_normals = false,
            }
        }
        triangles.push([a, b, c]);
    }
    FaceTriangles {
        positions,
        normals: have_normals.then_some(normals),
        triangles,
    }
}

/// A position snapped to the welding grid.
type Key = (i64, i64, i64);

fn quantize(p: DVec3, step: f64) -> Key {
    (
        (p.x / step).round() as i64,
        (p.y / step).round() as i64,
        (p.z / step).round() as i64,
    )
}

/// Merges points closer than a tolerance. Unlike rounding to a grid, points
/// straddling a cell boundary still merge, because neighbouring cells are
/// searched too.
struct Welder {
    tolerance: f64,
    cells: HashMap<Key, Vec<(DVec3, u32)>>,
}

impl Welder {
    fn new(tolerance: f64) -> Self {
        Self {
            tolerance,
            cells: HashMap::new(),
        }
    }

    fn find(&self, p: DVec3) -> Option<u32> {
        let (x, y, z) = quantize(p, self.tolerance);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(points) = self.cells.get(&(x + dx, y + dy, z + dz))
                        && let Some((_, index)) =
                            points.iter().find(|(q, _)| q.distance(p) <= self.tolerance)
                    {
                        return Some(*index);
                    }
                }
            }
        }
        None
    }

    /// The index of the point at `p`, creating it with `create` if new.
    fn index(&mut self, p: DVec3, create: impl FnOnce() -> u32) -> u32 {
        if let Some(index) = self.find(p) {
            return index;
        }
        let index = create();
        self.cells
            .entry(quantize(p, self.tolerance))
            .or_default()
            .push((p, index));
        index
    }
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// Tessellate for display, with face groups.
pub(crate) fn tessellate(
    body: &Body,
    quality: TessellationQuality,
) -> Result<Tessellation, KernelError> {
    let mut tolerance = body.tolerance(quality.chord_fraction(), quality.max_angle());
    for _ in 0..4 {
        let result = tessellate_at(body, tolerance)?;
        if result.mesh.faces.len() <= MAX_TRIANGLES {
            return Ok(result);
        }
        tolerance *= 2.5;
    }
    Err(KernelError::new(
        "The part is too detailed to display",
        "Choose a coarser tessellation quality",
    ))
}

fn tessellate_at(body: &Body, tolerance: f64) -> Result<Tessellation, KernelError> {
    let size = body.size();
    let meshes = body.face_meshes(tolerance)?;
    let degenerate = (size * 1.0e-9).powi(2);
    let missing_faces = meshes.iter().filter(|m| m.is_none()).count();
    let faces: Vec<Option<FaceTriangles>> = meshes
        .iter()
        .map(|m| m.as_ref().map(|poly| face_triangles(poly, degenerate)))
        .collect();
    // Positions on shared edges coincide (truck meshes each edge once per
    // shell); quantize well below the chord tolerance to match them.
    let step = (tolerance * 1.0e-2).max(size * 1.0e-10);
    let mut parent: Vec<usize> = (0..faces.len()).collect();
    {
        // position -> (face, normal) for every vertex.
        let mut welder = Welder::new(step);
        let mut at: Vec<Vec<(usize, Option<DVec3>)>> = Vec::new();
        for (index, face) in faces.iter().enumerate() {
            let Some(face) = face else { continue };
            let mut seen = std::collections::HashSet::new();
            for triangle in &face.triangles {
                for &v in triangle {
                    if seen.insert(v) {
                        let normal = face.normals.as_ref().map(|n| n[v]);
                        let slot = welder.index(face.positions[v], || {
                            at.push(Vec::new());
                            (at.len() - 1) as u32
                        });
                        at[slot as usize].push((index, normal));
                    }
                }
            }
        }
        // For each face pair sharing vertices: smooth when every shared vertex agrees.
        let mut pairs: HashMap<(usize, usize), (usize, bool)> = HashMap::new();
        for entries in &at {
            for i in 0..entries.len() {
                for j in i + 1..entries.len() {
                    let (fa, na) = entries[i];
                    let (fb, nb) = entries[j];
                    if fa == fb {
                        continue;
                    }
                    let key = (fa.min(fb), fa.max(fb));
                    let smooth = match (na, nb) {
                        (Some(a), Some(b)) => {
                            a.normalize_or_zero().dot(b.normalize_or_zero()) > SMOOTH_COS
                        }
                        _ => false,
                    };
                    let entry = pairs.entry(key).or_insert((0, true));
                    entry.0 += 1;
                    entry.1 &= smooth;
                }
            }
        }
        let mut keys: Vec<_> = pairs.into_iter().collect();
        keys.sort_unstable_by_key(|(key, _)| *key);
        for ((a, b), (shared, smooth)) in keys {
            // Two shared points make a shared edge; one is just a corner.
            if shared >= 2 && smooth {
                let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                if ra != rb {
                    parent[ra.max(rb)] = ra.min(rb);
                }
            }
        }
    }
    // Renumber groups in face order.
    let mut group_of_root: HashMap<usize, u32> = HashMap::new();
    let mut group_faces: Vec<Vec<u32>> = Vec::new();
    let mut face_group = vec![u32::MAX; faces.len()];
    for (index, face) in faces.iter().enumerate() {
        if face.as_ref().is_none_or(|f| f.triangles.is_empty()) {
            continue;
        }
        let root = find(&mut parent, index);
        let group = *group_of_root.entry(root).or_insert_with(|| {
            group_faces.push(Vec::new());
            (group_faces.len() - 1) as u32
        });
        group_faces[group as usize].push(index as u32);
        face_group[index] = group;
    }
    let mut mesh = Mesh::default();
    let mut infos = Vec::with_capacity(group_faces.len());
    for (group, members) in group_faces.iter().enumerate() {
        let mut weld = Welder::new(step);
        let mut area = 0.0;
        let mut normal_sum = DVec3::ZERO;
        let mut largest = (0.0, DVec3::ZERO, DVec3::ZERO);
        let mut normals = Vec::new();
        for &face_index in members {
            let face = faces[face_index as usize].as_ref().unwrap();
            for triangle in &face.triangles {
                let corners = triangle.map(|v| face.positions[v]);
                let indices = corners.map(|p| {
                    weld.index(p, || {
                        mesh.positions.push(p.as_vec3());
                        (mesh.positions.len() - 1) as u32
                    })
                });
                if indices[0] == indices[1] || indices[1] == indices[2] || indices[0] == indices[2]
                {
                    continue;
                }
                let [a, b, c] = indices.map(|i| mesh.positions[i as usize]);
                if (b - a).cross(c - a).length_squared() <= 1.0e-18 {
                    continue;
                }
                let cross = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
                let triangle_area = cross.length() * 0.5;
                area += triangle_area;
                normal_sum += cross;
                normals.push(cross.normalize_or_zero());
                if triangle_area > largest.0 {
                    largest = (
                        triangle_area,
                        (corners[0] + corners[1] + corners[2]) / 3.0,
                        cross.normalize_or_zero(),
                    );
                }
                mesh.faces.push(indices.to_vec());
                mesh.groups.push(group as u32);
            }
        }
        let average = normal_sum.normalize_or_zero();
        let planar = normals.iter().all(|n| n.dot(average) > 1.0 - 1.0e-6);
        infos.push(FaceInfo {
            kind: if planar {
                SurfaceKind::Planar
            } else {
                SurfaceKind::Curved
            },
            brep_faces: members.clone(),
            point: largest.1.as_vec3(),
            normal: if planar { average } else { largest.2 }.as_vec3(),
            area: area as f32,
        });
    }
    if mesh.faces.is_empty() {
        return Err(KernelError::new(
            "The part could not be tessellated",
            "Simplify the last feature or choose another tessellation quality",
        ));
    }
    let _ = face_group;
    mesh.validate().map_err(|error| {
        KernelError::new(
            format!("The part tessellation is invalid ({error})"),
            "Try another tessellation quality",
        )
    })?;
    Ok(Tessellation {
        mesh,
        faces: infos,
        missing_faces,
    })
}

/// Bake a body into an editable polygon mesh: hole-free planar faces become
/// single n-gons, curved faces stay triangles, and all faces share welded
/// vertices so polygon tools see one connected surface.
pub(crate) fn editable_mesh(
    body: &Body,
    quality: TessellationQuality,
) -> Result<Mesh, KernelError> {
    let tolerance = body.tolerance(quality.chord_fraction(), quality.max_angle());
    let size = body.size();
    let meshes = body.face_meshes(tolerance)?;
    let step = (tolerance * 1.0e-2).max(size * 1.0e-10);
    let degenerate = (size * 1.0e-9).powi(2);
    let mut weld = Welder::new(step);
    let mut positions: Vec<Vec3> = Vec::new();
    let mut index_of = |p: DVec3, positions: &mut Vec<Vec3>| -> u32 {
        weld.index(p, || {
            positions.push(p.as_vec3());
            (positions.len() - 1) as u32
        })
    };
    let mut polygons: Vec<Vec<u32>> = Vec::new();
    for poly in meshes.iter().flatten() {
        let face = face_triangles(poly, degenerate);
        let local: Vec<u32> = face
            .positions
            .iter()
            .map(|p| index_of(*p, &mut positions))
            .collect();
        let triangles: Vec<[u32; 3]> = face
            .triangles
            .iter()
            .map(|t| t.map(|v| local[v]))
            .filter(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2])
            .collect();
        if let Some(ngon) = planar_outline(&triangles, &positions) {
            polygons.push(ngon);
        } else {
            polygons.extend(triangles.iter().map(|t| t.to_vec()));
        }
    }
    let mesh = Mesh::new(positions, polygons);
    if mesh.validate().is_ok() {
        return Ok(mesh);
    }
    // An n-gon touched itself after welding: fall back to triangles only.
    let mut triangles = Vec::new();
    for (triangle, _) in mesh.triangles_with_faces() {
        triangles.push(triangle.to_vec());
    }
    let fallback = Mesh::new(mesh.positions, triangles);
    fallback.validate().map_err(|error| {
        KernelError::new(
            format!("The baked mesh is invalid ({error})"),
            "Try another tessellation quality",
        )
    })?;
    Ok(fallback)
}

/// The single boundary loop of a planar triangle fan, when there is exactly one.
fn planar_outline(triangles: &[[u32; 3]], positions: &[Vec3]) -> Option<Vec<u32>> {
    if triangles.len() < 2 {
        return None;
    }
    let normal = |t: &[u32; 3]| {
        let [a, b, c] = t.map(|i| positions[i as usize].as_dvec3());
        (b - a).cross(c - a).normalize_or_zero()
    };
    let reference = normal(&triangles[0]);
    if !triangles
        .iter()
        .all(|t| normal(t).dot(reference) > 1.0 - 1.0e-6)
    {
        return None;
    }
    let mut directed: HashMap<(u32, u32), usize> = HashMap::new();
    for t in triangles {
        for k in 0..3 {
            *directed.entry((t[k], t[(k + 1) % 3])).or_default() += 1;
        }
    }
    let mut next: HashMap<u32, u32> = HashMap::new();
    for &(a, b) in directed
        .keys()
        .filter(|(a, b)| !directed.contains_key(&(*b, *a)))
    {
        if next.insert(a, b).is_some() {
            return None;
        }
    }
    let start = *next.keys().min()?;
    let mut outline = vec![start];
    let mut current = start;
    loop {
        let n = *next.get(&current)?;
        if n == start {
            break;
        }
        if outline.len() > next.len() || outline.len() >= 4096 {
            return None;
        }
        outline.push(n);
        current = n;
    }
    // Every boundary edge must belong to this one loop (no holes).
    (outline.len() == next.len()).then_some(outline)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::primitive;
    use forma_core::PrimitiveShape;

    #[test]
    fn cylinders_show_three_faces_with_smooth_sides() {
        let body = primitive(&PrimitiveShape::Cylinder {
            radius: 0.5,
            height: 2.0,
        })
        .unwrap();
        let tessellation = tessellate(&body, TessellationQuality::Medium).unwrap();
        // truck builds the side from several patches; they merge into one group.
        assert_eq!(
            tessellation.faces.len(),
            3,
            "{:?}",
            tessellation
                .faces
                .iter()
                .map(|f| &f.brep_faces)
                .collect::<Vec<_>>()
        );
        let curved = tessellation
            .faces
            .iter()
            .filter(|f| f.kind == SurfaceKind::Curved)
            .count();
        assert_eq!(curved, 1);
        let mesh = &tessellation.mesh;
        assert!(mesh.has_groups());
        // Only the two cap circles are drawn: no diagonals, no side seams.
        let edges = mesh.display_edges();
        for [a, b] in edges {
            let (pa, pb) = (mesh.positions[a as usize], mesh.positions[b as usize]);
            assert!(
                (pa.y - pb.y).abs() < 1.0e-6 && (pa.y.abs() - 1.0).abs() < 1.0e-5,
                "edge {pa:?}–{pb:?} is not on a cap rim"
            );
        }
        // Caps and side do not share vertices (crisp creases).
        let cap_vertices: std::collections::HashSet<u32> = mesh
            .faces
            .iter()
            .zip(&mesh.groups)
            .filter(|(_, g)| tessellation.faces[**g as usize].kind == SurfaceKind::Planar)
            .flat_map(|(f, _)| f.iter().copied())
            .collect();
        let side_vertices: std::collections::HashSet<u32> = mesh
            .faces
            .iter()
            .zip(&mesh.groups)
            .filter(|(_, g)| tessellation.faces[**g as usize].kind == SurfaceKind::Curved)
            .flat_map(|(f, _)| f.iter().copied())
            .collect();
        assert!(cap_vertices.is_disjoint(&side_vertices));
    }

    #[test]
    fn baked_boolean_results_are_closed() {
        use crate::boolean::{BoolOp, boolean, transform_body};
        let block = primitive(&PrimitiveShape::Box {
            size: Vec3::splat(2.0),
        })
        .unwrap();
        let tool = primitive(&PrimitiveShape::Cylinder {
            radius: 0.4,
            height: 3.0,
        })
        .unwrap();
        let tool = transform_body(
            &tool,
            glam::DMat4::from_scale(glam::DVec3::new(1.0, 1.0, 1.0)),
        )
        .unwrap();
        let drilled = boolean(&block, &tool, BoolOp::Subtract).unwrap();
        let mesh = editable_mesh(&drilled, TessellationQuality::Medium).unwrap();
        crate::convert::check_closed_manifold(&mesh).unwrap();
        let display = tessellate(&drilled, TessellationQuality::Medium).unwrap();
        assert_eq!(display.faces.len(), 7);
    }

    #[test]
    fn baked_boxes_are_six_quads_and_cylinders_keep_ngon_caps() {
        let body = primitive(&PrimitiveShape::Box {
            size: Vec3::new(1.0, 2.0, 3.0),
        })
        .unwrap();
        let mesh = editable_mesh(&body, TessellationQuality::Medium).unwrap();
        assert_eq!(mesh.faces.len(), 6);
        assert_eq!(mesh.positions.len(), 8);
        assert!(mesh.faces.iter().all(|f| f.len() == 4));
        assert!(mesh.groups.is_empty());
        let body = primitive(&PrimitiveShape::Cylinder {
            radius: 0.5,
            height: 1.0,
        })
        .unwrap();
        let mesh = editable_mesh(&body, TessellationQuality::Medium).unwrap();
        let ngons = mesh.faces.iter().filter(|f| f.len() > 4).count();
        assert_eq!(ngons, 2, "both caps become n-gons");
        // The baked volume matches the solid within the chord tolerance.
        let volume: f64 = mesh
            .triangles()
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| mesh.positions[i as usize].as_dvec3());
                a.dot(b.cross(c)) / 6.0
            })
            .sum();
        let exact = std::f64::consts::PI * 0.25;
        assert!((volume - exact).abs() < exact * 0.01, "{volume} vs {exact}");
        // Welded: every edge is shared by exactly two polygons.
        assert!(crate::convert::check_closed_manifold(&mesh).is_ok());
    }
}
