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
