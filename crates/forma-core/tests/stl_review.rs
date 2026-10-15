//! Independent adversarial exchange checks assembled during implementation review.
use forma_core::Scene;

fn ascii_triangle() -> &'static str {
    "solid review\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid review\n"
}

#[test]
fn ascii_stl_accepts_lf_crlf_and_legacy_cr_line_endings() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("line-endings.stl");
    for newline in ["\n", "\r\n", "\r"] {
        std::fs::write(&path, ascii_triangle().replace('\n', newline)).unwrap();
        let mut scene = Scene::empty();
        let id = scene
            .import_stl(&path, 1.)
            .unwrap_or_else(|error| panic!("Line ending {newline:?}: {error:#}"));
        assert_eq!(scene.object_mesh(id).unwrap().faces.len(), 1);
    }
}
