//! Presentation metadata for editor commands: the label and shortcut shown in
//! tooltips, menus and the command palette, plus an icon and palette group.
//! Shortcuts use the keycap format understood by `components::keycaps`.
use super::assets::Icon;
use crate::app::{Command, EditMode, Tool};
use crate::platform_shortcut;
use crate::sketch::{DrawCommand, DrawTool};
use forma_core::Primitive;
use forma_render::RenderMode;

#[derive(Clone, Copy)]
pub(crate) struct CommandInfo {
    pub label: &'static str,
    pub shortcut: Option<&'static str>,
    pub detail: Option<&'static str>,
}

const fn info(label: &'static str) -> CommandInfo {
    CommandInfo {
        label,
        shortcut: None,
        detail: None,
    }
}
const fn keyed(label: &'static str, shortcut: &'static str) -> CommandInfo {
    CommandInfo {
        label,
        shortcut: Some(shortcut),
        detail: None,
    }
}
const fn explained(label: &'static str, detail: &'static str) -> CommandInfo {
    CommandInfo {
        label,
        shortcut: None,
        detail: Some(detail),
    }
}

pub(crate) fn command_info(command: Command) -> CommandInfo {
    match command {
        Command::CheckForUpdates => info("Check for updates"),
        Command::ToggleUpdates => info("Updates and download preferences"),
        Command::DownloadUpdate => info("Download the available update"),
        Command::RestartUpdate => info("Save your work and restart to update"),
        Command::SkipUpdate => info("Skip this version until a newer release"),
        Command::ToggleUpdateChecks => info("Check for updates automatically"),
        Command::ToggleUpdateDownloads => info("Download updates automatically"),
        Command::New => keyed("New project", platform_shortcut("⌘ N", "Ctrl+N")),
        Command::Open => keyed("Open project…", platform_shortcut("⌘ O", "Ctrl+O")),
        Command::Save => keyed("Save project", platform_shortcut("⌘ S", "Ctrl+S")),
        Command::SaveAs => keyed(
            "Save project as…",
            platform_shortcut("⇧ ⌘ S", "Ctrl+Shift+S"),
        ),
        Command::ImportObj => info("Import Wavefront OBJ…"),
        Command::ImportStl => explained("Import STL…", "Choose the file's units first"),
        Command::ExportObj => info("Export Wavefront OBJ…"),
        Command::ExportStl => explained(
            "Export STL…",
            "Selection or visible scene, in the units you choose",
        ),
        Command::ExportImage => explained(
            "Export image…",
            "PNG of the current viewport mode · keeps rendering while you edit",
        ),
        Command::Add(Primitive::Cube) => info("Cube"),
        Command::Add(Primitive::Sphere) => info("Sphere"),
        Command::Add(Primitive::Cylinder) => info("Cylinder"),
        Command::Add(Primitive::Torus) => info("Torus"),
        Command::Add(Primitive::Plane) => info("Plane"),
        Command::Select(_) => info("Select object"),
        Command::ToggleVisible(_) => info("Show or hide"),
        Command::Delete => keyed("Delete", "Delete"),
        Command::Duplicate => keyed("Duplicate", "⇧ D"),
        Command::Undo => keyed("Undo", platform_shortcut("⌘ Z", "Ctrl+Z")),
        Command::Redo => keyed("Redo", platform_shortcut("⇧ ⌘ Z", "Ctrl+Shift+Z")),
        Command::FrameSelected => keyed("Frame selection", "F"),
        Command::FrameAll => keyed("Frame all", "Home"),
        Command::ViewFront => keyed("Front", "1"),
        Command::ViewBack => keyed("Back", platform_shortcut("⌥ 1", "Alt+1")),
        Command::ViewRight => keyed("Right", "3"),
        Command::ViewLeft => keyed("Left", platform_shortcut("⌥ 3", "Alt+3")),
        Command::ViewTop => keyed("Top", "7"),
        Command::ViewBottom => keyed("Bottom", platform_shortcut("⌥ 7", "Alt+7")),
        Command::ViewPerspective => keyed("Reset perspective", "0"),
        Command::ToggleProjection => keyed("Perspective / orthographic", "5"),
        Command::SetMode(RenderMode::Wireframe) => keyed("Wireframe", "Z then 4"),
        Command::SetMode(RenderMode::Solid) => keyed("Solid", "X"),
        Command::SetMode(RenderMode::MaterialPreview) => keyed("Material preview", "C"),
        Command::SetMode(RenderMode::Rendered) => keyed("Rendered", "V"),
        Command::SetTool(Tool::Select) => keyed("Select", "Q"),
        Command::SetTool(Tool::Move) => keyed("Move", "G"),
        Command::SetTool(Tool::Rotate) => keyed("Rotate", "R"),
        Command::SetTool(Tool::Scale) => keyed("Scale", "S"),
        Command::SetEditMode(EditMode::Object) => keyed("Object", "Tab"),
        Command::SetEditMode(EditMode::Vertex) => keyed("Vertex", "1"),
        Command::SetEditMode(EditMode::Edge) => keyed("Edge", "2"),
        Command::SetEditMode(EditMode::Face) => keyed("Face", "3"),
        Command::ToggleEdit => keyed("Edit components", "Tab"),
        Command::Inset => keyed("Inset faces", "I"),
        Command::SelectAll => keyed("Select all", "A"),
        Command::DeselectAll => keyed("Deselect all", platform_shortcut("⌥ A", "Alt+A")),
        Command::InvertSelection => keyed("Invert selection", platform_shortcut("⌘ I", "Ctrl+I")),
        Command::SelectLinked => keyed("Select linked", "L"),
        Command::BoxSelect => keyed("Box select", "B"),
        Command::Extrude => keyed("Extrude faces", "E"),
        Command::Subdivide => explained("Subdivide", "Catmull–Clark, one level"),
        Command::Bevel => CommandInfo {
            label: "Bevel edges",
            shortcut: Some(platform_shortcut("⌘ B", "Ctrl+B")),
            detail: Some("Every edge of a closed convex mesh · Object mode"),
        },
        Command::Triangulate => {
            explained("Triangulate", "Selected faces, or entire selected objects")
        }
        Command::FlipNormals => explained(
            "Flip normals",
            "Reverse the winding of selected faces or objects",
        ),
        Command::RecalculateNormals => explained(
            "Recalculate normals",
            "Make normals consistent; closed shells point outward",
        ),
        Command::ApplyScale => explained(
            "Apply scale",
            "Bake scale into mesh data, keeping world geometry and children",
        ),
        Command::ApplyRotationScale => explained(
            "Apply rotation and scale",
            "Bake rotation and scale, keeping world geometry and children",
        ),
        Command::CenterOrigin => explained(
            "Origin to geometry",
            "Move each origin to its mesh bounds center; geometry stays put",
        ),
        Command::ToggleGrid => info("Ground grid"),
        Command::ToggleMeasurements => keyed("Measurements", "⇧ M"),
        Command::ToggleViewportDenoise => info("AI denoising for the Rendered viewport"),
        Command::ToggleRenderDenoise => info("High quality AI denoising for PNG exports"),
        Command::SetDenoiseQuality(_) => {
            explained("Viewport denoise quality", "Exports always use High")
        }
        Command::MaterialPreset(_) => info("Apply surface preset"),
        Command::SetShader(_) => info("Surface shader"),
        Command::SetTextureMapping(_) => info("Generated image coordinates"),
        Command::LoadTexture(_) => info("Choose a PNG or JPEG image"),
        Command::ClearTexture(_) => info("Remove image"),
        Command::EditShader => info("Edit custom surface code"),
        Command::ApplyShader => keyed(
            "Compile and apply",
            platform_shortcut("⌘ Return", "Ctrl+Return"),
        ),
        Command::CloseShader => keyed("Close", "Esc"),
        Command::TogglePalette => keyed("Commands", platform_shortcut("⌘ K", "Ctrl+K")),
        Command::ToggleTheme => keyed("Color theme", platform_shortcut("⇧ ⌘ T", "Ctrl+Shift+T")),
        Command::ToggleHelp => keyed("Keyboard shortcuts", "?"),
        Command::TogglePreviewSettings => info("Preview lighting"),
        Command::SetPreviewStudio(_) => info("Use this environment for material preview"),
        Command::TogglePreviewWorld => {
            info("Use the project's world instead of a studio environment")
        }
        Command::TogglePreviewAo => info("Ambient occlusion for grounded contact shading"),
        Command::LoadPreviewHdri => info("Load a Radiance HDR environment"),
        Command::ResetPreview => info("Reset preview lighting"),
        Command::Draw(command) => draw_info(command),
    }
}

