//! Presentation metadata for editor commands: the label and shortcut shown in
//! tooltips, menus and the command palette, plus an icon and palette group.
//! Shortcuts use the keycap format understood by `components::keycaps`.
use super::assets::Icon;
use crate::app::{Command, EditMode, Tool};
use crate::cad::{CadCommand, PartShape};
use crate::cad_tools::{Operation, ToolCommand};
use crate::platform_shortcut;
use crate::sketch::{DrawCommand, DrawTool};
use crate::timeline::TimelineCommand;
use forma_core::FeatureOperation;
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

/// What a mesh primitive and a parametric part are, wherever both are offered.
pub(crate) const MESH_DETAIL: &str = "Editable polygons";
pub(crate) const PART_DETAIL: &str = "Parametric solid with history";
pub(crate) const SKETCH_DETAIL: &str = "Profiles to extrude";

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
        Command::ImportStl => explained(
            "Import STL…",
            "Choose the file's units, then place its meshes",
        ),
        Command::ExportObj => info("Export Wavefront OBJ…"),
        Command::ExportStl => explained(
            "Export STL…",
            "Save the selection, or the visible scene, in your units",
        ),
        Command::ExportImage => explained(
            "Export image…",
            "Save a PNG of the viewport; it renders while you keep editing",
        ),
        Command::Add(Primitive::Cube) => explained("Cube", MESH_DETAIL),
        Command::Add(Primitive::Sphere) => explained("Sphere", MESH_DETAIL),
        Command::Add(Primitive::Cylinder) => explained("Cylinder", MESH_DETAIL),
        Command::Add(Primitive::Torus) => explained("Torus", MESH_DETAIL),
        Command::Add(Primitive::Plane) => explained("Plane", MESH_DETAIL),
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
        Command::Subdivide => {
            explained("Subdivide", "Smooth the mesh with one Catmull–Clark level")
        }
        Command::Bevel => CommandInfo {
            label: "Bevel edges",
            shortcut: Some(platform_shortcut("⌘ B", "Ctrl+B")),
            detail: Some("Bevel every edge of a closed convex mesh at once"),
        },
        Command::Triangulate => explained(
            "Triangulate",
            "Split the selected faces, or whole objects, into triangles",
        ),
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
        Command::Cad(command) => cad_info(command),
        Command::CadTool(command) => tool_info(command),
        Command::Timeline(command) => timeline_info(command),
    }
}

fn tool_info(command: ToolCommand) -> CommandInfo {
    match command {
        ToolCommand::Extrude => explained_keyed(
            "Extrude",
            "Pull sketch regions into a part, or join and cut on its face",
            "E",
        ),
        ToolCommand::Revolve => explained_keyed(
            "Revolve",
            "Spin sketch regions around a line or sketch axis",
            "⇧ R",
        ),
        ToolCommand::PushPull => explained_keyed(
            "Push/Pull",
            "Drag a flat face of a part or mesh along its normal",
            "P",
        ),
        ToolCommand::Chamfer => CommandInfo {
            label: "Chamfer edges",
            shortcut: Some(platform_shortcut("⌘ B", "Ctrl+B")),
            detail: Some("Cut a flat bevel along the selected straight edges"),
        },
        ToolCommand::SketchOnFace => explained_keyed(
            "Sketch on face",
            "Draw on the selected or pointed-at flat face",
            "D",
        ),
        ToolCommand::Done => keyed("Done", "Return"),
        ToolCommand::Cancel => keyed("Cancel", "Esc"),
        ToolCommand::SetOperation(operation) => match operation {
            Operation::NewPart => explained_keyed("New part", "Make a separate part", "N"),
            Operation::Join => explained_keyed("Join", "Add material to the part", "J"),
            Operation::Cut => explained_keyed("Cut", "Remove material from the part", "K"),
            Operation::Intersect => {
                explained_keyed("Intersect", "Keep only where they overlap", "I")
            }
        },
        ToolCommand::ToggleSymmetric => explained_keyed(
            "Symmetric",
            "Extend equally to both sides of the sketch",
            "S",
        ),
        ToolCommand::Flip => {
            explained_keyed("Flip direction", "Extrude or turn the other way", "F")
        }
        ToolCommand::SketchAxis(false) => {
            explained_keyed("Sketch X axis", "Revolve around the sketch's x axis", "X")
        }
        ToolCommand::SketchAxis(true) => {
            explained_keyed("Sketch Y axis", "Revolve around the sketch's y axis", "Y")
        }
        ToolCommand::AdjustOperation(operation) => match operation {
            Operation::Cut => info("Cut instead"),
            Operation::Intersect => info("Intersect instead"),
            _ => info("Join instead"),
        },
        ToolCommand::AdjustSymmetric => info("Extend equally to both sides"),
        ToolCommand::AdjustFlip => info("Reverse the direction"),
    }
}

