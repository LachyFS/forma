//! Conversions between Forma's glam types and truck's cgmath types, and a
//! small triangle mesh used for robustness checks (bounds, volume, inside
//! tests) that never feeds back into truck.
use glam::{DMat4, DVec3};
use truck_modeling::{InnerSpace, Matrix4, Point3, Vector3};

pub(crate) fn point(v: DVec3) -> Point3 {
    Point3::new(v.x, v.y, v.z)
}

pub(crate) fn vector(v: DVec3) -> Vector3 {
    Vector3::new(v.x, v.y, v.z)
}

pub(crate) fn dvec(p: Point3) -> DVec3 {
    DVec3::new(p.x, p.y, p.z)
}

pub(crate) fn dvec_v(v: Vector3) -> DVec3 {
    DVec3::new(v.x, v.y, v.z)
}

pub(crate) fn matrix_d(m: DMat4) -> Matrix4 {
    let c = m.to_cols_array();
    Matrix4::new(
        c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7], c[8], c[9], c[10], c[11], c[12], c[13],
        c[14], c[15],
    )
}

pub(crate) fn finite(v: Vector3) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite() && v.magnitude2() > 0.0
}

/// Axis-aligned bounds.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Bounds {
    pub min: DVec3,
    pub max: DVec3,
}

impl Bounds {
    pub fn empty() -> Self {
        Self {
            min: DVec3::splat(f64::INFINITY),
            max: DVec3::splat(f64::NEG_INFINITY),
        }
    }