fn draw_info(command: DrawCommand) -> CommandInfo {
    match command {
        DrawCommand::NewSketch => CommandInfo {
            label: "New sketch",
            shortcut: Some("D"),
            detail: Some("On the face under the pointer, the view's plane, or the ground"),
        },
        DrawCommand::EditSketch => keyed("Edit sketch", "Return"),
        DrawCommand::Finish => keyed("Finish sketch", "Return"),
        DrawCommand::Tool(DrawTool::Select) => explained_keyed(
            "Select",
            "Click or drag a box · drag points and curves",
            "Q",
        ),
        DrawCommand::Tool(DrawTool::Line) => explained_keyed(
            "Line",
            "Click points; type a length, Tab for angle · A for a tangent arc",
            "L",
        ),
        DrawCommand::Tool(DrawTool::Rectangle) => explained_keyed(
            "Rectangle",
            "Type width, Tab for height · R for centre",
            "R",
        ),
        DrawCommand::Tool(DrawTool::Circle) => {
            explained_keyed("Circle", "Centre, then radius · type a radius", "C")
        }
        DrawCommand::Tool(DrawTool::Arc) => {
            explained_keyed("Arc", "Start, end, then a point on the arc", "A")
        }
        DrawCommand::Tool(DrawTool::Polygon) => explained_keyed(
            "Polygon",
            "Centre, then a corner · + and − change sides",
            "P",
        ),
        DrawCommand::Tool(DrawTool::Trim) => explained_keyed(
            "Trim",
            "Remove the piece of a curve between its nearest crossings",
            "T",
        ),
        DrawCommand::ToggleConstruction => explained_keyed(
            "Construction",
            "Guides that never form profiles · toggles selected curves",
            "X",
        ),
        DrawCommand::ToggleRectangleMode => keyed("Corner or centre rectangle", "R"),
        DrawCommand::PolygonSides(delta) if delta < 0 => keyed("Fewer sides", "−"),
        DrawCommand::PolygonSides(_) => keyed("More sides", "+"),
    }
}

