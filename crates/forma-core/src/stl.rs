//! Bounded STL exchange. Facet winding is authoritative; stored facet normals
//! and vendor-specific binary attributes never override editable geometry.
use crate::mesh::Triangulation;
use crate::scene::{MAX_FILE_BYTES, atomic_write, read_limited};
use crate::{Mesh, Scene, Vec3};
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
};

const HEADER_BYTES: usize = 84;
const FACET_BYTES: usize = 50;
const GEOMETRY_LIMIT: usize = 2_000_000;

fn check_units(metres_per_unit: f32) -> Result<()> {
    ensure!(
        metres_per_unit.is_finite() && metres_per_unit > 0.,
        "STL unit scale must be finite and positive"
    );
    Ok(())
}

pub(crate) fn read(path: &Path, metres_per_unit: f32) -> Result<Mesh> {
    check_units(metres_per_unit)?;
    let bytes = read_limited(path)?;
    let mut builder = MeshBuilder::new(metres_per_unit);
    let count = bytes
        .get(80..84)
        .map(|field| u32::from_le_bytes(field.try_into().unwrap()) as usize);
    // A binary header may begin with "solid". Its declared record count and
    // exact byte length take precedence over any textual signature.
    if count
        .is_some_and(|n| HEADER_BYTES as u64 + n as u64 * FACET_BYTES as u64 == bytes.len() as u64)
    {
        read_binary(&bytes, count.unwrap(), &mut builder)?;
    } else if let Ok(text) = std::str::from_utf8(&bytes) {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        if text
            .split_ascii_whitespace()
            .next()
            .is_some_and(|word| word.eq_ignore_ascii_case("solid"))
        {
            read_ascii(text, &mut builder)?;
        } else {
            binary_length_error(count)?;
        }
    } else {
        binary_length_error(count)?;
    }
    builder.mesh.validate().context("Invalid STL geometry")?;
    Ok(builder.mesh)
}

fn binary_length_error(count: Option<usize>) -> Result<()> {
    if let Some(count) = count {
        ensure!(
            count <= GEOMETRY_LIMIT,
            "STL exceeds the two million triangle limit"
        );
        bail!(
            "STL binary length does not match its triangle count (truncated file or trailing data)"
        );
    }
    bail!("STL is not ASCII geometry or a complete 84-byte binary header")
}

fn read_binary(bytes: &[u8], count: usize, builder: &mut MeshBuilder) -> Result<()> {
    ensure!(
        count <= GEOMETRY_LIMIT,
        "STL exceeds the two million triangle limit"
    );
    for (index, facet) in bytes[HEADER_BYTES..].chunks_exact(FACET_BYTES).enumerate() {
        let values: [f32; 12] = std::array::from_fn(|i| {
            f32::from_le_bytes(facet[i * 4..i * 4 + 4].try_into().unwrap())
        });
        ensure!(
            values.iter().all(|value| value.is_finite()),
            "STL triangle {} has a non-finite coordinate or normal",
            index + 1
        );
        let points = std::array::from_fn(|i| {
            Vec3::new(values[3 + i * 3], values[4 + i * 3], values[5 + i * 3])
        });
        builder
            .push(points)
            .with_context(|| format!("STL triangle {}", index + 1))?;
        // The final two bytes are opaque attributes (often vendor colors),
        // not a variable-length payload. Export writes the standard zero value.
    }
    Ok(())
}

