//! Native workspace chrome. Rendering and interaction state live in `Studio`;
//! this module owns layout, the design system and session-only panel state.
//!
//! - `tokens`, `assets`, `components`, `commands`: the design system.
//! - `titlebar`, `modes`, `hud`, `outliner`, `inspector`, `statusbar`: the shell.
//! - `slots`: reserved hooks other features fill (viewport dock, adjust panel).
//! - `timeline`, `feature_inspector`: the part history dock and its inspector.
//! - `overlays`: menus, palette, dialogs and popovers.
pub(crate) mod assets;
pub(crate) mod commands;
pub(crate) mod components;
mod feature_inspector;
mod hud;
mod inspector;
pub(crate) mod modes;
mod outliner;
mod overlays;
pub(crate) mod slots;
mod statusbar;
pub(crate) mod timeline;
mod titlebar;
pub(crate) mod tokens;

use crate::app::{Command, Field, Studio};
use crate::theme::Colors;
use components::{col, row, tip};
use gpui::{prelude::*, *};
use std::{cell::Cell, rc::Rc};
use tokens::*;

pub(crate) use assets::{Assets, register_fonts};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PanelSection {
    Transform,
    Surface,
    Geometry,
    Render,
    Textures,
    Denoise,
    Lighting,
    Display,
    Sketch,
    Part,
}

const SECTION_COUNT: usize = 10;

impl PanelSection {
    fn contains_field(self, field: Field) -> bool {
        matches!(
            (self, field),
            (
                Self::Transform,
                Field::Translation(_) | Field::Rotation(_) | Field::Scale(_) | Field::Dimensions(_)
            ) | (
                Self::Surface,
                Field::Color(_) | Field::Roughness | Field::Metallic | Field::Emission | Field::Ior
            ) | (
                Self::Textures,
                Field::NormalStrength | Field::TextureScale(_) | Field::TextureOffset(_)
            ) | (
                Self::Render,
                Field::Samples | Field::Bounces | Field::Exposure | Field::WorldStrength
            ) | (Self::Denoise, Field::DenoiseStart)
                | (Self::Part, Field::Feature(_))
        )
    }

    fn tab(self) -> InspectorTab {
        match self {
            Self::Transform
            | Self::Surface
            | Self::Geometry
            | Self::Textures
            | Self::Sketch
            | Self::Part => InspectorTab::Object,
            Self::Render | Self::Denoise | Self::Lighting | Self::Display => InspectorTab::Render,
        }
    }

    const FIELD_SECTIONS: [Self; 6] = [
        Self::Transform,
        Self::Surface,
        Self::Textures,
        Self::Render,
        Self::Denoise,
        Self::Part,
    ];
}

/// Inspector pages: the selection's properties, and viewport/render settings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum InspectorTab {
    Object,
    Render,
}

pub(crate) const SIDEBAR_DEFAULT_WIDTH: f32 = 304.;
const SIDEBAR_MIN_WIDTH: f32 = 240.;
const SIDEBAR_MAX_WIDTH: f32 = 560.;
const VIEWPORT_MIN_WIDTH: f32 = 360.;
const DIVIDER_WIDTH: f32 = 1.;
pub(crate) const TITLEBAR_HEIGHT: f32 = 40.;
pub(crate) const STATUSBAR_HEIGHT: f32 = 26.;
const OUTLINER_MIN_HEIGHT: f32 = 96.;
const INSPECTOR_MIN_HEIGHT: f32 = 200.;
const SPLIT_HANDLE: f32 = 1.;

#[derive(Clone, Copy)]
pub(crate) struct ActiveFieldLayout {
    pub field: Field,
    pub bounds: Bounds<Pixels>,
}

/// A pointer drag owned by the chrome. Moves and releases are captured across
/// the whole window so fast drags never leak into the viewport.
enum PanelDrag {
    Sidebar {
        pointer_x: f32,
        width: f32,
    },
    Outliner {
        pointer_y: f32,
        height: f32,
    },
    /// Horizontal drag on a numeric field. Becomes a click if it never moves.
    Scrub {
        field: Field,
        origin_x: f32,
        start: f32,
        before: Box<forma_core::Scene>,
        moved: bool,
    },
}