fn timeline_info(command: TimelineCommand) -> CommandInfo {
    match command {
        TimelineCommand::Select(_) => explained("Select feature", "Double-click to edit"),
        TimelineCommand::Previous => keyed("Previous feature", "←"),
        TimelineCommand::Next => keyed("Next feature", "→"),
        TimelineCommand::Deselect => keyed("Deselect feature", "Esc"),
        TimelineCommand::Edit => keyed("Edit feature", "Return"),
        TimelineCommand::EditSketch => explained_keyed(
            "Edit sketch",
            "Open the feature's sketch in Draw; the part follows each change",
            "Return",
        ),
        TimelineCommand::Rename => keyed("Rename feature", "F2"),
        TimelineCommand::ToggleSuppress => explained(
            "Suppress or unsuppress",
            "Keep the feature in the history without evaluating it",
        ),
        TimelineCommand::Delete => keyed("Delete feature", "Delete"),
        TimelineCommand::RollBackHere => explained(
            "Roll back to here",
            "Evaluate the history only up to this feature",
        ),
        TimelineCommand::RollToEnd => explained("Roll to end", "Evaluate the whole history again"),
        TimelineCommand::StepMarker(step) if step < 0 => keyed("Roll back one feature", "⇧ ←"),
        TimelineCommand::StepMarker(_) => keyed("Roll forward one feature", "⇧ →"),
        TimelineCommand::SetOperation(FeatureOperation::Join) => {
            explained("Join", "Add to the body before this feature")
        }
        TimelineCommand::SetOperation(FeatureOperation::Cut) => {
            explained("Cut", "Remove from the body before this feature")
        }
        TimelineCommand::SetOperation(FeatureOperation::Intersect) => {
            explained("Intersect", "Keep only the overlap with the body")
        }
        TimelineCommand::SetOperation(FeatureOperation::NewBody) => {
            explained("New body", "A separate solid, without a boolean")
        }
        TimelineCommand::SetExtent(mode) => explained(mode.label(), mode.detail()),
        TimelineCommand::ToggleFlip => explained("Flip direction", "Extrude the other way"),
    }
}

fn cad_info(command: CadCommand) -> CommandInfo {
    match command {
        CadCommand::AddPart(PartShape::Box) => explained("Box part", PART_DETAIL),
        CadCommand::AddPart(PartShape::Cylinder) => explained("Cylinder part", PART_DETAIL),
        CadCommand::AddPart(PartShape::Sphere) => explained("Sphere part", PART_DETAIL),
        CadCommand::ExtrudeSketch => explained(
            "Extrude sketch",
            "Extrude the selected sketch's outer regions 1 m into a new part",
        ),
        CadCommand::Combine(FeatureOperation::Cut) => explained(
            "Cut",
            "Subtract the other selected objects from the active one",
        ),
        CadCommand::Combine(FeatureOperation::Intersect) => explained(
            "Intersect",
            "Keep only where the selected objects overlap the active one",
        ),
        CadCommand::Combine(_) => explained(
            "Join",
            "Merge the other selected objects into the active one",
        ),
        CadCommand::ConvertToPart => explained(
            "Convert to part",
            "Closed meshes become B-rep solids; flat polygons become faces",
        ),
        CadCommand::ConvertToMesh => explained(
            "Convert to mesh",
            "Bake the part into editable polygons. Its feature history is dropped; undo restores it.",
        ),
        CadCommand::ImportStep => info("Import STEP…"),
        CadCommand::ExportStep => explained(
            "Export STEP…",
            "Save the selected parts, or all visible parts, in millimetres",
        ),
    }
}

