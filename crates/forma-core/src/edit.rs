//! Polygon component operations shared by interactive editing tools.
use crate::{Mat4, Mesh, Vec3};
use anyhow::{Result, ensure};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MeshElement {
    Face(usize),
    Edge([u32; 2]),
    Vertex(u32),
}

impl MeshElement {
    /// Resolve an existing selection without rebuilding the topology edge set.
    /// Selection owners must clear or remap elements after topology changes.
    pub fn vertex_indices(self, mesh: &Mesh) -> Vec<u32> {
        match self {
            Self::Face(i) => mesh.faces.get(i).cloned().unwrap_or_default(),
            Self::Edge([a, b])
                if a != b
                    && (a as usize) < mesh.positions.len()
                    && (b as usize) < mesh.positions.len() =>
            {
                vec![a, b]
            }
            Self::Vertex(i) if (i as usize) < mesh.positions.len() => vec![i],
            _ => vec![],
        }
    }
    pub fn vertices(self, mesh: &Mesh) -> Vec<u32> {
        match self {
            Self::Face(i) => mesh.faces.get(i).cloned().unwrap_or_default(),
            Self::Edge([a, b]) if mesh.edges().contains(&[a.min(b), a.max(b)]) => vec![a, b],
            Self::Vertex(i) if (i as usize) < mesh.positions.len() => vec![i],
            _ => vec![],
        }
    }
}

impl Mesh {
    /// Delete a face, or a vertex/edge and its incident faces. Keep the mesh
    /// valid: deleting its last surface requires deleting the object instead.
    /// Failures leave the original mesh untouched.
    pub fn delete_element(&mut self, element: MeshElement) -> Result<()> {
        self.delete_elements(&[element].into_iter().collect())
    }

    /// Delete a component set in one transaction, so face/vertex index changes
    /// cannot redirect later deletions in the same selection.
    pub fn delete_elements(
        &mut self,
        elements: &std::collections::BTreeSet<MeshElement>,
    ) -> Result<()> {
        use std::collections::BTreeSet;
        ensure!(!elements.is_empty(), "Select components first");
        let edges: BTreeSet<_> = self.edges().into_iter().collect();
        ensure!(
            elements.iter().all(|e| match e {
                MeshElement::Edge([a, b]) => edges.contains(&[(*a).min(*b), (*a).max(*b)]),
                _ => !e.vertex_indices(self).is_empty(),
            }),
            "Selection contains a missing component"
        );
        let vertices: BTreeSet<_> = elements
            .iter()
            .filter_map(|e| {
                if let MeshElement::Vertex(i) = e {
                    Some(*i)
                } else {
                    None
                }
            })
            .collect();
        let mut edited = self.clone();
        edited.faces = self
            .faces
            .iter()
            .enumerate()
            .filter(|(index, face)| {
                !elements.contains(&MeshElement::Face(*index))
                    && !face.iter().any(|i| vertices.contains(i))
                    && !face
                        .iter()
                        .zip(face.iter().cycle().skip(1))
                        .take(face.len())
                        .any(|(&a, &b)| elements.contains(&MeshElement::Edge([a.min(b), a.max(b)])))
            })
            .map(|(_, f)| f.clone())
            .collect();
        let mut remap = vec![0; self.positions.len()];
        edited.positions.clear();
        for (index, p) in self.positions.iter().enumerate() {
            if !vertices.contains(&(index as u32)) {
                remap[index] = edited.positions.len() as u32;
                edited.positions.push(*p);
            }
        }
        for face in &mut edited.faces {
            for i in face {
                *i = remap[*i as usize];
            }
        }
        ensure!(
            !edited.faces.is_empty(),
            "Cannot delete the last face; use Object mode to delete the object"
        );
        edited.validate()?;
        *self = edited;
        Ok(())
    }

    /// Extrude connected faces as a region: shared cap vertices stay shared,
    /// and walls are created only at the selection boundary.
    pub fn extrude_region(
        &mut self,
        faces: &std::collections::BTreeSet<usize>,
        offset: Vec3,
    ) -> Result<()> {
        ensure!(
            offset.is_finite() && offset.length() > 1e-6,
            "Move the extrusion away from its source"
        );
        let (vertices, boundary) = self.region_boundary(faces)?;
        let positions = vertices
            .iter()
            .map(|&i| (i, self.positions[i as usize] + offset))
            .collect();
        self.replace_region(faces, &boundary, positions)
    }

