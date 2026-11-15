//! Primary workspace modes shown in the title bar switcher. The list is data:
//! a feature adds a mode by adding an entry here and returning
//! `ModeState::Available(command)` once its command exists.
use super::assets::Icon;
use crate::app::{Command, EditMode, Studio};

/// What a mode button does right now.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum ModeState {
    /// The current mode; clicking does nothing.
    Active,
    /// Clicking runs this command.
    Available(Command),
    /// Shown disabled with this explanation. No mode needs it right now.
    #[allow(dead_code)]
    Unavailable(&'static str),
}

pub(crate) struct WorkspaceMode {
    pub id: &'static str,
    pub label: &'static str,
    pub icon: Icon,
    pub shortcut: Option<&'static str>,
    pub hint: &'static str,
    pub state: fn(&Studio) -> ModeState,
}

pub(crate) const MODES: &[WorkspaceMode] = &[
    WorkspaceMode {
        id: "object",
        label: "Object",
        icon: Icon::ObjectMode,
        shortcut: Some("Tab"),
        hint: "Select, arrange and transform objects",
        state: |s| {
            if s.is_drawing() {
                ModeState::Available(Command::Draw(crate::sketch::DrawCommand::Finish))
            } else if s.edit_mode == EditMode::Object {
                ModeState::Active
            } else {
                ModeState::Available(Command::SetEditMode(EditMode::Object))
            }
        },
    },
    WorkspaceMode {
        id: "edit",
        label: "Edit",
        icon: Icon::EditMode,
        shortcut: Some("Tab"),
        hint: "Edit vertices, edges and faces of the active mesh",
        state: |s| {
            if s.edit_mode.is_component() {
                ModeState::Active
            } else {
                ModeState::Available(Command::ToggleEdit)
            }
        },
    },
    WorkspaceMode {
        id: "draw",
        label: "Draw",
        icon: Icon::DrawMode,
        shortcut: Some("D"),
        hint: "Sketch 2D profiles on planes and faces",
        state: draw_mode_state,
    },
];

/// Hook for the Draw feature: return `Active` while drawing and
/// `Available(Command::…)` to enter it.
pub(crate) fn draw_mode_state(s: &Studio) -> ModeState {
    if s.is_drawing() {
        ModeState::Active
    } else {
        ModeState::Available(Command::Draw(crate::sketch::DrawCommand::NewSketch))
    }
}
