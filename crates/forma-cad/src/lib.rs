//! Forma's CAD kernel layer on the [truck] B-rep kernel.
//!
//! The crate turns pure [`forma_core::PartData`] feature histories into
//! solids, tessellates them into ordinary Forma meshes with per-face groups,
//! converts parts to and from editable meshes, and reads and writes STEP.
//! Kernel calls can panic, abort or hang on hard inputs, so applications run
//! them through [`host::CadHost`], which drives an isolated worker process
//! (`forma --cad-worker`) with timeouts and automatic restarts.
//!
//! [truck]: https://github.com/ricosjp/truck
#![forbid(unsafe_code)]

mod body;
mod boolean;
mod geom;

/// A user-facing kernel failure: what went wrong and what to try instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelError {
    pub message: String,
    pub suggestion: String,
}

impl KernelError {
    pub fn new(message: impl Into<String>, suggestion: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            suggestion: suggestion.into(),
        }
    }
}

impl std::fmt::Display for KernelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.suggestion.is_empty() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{} · {}", self.message, self.suggestion)
        }
    }
}

impl std::error::Error for KernelError {}
