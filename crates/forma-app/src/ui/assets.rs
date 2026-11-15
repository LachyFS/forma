//! Embedded icons and fonts. Assets are compiled into the executable so the
//! editor never depends on installed files or the working directory.
//!
//! Icons are Lucide (ISC, `assets/icons/LICENSE-lucide.txt`) plus `forma-*`
//! icons drawn on the same 24-unit grid and stroke. Inter is licensed under
//! the SIL Open Font License 1.1 (`assets/fonts/LICENSE-Inter.txt`).
use gpui::{App, AssetSource, Result, SharedString, Styled, Svg, px, rgb, svg};
use std::borrow::Cow;

macro_rules! icons {
    ($($variant:ident => $file:literal,)*) => {
        /// Every icon the shell can draw. Each variant embeds one SVG.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub(crate) enum Icon { $($variant,)* }

        impl Icon {
            pub(crate) const ALL: &[Icon] = &[$(Icon::$variant,)*];

            pub(crate) fn path(self) -> &'static str {
                match self { $(Icon::$variant => concat!("icons/", $file, ".svg"),)* }
            }

            fn embedded(self) -> &'static [u8] {
                match self {
                    $(Icon::$variant => include_bytes!(
                        concat!("../../../../assets/icons/", $file, ".svg")
                    ),)*
                }
            }

            fn bytes(path: &str) -> Option<&'static [u8]> {
                Icon::ALL
                    .iter()
                    .find(|icon| icon.path() == path)
                    .map(|icon| icon.embedded())
            }
        }
    };
}

icons! {
    // Tools and modes.
    Select => "mouse-pointer-2",
    Move => "move",
    Rotate => "rotate-3d",
    Scale => "scaling",
    ObjectMode => "box",
    EditMode => "vector-square",
    DrawMode => "pencil-ruler",
    Vertex => "forma-vertex",
    Edge => "forma-edge",
    Face => "forma-face",
    // Geometry and object kinds.
    Cube => "box",
    Sphere => "forma-sphere",
    Cylinder => "cylinder",
    Torus => "torus",
    Plane => "forma-plane",
    Mesh => "forma-mesh",
    Part => "cuboid",
    Sketch => "spline",
    Cone => "forma-cone",
    // CAD features.
    Revolve => "forma-revolve",
    Join => "forma-join",
    Subtract => "forma-subtract",
    Intersect => "forma-intersect",
    PushPull => "forma-push-pull",
    ExtrudeCut => "forma-extrude-cut",
    // Draw tools.
    DrawLine => "pen-line",
    DrawRectangle => "square",
    DrawCenterRectangle => "square-dot",
    DrawCircle => "circle",
    DrawArc => "spline",
    DrawPolygon => "hexagon",
    DrawTrim => "scissors",
    DrawConstruction => "square-dashed",
    Minus => "minus",
    Light => "lightbulb",
    Camera => "camera",
    Empty => "forma-empty",
    Collection => "folder",
    CollectionOpen => "folder-open",
    Scene => "layers",
    // CAD tools.
    Symmetric => "unfold-vertical",
    FlipDirection => "flip-horizontal-2",
    NewPart => "square-plus",
    // Mesh operations.
    Extrude => "arrow-up-from-line",
    Inset => "square-square",
    Bevel => "forma-bevel",
    Subdivide => "grid-2x2",
    Triangulate => "triangle",
    FlipNormals => "arrow-down-up",
    NormalsOutside => "arrow-up-from-dot",
    ApplyRotation => "rotate-3d",
    Origin => "locate-fixed",
    Duplicate => "copy",
    Delete => "trash-2",
    // Viewport.
    ShadingWire => "forma-shading-wire",
    ShadingSolid => "forma-shading-solid",
    ShadingMaterial => "forma-shading-material",
    ShadingRendered => "forma-shading-rendered",
    Perspective => "forma-perspective",
    Orthographic => "forma-orthographic",
    Views => "axis-3d",
    FrameSelected => "focus",
    FrameAll => "maximize",
    Grid => "grid-3x3",
    Measure => "ruler",
    Lighting => "sun",
    History => "history",
    // Files and workspace.
    Open => "folder-open",
    Save => "save",
    Export => "share",
    ExportImage => "image-down",
    Import => "import",
    Image => "image",
    Search => "search",
    Command => "command",
    Theme => "palette",
    Keyboard => "keyboard",
    Help => "circle-help",
    Undo => "undo-2",
    Redo => "redo-2",
    Reset => "rotate-ccw",
    Denoise => "sparkles",
    Code => "braces",
    Material => "blend",
    Settings => "sliders-horizontal",
    Update => "circle-arrow-up",
    Device => "gpu",
    Timer => "timer",
    Rename => "pencil",
    // Generic.
    Eye => "eye",
    EyeOff => "eye-off",
    ChevronDown => "chevron-down",
    ChevronRight => "chevron-right",
    ChevronUp => "chevron-up",
    Plus => "plus",
    Close => "x",
    Check => "check",
    More => "ellipsis",
    Info => "info",
    Error => "circle-alert",
    Warning => "triangle-alert",
    Success => "circle-check",
    Logo => "forma-mark",
}

/// Draw `icon` at `size` pixels in `color`. SVGs are rasterized once per size
/// by GPUI's sprite atlas, so icons are cheap to repeat.
pub(crate) fn icon(icon: Icon, size: f32, color: u32) -> Svg {
    svg()
        .path(icon.path())
        .size(px(size))
        .flex_shrink_0()
        .text_color(rgb(color))
}

pub(crate) struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(Icon::bytes(path).map(Cow::Borrowed))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths: Vec<&str> = Icon::ALL
            .iter()
            .map(|icon| icon.path())
            .filter(|candidate| candidate.starts_with(path))
            .collect();
        paths.sort_unstable();
        paths.dedup();
        Ok(paths.into_iter().map(SharedString::from).collect())
    }
}

const INTER: [&[u8]; 3] = [
    include_bytes!("../../../../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../../../../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../../../../assets/fonts/Inter-SemiBold.ttf"),
];

/// Register Inter before the first window opens. If a platform rejects the
/// fonts, GPUI falls back to the system UI face and the editor still works.
pub(crate) fn register_fonts(cx: &mut App) {
    let fonts = INTER.iter().map(|font| Cow::Borrowed(*font)).collect();
    if let Err(error) = cx.text_system().add_fonts(fonts) {
        eprintln!("Inter could not be registered; using the system font: {error:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_embedded_and_listed() {
        for icon in Icon::ALL {
            let bytes = Assets.load(icon.path()).unwrap().unwrap();
            let text = std::str::from_utf8(&bytes).unwrap();
            assert!(text.contains("viewBox=\"0 0 24 24\""), "{icon:?}");
        }
        assert!(Assets.load("icons/missing.svg").unwrap().is_none());
        let listed = Assets.list("icons/").unwrap();
        assert!(
            Icon::ALL
                .iter()
                .all(|icon| listed.iter().any(|path| path.as_ref() == icon.path())),
            "list must report every embedded icon"
        );
    }
}