    /// Inset a coplanar region by intersecting its offset boundary edges. Unlike
    /// scaling toward a centroid, thickness stays uniform on rectangular faces.
    pub fn inset_region(
        &mut self,
        faces: &std::collections::BTreeSet<usize>,
        thickness: f32,
    ) -> Result<()> {
        ensure!(
            thickness.is_finite() && thickness > 1e-6,
            "Inset thickness must be positive"
        );
        let (vertices, boundary) = self.region_boundary(faces)?;
        ensure!(!boundary.is_empty(), "Inset needs a region with a boundary");
        let normal = self.face_normal(*faces.first().unwrap());
        let point = self.positions[self.faces[*faces.first().unwrap()][0] as usize];
        ensure!(
            faces
                .iter()
                .all(|&f| self.face_normal(f).dot(normal) > 0.999)
                && vertices
                    .iter()
                    .all(|&i| (self.positions[i as usize] - point).dot(normal).abs() < 1e-4),
            "Inset a coplanar face region; select one face for curved surfaces"
        );
        let mut positions: std::collections::BTreeMap<_, _> = vertices
            .iter()
            .map(|&i| (i, self.positions[i as usize]))
            .collect();
        for &vertex in &vertices {
            let incoming: Vec<_> = boundary.iter().filter(|edge| edge[1] == vertex).collect();
            let outgoing: Vec<_> = boundary.iter().filter(|edge| edge[0] == vertex).collect();
            if incoming.is_empty() && outgoing.is_empty() {
                continue;
            }
            ensure!(
                incoming.len() == 1 && outgoing.len() == 1,
                "Inset requires a manifold boundary"
            );
            let p = self.positions[vertex as usize];
            let a = (p - self.positions[incoming[0][0] as usize]).normalize();
            let b = (self.positions[outgoing[0][1] as usize] - p).normalize();
            let n1 = normal.cross(a);
            let n2 = normal.cross(b);
            let denominator = 1. + n1.dot(n2);
            ensure!(
                denominator > 1e-6,
                "Inset boundary contains a reversing edge"
            );
            positions.insert(vertex, p + (n1 + n2) * (thickness / denominator));
        }
        for &[a, b] in &boundary {
            ensure!(
                (positions[&b] - positions[&a])
                    .dot(self.positions[b as usize] - self.positions[a as usize])
                    > 1e-8,
                "Inset is too large for this region"
            );
        }
        self.replace_region(faces, &boundary, positions)
    }

    fn region_boundary(
        &self,
        faces: &std::collections::BTreeSet<usize>,
    ) -> Result<(std::collections::BTreeSet<u32>, Vec<[u32; 2]>)> {
        use std::collections::{BTreeMap, BTreeSet};
        self.validate()?;
        ensure!(
            !faces.is_empty() && faces.iter().all(|&i| i < self.faces.len()),
            "Select faces first"
        );
        let mut edges: BTreeMap<[u32; 2], Vec<[u32; 2]>> = BTreeMap::new();
        let mut vertices = BTreeSet::new();
        for &i in faces {
            let face = &self.faces[i];
            vertices.extend(face);
            for (&a, &b) in face
                .iter()
                .zip(face.iter().cycle().skip(1))
                .take(face.len())
            {
                edges.entry([a.min(b), a.max(b)]).or_default().push([a, b]);
            }
        }
        ensure!(
            edges
                .values()
                .all(|e| e.len() == 1 || (e.len() == 2 && e[0] == [e[1][1], e[1][0]])),
            "Region must be manifold with consistent face winding"
        );
        Ok((
            vertices,
            edges
                .into_values()
                .filter(|e| e.len() == 1)
                .map(|e| e[0])
                .collect(),
        ))
    }

    fn replace_region(
        &mut self,
        faces: &std::collections::BTreeSet<usize>,
        boundary: &[[u32; 2]],
        positions: std::collections::BTreeMap<u32, Vec3>,
    ) -> Result<()> {
        ensure!(
            self.positions.len() + positions.len() <= 2_000_000
                && self.faces.len() + boundary.len() <= 2_000_000,
            "Mesh vertex/face limit exceeded"
        );
        let mut edited = self.clone();
        let mut remap = std::collections::BTreeMap::new();
        for (index, position) in positions {
            remap.insert(index, edited.positions.len() as u32);
            edited.positions.push(position);
        }
        for &i in faces {
            edited.faces[i] = self.faces[i].iter().map(|i| remap[i]).collect();
        }
        for &[a, b] in boundary {
            edited.faces.push(vec![a, b, remap[&b], remap[&a]]);
        }
        edited.validate()?;
        *self = edited;
        Ok(())
    }