fn read_ascii(text: &str, builder: &mut MeshBuilder) -> Result<()> {
    ensure!(
        !text
            .chars()
            .any(|c| c.is_control() && !c.is_ascii_whitespace()),
        "ASCII STL contains invalid control characters"
    );
    let mut tokens = AsciiTokens {
        remaining: text,
        line: 1,
    };
    while let Some(kind) = tokens.next() {
        ensure!(kind.len() <= 128, "ASCII STL keyword is too long");
        ensure!(
            kind.eq_ignore_ascii_case("solid"),
            "ASCII STL line {}: expected solid, found {kind}",
            tokens.line
        );
        tokens.skip_name();
        loop {
            let kind = tokens.next().context("ASCII STL is missing endsolid")?;
            ensure!(kind.len() <= 128, "ASCII STL keyword is too long");
            if kind.eq_ignore_ascii_case("endsolid") {
                tokens.skip_name();
                break;
            }
            ensure!(
                kind.eq_ignore_ascii_case("facet"),
                "ASCII STL line {}: expected facet or endsolid, found {kind}",
                tokens.line
            );
            let line = tokens.line;
            tokens.expect("normal")?;
            tokens.vector()?;
            tokens.expect("outer")?;
            tokens.expect("loop")?;
            let mut points = [Vec3::ZERO; 3];
            for point in &mut points {
                tokens.expect("vertex")?;
                *point = tokens.vector()?;
            }
            tokens.expect("endloop")?;
            tokens.expect("endfacet")?;
            builder
                .push(points)
                .with_context(|| format!("ASCII STL facet at line {line}"))?;
        }
    }
    Ok(())
}

struct MeshBuilder {
    mesh: Mesh,
    vertices: HashMap<[u32; 3], u32>,
    metres_per_unit: f64,
}

impl MeshBuilder {
    fn new(metres_per_unit: f32) -> Self {
        Self {
            mesh: Mesh {
                positions: Vec::new(),
                faces: Vec::new(),
            },
            vertices: HashMap::new(),
            metres_per_unit: f64::from(metres_per_unit),
        }
    }

    fn push(&mut self, points: [Vec3; 3]) -> Result<()> {
        ensure!(
            self.mesh.faces.len() < GEOMETRY_LIMIT,
            "STL exceeds the two million triangle limit"
        );
        let points = points.map(|p| {
            let p = (p.as_dvec3() * self.metres_per_unit).as_vec3();
            // Canonicalize signed zero so adjoining facets share exact vertices.
            Vec3::from_array(p.to_array().map(|v| if v == 0. { 0. } else { v }))
        });
        ensure!(
            points
                .iter()
                .all(|p| p.is_finite() && p.abs().max_element() <= 1.0e8),
            "STL coordinates exceed the mesh range after unit conversion"
        );
        let [a, b, c] = points;
        ensure!(
            (b - a).cross(c - a).length_squared() > 1.0e-20,
            "STL triangle is degenerate or too small after unit conversion"
        );
        let mut face = Vec::with_capacity(3);
        for point in points {
            let key = point.to_array().map(f32::to_bits);
            let vertex = if let Some(&index) = self.vertices.get(&key) {
                index
            } else {
                ensure!(
                    self.mesh.positions.len() < GEOMETRY_LIMIT,
                    "STL exceeds the two million vertex limit"
                );
                let index = self.mesh.positions.len() as u32;
                self.mesh.positions.push(point);
                self.vertices.insert(key, index);
                index
            };
            face.push(vertex);
        }
        self.mesh.faces.push(face);
        Ok(())
    }
}

struct AsciiTokens<'a> {
    remaining: &'a str,
    line: usize,
}

impl<'a> AsciiTokens<'a> {
    fn next(&mut self) -> Option<&'a str> {
        let trimmed = self
            .remaining
            .trim_start_matches(|c: char| c.is_ascii_whitespace());
        let whitespace = &self.remaining.as_bytes()[..self.remaining.len() - trimmed.len()];
        self.line += whitespace
            .iter()
            .enumerate()
            .filter(|&(i, &b)| b == b'\r' || (b == b'\n' && (i == 0 || whitespace[i - 1] != b'\r')))
            .count();
        if trimmed.is_empty() {
            self.remaining = trimmed;
            return None;
        }
        let end = trimmed
            .find(|c: char| c.is_ascii_whitespace())
            .unwrap_or(trimmed.len());
        self.remaining = &trimmed[end..];
        Some(&trimmed[..end])
    }

    fn expect(&mut self, expected: &str) -> Result<()> {
        let found = self
            .next()
            .context("ASCII STL ends before its geometry is complete")?;
        ensure!(found.len() <= 128, "ASCII STL keyword is too long");
        ensure!(
            found.eq_ignore_ascii_case(expected),
            "ASCII STL line {}: expected {expected}, found {found}",
            self.line
        );
        Ok(())
    }

    fn vector(&mut self) -> Result<Vec3> {
        let mut values = [0.; 3];
        for value in &mut values {
            let token = self
                .next()
                .context("ASCII STL ends in an incomplete vector")?;
            ensure!(token.len() <= 128, "ASCII STL numeric token is too long");
            *value = token
                .parse::<f32>()
                .with_context(|| format!("ASCII STL line {}: invalid number {token}", self.line))?;
            ensure!(
                value.is_finite(),
                "ASCII STL line {}: non-finite number",
                self.line
            );
        }
        Ok(Vec3::from_array(values))
    }

    fn skip_name(&mut self) {
        if let Some(end) = self.remaining.find(['\r', '\n']) {
            let newline_bytes = if self.remaining[end..].starts_with("\r\n") {
                2
            } else {
                1
            };
            self.remaining = &self.remaining[end + newline_bytes..];
            self.line += 1;
        } else {
            self.remaining = "";
        }
    }
}