/// Dropdown menus anchored to their trigger.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MenuKind {
    Add,
    View,
}

/// Where a menu opens from. Each trigger records its bounds every frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Anchor {
    ToolRail,
    Outliner,
    ViewHud,
}
const ANCHOR_COUNT: usize = 3;

pub(crate) struct OpenMenu {
    pub kind: MenuKind,
    pub from: Anchor,
    pub anchor: Bounds<Pixels>,
    pub index: usize,
}

/// Workspace layout and disclosure state never enter the document or undo history.
pub(crate) struct PanelState {
    open: [bool; SECTION_COUNT],
    pub(crate) collapsed_collections: std::collections::BTreeSet<u64>,
    sidebar_width: f32,
    outliner_height: Option<f32>,
    drag: Option<PanelDrag>,
    inspector_scroll: ScrollHandle,
    outliner_scroll: ScrollHandle,
    pub(crate) active_field_bounds: Rc<Cell<Option<ActiveFieldLayout>>>,
    field_scroll_request: Rc<Cell<u64>>,
    pub(crate) tab: InspectorTab,
    pub(crate) menu: Option<OpenMenu>,
    anchors: Rc<[Cell<Bounds<Pixels>>; ANCHOR_COUNT]>,
    recent: Vec<Command>,
    /// The name field being edited lives in the outliner row, not the inspector.
    pub(crate) rename_in_outliner: bool,
    /// The update version whose ready notice was dismissed for this session.
    pub(crate) dismissed_update: Option<String>,
    /// Feature timeline selection, context menu and marker drag.
    pub(crate) timeline: crate::timeline::TimelineState,
}

impl Default for PanelState {
    fn default() -> Self {
        let mut open = [false; SECTION_COUNT];
        for section in [
            PanelSection::Transform,
            PanelSection::Surface,
            PanelSection::Render,
            PanelSection::Denoise,
            PanelSection::Lighting,
            PanelSection::Display,
            PanelSection::Sketch,
            PanelSection::Part,
        ] {
            open[section as usize] = true;
        }
        Self {
            open,
            collapsed_collections: Default::default(),
            sidebar_width: SIDEBAR_DEFAULT_WIDTH,
            outliner_height: None,
            drag: None,
            inspector_scroll: ScrollHandle::new(),
            outliner_scroll: ScrollHandle::new(),
            active_field_bounds: Default::default(),
            field_scroll_request: Default::default(),
            tab: InspectorTab::Object,
            menu: None,
            anchors: Rc::new(Default::default()),
            recent: Vec::new(),
            rename_in_outliner: false,
            dismissed_update: None,
            timeline: Default::default(),
        }
    }
}

impl PanelState {
    pub(crate) fn is_inspector_field(field: Field) -> bool {
        PanelSection::FIELD_SECTIONS
            .into_iter()
            .any(|section| section.contains_field(field))
    }

    pub(crate) fn is_open(&self, section: PanelSection) -> bool {
        self.open[section as usize]
    }