fn draw_info(command: DrawCommand) -> CommandInfo {
    match command {
        DrawCommand::NewSketch => CommandInfo {
            label: "New sketch",
            shortcut: Some("D"),
            detail: Some("Draw on the face under the pointer, the view's plane or the ground"),
        },
        DrawCommand::NewGroundSketch => explained(
            "Draw on the ground plane",
            "Lay out a footprint, then extrude it upward",
        ),
        DrawCommand::EditSketch => keyed("Edit sketch", "Return"),
        DrawCommand::Finish => keyed("Finish sketch", "Return"),
        DrawCommand::Tool(DrawTool::Select) => explained_keyed(
            "Select",
            "Click or drag a box to select; drag points and curves to move them",
            "Q",
        ),
        DrawCommand::Tool(DrawTool::Line) => explained_keyed(
            "Line",
            "Click points; type a length, Tab for the angle, A for a tangent arc",
            "L",
        ),
        DrawCommand::Tool(DrawTool::Rectangle) => explained_keyed(
            "Rectangle",
            "Type a width, Tab for the height, R to draw from the centre",
            "R",
        ),
        DrawCommand::Tool(DrawTool::Circle) => explained_keyed(
            "Circle",
            "Click the centre, then the edge, or type a radius",
            "C",
        ),
        DrawCommand::Tool(DrawTool::Arc) => explained_keyed(
            "Arc",
            "Click the start, the end, then a point on the arc",
            "A",
        ),
        DrawCommand::Tool(DrawTool::Polygon) => explained_keyed(
            "Polygon",
            "Click the centre, then a corner; + and − change the sides",
            "P",
        ),
        DrawCommand::Tool(DrawTool::Trim) => explained_keyed(
            "Trim",
            "Remove the piece of a curve between its nearest crossings",
            "T",
        ),
        DrawCommand::ToggleConstruction => explained_keyed(
            "Construction",
            "Draw guides that never form profiles, or toggle selected curves",
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
        Command::ToggleTheme => Icon::Theme,
        Command::ToggleUpdates => Icon::Settings,
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
        Command::SelectAll | Command::DeselectAll | Command::InvertSelection => Icon::Select,
        Command::SelectLinked => Icon::Mesh,
        Command::BoxSelect => Icon::DrawConstruction,
        Command::ToggleGrid => Icon::Grid,
        Command::ToggleMeasurements => Icon::Measure,
        Command::ToggleViewportDenoise | Command::ToggleRenderDenoise => Icon::Denoise,
        Command::SetShader(_) => Icon::Material,
        Command::EditShader => Icon::Code,
        Command::ToggleHelp => Icon::Keyboard,
        Command::TogglePreviewSettings => Icon::Lighting,
        Command::CheckForUpdates => Icon::Update,
        Command::Draw(DrawCommand::NewSketch | DrawCommand::NewGroundSketch) => Icon::DrawMode,
        Command::Draw(DrawCommand::EditSketch) => Icon::Sketch,
        Command::Draw(DrawCommand::Finish) => Icon::Check,
        Command::Draw(DrawCommand::Tool(tool)) => crate::sketch::hud::tool_icon(tool),
        Command::Draw(DrawCommand::ToggleConstruction) => Icon::DrawConstruction,
        Command::Draw(DrawCommand::ToggleRectangleMode) => Icon::DrawCenterRectangle,
        Command::Draw(DrawCommand::PolygonSides(_)) => Icon::DrawPolygon,
        Command::Cad(CadCommand::AddPart(PartShape::Box)) => Icon::PartBox,
        Command::Cad(CadCommand::AddPart(PartShape::Cylinder)) => Icon::PartCylinder,
        Command::Cad(CadCommand::AddPart(PartShape::Sphere)) => Icon::PartSphere,
        Command::Cad(CadCommand::ExtrudeSketch) => Icon::Extrude,
        Command::Cad(CadCommand::Combine(FeatureOperation::Cut)) => Icon::Subtract,
        Command::Cad(CadCommand::Combine(FeatureOperation::Intersect)) => Icon::Intersect,
        Command::Cad(CadCommand::Combine(_)) => Icon::Join,
        Command::Cad(CadCommand::ConvertToPart) => Icon::Part,
        Command::Cad(CadCommand::ConvertToMesh) => Icon::Mesh,
        Command::Cad(CadCommand::ImportStep) => Icon::Import,
        Command::Cad(CadCommand::ExportStep) => Icon::Export,
        Command::CadTool(ToolCommand::Extrude) => Icon::Extrude,
        Command::CadTool(ToolCommand::Revolve) => Icon::Revolve,
        Command::CadTool(ToolCommand::PushPull) => Icon::PushPull,
        Command::CadTool(ToolCommand::Chamfer) => Icon::Bevel,
        Command::CadTool(ToolCommand::SketchOnFace) => Icon::DrawMode,
        Command::CadTool(ToolCommand::Done) => Icon::Check,
        Command::CadTool(ToolCommand::Cancel) => Icon::Close,
        Command::CadTool(ToolCommand::SetOperation(operation)) => {
            crate::cad_tools::hud_operation_icon(operation)
        }
        Command::CadTool(ToolCommand::ToggleSymmetric | ToolCommand::AdjustSymmetric) => {
            Icon::Symmetric
        }
        Command::CadTool(ToolCommand::Flip | ToolCommand::AdjustFlip) => Icon::FlipDirection,
        Command::Timeline(command) => return timeline_icon(command),
        _ => return None,
    })
}

