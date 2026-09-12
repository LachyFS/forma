use crate::scene::{MAX_FILE_BYTES, atomic_write, read_limited};
use crate::{Mesh, Scene};
use anyhow::{Context, Result, bail, ensure};
use glam::Vec3;
use std::{fmt::Write as _, path::Path};

pub(crate) fn read(path: &Path) -> Result<Mesh> {
    let bytes = read_limited(path)?;
    let text = std::str::from_utf8(&bytes).context("OBJ must be UTF-8 text")?;
    let mut mesh = Mesh {
        positions: Vec::new(),
        faces: Vec::new(),
    };
    let mut texture_count = 0_usize;
    let mut normal_count = 0_usize;
    let mut continuation = String::new();
    for (line_index, raw) in text.lines().enumerate() {
        let line_number = line_index + 1;
        let raw = raw.split('#').next().unwrap_or("").trim();
        if let Some(prefix) = raw.strip_suffix('\\') {
            continuation.push_str(prefix);
            continuation.push(' ');
            ensure!(
                continuation.len() <= 1_000_000,
                "OBJ line {line_number} is too long"
            );
            continue;
        }
        continuation.push_str(raw);
        let line = continuation.trim();
        let fields: Vec<_> = line.split_whitespace().collect();
        let result = parse_line(&fields, &mut mesh, &mut texture_count, &mut normal_count);
        result.with_context(|| format!("OBJ line {line_number}"))?;
        continuation.clear();
        ensure!(
            mesh.positions.len() <= 2_000_000 && mesh.faces.len() <= 2_000_000,
            "OBJ exceeds the two million vertex/face limit"
        );
    }
    ensure!(
        continuation.is_empty(),
        "OBJ ends with an unfinished continuation"
    );
    mesh.validate().context("Invalid OBJ geometry")?;
    Ok(mesh)
}

fn parse_line(
    fields: &[&str],
    mesh: &mut Mesh,
    textures: &mut usize,
    normals: &mut usize,
) -> Result<()> {
    let Some(kind) = fields.first().copied() else {
        return Ok(());
    };
    match kind {
        "v" => {
            ensure!(
                [4, 5, 7, 8].contains(&fields.len()),
                "Vertex needs x y z, optional w or RGB"
            );
            let mut point = Vec3::new(number(fields[1])?, number(fields[2])?, number(fields[3])?);
            if fields.len() == 5 {
                let w = number(fields[4])?;
                ensure!(w.abs() > 1.0e-12, "Homogeneous vertex weight is zero");
                point /= w;
            }
            for value in &fields[4..] {
                number(value)?;
            }
            ensure!(point.is_finite(), "Vertex is not finite");
            mesh.positions.push(point);
        }
        "vt" => {
            ensure!(
                (2..=4).contains(&fields.len()),
                "Texture coordinate needs 1–3 components"
            );
            for value in &fields[1..] {
                number(value)?;
            }
            *textures += 1;
        }
        "vn" => {
            ensure!(fields.len() == 4, "Normal needs three components");
            for value in &fields[1..] {
                number(value)?;
            }
            *normals += 1;
        }
        "f" => {
            ensure!(
                (4..=4097).contains(&fields.len()),
                "Face needs 3–4096 vertices"
            );
            let mut face = Vec::with_capacity(fields.len() - 1);
            for vertex in &fields[1..] {
                let parts: Vec<_> = vertex.split('/').collect();
                ensure!(
                    (1..=3).contains(&parts.len()) && !parts[0].is_empty(),
                    "Malformed face vertex {vertex}"
                );
                face.push(index(parts[0], mesh.positions.len(), "vertex")?);
                if parts.len() >= 2 && !parts[1].is_empty() {
                    index(parts[1], *textures, "texture")?;
                }
                if parts.len() == 3 && !parts[2].is_empty() {
                    index(parts[2], *normals, "normal")?;
                }
                ensure!(
                    parts.len() == 1 || parts.last().is_some_and(|part| !part.is_empty()),
                    "Incomplete face vertex {vertex}"
                );
            }
            mesh.faces.push(face);
        }
        // Polygon groups and shading declarations do not change connectivity.
        // OBJ normals are intentionally recomputed from editable geometry.
        "o" | "g" | "s" | "mtllib" | "usemtl" | "vp" | "l" | "p" => {}
        _ => bail!("Unsupported OBJ directive {kind}; polygon meshes are supported"),
    }
    Ok(())
}

fn number(text: &str) -> Result<f32> {
    let value: f32 = text
        .parse()
        .with_context(|| format!("Invalid number {text}"))?;
    ensure!(value.is_finite(), "Non-finite number {text}");
    Ok(value)
}

fn index(text: &str, count: usize, kind: &str) -> Result<u32> {
    let parsed: i64 = text
        .parse()
        .with_context(|| format!("Invalid {kind} index {text}"))?;
    ensure!(parsed != 0, "OBJ {kind} indices cannot be zero");
    let resolved = if parsed > 0 {
        parsed - 1
    } else {
        count as i64 + parsed
    };
    ensure!(
        resolved >= 0 && resolved < count as i64,
        "{kind} index {text} is out of range"
    );
    Ok(resolved as u32)
}

pub(crate) fn write(scene: &Scene, path: &Path) -> Result<()> {
    scene.validate().context("Cannot export invalid scene")?;
    let mut output = String::from("# Forma — world-space polygon geometry, right handed +Y up\n");
    let mut offset = 1_u32;
    for instance in scene.mesh_instances() {
        let name: String = instance
            .name
            .chars()
            .map(|ch| {
                if ch.is_whitespace() || ch == '#' {
                    '_'
                } else {
                    ch
                }
            })
            .collect();
        writeln!(output, "\no {name}")?;
        let matrix = instance.world_transform;
        for &point in &instance.mesh.positions {
            let p = matrix.transform_point3(point);
            writeln!(output, "v {} {} {}", p.x, p.y, p.z)?;
        }
        let mirrored = matrix.determinant() < 0.0;
        for face in &instance.mesh.faces {
            output.push('f');
            // Baking a reflection reverses the geometric winding. Preserve
            // outward faces in the exported mesh by reversing polygon order.
            if mirrored {
                for &index in face.iter().rev() {
                    write!(output, " {}", offset + index)?;
                }
            } else {
                for &index in face {
                    write!(output, " {}", offset + index)?;
                }
            }
            output.push('\n');
        }
        offset += instance.mesh.positions.len() as u32;
        ensure!(
            output.len() as u64 <= MAX_FILE_BYTES,
            "Export exceeds the 256 MiB limit"
        );
    }
    atomic_write(path, output.as_bytes())
        .with_context(|| format!("Could not export {}", path.display()))
}