    pub(crate) fn scroll_to_active_field(
        &self,
        field: Field,
        window: &mut Window,
        cx: &mut Context<Studio>,
    ) {
        let request = self.field_scroll_request.get().wrapping_add(1);
        self.field_scroll_request.set(request);
        if !Self::is_inspector_field(field) {
            return;
        }
        // Wait for the newly revealed field's layout, then move only the amount
        // needed to show it. Ignore superseded requests and cancelled editing.
        self.active_field_bounds.set(None);
        let bounds = self.active_field_bounds.clone();
        let requests = self.field_scroll_request.clone();
        let scroll = self.inspector_scroll.clone();
        let studio = cx.weak_entity();
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |window, cx| {
                if requests.get() != request {
                    return;
                }
                let Some(studio) = studio.upgrade() else {
                    return;
                };
                if !studio.read(cx).field_is_active(field) {
                    return;
                }
                let Some(layout) = bounds.get() else {
                    return;
                };
                if layout.field != field {
                    return;
                }
                let current = scroll.offset();
                let offset = reveal_field_offset(
                    scroll.bounds(),
                    layout.bounds,
                    current,
                    scroll.max_offset(),
                );
                if offset != current {
                    scroll.set_offset(offset);
                    window.refresh();
                }
            });
        });
    }

    /// Native smoke observes the same layout bounds used for keyboard reveal.
    pub(crate) fn active_field_is_visible(&self, field: Field) -> bool {
        self.active_field_bounds.get().is_some_and(|layout| {
            let viewport = self.inspector_scroll.bounds();
            layout.field == field
                && layout.bounds.top() >= viewport.top() - px(0.5)
                && layout.bounds.bottom() <= viewport.bottom() + px(0.5)
        })
    }

    pub(crate) fn inspector_scroll_offset(&self) -> Point<Pixels> {
        self.inspector_scroll.offset()
    }

    /// Scroll the inspector to `y` pixels from the top (clamped by layout).
    pub(crate) fn scroll_inspector_to(&self, y: f32) {
        self.inspector_scroll.set_offset(point(px(0.), px(-y)));
    }

    /// Show the page and section that hold `field`.
    pub(crate) fn reveal_field(&mut self, field: Field) {
        for section in PanelSection::FIELD_SECTIONS {
            if section.contains_field(field) {
                self.open[section as usize] = true;
                self.tab = section.tab();
            }
        }
        if field != Field::Name {
            self.rename_in_outliner = false;
        }
        if let Field::Feature(_) = field {
            self.tab = InspectorTab::Object;
            self.open[PanelSection::Part as usize] = true;
        }
    }

    fn clamp_sidebar_width(width: f32, window_width: f32) -> f32 {
        let maximum =
            (window_width - DIVIDER_WIDTH - VIEWPORT_MIN_WIDTH).clamp(0., SIDEBAR_MAX_WIDTH);
        width.clamp(SIDEBAR_MIN_WIDTH.min(maximum), maximum)
    }

    /// Set the sidebar width directly (clamped at layout), as a drag would.
    pub(crate) fn set_sidebar_width(&mut self, width: f32) {
        self.sidebar_width = width;
    }

    pub(crate) fn sidebar_width(&self, window_width: f32) -> f32 {
        Self::clamp_sidebar_width(self.sidebar_width, window_width)
    }

    fn clamp_outliner_height(height: f32, sidebar_height: f32) -> f32 {
        let maximum = (sidebar_height - INSPECTOR_MIN_HEIGHT).max(OUTLINER_MIN_HEIGHT);
        height.clamp(OUTLINER_MIN_HEIGHT, maximum)
    }

    /// Fits `rows` outliner rows by default, up to 40% of the sidebar, until
    /// the user drags the split.
    pub(crate) fn outliner_height(&self, sidebar_height: f32, rows: usize) -> f32 {
        let height = self.outliner_height.unwrap_or_else(|| {
            (outliner::HEADER + rows as f32 * outliner::ROW + SP_3)
                .clamp(150., (sidebar_height * 0.4).max(150.))
        });
        Self::clamp_outliner_height(height, sidebar_height)
    }

    /// End a sidebar width drag. Returns whether one was active.
    pub(crate) fn finish_sidebar_resize(&mut self) -> bool {
        if matches!(self.drag, Some(PanelDrag::Sidebar { .. })) {
            self.drag = None;
            true
        } else {
            false
        }
    }

    pub(crate) fn is_resizing(&self) -> bool {
        matches!(
            self.drag,
            Some(PanelDrag::Sidebar { .. } | PanelDrag::Outliner { .. })
        )
    }

    /// True while a numeric scrub is applying live values.
    pub(crate) fn is_scrubbing(&self) -> bool {
        matches!(self.drag, Some(PanelDrag::Scrub { moved: true, .. }))
    }

    pub(crate) fn scrubbing_field(&self) -> Option<Field> {
        match self.drag {
            Some(PanelDrag::Scrub {
                field, moved: true, ..
            }) => Some(field),
            _ => None,
        }
    }

    pub(crate) fn close_menu(&mut self) {
        self.menu = None;
    }

    /// Record a trigger's bounds so its menu opens beneath it.
    pub(crate) fn anchor_cell(&self, anchor: Anchor) -> AnchorRecorder {
        AnchorRecorder {
            cells: self.anchors.clone(),
            slot: anchor as usize,
        }
    }

    pub(crate) fn recent_commands(&self) -> &[Command] {
        &self.recent
    }

    /// Commands run from the palette appear first next time it opens.
    pub(crate) fn remember_command(&mut self, command: Command) {
        if matches!(
            command,
            Command::TogglePalette | Command::Select(_) | Command::ToggleVisible(_)
        ) {
            return;
        }
        self.recent.retain(|recent| *recent != command);
        self.recent.insert(0, command);
        self.recent.truncate(4);
    }
}