    pub fn duplicate_faces(
        &mut self,
        faces: &std::collections::BTreeSet<usize>,
    ) -> Result<std::collections::BTreeSet<usize>> {
        let (vertices, _) = self.region_boundary(faces)?;
        ensure!(
            self.positions.len() + vertices.len() <= 2_000_000
                && self.faces.len() + faces.len() <= 2_000_000,
            "Mesh vertex/face limit exceeded"
        );
        let mut edited = self.clone();
        let mut remap = std::collections::BTreeMap::new();
        for i in vertices {
            remap.insert(i, edited.positions.len() as u32);
            edited.positions.push(self.positions[i as usize]);
        }
        let mut selected = std::collections::BTreeSet::new();
        for &i in faces {
            selected.insert(edited.faces.len());
            edited
                .faces
                .push(self.faces[i].iter().map(|i| remap[i]).collect());
        }
        edited.validate()?;
        *self = edited;
        Ok(selected)
    }

    /// Transform selected vertices in world space, leaving the instance's
    /// transform and unselected vertices unchanged. Supports parent transforms.
    pub fn transform_vertices(&mut self, vertices: &[u32], world: Mat4, operation: Mat4) {
        if operation.abs_diff_eq(Mat4::IDENTITY, 1e-7) {
            return;
        }
        let local = world.inverse() * operation * world;
        let indices: std::collections::BTreeSet<_> = vertices.iter().copied().collect();
        for index in indices {
            if let Some(position) = self.positions.get_mut(index as usize) {
                let p: Vec3 = local.transform_point3(*position);
                if p.is_finite() {
                    *position = p;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Primitive;

    #[test]
    fn deleting_each_component_keeps_valid_topology() {
        for (element, faces, vertices) in [
            (MeshElement::Face(0), 5, 8),
            (MeshElement::Edge([0, 1]), 4, 8),
            (MeshElement::Vertex(0), 3, 7),
        ] {
            let mut mesh = Mesh::primitive(Primitive::Cube);
            mesh.delete_element(element).unwrap();
            assert_eq!(mesh.faces.len(), faces);
            assert_eq!(mesh.positions.len(), vertices);
            mesh.validate().unwrap();
        }
    }

    #[test]
    fn invalid_and_last_surface_deletion_are_atomic() {
        let original = Mesh::primitive(Primitive::Plane);
        for element in [
            MeshElement::Face(0),
            MeshElement::Face(8),
            MeshElement::Edge([0, 2]),
            MeshElement::Vertex(99),
        ] {
            let mut mesh = original.clone();
            assert!(mesh.delete_element(element).is_err());
            assert_eq!(mesh, original);
        }
    }

    #[test]
    fn rotation_and_scale_preserve_component_pivot_and_other_vertices() {
        let original = Mesh::primitive(Primitive::Cube);
        for element in [
            MeshElement::Face(0),
            MeshElement::Edge([0, 1]),
            MeshElement::Vertex(0),
        ] {
            let vertices = element.vertices(&original);
            let center = vertices
                .iter()
                .map(|i| original.positions[*i as usize])
                .sum::<Vec3>()
                / vertices.len() as f32;
            for operation in [
                Mat4::from_rotation_z(0.8),
                Mat4::from_scale(Vec3::new(2., 1., 1.)),
            ] {
                let mut mesh = original.clone();
                mesh.transform_vertices(
                    &vertices,
                    Mat4::IDENTITY,
                    Mat4::from_translation(center) * operation * Mat4::from_translation(-center),
                );
                let after = vertices
                    .iter()
                    .map(|i| mesh.positions[*i as usize])
                    .sum::<Vec3>()
                    / vertices.len() as f32;
                assert!(after.distance(center) < 1e-5);
                for (i, p) in mesh.positions.iter().enumerate() {
                    if !vertices.contains(&(i as u32)) {
                        assert_eq!(*p, original.positions[i]);
                    }
                }
            }
        }
    }

    #[test]
    fn component_transform_respects_world_coordinates_and_unselected_vertices() {
        let original = Mesh::primitive(Primitive::Cube);
        let world = Mat4::from_translation(Vec3::new(4., 2., -3.))
            * Mat4::from_rotation_y(0.7)
            * Mat4::from_scale(Vec3::new(2., 3., 0.5));
        for element in [
            MeshElement::Face(0),
            MeshElement::Edge([0, 1]),
            MeshElement::Vertex(0),
        ] {
            let vertices = element.vertices(&original);
            let mut mesh = original.clone();
            let delta = Vec3::new(1., -2., 3.);
            mesh.transform_vertices(&vertices, world, Mat4::from_translation(delta));
            for (i, p) in mesh.positions.iter().enumerate() {
                let expected = world.transform_point3(original.positions[i])
                    + if vertices.contains(&(i as u32)) {
                        delta
                    } else {
                        Vec3::ZERO
                    };
                assert!(world.transform_point3(*p).distance(expected) < 1e-5);
            }
        }
    }
}

#[cfg(test)]
mod modelling_tests {
    use super::*;
    use std::collections::BTreeSet;
    fn patch() -> Mesh {
        Mesh {
            positions: vec![
                Vec3::new(-2., 0., -1.),
                Vec3::new(0., 0., -1.),
                Vec3::new(2., 0., -1.),
                Vec3::new(-2., 0., 1.),
                Vec3::new(0., 0., 1.),
                Vec3::new(2., 0., 1.),
            ],
            faces: vec![vec![0, 3, 4, 1], vec![1, 4, 5, 2]],
        }
    }
    #[test]
    fn region_extrusion_keeps_shared_cap_vertices_and_only_boundary_walls() {
        let mut mesh = patch();
        mesh.extrude_region(&[0, 1].into_iter().collect(), Vec3::Y)
            .unwrap();
        assert_eq!(mesh.positions.len(), 12);
        assert_eq!(mesh.faces.len(), 8);
        assert_eq!(
            mesh.faces[0]
                .iter()
                .filter(|i| mesh.faces[1].contains(i))
                .count(),
            2
        );
        mesh.validate().unwrap();
    }
    #[test]
    fn region_inset_has_even_thickness_and_preserves_internal_edges() {
        let mut mesh = patch();
        mesh.inset_region(&[0, 1].into_iter().collect(), 0.25)
            .unwrap();
        assert_eq!(mesh.positions.len(), 12);
        assert_eq!(mesh.faces.len(), 8);
        let cap: BTreeSet<_> = mesh.faces[..2].iter().flatten().copied().collect();
        assert_eq!(cap.len(), 6);
        for i in cap {
            let p = mesh.positions[i as usize];
            assert!((p.z.abs() - 0.75).abs() < 1e-6);
            assert!(p.x.abs() <= 1.75);
        }
        mesh.validate().unwrap();
    }
    #[test]
    fn failed_tools_are_atomic_and_batch_delete_does_not_shift_selection() {
        let original = patch();
        for invalid in [0., -0.1, 1., 2., f32::NAN] {
            let mut mesh = original.clone();
            assert!(
                mesh.inset_region(&[0, 1].into_iter().collect(), invalid)
                    .is_err()
            );
            assert_eq!(mesh, original);
        }
        let mut mesh = Mesh::primitive(crate::Primitive::Cube);
        let before = mesh.clone();
        mesh.delete_elements(
            &[MeshElement::Face(0), MeshElement::Face(2)]
                .into_iter()
                .collect(),
        )
        .unwrap();
        assert_eq!(
            mesh.faces,
            vec![
                before.faces[1].clone(),
                before.faces[3].clone(),
                before.faces[4].clone(),
                before.faces[5].clone()
            ]
        );
    }
    #[test]
    fn duplicate_region_is_disconnected_but_shares_internal_vertices() {
        let mut mesh = patch();
        let selected = mesh.duplicate_faces(&[0, 1].into_iter().collect()).unwrap();
        assert_eq!(selected, [2, 3].into_iter().collect());
        assert_eq!(mesh.positions.len(), 12);
        assert!(mesh.faces[2..].iter().flatten().all(|i| *i >= 6));
        assert_eq!(
            mesh.faces[2]
                .iter()
                .filter(|i| mesh.faces[3].contains(i))
                .count(),
            2
        );
    }
}