pub(crate) fn write(
    scene: &Scene,
    path: &Path,
    metres_per_unit: f32,
    selection: Option<&BTreeSet<u64>>,
) -> Result<()> {
    check_units(metres_per_unit)?;
    scene.validate().context("Cannot export invalid scene")?;
    if let Some(selected) = selection {
        ensure!(
            selected.iter().all(|&id| scene.object(id).is_some()),
            "STL export selection contains a missing object"
        );
    }
    let instances: Vec<_> = scene
        .mesh_instances()
        .filter(|instance| {
            scene.is_effectively_visible(instance.id)
                && selection.is_none_or(|selected| selected.contains(&instance.id))
        })
        .collect();
    let count = instances
        .iter()
        .flat_map(|instance| &instance.mesh.faces)
        .try_fold(0usize, |count, face| -> Result<usize> {
            let count = count + face.len() - 2;
            ensure!(
                count <= GEOMETRY_LIMIT,
                "STL export exceeds the two million triangle limit"
            );
            Ok(count)
        })?;
    ensure!(count > 0, "No visible mesh geometry to export");
    let bytes = HEADER_BYTES + count * FACET_BYTES;
    ensure!(
        bytes as u64 <= MAX_FILE_BYTES,
        "STL export exceeds the 256 MiB limit"
    );
    let mut output = vec![0; HEADER_BYTES];
    output.reserve(bytes - HEADER_BYTES);
    let header = b"Forma binary STL";
    output[..header.len()].copy_from_slice(header);
    output[80..84].copy_from_slice(&(count as u32).to_le_bytes());
    let mut scratch = Triangulation::default();
    for instance in instances {
        let matrix = instance.world_transform.as_dmat4();
        let determinant = matrix.determinant();
        ensure!(
            matrix.is_finite() && determinant.is_finite() && determinant != 0.,
            "Object {} has an invalid world transform",
            instance.name
        );
        for face in &instance.mesh.faces {
            for triangle in instance.mesh.triangulate_face(face, &mut scratch) {
                let mut points = triangle.map(|i| {
                    (matrix.transform_point3(instance.mesh.positions[i as usize].as_dvec3())
                        / f64::from(metres_per_unit))
                    .as_vec3()
                });
                ensure!(
                    points.iter().all(|p| p.is_finite()),
                    "Object {} exceeds STL coordinate precision in the selected units",
                    instance.name
                );
                if determinant < 0. {
                    points.swap(1, 2);
                }
                let [a, b, c] = points.map(|p| p.as_dvec3());
                let normal = (b - a).cross(c - a);
                ensure!(
                    normal.is_finite() && normal.length_squared() > 0.,
                    "Object {} has a triangle that collapses at STL coordinate precision",
                    instance.name
                );
                for point in std::iter::once(normal.normalize().as_vec3()).chain(points) {
                    for value in point.to_array() {
                        output.extend_from_slice(&value.to_le_bytes());
                    }
                }
                output.extend_from_slice(&0u16.to_le_bytes());
            }
        }
    }
    ensure!(
        output.len() == bytes,
        "STL export could not triangulate all polygons"
    );
    atomic_write(path, &output).with_context(|| format!("Could not export {}", path.display()))
}