/// Writes a trigger element's laid-out bounds into its anchor slot.
pub(crate) struct AnchorRecorder {
    cells: Rc<[Cell<Bounds<Pixels>>; ANCHOR_COUNT]>,
    slot: usize,
}

impl AnchorRecorder {
    pub(crate) fn element(self) -> impl IntoElement {
        canvas(
            move |bounds, _, _| self.cells[self.slot].set(bounds),
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
        .size_full()
    }
}

fn reveal_field_offset(
    viewport: Bounds<Pixels>,
    field: Bounds<Pixels>,
    current: Point<Pixels>,
    maximum: Size<Pixels>,
) -> Point<Pixels> {
    if viewport.size.height <= px(0.) {
        return current;
    }
    let margin = px(4.).min((viewport.size.height - field.size.height).max(px(0.)) * 0.5);
    let delta = if field.top() < viewport.top() + margin {
        viewport.top() + margin - field.top()
    } else if field.bottom() > viewport.bottom() - margin {
        viewport.bottom() - margin - field.bottom()
    } else {
        px(0.)
    };
    point(
        current.x,
        (current.y + delta).clamp(-maximum.height, px(0.)),
    )
}

/// Display units per pointer pixel when scrubbing a numeric field, and the
/// number of decimals typed into the field.
fn scrub_step(field: Field) -> (f32, usize) {
    match field {
        Field::Translation(_) | Field::Dimensions(_) => (0.01, 3),
        Field::Rotation(_) | Field::PreviewRotation => (0.5, 1),
        Field::Scale(_) | Field::TextureScale(_) | Field::TextureOffset(_) => (0.01, 3),
        Field::Color(_) | Field::Roughness | Field::Metallic | Field::Ior => (0.005, 3),
        Field::NormalStrength | Field::WorldStrength | Field::PreviewStrength => (0.01, 2),
        Field::Emission => (0.05, 2),
        Field::Exposure => (0.02, 2),
        Field::PreviewOpacity | Field::PreviewBlur => (0.5, 1),
        Field::Samples => (4., 0),
        Field::Bounces => (0.1, 0),
        Field::DenoiseStart => (1., 0),
        Field::Feature(field) => field.scrub_step(),
        Field::Name => (0., 0),
        Field::CadAdjust(param) => param.scrub_step(),
    }
}

#[cfg(test)]
mod field_scroll_tests {
    use super::reveal_field_offset;
    use gpui::{Bounds, Point, point, px, size};

    #[test]
    fn tab_does_not_scroll_fields_already_in_view() {
        let viewport = Bounds::new(point(px(100.), px(100.)), size(px(200.), px(100.)));
        let current = point(px(-9.), px(-100.));
        let maximum = size(px(20.), px(500.));
        for top in [104., 130., 175.] {
            let field = Bounds::new(point(px(110.), px(top)), size(px(150.), px(21.)));
            assert_eq!(
                reveal_field_offset(viewport, field, current, maximum),
                current
            );
        }
    }

