//! Messages between the application and the CAD worker process, and the
//! length-prefixed framing that carries them over pipes.
//!
//! Frames are `MAGIC (8 bytes) | length (u32 LE) | JSON`. The reader skips
//! anything before the magic, so stray text a dependency prints to stdout
//! (truck-stepio does on malformed files) cannot corrupt the stream.
use forma_core::{FeatureIssue, Mesh, PartData};
use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

/// Largest frame accepted in either direction.
pub const MAX_FRAME_BYTES: usize = 512 * 1024 * 1024;
const MAGIC: [u8; 8] = [0x00, b'F', b'C', b'A', b'D', 0x01, 0xFE, 0xFF];

/// Regenerate one part: evaluate its active features and tessellate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegenRequest {
    /// Part data block id; results are matched by id and `part.stamp`.
    pub part_id: u64,
    pub part: PartData,
    /// Feature ids to treat as failed without evaluating them. The host adds
    /// a feature here after it crashed or hung the worker, so the rest of the
    /// history still regenerates.
    #[serde(default)]
    pub skip: Vec<FeatureIssue>,
}

/// Surface class of a displayed face.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceKind {
    Planar,
    Curved,
}

/// One face group of a part tessellation. Mesh polygon `i` belongs to face
/// group `mesh.groups[i]`; smooth (tangent-continuous) neighbouring B-rep
/// faces share a group, so a cylinder split into several B-rep faces still
/// shows and picks as one face.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FaceInfo {
    pub kind: SurfaceKind,
    /// Indices of the B-rep faces in this group, in kernel face order.
    pub brep_faces: Vec<u32>,
    /// A point on the face, in part space.
    pub point: Vec3,
    /// The plane normal for planar faces; the normal at `point` otherwise.
    pub normal: Vec3,
    /// Surface area in square metres.
    pub area: f32,
}

/// Timings and sizes of one regeneration.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RegenStats {
    pub total_ms: f64,
    pub evaluate_ms: f64,
    pub tessellate_ms: f64,
    /// Leading features reused from the worker's cache.
    pub reused_features: usize,
    /// Features evaluated in this request.
    pub evaluated_features: usize,
    pub brep_faces: usize,
    pub triangles: usize,
}

/// The regenerated tessellation of one part.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegenResult {
    pub part_id: u64,
    /// The part stamp that was evaluated; drop results whose stamp no longer
    /// matches the part.
    pub stamp: u64,
    /// `None` when the active history produces no body (for example when
    /// every feature is suppressed or failed).
    pub mesh: Option<Mesh>,
    /// Face groups of `mesh`, indexed by group id.
    #[serde(default)]
    pub faces: Vec<FaceInfo>,
    #[serde(default)]
    pub issues: Vec<FeatureIssue>,
    #[serde(default)]
    pub stats: RegenStats,
}

/// A part placed for export.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepPart {
    pub name: String,
    pub part: PartData,
    /// Part space to world space (the object's world transform).
    pub transform: Mat4,
}

/// Requests understood by the worker.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "snake_case")]
pub enum Request {
    Regenerate {
        id: u64,
        regen: RegenRequest,
    },
    /// Bake a part into an editable polygon mesh in part space.
    PartToMesh {
        id: u64,
        part: PartData,
    },
    ExportStep {
        id: u64,
        parts: Vec<StepPart>,
    },
    /// Check a STEP document and return it as a part history.
    ImportStep {
        id: u64,
        name: String,
        text: String,
    },
    /// Test-only failure injection; ignored unless the worker was started
    /// with `FORMA_CAD_WORKER_TEST_HOOKS=1`.
    Debug {
        id: u64,
        action: DebugAction,
    },
    Shutdown,
}

