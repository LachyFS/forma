//! Explicit units and scope for unitless mesh exchange.

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum StlDirection {
    Import,
    Export,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum StlUnit {
    Millimetres,
    Centimetres,
    Metres,
    Inches,
}

impl StlUnit {
    pub(crate) const ALL: [Self; 4] = [
        Self::Millimetres,
        Self::Centimetres,
        Self::Metres,
        Self::Inches,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Millimetres => "Millimetres · mm",
            Self::Centimetres => "Centimetres · cm",
            Self::Metres => "Metres · m",
            Self::Inches => "Inches · in",
        }
    }

    pub(crate) fn metres(self) -> f32 {
        match self {
            Self::Millimetres => 0.001,
            Self::Centimetres => 0.01,
            Self::Metres => 1.,
            Self::Inches => 0.0254,
        }
    }

    pub(crate) fn cycle(self, backwards: bool) -> Self {
        let index = Self::ALL.iter().position(|&unit| unit == self).unwrap();
        Self::ALL[(index + if backwards { 3 } else { 1 }) % 4]
    }
}

#[derive(Clone, Copy)]
pub(crate) struct StlDialog {
    pub direction: StlDirection,
    pub unit: StlUnit,
    pub selected_only: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum MeshImport {
    Obj,
    Stl(StlUnit),
}

/// Why an export format has nothing to write right now, if so. Mirrors the
/// scopes the exports use (visible meshes and part tessellations for OBJ and
/// STL; selected or visible parts for STEP), so the export menu explains a
/// dead end instead of opening a file dialog for it.
pub(crate) fn export_unavailable(
    scene: &forma_core::Scene,
    command: crate::app::Command,
) -> Option<&'static str> {
    use crate::app::Command;
    use crate::cad::CadCommand;
    match command {
        Command::ExportObj | Command::ExportStl => (!scene
            .mesh_instances()
            .any(|mesh| scene.is_effectively_visible(mesh.id)))
        .then_some("Nothing visible to export"),
        Command::Cad(CadCommand::ExportStep) => {
            let mut parts = scene.objects.iter().filter(|object| object.is_part());
            match parts.next() {
                None => Some("Needs a CAD part"),
                Some(first) => (!std::iter::once(first)
                    .chain(parts)
                    .any(|object| scene.is_effectively_visible(object.id)))
                .then_some("Every part is hidden"),
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::export_unavailable;
    use crate::app::Command;
    use crate::cad::CadCommand;
    use forma_core::{Primitive, Scene};

    const STEP: Command = Command::Cad(CadCommand::ExportStep);

    #[test]
    fn exports_explain_when_there_is_nothing_to_write() {
        let mut scene = Scene::empty();
        assert!(export_unavailable(&scene, Command::ExportObj).is_some());
        assert!(export_unavailable(&scene, Command::ExportStl).is_some());
        assert_eq!(export_unavailable(&scene, STEP), Some("Needs a CAD part"));
        assert_eq!(export_unavailable(&scene, Command::ExportImage), None);

        let cube = scene.add(Primitive::Cube);
        assert_eq!(export_unavailable(&scene, Command::ExportObj), None);
        assert_eq!(export_unavailable(&scene, STEP), Some("Needs a CAD part"));

        scene.object_mut(cube).unwrap().visible = false;
        assert!(export_unavailable(&scene, Command::ExportStl).is_some());

        let mut startup = crate::startup::studio_scene();
        assert_eq!(export_unavailable(&startup, STEP), None);
        let bracket = crate::startup::hero(&startup).unwrap();
        startup.object_mut(bracket).unwrap().visible = false;
        assert_eq!(
            export_unavailable(&startup, STEP),
            Some("Every part is hidden")
        );
    }
}