    #[test]
    fn tab_reveals_only_the_clipped_edge_and_preserves_horizontal_position() {
        let viewport = Bounds::new(point(px(100.), px(100.)), size(px(200.), px(100.)));
        let current = point(px(-9.), px(-100.));
        let maximum = size(px(20.), px(500.));
        let above = Bounds::new(point(px(110.), px(90.)), size(px(150.), px(21.)));
        let below = Bounds::new(point(px(110.), px(190.)), size(px(150.), px(21.)));
        assert_eq!(
            reveal_field_offset(viewport, above, current, maximum),
            point(px(-9.), px(-86.))
        );
        assert_eq!(
            reveal_field_offset(viewport, below, current, maximum),
            point(px(-9.), px(-115.))
        );
    }

    #[test]
    fn field_reveal_clamps_to_scroll_limits_and_ignores_empty_viewports() {
        let viewport = Bounds::new(point(px(100.), px(100.)), size(px(200.), px(100.)));
        let field = Bounds::new(point(px(110.), px(400.)), size(px(150.), px(21.)));
        assert_eq!(
            reveal_field_offset(viewport, field, Point::default(), size(px(0.), px(50.))),
            point(px(0.), px(-50.))
        );
        let empty = Bounds::new(viewport.origin, size(px(200.), px(0.)));
        let current = point(px(0.), px(-10.));
        assert_eq!(
            reveal_field_offset(empty, field, current, size(px(0.), px(50.))),
            current
        );
    }
}

#[cfg(test)]
mod panel_tests {
    use super::{
        Field, INSPECTOR_MIN_HEIGHT, InspectorTab, OUTLINER_MIN_HEIGHT, PanelSection, PanelState,
        SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH, VIEWPORT_MIN_WIDTH, scrub_step,
    };
    use crate::app::Command;

    #[test]
    fn revealing_a_field_opens_its_section_and_page() {
        let mut panels = PanelState::default();
        panels.open[PanelSection::Textures as usize] = false;
        panels.reveal_field(Field::TextureScale(1));
        assert!(panels.is_open(PanelSection::Textures));
        assert_eq!(panels.tab, InspectorTab::Object);
        panels.reveal_field(Field::Samples);
        assert_eq!(panels.tab, InspectorTab::Render);
        panels.reveal_field(Field::Translation(0));
        assert_eq!(panels.tab, InspectorTab::Object);
        assert!(!PanelState::is_inspector_field(Field::PreviewBlur));
        assert!(!PanelState::is_inspector_field(Field::Name));
    }

    #[test]
    fn recent_commands_are_unique_newest_first_and_bounded() {
        let mut panels = PanelState::default();
        for command in [
            Command::FrameAll,
            Command::ToggleGrid,
            Command::FrameAll,
            Command::TogglePalette,
            Command::Undo,
            Command::Redo,
            Command::Save,
        ] {
            panels.remember_command(command);
        }
        assert_eq!(
            panels.recent_commands(),
            [
                Command::Save,
                Command::Redo,
                Command::Undo,
                Command::FrameAll
            ]
        );
    }

    #[test]
    fn panel_sizes_respect_the_viewport_and_inspector_minimums() {
        assert_eq!(
            PanelState::clamp_sidebar_width(9000., 1512.),
            SIDEBAR_MAX_WIDTH
        );
        assert_eq!(
            PanelState::clamp_sidebar_width(10., 1512.),
            SIDEBAR_MIN_WIDTH
        );
        assert!(PanelState::clamp_sidebar_width(560., 1000.) <= 1000. - VIEWPORT_MIN_WIDTH);
        let panels = PanelState::default();
        assert_eq!(panels.outliner_height(584., 0), 150.);
        assert!((panels.outliner_height(584., 6) - (36. + 6. * 26. + 12.)).abs() < 0.01);
        assert!((panels.outliner_height(584., 500) - 584. * 0.4).abs() < 0.01);
        assert_eq!(
            PanelState::clamp_outliner_height(9000., 584.),
            584. - INSPECTOR_MIN_HEIGHT
        );
        assert_eq!(
            PanelState::clamp_outliner_height(0., 584.),
            OUTLINER_MIN_HEIGHT
        );
    }