/// Icon for a timeline command in menus and the palette.
pub(crate) fn timeline_icon(command: TimelineCommand) -> Option<Icon> {
    Some(match command {
        TimelineCommand::EditSketch | TimelineCommand::Edit => Icon::Sketch,
        TimelineCommand::Rename => Icon::Rename,
        TimelineCommand::ToggleSuppress => Icon::EyeOff,
        TimelineCommand::Delete => Icon::Delete,
        TimelineCommand::RollBackHere | TimelineCommand::StepMarker(_) => Icon::History,
        TimelineCommand::RollToEnd => Icon::ChevronRight,
        TimelineCommand::SetOperation(operation) => operation_icon(operation),
        _ => return None,
    })
}

/// Glyph for how a feature combines with the body before it.
pub(crate) fn operation_icon(operation: FeatureOperation) -> Icon {
    match operation {
        FeatureOperation::Join => Icon::Join,
        FeatureOperation::Cut => Icon::Subtract,
        FeatureOperation::Intersect => Icon::Intersect,
        FeatureOperation::NewBody => Icon::Part,
    }
}

pub(crate) fn shading_icon(mode: RenderMode) -> Icon {
    match mode {
        RenderMode::Wireframe => Icon::ShadingWire,
        RenderMode::Solid => Icon::ShadingSolid,
        RenderMode::MaterialPreview => Icon::ShadingMaterial,
        RenderMode::Rendered => Icon::ShadingRendered,
    }
}

/// Palette sections, in the order the unfiltered palette lists them:
/// making geometry first, then editing, viewing and files.
pub(crate) const PALETTE_GROUPS: [&str; 13] = [
    "Add",
    "Draw",
    "Part",
    "Mesh",
    "Object",
    "Select",
    "Timeline",
    "View",
    "Shading",
    "File",
    "Import",
    "Export",
    "Workspace",
];

/// Position of `group` in `PALETTE_GROUPS`.
pub(crate) fn group_rank(group: &str) -> usize {
    PALETTE_GROUPS
        .iter()
        .position(|candidate| *candidate == group)
        .unwrap_or(PALETTE_GROUPS.len())
}

/// Palette section for a command; also the category shown beside a search
/// result.
pub(crate) fn command_group(command: Command) -> &'static str {
    match command {
        Command::New | Command::Open | Command::Save | Command::SaveAs => "File",
        Command::ImportObj | Command::ImportStl | Command::Cad(CadCommand::ImportStep) => "Import",
        Command::ExportObj
        | Command::ExportStl
        | Command::ExportImage
        | Command::Cad(CadCommand::ExportStep) => "Export",
        Command::Add(_) | Command::Cad(CadCommand::AddPart(_)) => "Add",
        Command::Draw(_) => "Draw",
        Command::Cad(_) | Command::CadTool(_) => "Part",
        Command::Timeline(_) => "Timeline",
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

/// Other words a command answers to in palette search, for names used by
/// other modellers ("boolean", "union") and the kind of object it makes.
pub(crate) fn command_keywords(command: Command) -> &'static str {
    match command {
        Command::Add(_) => "mesh primitive polygon",
        Command::Cad(CadCommand::AddPart(_)) => "solid primitive parametric brep cad",
        Command::Cad(CadCommand::Combine(FeatureOperation::Join)) => "combine union merge",
        Command::Cad(CadCommand::Combine(FeatureOperation::Cut)) => "combine subtract difference",
        Command::Cad(CadCommand::Combine(_)) => "combine common",
        Command::Cad(CadCommand::ConvertToPart) => "solid brep cad",
        Command::Cad(CadCommand::ConvertToMesh) => "polygons bake",
        Command::CadTool(ToolCommand::Chamfer) | Command::Bevel => "chamfer bevel",
        Command::CadTool(ToolCommand::PushPull) => "press pull offset face",
        Command::ToggleTheme => "appearance dark light preferences",
        Command::ToggleHelp => "help keys hotkeys reference",
        Command::ExportImage => "render screenshot",
        Command::ToggleProjection => "ortho",
        Command::SetMode(_) => "shading",
        _ => "",
    }
}