const fn explained_keyed(
    label: &'static str,
    detail: &'static str,
    shortcut: &'static str,
) -> CommandInfo {
    CommandInfo {
        label,
        shortcut: Some(shortcut),
        detail: Some(detail),
    }
}

/// Icon shown beside a command in menus and the palette.
pub(crate) fn command_icon(command: Command) -> Option<Icon> {
    Some(match command {
        Command::New => Icon::Plus,
        Command::Open => Icon::Open,
        Command::Save | Command::SaveAs => Icon::Save,
        Command::ImportObj | Command::ImportStl => Icon::Import,
        Command::ExportObj | Command::ExportStl => Icon::Export,
        Command::ExportImage => Icon::ExportImage,
        Command::Add(Primitive::Cube) => Icon::Cube,
        Command::Add(Primitive::Sphere) => Icon::Sphere,
        Command::Add(Primitive::Cylinder) => Icon::Cylinder,
        Command::Add(Primitive::Torus) => Icon::Torus,
        Command::Add(Primitive::Plane) => Icon::Plane,
        Command::Delete => Icon::Delete,
        Command::Duplicate => Icon::Duplicate,
        Command::Undo => Icon::Undo,
        Command::Redo => Icon::Redo,
        Command::FrameSelected => Icon::FrameSelected,
        Command::FrameAll => Icon::FrameAll,
        Command::ViewFront
        | Command::ViewBack
        | Command::ViewRight
        | Command::ViewLeft
        | Command::ViewTop
        | Command::ViewBottom => Icon::Views,
        Command::ViewPerspective => Icon::Perspective,
        Command::ToggleProjection => Icon::Orthographic,
        Command::SetMode(mode) => shading_icon(mode),
        Command::SetTool(Tool::Select) => Icon::Select,
        Command::SetTool(Tool::Move) => Icon::Move,
        Command::SetTool(Tool::Rotate) => Icon::Rotate,
        Command::SetTool(Tool::Scale) => Icon::Scale,
        Command::SetEditMode(EditMode::Object) => Icon::ObjectMode,
        Command::SetEditMode(EditMode::Vertex) => Icon::Vertex,
        Command::SetEditMode(EditMode::Edge) => Icon::Edge,
        Command::SetEditMode(EditMode::Face) => Icon::Face,
        Command::ToggleEdit => Icon::EditMode,
        Command::Extrude => Icon::Extrude,
        Command::Inset => Icon::Inset,
        Command::Bevel => Icon::Bevel,
        Command::Subdivide => Icon::Subdivide,
        Command::Triangulate => Icon::Triangulate,
        Command::FlipNormals => Icon::FlipNormals,
        Command::RecalculateNormals => Icon::NormalsOutside,
        Command::ApplyScale => Icon::Scale,
        Command::ApplyRotationScale => Icon::ApplyRotation,
        Command::CenterOrigin => Icon::Origin,
        Command::SelectAll
        | Command::DeselectAll
        | Command::InvertSelection
        | Command::SelectLinked
        | Command::BoxSelect => Icon::Select,
        Command::ToggleGrid => Icon::Grid,
        Command::ToggleMeasurements => Icon::Measure,
        Command::ToggleViewportDenoise | Command::ToggleRenderDenoise => Icon::Denoise,
        Command::SetShader(_) => Icon::Material,
        Command::EditShader => Icon::Code,
        Command::ToggleHelp => Icon::Keyboard,
        Command::ToggleTheme => Icon::Theme,
        Command::TogglePreviewSettings => Icon::Lighting,
        Command::CheckForUpdates | Command::ToggleUpdates => Icon::Update,
        Command::Draw(DrawCommand::NewSketch) => Icon::DrawMode,
        Command::Draw(DrawCommand::EditSketch) => Icon::Sketch,
        Command::Draw(DrawCommand::Finish) => Icon::Check,
        Command::Draw(DrawCommand::Tool(tool)) => crate::sketch::hud::tool_icon(tool),
        Command::Draw(DrawCommand::ToggleConstruction) => Icon::DrawConstruction,
        Command::Draw(DrawCommand::ToggleRectangleMode) => Icon::DrawCenterRectangle,
        Command::Draw(DrawCommand::PolygonSides(_)) => Icon::DrawPolygon,
        _ => return None,
    })
}