    #[test]
    fn every_numeric_field_scrubs() {
        use Field::*;
        for field in [
            Translation(0),
            Rotation(1),
            Scale(2),
            Dimensions(0),
            Color(1),
            Roughness,
            Metallic,
            Ior,
            Emission,
            NormalStrength,
            TextureScale(0),
            TextureOffset(1),
            Exposure,
            WorldStrength,
            Samples,
            Bounces,
            DenoiseStart,
            PreviewRotation,
            PreviewStrength,
            PreviewOpacity,
            PreviewBlur,
            Feature(crate::timeline::FeatureField::Distance),
            Feature(crate::timeline::FeatureField::Rotation(2)),
        ] {
            assert!(scrub_step(field).0 > 0., "{field:?}");
        }
        assert_eq!(scrub_step(Name).0, 0.);
    }
}

impl Studio {
    pub(crate) fn begin_sidebar_resize(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.click_count == 2 {
            self.panels.sidebar_width = SIDEBAR_DEFAULT_WIDTH;
            self.panels.finish_sidebar_resize();
        } else {
            self.panels.drag = Some(PanelDrag::Sidebar {
                pointer_x: event.position.x.into(),
                width: self
                    .panels
                    .sidebar_width(window.viewport_size().width.into()),
            });
        }
        cx.notify();
        cx.stop_propagation();
    }

    fn begin_outliner_resize(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sidebar_height = sidebar_height(window);
        if event.click_count == 2 {
            self.panels.outliner_height = None;
            self.panels.drag = None;
        } else {
            self.panels.drag = Some(PanelDrag::Outliner {
                pointer_y: event.position.y.into(),
                height: self
                    .panels
                    .outliner_height(sidebar_height, self.outliner_rows()),
            });
        }
        cx.notify();
        cx.stop_propagation();
    }

    /// Press on a numeric field: drag to scrub, or release to type.
    pub(crate) fn begin_scrub(
        &mut self,
        field: Field,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus);
        if self.active_field.is_some() {
            self.commit_field(cx);
            if self.active_field.is_some() {
                return;
            }
        }
        let Some(start) = self.field_numeric_value(field) else {
            return;
        };
        self.panels.drag = Some(PanelDrag::Scrub {
            field,
            origin_x: event.position.x.into(),
            start,
            before: Box::new(self.scene.clone()),
            moved: false,
        });
        cx.notify();
    }

