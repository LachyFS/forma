//! Polygon component operations shared by interactive editing tools.
use crate::{Mat4, Mesh, Vec3};
use anyhow::{Result, ensure};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshElement {
    Face(usize),
    Edge([u32; 2]),
    Vertex(u32),
}

impl MeshElement {
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
        ensure!(
            !element.vertices(self).is_empty(),
            "Select a component first"
        );
        let mut edited = self.clone();
        match element {
            MeshElement::Face(i) => {
                edited.faces.remove(i);
            }
            MeshElement::Edge([a, b]) => {
                edited.faces.retain(|f| {
                    !f.iter()
                        .zip(f.iter().cycle().skip(1))
                        .take(f.len())
                        .any(|(&x, &y)| (x == a && y == b) || (x == b && y == a))
                });
            }
            MeshElement::Vertex(v) => {
                edited.faces.retain(|f| !f.contains(&v));
                edited.positions.remove(v as usize);
                for face in &mut edited.faces {
                    for i in face {
                        if *i > v {
                            *i -= 1;
                        }
                    }
                }
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

    /// Transform selected vertices in world space, leaving the instance's
    /// transform and unselected vertices unchanged. Supports parent transforms.
    pub fn transform_vertices(&mut self, vertices: &[u32], world: Mat4, operation: Mat4) {
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
