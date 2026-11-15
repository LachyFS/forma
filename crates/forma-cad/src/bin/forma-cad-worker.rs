//! Standalone CAD worker, speaking the same protocol as `forma --cad-worker`.
//! Used by the crash-isolation tests and for kernel debugging.
fn main() {
    forma_cad::worker::serve_stdio()
}