    /// Pointer motion for every chrome drag: panel sizes and numeric scrubs.
    pub(crate) fn move_sidebar_resize(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.panels.drag.as_mut() else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.finish_panel_drag(cx);
            return;
        }
        match drag {
            PanelDrag::Sidebar { pointer_x, width } => {
                self.panels.sidebar_width = PanelState::clamp_sidebar_width(
                    *width + *pointer_x - f32::from(event.position.x),
                    window.viewport_size().width.into(),
                );
            }
            PanelDrag::Outliner { pointer_y, height } => {
                self.panels.outliner_height = Some(PanelState::clamp_outliner_height(
                    *height + f32::from(event.position.y) - *pointer_y,
                    sidebar_height(window),
                ));
            }
            PanelDrag::Scrub {
                field,
                origin_x,
                start,
                moved,
                ..
            } => {
                let dx = f32::from(event.position.x) - *origin_x;
                if !*moved && dx.abs() < 3. {
                    return;
                }
                *moved = true;
                let field = *field;
                let (step, decimals) = scrub_step(field);
                let fine = if event.modifiers.shift { 0.1 } else { 1. };
                let value = *start + dx * step * fine;
                self.active_field = Some((field, format!("{value:.decimals$}")));
                self.commit_field(cx);
                // Out-of-range intermediate values keep the last valid result.
                self.active_field = None;
            }
        }
        cx.notify();
        cx.stop_propagation();
    }

    pub(crate) fn end_sidebar_resize(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left || self.panels.drag.is_none() {
            return;
        }
        if let Some(PanelDrag::Scrub {
            field,
            moved: false,
            ..
        }) = self.panels.drag
        {
            self.panels.drag = None;
            self.begin_field(field, window, cx);
        } else {
            self.finish_panel_drag(cx);
        }
        cx.notify();
        cx.stop_propagation();
    }

    /// End any chrome drag, recording a completed scrub as one undo step.
    pub(crate) fn finish_panel_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.panels.drag.take() else {
            return;
        };
        if let PanelDrag::Scrub {
            before,
            moved: true,
            field,
            ..
        } = drag
            // Adjust-panel scrubs amend the last operation's own undo step.
            && !matches!(field, Field::CadAdjust(_))
            && *before != self.scene
        {
            self.history.checkpoint(&before);
            self.dirty = true;
        }
        cx.notify();
    }

    /// Rows the outliner lists: collections plus objects.
    fn outliner_rows(&self) -> usize {
        self.scene.objects.len() + self.scene.collections.len()
    }

    pub(crate) fn toggle_section(&mut self, section: PanelSection, cx: &mut Context<Self>) {
        let open = &mut self.panels.open[section as usize];
        *open = !*open;
        // A collapsed field must not keep consuming keyboard input.
        if !*open
            && self
                .active_field
                .as_ref()
                .is_some_and(|(field, _)| section.contains_field(*field))
        {
            self.active_field = None;
            self.status = "Edit cancelled".into();
        }
        cx.notify();
    }

    pub(crate) fn set_inspector_tab(&mut self, tab: InspectorTab, cx: &mut Context<Self>) {
        if self.panels.tab == tab {
            return;
        }
        self.panels.tab = tab;
        if self
            .active_field
            .as_ref()
            .is_some_and(|(field, _)| *field != Field::Name)
        {
            self.active_field = None;
            self.status = "Edit cancelled".into();
        }
        cx.notify();
    }

    pub(crate) fn open_menu(&mut self, kind: MenuKind, anchor: Anchor, cx: &mut Context<Self>) {
        if self
            .panels
            .menu
            .as_ref()
            .is_some_and(|menu| menu.kind == kind)
        {
            self.panels.menu = None;
        } else {
            self.active_field = None;
            self.palette_open = false;
            self.preview_open = false;
            self.panels.menu = Some(OpenMenu {
                kind,
                from: anchor,
                anchor: self.panels.anchors[anchor as usize].get(),
                index: overlays::menu::first_item(kind),
            });
        }
        cx.notify();
    }

    /// Keyboard navigation for an open dropdown. Returns whether it consumed
    /// the key; every key is consumed while a menu is open.
    pub(crate) fn handle_menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(menu) = self.panels.menu.as_mut() else {
            return false;
        };
        let items = overlays::menu::items(menu.kind, self.scene.camera);
        match event.keystroke.key.as_str() {
            "escape" => self.panels.menu = None,
            "up" => menu.index = overlays::menu::step(&items, menu.index, -1),
            "down" => menu.index = overlays::menu::step(&items, menu.index, 1),
            "enter" => {
                if let Some(command) = items.get(menu.index).and_then(|item| item.command()) {
                    self.panels.menu = None;
                    self.execute(command, window, cx);
                }
            }
            _ => {}
        }
        cx.notify();
        true
    }
}

fn sidebar_height(window: &Window) -> f32 {
    f32::from(window.viewport_size().height) - TITLEBAR_HEIGHT - STATUSBAR_HEIGHT
}

/// Capture across the window so fast drags and releases outside a handle
/// still work, without sending the same gesture to the viewport tools.
fn drag_capture(cx: &mut Context<Studio>) -> impl IntoElement {
    let on_move = cx.listener(Studio::move_sidebar_resize);
    let on_up = cx.listener(Studio::end_sidebar_resize);
    canvas(
        |_, _, _| (),
        move |_, _, window, _| {
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture {
                    on_move(event, window, cx);
                }
            });
            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture {
                    on_up(event, window, cx);
                }
            });
        },
    )
    .absolute()
    .size_full()
}