pub(crate) fn shading_icon(mode: RenderMode) -> Icon {
    match mode {
        RenderMode::Wireframe => Icon::ShadingWire,
        RenderMode::Solid => Icon::ShadingSolid,
        RenderMode::MaterialPreview => Icon::ShadingMaterial,
        RenderMode::Rendered => Icon::ShadingRendered,
    }
}

/// Palette section for a command. `palette_commands` lists commands in this
/// order so keyboard navigation and grouped drawing agree.
pub(crate) fn command_group(command: Command) -> &'static str {
    match command {
        Command::New
        | Command::Open
        | Command::Save
        | Command::SaveAs
        | Command::ImportObj
        | Command::ImportStl
        | Command::ExportObj
        | Command::ExportStl
        | Command::ExportImage => "File",
        Command::Add(_) => "Add",
        Command::Draw(_) => "Draw",
        Command::Duplicate
        | Command::Delete
        | Command::ApplyScale
        | Command::ApplyRotationScale
        | Command::CenterOrigin => "Object",
        Command::Subdivide
        | Command::Extrude
        | Command::Inset
        | Command::Bevel
        | Command::Triangulate
        | Command::FlipNormals
        | Command::RecalculateNormals => "Mesh",
        Command::SelectAll
        | Command::DeselectAll
        | Command::InvertSelection
        | Command::SelectLinked
        | Command::BoxSelect
        | Command::SetEditMode(_)
        | Command::ToggleEdit => "Select",
        Command::SetMode(_)
        | Command::SetShader(_)
        | Command::EditShader
        | Command::TogglePreviewSettings
        | Command::ToggleViewportDenoise
        | Command::ToggleRenderDenoise => "Shading",
        Command::FrameSelected
        | Command::FrameAll
        | Command::ViewFront
        | Command::ViewBack
        | Command::ViewRight
        | Command::ViewLeft
        | Command::ViewTop
        | Command::ViewBottom
        | Command::ViewPerspective
        | Command::ToggleProjection
        | Command::ToggleGrid
        | Command::ToggleMeasurements => "View",
        _ => "Workspace",
    }
}