impl Request {
    pub fn id(&self) -> u64 {
        match self {
            Self::Regenerate { id, .. }
            | Self::PartToMesh { id, .. }
            | Self::ExportStep { id, .. }
            | Self::ImportStep { id, .. }
            | Self::Debug { id, .. } => *id,
            Self::Shutdown => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DebugAction {
    /// Report progress for `feature`, then loop forever.
    Hang { feature: u64 },
    /// Report progress for `feature`, then abort the process.
    Abort { feature: u64 },
    /// Panic inside the request handler.
    Panic,
    /// Write a frame the host cannot decode, then stay alive.
    BadFrame,
    /// Answer immediately.
    Echo,
}

/// Messages from the worker.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "response", rename_all = "snake_case")]
pub enum Response {
    /// The worker is about to evaluate `feature` for request `id`.
    Progress {
        id: u64,
        feature: u64,
    },
    Regenerated {
        id: u64,
        result: RegenResult,
    },
    Mesh {
        id: u64,
        result: Result<Mesh, String>,
    },
    Step {
        id: u64,
        result: Result<String, String>,
    },
    Imported {
        id: u64,
        result: Result<PartData, String>,
    },
    Echo {
        id: u64,
    },
    Error {
        id: u64,
        message: String,
    },
}

impl Response {
    /// Replace values JSON cannot carry. serde_json writes NaN and
    /// infinities as `null`, which the reader then rejects, so a single
    /// degenerate normal would otherwise lose the whole reply.
    pub fn sanitize(&mut self) {
        let clean = |v: &mut f32| {
            if !v.is_finite() {
                *v = 0.0;
            }
        };
        let clean64 = |v: &mut f64| {
            if !v.is_finite() {
                *v = 0.0;
            }
        };
        match self {
            Self::Regenerated { result, .. } => {
                for face in &mut result.faces {
                    for v in [&mut face.point, &mut face.normal] {
                        for c in [&mut v.x, &mut v.y, &mut v.z] {
                            clean(c);
                        }
                    }
                    clean(&mut face.area);
                }
                let stats = &mut result.stats;
                for v in [
                    &mut stats.total_ms,
                    &mut stats.evaluate_ms,
                    &mut stats.tessellate_ms,
                ] {
                    clean64(v);
                }
                let broken = |mesh: &Mesh| mesh.positions.iter().any(|p| !p.is_finite());
                if result.mesh.as_ref().is_some_and(broken) {
                    result.mesh = None;
                    result.faces.clear();
                    result.issues.push(FeatureIssue {
                        feature: 0,
                        message: "The tessellation contained invalid coordinates".into(),
                        suggestion: "Try another tessellation quality".into(),
                        warning: false,
                    });
                }
            }
            Self::Mesh {
                result: Ok(mesh), ..
            } if mesh.positions.iter().any(|p| !p.is_finite()) => {
                *self = Self::Error {
                    id: self.id(),
                    message: "The baked mesh contained invalid coordinates".into(),
                };
            }
            _ => {}
        }
    }

    pub fn id(&self) -> u64 {
        match self {
            Self::Progress { id, .. }
            | Self::Regenerated { id, .. }
            | Self::Mesh { id, .. }
            | Self::Step { id, .. }
            | Self::Imported { id, .. }
            | Self::Echo { id }
            | Self::Error { id, .. } => *id,
        }
    }

    pub fn is_final(&self) -> bool {
        !matches!(self, Self::Progress { .. })
    }
}

/// Write one frame and flush it.
pub fn write_frame<T: Serialize>(writer: &mut impl Write, message: &T) -> io::Result<()> {
    let json = serde_json::to_vec(message).map_err(io::Error::other)?;
    if json.len() > MAX_FRAME_BYTES {
        return Err(io::Error::other("CAD message exceeds the frame limit"));
    }
    let mut frame = Vec::with_capacity(json.len() + 12);
    frame.extend_from_slice(&MAGIC);
    frame.extend_from_slice(&(json.len() as u32).to_le_bytes());
    frame.extend_from_slice(&json);
    writer.write_all(&frame)?;
    writer.flush()
}

/// Read the next frame. Returns `Ok(None)` at end of stream.
pub fn read_frame<T: for<'de> Deserialize<'de>>(reader: &mut impl Read) -> io::Result<Option<T>> {
    // Find the magic, skipping stray bytes.
    let mut matched = 0;
    let mut byte = [0u8; 1];
    while matched < MAGIC.len() {
        if reader.read(&mut byte)? == 0 {
            return Ok(None);
        }
        if byte[0] == MAGIC[matched] {
            matched += 1;
        } else {
            matched = usize::from(byte[0] == MAGIC[0]);
        }
    }
    let mut length = [0u8; 4];
    reader.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(io::Error::other("CAD message exceeds the frame limit"));
    }
    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload)?;
    serde_json::from_slice(&payload)
        .map(Some)
        .map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_finite_values_are_sanitized_before_encoding() {
        // Review case: a NaN normal serialized as null and the host could
        // not decode the reply.
        let mut response = Response::Regenerated {
            id: 1,
            result: RegenResult {
                part_id: 1,
                stamp: 1,
                mesh: None,
                faces: vec![FaceInfo {
                    kind: SurfaceKind::Planar,
                    brep_faces: vec![0],
                    point: Vec3::ZERO,
                    normal: Vec3::new(f32::NAN, 0.0, 0.0),
                    area: f32::INFINITY,
                }],
                issues: Vec::new(),
                stats: RegenStats::default(),
            },
        };
        let mut raw = Vec::new();
        write_frame(&mut raw, &response).unwrap();
        assert!(read_frame::<Response>(&mut raw.as_slice()).is_err());
        response.sanitize();
        let mut buffer = Vec::new();
        write_frame(&mut buffer, &response).unwrap();
        let decoded: Response = read_frame(&mut buffer.as_slice()).unwrap().unwrap();
        assert_eq!(decoded.id(), 1);
    }

    #[test]
    fn frames_survive_stray_output_between_them() {
        let mut buffer = Vec::new();
        buffer.extend_from_slice(b"Lookup failed for #195\n");
        write_frame(&mut buffer, &Response::Echo { id: 7 }).unwrap();
        buffer.extend_from_slice(b"\x00FCA noise");
        write_frame(&mut buffer, &Response::Progress { id: 8, feature: 3 }).unwrap();
        let mut reader = buffer.as_slice();
        let first: Response = read_frame(&mut reader).unwrap().unwrap();
        assert_eq!(first.id(), 7);
        let second: Response = read_frame(&mut reader).unwrap().unwrap();
        assert!(matches!(second, Response::Progress { id: 8, feature: 3 }));
        assert!(read_frame::<Response>(&mut reader).unwrap().is_none());
    }
}