/// Vertical resize handle between the viewport and the sidebar: a hairline
/// with a wider invisible grip that lights up on hover.
fn sidebar_divider(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> impl IntoElement {
    let dragging = matches!(s.panels.drag, Some(PanelDrag::Sidebar { .. }));
    div()
        .relative()
        .w(px(DIVIDER_WIDTH))
        .h_full()
        .flex_shrink_0()
        .bg(rgb(t.hairline))
        .child(
            div()
                .id("sidebar-resize")
                .absolute()
                .top_0()
                .bottom_0()
                .left(px(-3.))
                .w(px(7.))
                .flex()
                .justify_center()
                .cursor(CursorStyle::ResizeLeftRight)
                .group("sidebar-resize")
                .child(
                    div()
                        .w(px(2.))
                        .h_full()
                        .when(dragging, |d| d.bg(rgb(t.accent)))
                        .group_hover("sidebar-resize", move |d| d.bg(rgb(t.accent_border))),
                )
                .tooltip(tip(t, "Drag to resize panels", Some("double-click resets")))
                .on_mouse_down(MouseButton::Left, cx.listener(Studio::begin_sidebar_resize)),
        )
}

/// Horizontal resize handle between the outliner and the inspector.
fn split_handle(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> impl IntoElement {
    let dragging = matches!(s.panels.drag, Some(PanelDrag::Outliner { .. }));
    div()
        .relative()
        .h(px(SPLIT_HANDLE))
        .w_full()
        .flex_shrink_0()
        .bg(rgb(t.hairline))
        .child(
            div()
                .id("outliner-resize")
                .absolute()
                .left_0()
                .right_0()
                .top(px(-3.))
                .h(px(7.))
                .flex()
                .items_center()
                .cursor(CursorStyle::ResizeUpDown)
                .group("outliner-resize")
                .child(
                    div()
                        .h(px(2.))
                        .w_full()
                        .when(dragging, |d| d.bg(rgb(t.accent)))
                        .group_hover("outliner-resize", move |d| d.bg(rgb(t.accent_border))),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(Studio::begin_outliner_resize),
                ),
        )
}

pub fn render(
    studio: &Studio,
    viewport: AnyElement,
    window: &Window,
    cx: &mut Context<Studio>,
) -> AnyElement {
    let t = studio.theme_colors();
    let width = f32::from(window.viewport_size().width);
    let sidebar_width = studio.panels.sidebar_width(width);
    let outliner_height = studio
        .panels
        .outliner_height(sidebar_height(window), studio.outliner_rows());
    col()
        .relative()
        .size_full()
        .overflow_hidden()
        .bg(rgb(t.canvas))
        .text_color(rgb(t.text))
        .text_size(px(TEXT_MD))
        .font_family(FONT_UI)
        .child(drag_capture(cx))
        .child(titlebar::titlebar(t, studio, window, cx))
        .child(
            row()
                .items_start()
                .flex_1()
                .min_h(px(0.))
                .w_full()
                .overflow_hidden()
                .child(hud::viewport_area(
                    t,
                    studio,
                    viewport,
                    width - sidebar_width - DIVIDER_WIDTH,
                    cx,
                ))
                .child(sidebar_divider(t, studio, cx))
                .child(
                    col()
                        .w(px(sidebar_width))
                        .h_full()
                        .flex_shrink_0()
                        .bg(rgb(t.surface))
                        .child(
                            outliner::outliner(t, studio, cx)
                                .h(px(outliner_height))
                                .flex_shrink_0(),
                        )
                        .child(split_handle(t, studio, cx))
                        .child(inspector::inspector(t, studio, sidebar_width, cx)),
                ),
        )
        .child(statusbar::statusbar(t, studio, window, cx))
        .children(overlays::overlays(t, studio, window, cx))
        .when(studio.panels.is_resizing(), |d| {
            d.child(
                div()
                    .absolute()
                    .inset_0()
                    .occlude()
                    .cursor(match studio.panels.drag {
                        Some(PanelDrag::Outliner { .. }) => CursorStyle::ResizeUpDown,
                        _ => CursorStyle::ResizeLeftRight,
                    }),
            )
        })
        .into_any_element()
}