    pub fn add(&mut self, p: DVec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    pub fn is_empty(&self) -> bool {
        !(self.min.cmple(self.max).all())
    }

    pub fn diagonal(&self) -> f64 {
        if self.is_empty() {
            0.0
        } else {
            (self.max - self.min).length()
        }
    }

    pub fn center(&self) -> DVec3 {
        (self.min + self.max) * 0.5
    }

    pub fn union(&self, other: &Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// True when the boxes are separated by more than `margin` on some axis.
    pub fn disjoint(&self, other: &Self, margin: f64) -> bool {
        (0..3).any(|i| self.max[i] < other.min[i] - margin || other.max[i] < self.min[i] - margin)
    }

    /// True when `other` lies inside this box, shrunk by `margin`.
    pub fn contains_box(&self, other: &Self, margin: f64) -> bool {
        (0..3).all(|i| other.min[i] > self.min[i] + margin && other.max[i] < self.max[i] - margin)
    }
}

/// An oriented triangle mesh with the source B-rep face of each triangle.
#[derive(Clone, Debug, Default)]
pub(crate) struct TriMesh {
    pub positions: Vec<DVec3>,
    pub triangles: Vec<[u32; 3]>,
    pub faces: Vec<u32>,
}

impl TriMesh {
    pub fn bounds(&self) -> Bounds {
        let mut bounds = Bounds::empty();
        for triangle in &self.triangles {
            for &i in triangle {
                bounds.add(self.positions[i as usize]);
            }
        }
        bounds
    }

    pub fn triangle(&self, index: usize) -> [DVec3; 3] {
        self.triangles[index].map(|i| self.positions[i as usize])
    }

    /// Signed volume; positive for outward-facing closed meshes.
    pub fn volume(&self) -> f64 {
        let origin = self.bounds().center();
        self.triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| self.positions[i as usize] - origin);
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    }

    /// Point-in-solid by ray parity, voted over three skewed directions so a
    /// ray grazing an edge cannot flip the answer.
    pub fn contains(&self, point: DVec3) -> bool {
        const DIRECTIONS: [DVec3; 3] = [
            DVec3::new(0.577_350_3, 0.577_350_2, 0.577_350_4),
            DVec3::new(-0.267_261_2, 0.534_522_5, 0.801_783_7),
            DVec3::new(0.801_783_7, -0.267_261_2, 0.534_522_5),
        ];
        let inside = DIRECTIONS
            .iter()
            .filter(|direction| {
                let mut crossings = 0;
                for i in 0..self.triangles.len() {
                    let [a, b, c] = self.triangle(i);
                    if ray_triangle(point, **direction, a, b, c).is_some() {
                        crossings += 1;
                    }
                }
                crossings % 2 == 1
            })
            .count();
        inside >= 2
    }

    /// Whether any edge of `self` crosses a triangle of `other`.
    pub fn edges_cross(&self, other: &TriMesh) -> bool {
        let bounds = other.bounds();
        let pad = bounds.diagonal() * 1.0e-9;
        for t in &self.triangles {
            for k in 0..3 {
                let a = self.positions[t[k] as usize];
                let b = self.positions[t[(k + 1) % 3] as usize];
                let mut segment = Bounds::empty();
                segment.add(a);
                segment.add(b);
                if segment.disjoint(&bounds, pad) {
                    continue;
                }
                let direction = b - a;
                let length = direction.length();
                if length <= 0.0 {
                    continue;
                }
                for i in 0..other.triangles.len() {
                    let [p, q, r] = other.triangle(i);
                    if let Some(t) = ray_triangle(a, direction / length, p, q, r)
                        && t <= length
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Distance from `point` to the triangles of B-rep face `face`, with the
    /// normal of the closest triangle.
    pub fn face_distance(&self, face: u32, point: DVec3) -> Option<(f64, DVec3)> {
        let mut best: Option<(f64, DVec3)> = None;
        for (i, _) in self.faces.iter().enumerate().filter(|(_, f)| **f == face) {
            let [a, b, c] = self.triangle(i);
            let normal = (b - a).cross(c - a);
            if normal.length_squared() <= 0.0 || !normal.is_finite() {
                continue;
            }
            let distance = point.distance(closest_on_triangle(point, a, b, c));
            if best.is_none_or(|(d, _)| distance < d) {
                best = Some((distance, normal.normalize()));
            }
        }
        best
    }
}

/// Möller–Trumbore; returns the ray parameter of a hit in front of `origin`.
pub(crate) fn ray_triangle(
    origin: DVec3,
    direction: DVec3,
    a: DVec3,
    b: DVec3,
    c: DVec3,
) -> Option<f64> {
    let ab = b - a;
    let ac = c - a;
    let p = direction.cross(ac);
    let determinant = ab.dot(p);
    if determinant.abs() <= 1.0e-14 * ab.length() * ac.length() {
        return None;
    }
    let inverse = 1.0 / determinant;
    let t = origin - a;
    let u = t.dot(p) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = t.cross(ab);
    let v = direction.dot(q) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let distance = ac.dot(q) * inverse;
    (distance > 1.0e-12).then_some(distance)
}

pub(crate) fn closest_on_triangle(p: DVec3, a: DVec3, b: DVec3, c: DVec3) -> DVec3 {
    // Ericson, Real-Time Collision Detection, 5.1.5.
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denominator = 1.0 / (va + vb + vc);
    a + ab * (vb * denominator) + ac * (vc * denominator)
}

/// Newell normal of a closed polygon (unnormalized; its length is twice the area).
pub(crate) fn newell(points: &[DVec3]) -> DVec3 {
    let origin = points.first().copied().unwrap_or_default();
    let mut normal = DVec3::ZERO;
    for i in 0..points.len() {
        let p = points[i] - origin;
        let q = points[(i + 1) % points.len()] - origin;
        normal += p.cross(q);
    }
    normal
}

/// Solve min |x - x0|² subject to n_i · x = d_i, dropping dependent rows.
/// Returns `None` for inconsistent constraints.
pub(crate) fn project_onto_planes(x0: DVec3, planes: &[(DVec3, f64)]) -> Option<DVec3> {
    // Gram–Schmidt on the constraint normals keeps the system well conditioned.
    let mut basis: Vec<(DVec3, f64)> = Vec::new();
    let mut x = x0;
    for &(normal, offset) in planes {
        let length = normal.length();
        if length <= 0.0 || !length.is_finite() {
            return None;
        }
        let (mut n, mut d) = (normal / length, offset / length);
        for &(b, e) in &basis {
            let k = n.dot(b);
            n -= b * k;
            d -= e * k;
        }
        let residual = n.length();
        if residual < 1.0e-9 {
            // Dependent: consistent only when the offsets agree as well.
            if d.abs() > 1.0e-7 * (1.0 + offset.abs() / length) {
                return None;
            }
            continue;
        }
        let (n, d) = (n / residual, d / residual);
        x += n * (d - n.dot(x));
        basis.push((n, d));
    }
    x.is_finite().then_some(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plane_projection_finds_plane_intersections() {
        let x = project_onto_planes(
            DVec3::new(0.3, 0.2, 0.1),
            &[(DVec3::X, 1.0), (DVec3::Y, 2.0), (DVec3::Z, 3.0)],
        )
        .unwrap();
        assert!(x.distance(DVec3::new(1.0, 2.0, 3.0)) < 1e-12);
        let x = project_onto_planes(
            DVec3::new(0.0, 0.0, 5.0),
            &[(DVec3::X, 1.0), (DVec3::Y, 2.0)],
        )
        .unwrap();
        assert!(x.distance(DVec3::new(1.0, 2.0, 5.0)) < 1e-12);
        let tilted = DVec3::new(1.0, 1.0, 0.0);
        let x = project_onto_planes(DVec3::ZERO, &[(tilted, 2.0), (DVec3::X * 2.0, 2.0)]).unwrap();
        assert!((x.x - 1.0).abs() < 1e-12 && (x.y - 1.0).abs() < 1e-12);
        assert!(project_onto_planes(DVec3::ZERO, &[(DVec3::X, 1.0), (DVec3::X, 2.0)]).is_none());
    }
}
