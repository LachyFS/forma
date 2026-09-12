use crate::code_editor::{CodeEditor, EditorEvent};
use crate::render_worker::{RenderWorker, Request};
use crate::shading_pie::ShadingPie;
use crate::theme::{Colors, Theme, ThemePicker};
use crate::{command_modifier, platform_shortcut};
use forma_core::{
    DenoiseQuality, History, Material, MeshInstance, Object, Primitive, Scene, ShaderKind,
    TextureMapping, TextureSlot,
};
use forma_render::{Backend, Frame, PreviewSettings, RenderMode, RenderSettings, StudioLight};
use glam::{Vec2, Vec3};
use gpui::{prelude::*, *};
use std::{cell::Cell, path::PathBuf, rc::Rc, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Move,
    Rotate,
    Scale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Name,
    Color(usize),
    Translation(usize),
    Rotation(usize),
    Scale(usize),
    Roughness,
    Ior,
    NormalStrength,
    TextureScale(usize),
    TextureOffset(usize),
    Metallic,
    Emission,
    Exposure,
    WorldStrength,
    Samples,
    Bounces,
    DenoiseStart,
    PreviewRotation,
    PreviewStrength,
    PreviewOpacity,
    PreviewBlur,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    New,
    Open,
    Save,
    SaveAs,
    ImportObj,
    ExportObj,
    ExportImage,
    Add(Primitive),
    Select(u64),
    ToggleVisible(u64),
    Delete,
    Duplicate,
    Undo,
    Redo,
    FrameSelected,
    ViewFront,
    ViewRight,
    ViewTop,
    ViewPerspective,
    ToggleProjection,
    SetMode(RenderMode),
    SetTool(Tool),
    ToggleEdit,
    Extrude,
    Subdivide,
    ToggleGrid,
    ToggleViewportDenoise,
    ToggleRenderDenoise,
    SetDenoiseQuality(DenoiseQuality),
    MaterialPreset(usize),
    SetShader(ShaderKind),
    SetTextureMapping(TextureMapping),
    LoadTexture(TextureSlot),
    ClearTexture(TextureSlot),
    EditShader,
    ApplyShader,
    CloseShader,
    TogglePalette,
    ToggleTheme,
    ToggleHelp,
    TogglePreviewSettings,
    SetPreviewStudio(StudioLight),
    TogglePreviewWorld,
    TogglePreviewAo,
    LoadPreviewHdri,
    ResetPreview,
}

pub(crate) struct TransformDrag {
    pub tool: Tool,
    pub axis: Option<usize>,
    pub start: Vec2,
    pub original: Object,
    pub before: Scene,
    pub numeric: String,
    pub modal: bool,
    pub changed: bool,
}

pub struct Studio {
    pub scene: Scene,
    pub history: History,
    pub selected: Option<u64>,
    pub selected_face: Option<usize>,
    pub edit_mode: bool,
    pub settings: RenderSettings,
    pub samples: u32,
    pub denoised_generation: Option<u64>,
    pub denoise_error: Option<String>,
    pub render_ms: f64,
    pub device_name: String,
    pub status: String,
    pub project_name: String,
    pub dirty: bool,
    pub(crate) theme: Theme,
    pub(crate) theme_picker: Option<ThemePicker>,
    pub(crate) theme_path: Option<PathBuf>,
    pub palette_open: bool,
    pub palette_query: String,
    pub palette_index: usize,
    pub help_open: bool,
    pub preview_open: bool,
    pub preview_loading: bool,
    pub shader_editor: Option<Entity<CodeEditor>>,
    pub renderer_backend: Backend,
    pub shader_message: Option<String>,
    pub shader_compiling: bool,
    pub texture_loading: bool,
    shader_target: Option<u64>,
    shader_request_id: u64,
    texture_request_id: u64,
    pub(crate) panels: crate::ui::PanelState,
    pub(crate) shading_pie: Option<ShadingPie>,
    pub active_field: Option<(Field, String)>,
    pub tool: Tool,
    pub render_error: Option<String>,
    pub(crate) focus: FocusHandle,
    pub(crate) bounds: Rc<Cell<Bounds<Pixels>>>,
    pub(crate) viewport_scale: f32,
    pub(crate) frame: Option<Frame>,
    pub(crate) frame_image: Option<Arc<RenderImage>>,
    pub(crate) last_mouse: Vec2,
    pub(crate) navigation: Option<(MouseButton, bool)>,
    pub(crate) trackpad_gesture: crate::viewport::TrackpadGesture,
    pub(crate) transform_drag: Option<TransformDrag>,
    #[cfg(target_os = "macos")]
    _pinch_monitor: Option<crate::trackpad::PinchMonitor>,
    worker: RenderWorker,
    path: Option<PathBuf>,
    scene_revision: u64,
    generation: u64,
    field_replace: bool,
    exporting: bool,
    prompt_open: bool,
    scene_snapshot: Arc<Scene>,
    needs_render: bool,
    needs_snapshot: bool,
    project_epoch: u64,
    saving: bool,
    loading: bool,
    exporting_obj: bool,
    preview_load_id: u64,
}

impl Studio {
    pub fn new(
        backend: Backend,
        theme_path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (theme, theme_error) = match theme_path.as_deref().map(Theme::load).transpose() {
            Ok(theme) => (theme.unwrap_or_default(), None),
            Err(error) => (
                Theme::default(),
                Some(format!("Could not load color theme: {error}")),
            ),
        };
        let focus = cx.focus_handle();
        window.focus(&focus);
        let scene = Scene::default();
        let selected = scene.objects.first().map(|o| o.id);
        let (worker, notifications) = RenderWorker::new(backend);
        cx.on_release(|studio, cx| {
            if let Some(image) = studio.frame_image.take() {
                cx.drop_image(image, None);
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            while notifications.recv().await.is_ok() {
                if this.update(cx, |this, cx| this.receive_output(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        let entity = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            entity
                .update(cx, |s, cx| s.should_close(window, cx))
                .unwrap_or(true)
        });
        let scene_snapshot = Arc::new(scene.clone());
        cx.observe_window_activation(window, |s, w, cx| {
            if !w.is_window_active() {
                s.close_shading_pie(cx);
            }
        })
        .detach();
        cx.observe_window_bounds(window, |s, w, cx| {
            s.viewport_scale = w.scale_factor();
            s.resize_viewport(cx);
            s.close_shading_pie(cx);
        })
        .detach();
        let mut studio = Self {
            scene,
            history: History::default(),
            selected,
            selected_face: None,
            edit_mode: false,
            settings: RenderSettings {
                mode: RenderMode::MaterialPreview,
                width: 1000,
                height: 760,
                max_samples: 128,
                max_bounces: 8,
                exposure: 0.,
                show_grid: true,
                selected,
                preview: PreviewSettings::default(),
                denoise: Default::default(),
            },
            samples: 0,
            denoised_generation: None,
            denoise_error: None,
            render_ms: 0.,
            device_name: "Starting GPU renderer…".into(),
            status: theme_error.unwrap_or_else(|| "Ready · Select an object to begin".into()),
            project_name: "Studio study".into(),
            dirty: false,
            theme,
            theme_picker: None,
            theme_path,
            palette_open: false,
            palette_query: String::new(),
            palette_index: 0,
            help_open: false,
            preview_open: false,
            preview_loading: false,
            shader_editor: None,
            renderer_backend: backend,
            shader_message: None,
            shader_compiling: false,
            texture_loading: false,
            shader_target: None,
            shader_request_id: 0,
            texture_request_id: 0,
            panels: crate::ui::PanelState::default(),
            shading_pie: None,
            active_field: None,
            tool: Tool::Select,
            render_error: None,
            focus,
            bounds: Rc::new(Cell::new(Bounds::default())),
            viewport_scale: window.scale_factor(),
            frame: None,
            frame_image: None,
            last_mouse: Vec2::ZERO,
            navigation: None,
            trackpad_gesture: Default::default(),
            transform_drag: None,
            #[cfg(target_os = "macos")]
            _pinch_monitor: crate::trackpad::PinchMonitor::install(window, cx),
            worker,
            path: None,
            scene_revision: 1,
            generation: 0,
            field_replace: false,
            exporting: false,
            prompt_open: false,
            scene_snapshot,
            needs_render: false,
            needs_snapshot: false,
            project_epoch: 0,
            saving: false,
            loading: false,
            exporting_obj: false,
            preview_load_id: 0,
        };
        studio.invalidate(false, cx);
        studio
    }

    pub(crate) fn theme_colors(&self) -> Colors {
        self.theme_picker
            .as_ref()
            .map_or(self.theme, ThemePicker::preview)
            .colors()
    }

    pub(crate) fn cancel_theme(&mut self, cx: &mut Context<Self>) {
        self.theme_picker = None;
        cx.notify();
    }

    pub(crate) fn apply_theme(&mut self, theme: Theme, cx: &mut Context<Self>) {
        self.theme = theme;
        self.theme_picker = None;
        self.status = match self.theme_path.as_deref() {
            Some(path) => match theme.save(path) {
                Ok(()) => format!("Color theme · {}", theme.name()),
                Err(error) => format!(
                    "{} applied for this session · Could not save theme: {error}",
                    theme.name()
                ),
            },
            None => format!(
                "{} applied for this session · No preferences directory available",
                theme.name()
            ),
        };
        cx.notify();
    }

    pub fn selected_object(&self) -> Option<MeshInstance<'_>> {
        self.selected.and_then(|id| self.scene.mesh_instance(id))
    }

    pub(crate) fn should_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.dirty {
            return true;
        }
        if self.prompt_open {
            return false;
        }
        self.prompt_open = true;
        let answer = window.prompt(
            PromptLevel::Warning,
            "Close this project?",
            Some("This project has unsaved changes. Save it before closing to keep your work."),
            &["Cancel", "Discard changes"],
            cx,
        );
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let discard = answer.await.ok() == Some(1);
            let _ = this.update(cx, |s, _| {
                s.prompt_open = false;
            });
            if discard {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
        })
        .detach();
        false
    }

    fn request(&self) -> Request {
        Request {
            scene: self.scene_snapshot.clone(),
            camera: self.scene.camera,
            world: self.scene.world.clone(),
            settings: self.settings.clone(),
            revision: self.scene_revision,
            generation: self.generation,
        }
    }

    pub(crate) fn invalidate(&mut self, geometry: bool, cx: &mut Context<Self>) {
        self.update_viewport_dimensions();
        if geometry {
            self.scene_revision += 1;
            self.needs_snapshot = true;
        }
        self.generation += 1;
        self.denoised_generation = None;
        self.denoise_error = None;
        self.settings.selected = self.selected;
        self.samples = 0;
        if !self.needs_render {
            self.needs_render = true;
            let entity = cx.weak_entity();
            // Flush after this input/effect cycle, with no timer delay. Multiple
            // edits in one cycle share a single expensive geometry snapshot.
            cx.defer(move |cx| {
                let _ = entity.update(cx, |studio, _| studio.submit_render());
            });
        }
        cx.notify();
    }

    fn submit_render(&mut self) {
        if self.needs_snapshot {
            self.scene_snapshot = Arc::new(self.scene.clone());
            self.needs_snapshot = false;
        }
        self.worker.request(self.request());
        self.needs_render = false;
    }

    fn receive_output(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        {
            let mut output = self.worker.take_output();
            if let Some(name) = output.device.take() {
                self.device_name = name;
                changed = true;
            }
            if let Some((generation, frame, image)) = output.frame.take() {
                // Display the latest completed work while input advances. Requiring an
                // exact generation here would starve presentation throughout a drag.
                self.samples = if generation == self.generation {
                    frame.samples
                } else {
                    0
                };
                if let Some(error) = &frame.shader_error {
                    self.status = error.clone();
                }
                self.render_ms = frame.elapsed_ms;
                // Retain the clean image as the raw estimator keeps refining.
                // Switching back to raw between denoising updates causes flicker.
                if self.denoised_generation != Some(self.generation) {
                    self.frame = Some(frame);
                    if let Some(previous) = std::mem::replace(&mut self.frame_image, image) {
                        cx.drop_image(previous, None);
                    }
                }
                self.render_error = None;
                changed = true;
            }
            if let Some((generation, frame, image)) = output.denoised.take()
                && generation == self.generation
                && self.settings.mode.progressive()
                && self.settings.denoise.viewport
            {
                self.denoised_generation = Some(generation);
                self.denoise_error = None;
                self.samples = self.samples.max(frame.samples);
                self.frame = Some(frame);
                if let Some(previous) = self.frame_image.replace(image) {
                    cx.drop_image(previous, None);
                }
                changed = true;
            }
            if let Some((generation, error)) = output.denoise_error.take()
                && generation == self.generation
                && self.settings.denoise.viewport
            {
                self.denoise_error = Some(error);
                changed = true;
            }
            if let Some((generation, error)) = output.render_error.take()
                && (generation == self.generation || (generation == 0 && self.frame.is_none()))
            {
                self.render_error = Some(error.clone());
                self.status = error;
                changed = true;
            }
            while let Some(message) = output.messages.pop_front() {
                match message {
                    Ok(message) => self.status = message,
                    Err(error) => {
                        self.status = error.clone();
                        self.render_error = Some(error);
                    }
                }
                changed = true;
            }
            if output.export_finished {
                self.exporting = false;
                changed = true;
            }
        }
        if changed {
            cx.notify();
        }
    }

    /// Called after viewport layout and when a window changes its backing scale.
    pub(crate) fn resize_viewport(&mut self, cx: &mut Context<Self>) {
        if self.update_viewport_dimensions() {
            self.invalidate(false, cx);
        }
    }

    fn update_viewport_dimensions(&mut self) -> bool {
        let bounds = self.bounds.get();
        let (width, height) = crate::viewport::render_dimensions(
            bounds.size,
            self.viewport_scale,
            self.settings.mode,
        );
        if width > 32
            && height > 32
            && (width != self.settings.width || height != self.settings.height)
        {
            self.settings.width = width;
            self.settings.height = height;
            return true;
        }
        false
    }

    pub fn execute(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = &self.shader_editor {
            if matches!(command, Command::Undo | Command::Redo) {
                editor.update(cx, |editor, cx| editor.undo(command == Command::Redo, cx));
                return;
            }
            if !matches!(command, Command::ApplyShader | Command::CloseShader) {
                return;
            }
        }
        window.focus(&self.focus);
        self.shading_pie = None;
        if command != Command::ToggleTheme {
            self.theme_picker = None;
        }
        if !matches!(
            command,
            Command::TogglePreviewSettings
                | Command::SetPreviewStudio(_)
                | Command::TogglePreviewWorld
                | Command::TogglePreviewAo
                | Command::LoadPreviewHdri
                | Command::ResetPreview
        ) {
            self.preview_open = false;
        }
        if self.transform_drag.is_some() {
            self.finish_transform(false, cx);
        }
        self.active_field = None;
        if !matches!(command, Command::TogglePalette | Command::ToggleHelp) {
            self.palette_open = false;
            self.help_open = false;
        }
        match command {
            Command::New | Command::Open if self.dirty => {
                if self.prompt_open {
                    return;
                }
                self.prompt_open = true;
                let answer = window.prompt(
                    PromptLevel::Warning,
                    "Replace this project?",
                    Some("Save your current project first if you want to keep the changes."),
                    &["Cancel", "Discard changes"],
                    cx,
                );
                cx.spawn(async move |this, cx| {
                    let discard = answer.await.ok() == Some(1);
                    let _ = this.update(cx, |s, cx| {
                        s.prompt_open = false;
                        if discard {
                            if command == Command::New {
                                s.new_scene(cx);
                            } else {
                                s.choose_open(false, cx);
                            }
                        }
                    });
                })
                .detach();
            }
            Command::New => self.new_scene(cx),
            Command::Open => self.choose_open(false, cx),
            Command::ImportObj => self.choose_open(true, cx),
            Command::Save => {
                if let Some(path) = self.path.clone() {
                    self.save_to(path, cx);
                } else {
                    self.choose_save(Command::SaveAs, cx);
                }
            }
            Command::SaveAs | Command::ExportObj | Command::ExportImage => {
                self.choose_save(command, cx)
            }
            Command::Add(primitive) => {
                self.history.checkpoint(&self.scene);
                let id = self.scene.add(primitive);
                self.selected = Some(id);
                self.selected_face = None;
                self.dirty = true;
                self.status = format!("Added {}", primitive.label());
                self.invalidate(true, cx);
            }
            Command::Select(id) => {
                self.selected = Some(id);
                self.selected_face = None;
                self.status = self
                    .scene
                    .object(id)
                    .map(|o| format!("Selected {}", o.name))
                    .unwrap_or_default();
                self.invalidate(false, cx);
            }
            Command::ToggleVisible(id) => {
                self.history.checkpoint(&self.scene);
                if let Some(object) = self.scene.object_mut(id) {
                    object.visible = !object.visible;
                }
                self.dirty = true;
                self.invalidate(true, cx);
            }
            Command::Delete => {
                if let Some(id) = self.selected {
                    self.history.checkpoint(&self.scene);
                    self.scene.remove(id);
                    self.selected = None;
                    self.selected_face = None;
                    self.dirty = true;
                    self.status = platform_shortcut(
                        "Object deleted · ⌘Z to undo",
                        "Object deleted · Ctrl+Z to undo",
                    )
                    .into();
                    self.invalidate(true, cx);
                }
            }
            Command::Duplicate => {
                if let Some(id) = self.selected {
                    self.history.checkpoint(&self.scene);
                    self.selected = self.scene.duplicate(id);
                    self.selected_face = None;
                    self.dirty = true;
                    self.status = "Object duplicated".into();
                    self.invalidate(true, cx);
                }
            }
            Command::Undo | Command::Redo => {
                let changed = if command == Command::Undo {
                    self.history.undo(&mut self.scene)
                } else {
                    self.history.redo(&mut self.scene)
                };
                if changed {
                    self.restore_render_preferences();
                    if self
                        .selected
                        .is_some_and(|id| self.scene.object(id).is_none())
                    {
                        self.selected = self.scene.objects.first().map(|o| o.id);
                    }
                    self.selected_face = None;
                    self.dirty = true;
                    self.status = if command == Command::Undo {
                        "Undone"
                    } else {
                        "Redone"
                    }
                    .into();
                    self.invalidate(true, cx);
                }
            }
            Command::FrameSelected => {
                if let Some((center, radius)) = self.selected.and_then(|id| self.scene.bounds(id)) {
                    self.scene.camera.frame(center, radius);
                    self.invalidate(false, cx);
                }
            }
            Command::ViewFront
            | Command::ViewRight
            | Command::ViewTop
            | Command::ViewPerspective => {
                let camera = &mut self.scene.camera;
                match command {
                    Command::ViewFront => {
                        camera.yaw = 0.;
                        camera.pitch = 0.;
                        camera.orthographic = true;
                    }
                    Command::ViewRight => {
                        camera.yaw = std::f32::consts::FRAC_PI_2;
                        camera.pitch = 0.;
                        camera.orthographic = true;
                    }
                    Command::ViewTop => {
                        camera.yaw = 0.;
                        camera.pitch = std::f32::consts::FRAC_PI_2;
                        camera.orthographic = true;
                    }
                    _ => {
                        camera.yaw = 0.65;
                        camera.pitch = 0.36;
                        camera.orthographic = false;
                    }
                }
                self.invalidate(false, cx);
            }
            Command::ToggleProjection => {
                self.scene.camera.orthographic = !self.scene.camera.orthographic;
                self.invalidate(false, cx);
            }
            Command::SetMode(mode) => {
                if self.settings.mode != mode {
                    self.settings.mode = mode;
                    self.invalidate(false, cx);
                }
                self.status = format!("{} viewport", mode.label());
            }
            Command::SetTool(tool) => {
                self.tool = tool;
                self.status = format!("{tool:?} tool · Drag the selected object or an axis handle");
            }
            Command::ToggleEdit => {
                self.edit_mode = !self.edit_mode;
                self.selected_face = None;
                self.status = if self.edit_mode {
                    "Face mode · Click a face, then E to extrude or G / R / S to transform"
                } else {
                    "Object mode"
                }
                .into();
            }
            Command::Extrude => {
                if let (Some(id), Some(face)) = (self.selected, self.selected_face) {
                    let before = self.scene.clone();
                    let result = self
                        .scene
                        .object_mesh_mut(id)
                        .unwrap()
                        .extrude_face(face, 0.3);
                    match result {
                        Ok(()) => {
                            self.history.checkpoint(&before);
                            self.dirty = true;
                            self.status = "Face extruded 0.30 m · G to move the face".into();
                            self.invalidate(true, cx);
                        }
                        Err(error) => self.status = format!("Extrusion: {error}"),
                    }
                } else {
                    self.status = "Enter face mode (Tab), select a face, then extrude (E)".into();
                }
            }
            Command::Subdivide => {
                if let Some(id) = self.selected {
                    if self.scene.object_mesh(id).unwrap().faces.len() > 100_000 {
                        self.status = "Subdivision limit reached (100k input faces)".into();
                    } else {
                        let before = self.scene.clone();
                        match self.scene.object_mesh_mut(id).unwrap().subdivide_checked() {
                            Ok(()) => {
                                self.history.checkpoint(&before);
                                self.selected_face = None;
                                self.dirty = true;
                                self.status = "Catmull–Clark subdivision applied".into();
                                self.invalidate(true, cx);
                            }
                            Err(error) => self.status = format!("Subdivision: {error:#}"),
                        }
                    }
                }
            }
            Command::ToggleViewportDenoise
            | Command::ToggleRenderDenoise
            | Command::SetDenoiseQuality(_) => {
                self.history.checkpoint(&self.scene);
                match command {
                    Command::ToggleViewportDenoise => {
                        self.settings.denoise.viewport = !self.settings.denoise.viewport
                    }
                    Command::ToggleRenderDenoise => {
                        self.settings.denoise.render = !self.settings.denoise.render
                    }
                    Command::SetDenoiseQuality(quality) => self.settings.denoise.quality = quality,
                    _ => unreachable!(),
                }
                self.scene.render.denoise = self.settings.denoise;
                self.dirty = true;
                self.status = "Denoising updated".into();
                self.invalidate(false, cx);
            }
            Command::ToggleGrid => {
                self.settings.show_grid = !self.settings.show_grid;
                self.invalidate(false, cx);
            }
            Command::SetShader(ShaderKind::Custom) | Command::EditShader => {
                self.open_shader_editor(window, cx)
            }
            Command::SetShader(kind) => {
                self.edit_material(cx, |m| {
                    if kind == ShaderKind::Glass && m.shader != kind {
                        m.base_color = Vec3::ONE;
                        m.roughness = 0.06;
                        m.metallic = 0.0;
                        m.emission = Vec3::ZERO;
                    }
                    m.shader = kind;
                });
            }
            Command::SetTextureMapping(mapping) => self.edit_material(cx, |m| m.mapping = mapping),
            Command::LoadTexture(slot) => self.choose_texture(slot, cx),
            Command::ClearTexture(slot) => {
                self.texture_request_id += 1;
                self.texture_loading = false;
                self.edit_material(cx, |m| m.textures[slot as usize] = None);
            }
            Command::ApplyShader => {
                self.apply_shader(cx);
                if let Some(editor) = &self.shader_editor {
                    window.focus(&editor.read(cx).focus);
                }
            }
            Command::ToggleTheme => {
                self.theme_picker = if self.theme_picker.is_some() {
                    None
                } else {
                    Some(ThemePicker::new(self.theme))
                };
                self.navigation = None;
            }
            Command::CloseShader => {
                self.shader_request_id += 1;
                self.shader_editor = None;
                self.shader_target = None;
                self.shader_compiling = false;
                self.shader_message = None;
            }
            Command::MaterialPreset(preset) => {
                let (color, metallic, roughness, emission) = match preset {
                    1 => (Vec3::new(0.73, 0.76, 0.73), 0., 0.28, Vec3::ZERO),
                    2 => (Vec3::new(0.76, 0.32, 0.13), 1., 0.23, Vec3::ZERO),
                    3 => (Vec3::splat(0.055), 0.72, 0.3, Vec3::ZERO),
                    4 => (Vec3::new(0.9, 0.83, 0.66), 0., 0.5, Vec3::new(8., 7.4, 6.)),
                    _ => (Vec3::new(0.045, 0.42, 0.32), 0.45, 0.24, Vec3::ZERO),
                };
                self.edit_material(cx, |m| {
                    m.shader = ShaderKind::Pbr;
                    m.base_color = color;
                    m.metallic = metallic;
                    m.roughness = roughness;
                    m.emission = emission;
                });
            }
            Command::TogglePalette => {
                self.palette_open = !self.palette_open;
                self.palette_query.clear();
                self.palette_index = 0;
                self.help_open = false;
            }
            Command::ToggleHelp => {
                self.help_open = !self.help_open;
                self.palette_open = false;
            }
            Command::TogglePreviewSettings => {
                self.preview_open = !self.preview_open;
                self.navigation = None;
                if self.preview_open && self.settings.mode != RenderMode::MaterialPreview {
                    self.settings.mode = RenderMode::MaterialPreview;
                    self.invalidate(false, cx);
                }
            }
            Command::SetPreviewStudio(studio) => {
                self.preview_load_id += 1;
                self.preview_loading = false;
                self.settings.preview.studio = studio;
                self.settings.preview.hdri_path = None;
                self.settings.preview.use_scene_world = false;
                self.status = format!("{} environment", studio.label());
                self.invalidate(false, cx);
            }
            Command::TogglePreviewWorld => {
                self.preview_load_id += 1;
                self.preview_loading = false;
                self.settings.preview.use_scene_world = !self.settings.preview.use_scene_world;
                self.status = if self.settings.preview.use_scene_world {
                    "Preview uses the scene world"
                } else {
                    "Preview uses environment lighting"
                }
                .into();
                self.invalidate(false, cx);
            }
            Command::TogglePreviewAo => {
                self.settings.preview.ambient_occlusion = !self.settings.preview.ambient_occlusion;
                self.invalidate(false, cx);
            }
            Command::ResetPreview => {
                self.preview_load_id += 1;
                self.preview_loading = false;
                self.settings.preview = PreviewSettings::default();
                self.status = "Preview lighting reset".into();
                self.invalidate(false, cx);
            }
            Command::LoadPreviewHdri => self.choose_preview_hdri(cx),
        }
        cx.notify();
    }

    fn new_scene(&mut self, cx: &mut Context<Self>) {
        self.preview_load_id += 1;
        self.preview_loading = false;
        self.project_epoch += 1;
        self.scene = Scene::default();
        self.history = History::default();
        self.restore_render_preferences();
        self.selected = self.scene.objects.first().map(|o| o.id);
        self.selected_face = None;
        self.path = None;
        self.project_name = "Untitled".into();
        self.dirty = false;
        self.status = "New studio scene".into();
        self.invalidate(true, cx);
    }

    fn choose_preview_hdri(&mut self, cx: &mut Context<Self>) {
        if self.preview_loading {
            return;
        }
        self.preview_load_id += 1;
        let load_id = self.preview_load_id;
        self.preview_loading = true;
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Load Radiance HDR environment (.hdr)".into()),
        });
        cx.spawn(async move |this, cx| {
            let result = receiver.await;
            let _ = this.update(cx, |s, cx| {
                if s.preview_load_id != load_id {
                    return;
                }
                s.preview_loading = false;
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            s.load_preview_hdri(path, cx);
                        }
                    }
                    Ok(Err(error)) => s.status = format!("File dialog: {error}"),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn load_preview_hdri(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.preview_load_id += 1;
        let load_id = self.preview_load_id;
        let epoch = self.project_epoch;
        self.preview_loading = true;
        self.status = "Loading HDR environment…".into();
        let source = path.clone();
        let decode = cx
            .background_executor()
            .spawn(async move { forma_render::validate_hdri(&source) });
        cx.spawn(async move |this, cx| {
            let result = decode.await;
            let _ = this.update(cx, |s, cx| {
                if s.preview_load_id != load_id {
                    return;
                }
                s.preview_loading = false;
                if s.project_epoch == epoch {
                    match result {
                        Ok(()) => {
                            s.status = format!("HDR environment · {}", file_stem(&path));
                            s.settings.preview.hdri_path = Some(path);
                            s.settings.preview.use_scene_world = false;
                            s.invalidate(false, cx);
                        }
                        Err(error) => s.status = format!("HDR environment: {error:#}"),
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn choose_open(&mut self, import: bool, cx: &mut Context<Self>) {
        if self.loading {
            self.status = "A project open or import is already in progress".into();
            cx.notify();
            return;
        }
        let epoch = self.project_epoch;
        let generation = self.generation;
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(
                if import {
                    "Import Wavefront OBJ"
                } else {
                    "Open Forma project"
                }
                .into(),
            ),
        });
        cx.spawn(async move |this, cx| {
            let result = receiver.await;
            let _ = this.update(cx, |s, cx| {
                if !s.accepts_document_result(epoch, generation) {
                    return;
                }
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.first() {
                            s.open_from(path.clone(), import, cx);
                        }
                    }
                    Ok(Err(error)) => s.status = format!("File dialog: {error}"),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn accepts_document_result(&self, epoch: u64, generation: u64) -> bool {
        self.project_epoch == epoch
            && self.generation == generation
            && self.transform_drag.is_none()
            && self.active_field.is_none()
    }

    pub(crate) fn open_from(&mut self, path: PathBuf, import: bool, cx: &mut Context<Self>) {
        if self.loading {
            self.status = "A project open or import is already in progress".into();
            cx.notify();
            return;
        }
        self.loading = true;
        self.status = if import {
            "Importing OBJ…"
        } else {
            "Opening project…"
        }
        .into();
        let epoch = self.project_epoch;
        let generation = self.generation;
        // The worker owns its candidate. Neither parsing failures nor rejected
        // completions can mutate the live document or its undo history.
        let snapshot = import.then(|| self.scene.clone());
        let source = path.clone();
        let load = cx.background_executor().spawn(async move {
            if let Some(mut scene) = snapshot {
                let id = scene.import_obj(&source)?;
                if let Some((center, radius)) = scene.bounds(id) {
                    scene.camera.frame(center, radius);
                }
                Ok::<_, anyhow::Error>((scene, Some(id)))
            } else {
                Scene::load(&source).map(|scene| (scene, None))
            }
        });
        cx.spawn(async move |this, cx| {
            let result = load.await;
            let _ = this.update(cx, |s, cx| {
                s.loading = false;
                if !s.accepts_document_result(epoch, generation) {
                    if s.project_epoch == epoch {
                        s.status = if import {
                            "Import skipped because the project changed · Import again to apply it"
                        } else {
                            "Open skipped because the project changed · Open again to replace it"
                        }
                        .into();
                        cx.notify();
                    }
                    return;
                }
                match result {
                    Ok((scene, Some(id))) => {
                        s.history.checkpoint(&s.scene);
                        s.scene = scene;
                        s.selected = Some(id);
                        s.selected_face = None;
                        s.dirty = true;
                        s.navigation = None;
                        s.status = format!("Imported {}", path.display());
                        s.invalidate(true, cx);
                    }
                    Ok((scene, None)) => {
                        s.project_epoch += 1;
                        s.scene = scene;
                        s.restore_render_preferences();
                        s.history = History::default();
                        s.selected = s.scene.objects.first().map(|o| o.id);
                        s.selected_face = None;
                        s.navigation = None;
                        s.path = Some(path.clone());
                        s.project_name = file_stem(&path);
                        s.dirty = false;
                        s.status = "Project opened".into();
                        s.invalidate(true, cx);
                    }
                    Err(error) => {
                        s.status = format!(
                            "{} failed: {error:#}",
                            if import { "Import" } else { "Open" }
                        )
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn choose_save(&mut self, kind: Command, cx: &mut Context<Self>) {
        if kind == Command::ExportImage && self.exporting {
            self.status = "An image export is already running".into();
            return;
        }
        if kind == Command::ExportObj && self.exporting_obj {
            self.status = "An OBJ export is already running".into();
            return;
        }
        let epoch = self.project_epoch;
        let generation = self.generation;
        let extension = match kind {
            Command::ExportObj => "obj",
            Command::ExportImage => "png",
            _ => "forma",
        };
        let name = format!("{}.{}", self.project_name, extension);
        let directory = self
            .path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."));
        let receiver = cx.prompt_for_new_path(&directory, Some(&name));
        cx.spawn(async move |this, cx| {
            let result = receiver.await;
            let _ = this.update(cx, |s, cx| {
                if !s.accepts_document_result(epoch, generation) {
                    return;
                }
                match result {
                    Ok(Ok(Some(mut path))) => {
                        if path.extension().is_none() {
                            path.set_extension(extension);
                        }
                        match kind {
                            Command::ExportObj => {
                                s.export_obj_to(path, cx);
                            }
                            Command::ExportImage => {
                                s.export_image_to(path, cx);
                            }
                            _ => s.save_to(path, cx),
                        }
                    }
                    Ok(Err(error)) => s.status = format!("File dialog: {error}"),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn export_obj_to(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.exporting_obj {
            self.status = "An OBJ export is already running".into();
            cx.notify();
            return;
        }
        self.exporting_obj = true;
        self.status = "Exporting OBJ…".into();
        let scene = self.scene.clone();
        let generation = self.generation;
        let epoch = self.project_epoch;
        let destination = path.clone();
        let export = cx
            .background_executor()
            .spawn(async move { scene.export_obj(&destination) });
        cx.spawn(async move |this, cx| {
            let result = export.await;
            let _ = this.update(cx, |s, cx| {
                s.exporting_obj = false;
                if !s.accepts_document_result(epoch, generation) {
                    return;
                }
                s.status = match result {
                    Ok(()) => format!("OBJ exported · {}", path.display()),
                    Err(error) => format!("OBJ export failed: {error:#}"),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn export_image_to(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.exporting {
            self.status = "An image export is already running".into();
            cx.notify();
            return;
        }
        self.exporting = true;
        self.status = if self.settings.mode.progressive() {
            format!(
                "Rendering image · {} samples · You can keep editing",
                self.settings.max_samples
            )
        } else {
            format!("Exporting {} image…", self.settings.mode.label())
        };
        if self.needs_snapshot {
            self.scene_snapshot = Arc::new(self.scene.clone());
            self.needs_snapshot = false;
        }
        self.worker.export(self.request(), path);
        cx.notify();
    }

    pub(crate) fn save_to(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.saving {
            self.status = "A project save is already in progress".into();
            cx.notify();
            return;
        }
        self.saving = true;
        self.status = "Saving project…".into();
        let scene = self.scene.clone();
        let generation = self.generation;
        let epoch = self.project_epoch;
        let destination = path.clone();
        let save = cx
            .background_executor()
            .spawn(async move { scene.save(&destination) });
        cx.spawn(async move |this, cx| {
            let result = save.await;
            let _ = this.update(cx, |s, cx| {
                s.saving = false;
                if s.project_epoch != epoch {
                    return;
                }
                match result {
                    Ok(()) => {
                        s.project_name = file_stem(&path);
                        s.path = Some(path);
                        if s.generation == generation {
                            s.dirty = false;
                            s.status = "Project saved".into();
                        } else {
                            s.status = "Project saved · Newer changes remain unsaved".into();
                        }
                    }
                    Err(error) => s.status = format!("Save failed: {error:#}"),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn field_value(&self, field: Field) -> String {
        if let Some((active, text)) = &self.active_field
            && *active == field
        {
            return text.clone();
        }
        let object = self.selected_object();
        let value = match field {
            Field::Name => return object.map(|o| o.name.clone()).unwrap_or_default(),
            Field::Color(axis) => object.map(|o| linear_to_srgb(o.material.base_color[axis])),
            Field::Translation(axis) => object.map(|o| o.transform.translation[axis]),
            Field::Rotation(axis) => object.map(|o| o.transform.rotation[axis].to_degrees()),
            Field::Scale(axis) => object.map(|o| o.transform.scale[axis]),
            Field::Roughness => object.map(|o| o.material.roughness),
            Field::Ior => object.map(|o| o.material.ior),
            Field::NormalStrength => object.map(|o| o.material.normal_strength),
            Field::TextureScale(axis) => object.map(|o| o.material.texture_scale[axis]),
            Field::TextureOffset(axis) => object.map(|o| o.material.texture_offset[axis]),
            Field::Metallic => object.map(|o| o.material.metallic),
            Field::Emission => object.map(|o| o.material.emission.max_element()),
            Field::Exposure => Some(self.settings.exposure),
            Field::WorldStrength => Some(self.scene.world.strength),
            Field::PreviewRotation => Some(self.settings.preview.rotation.to_degrees()),
            Field::PreviewStrength => Some(self.settings.preview.strength),
            Field::PreviewOpacity => Some(self.settings.preview.world_opacity * 100.),
            Field::PreviewBlur => Some(self.settings.preview.background_blur * 100.),
            Field::Samples => return self.settings.max_samples.to_string(),
            Field::Bounces => return self.settings.max_bounces.to_string(),
            Field::DenoiseStart => return self.settings.denoise.start_sample.to_string(),
        };
        value
            .map(|v| format!("{v:.2}"))
            .unwrap_or_else(|| "—".into())
    }

    pub fn field_is_active(&self, field: Field) -> bool {
        self.active_field.as_ref().is_some_and(|(f, _)| *f == field)
    }

    pub fn begin_field(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        self.active_field = Some((field, self.field_value(field)));
        self.field_replace = true;
        self.status = "Enter a value · Return to apply · Esc to cancel".into();
        cx.notify();
    }

    fn commit_field(&mut self, cx: &mut Context<Self>) {
        let Some((field, text)) = self.active_field.take() else {
            return;
        };
        if field == Field::Name {
            let name = text.trim();
            if name.is_empty() || name.len() > 128 {
                self.status = "Use a name between 1 and 128 bytes".into();
                cx.notify();
                return;
            }
            self.history.checkpoint(&self.scene);
            if let Some(object) = self.selected.and_then(|id| self.scene.object_mut(id)) {
                object.name = name.to_owned();
            }
            self.dirty = true;
            self.status = "Object renamed".into();
            self.invalidate(true, cx);
            return;
        }
        let Ok(value) = text.parse::<f32>() else {
            self.status = "Enter a valid number".into();
            cx.notify();
            return;
        };
        if !value.is_finite() {
            self.status = "Value must be finite".into();
            cx.notify();
            return;
        }
        if matches!(
            field,
            Field::PreviewRotation
                | Field::PreviewStrength
                | Field::PreviewOpacity
                | Field::PreviewBlur
        ) {
            match field {
                Field::PreviewRotation => {
                    self.settings.preview.rotation =
                        value.to_radians().rem_euclid(std::f32::consts::TAU)
                }
                Field::PreviewStrength => self.settings.preview.strength = value.clamp(0., 10.),
                Field::PreviewOpacity => {
                    self.settings.preview.world_opacity = value.clamp(0., 100.) / 100.
                }
                Field::PreviewBlur => {
                    self.settings.preview.background_blur = value.clamp(0., 100.) / 100.
                }
                _ => unreachable!(),
            }
            self.status = "Preview lighting updated".into();
            self.invalidate(false, cx);
            return;
        }
        let geometry = !matches!(
            field,
            Field::Exposure
                | Field::WorldStrength
                | Field::Samples
                | Field::Bounces
                | Field::DenoiseStart
        );
        self.history.checkpoint(&self.scene);
        self.dirty = true;
        match field {
            Field::Exposure => self.settings.exposure = value.clamp(-10., 10.),
            Field::WorldStrength => self.scene.world.strength = value.clamp(0., 100.),
            Field::Samples => self.settings.max_samples = (value as u32).clamp(1, 4096),
            Field::Bounces => self.settings.max_bounces = (value as u32).clamp(1, 32),
            Field::DenoiseStart => {
                self.settings.denoise.start_sample = (value as u32).clamp(1, 4096)
            }
            _ => {
                if let Some(id) = self.selected {
                    match field {
                        Field::Color(axis) => {
                            self.scene.object_material_mut(id).unwrap().base_color[axis] =
                                srgb_to_linear(value.clamp(0., 1.))
                        }
                        Field::Roughness => {
                            self.scene.object_material_mut(id).unwrap().roughness =
                                value.clamp(0.02, 1.)
                        }
                        Field::Ior => {
                            self.scene.object_material_mut(id).unwrap().ior = value.clamp(1.01, 3.0)
                        }
                        Field::NormalStrength => {
                            self.scene.object_material_mut(id).unwrap().normal_strength =
                                value.clamp(0.0, 4.0)
                        }
                        Field::TextureScale(axis) => {
                            self.scene.object_material_mut(id).unwrap().texture_scale[axis] =
                                value.clamp(0.001, 1000.0)
                        }
                        Field::TextureOffset(axis) => {
                            self.scene.object_material_mut(id).unwrap().texture_offset[axis] =
                                value.clamp(-1000.0, 1000.0)
                        }
                        Field::Metallic => {
                            self.scene.object_material_mut(id).unwrap().metallic =
                                value.clamp(0., 1.)
                        }
                        Field::Emission => {
                            self.scene.object_material_mut(id).unwrap().emission =
                                Vec3::splat(value.clamp(0., 1000.))
                        }
                        Field::Translation(axis) => {
                            self.scene.object_mut(id).unwrap().transform.translation[axis] =
                                value.clamp(-100_000., 100_000.)
                        }
                        Field::Rotation(axis) => {
                            self.scene.object_mut(id).unwrap().transform.rotation[axis] =
                                value.to_radians().rem_euclid(std::f32::consts::TAU)
                        }
                        Field::Scale(axis) => {
                            self.scene.object_mut(id).unwrap().transform.scale[axis] =
                                value.clamp(0.001, 1000.)
                        }
                        _ => {}
                    }
                }
            }
        }
        self.scene.render.exposure = self.settings.exposure;
        self.scene.render.max_samples = self.settings.max_samples;
        self.scene.render.max_bounces = self.settings.max_bounces;
        self.scene.render.denoise = self.settings.denoise;
        self.status = "Value updated".into();
        self.invalidate(geometry, cx);
    }

    fn restore_render_preferences(&mut self) {
        self.settings.exposure = self.scene.render.exposure;
        self.settings.max_samples = self.scene.render.max_samples;
        self.settings.max_bounces = self.scene.render.max_bounces;
        self.settings.denoise = self.scene.render.denoise;
    }

    pub(crate) fn open_shading_pie(&mut self, pointer: Vec2, cx: &mut Context<Self>) {
        self.preview_open = false;
        let bounds = self.bounds.get();
        let min = Vec2::new(bounds.left().into(), bounds.top().into());
        let max = Vec2::new(bounds.right().into(), bounds.bottom().into());
        self.navigation = None;
        self.shading_pie = Some(ShadingPie::new(pointer, min, max));
        cx.notify();
    }

    pub(crate) fn close_shading_pie(&mut self, cx: &mut Context<Self>) {
        if self.shading_pie.take().is_some() {
            cx.notify();
        }
    }

    pub(crate) fn on_key_up(
        &mut self,
        event: &KeyUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "z"
            && let Some(pie) = self.shading_pie.as_mut()
        {
            if let Some(mode) = pie.release_trigger() {
                self.execute(Command::SetMode(mode), window, cx);
            } else {
                cx.notify();
            }
            cx.stop_propagation();
        }
    }

    pub(crate) fn on_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let mods = event.keystroke.modifiers;
        if key == "t" && command_modifier(mods) && mods.shift {
            if !event.is_held {
                self.execute(Command::ToggleTheme, window, cx);
            }
            cx.stop_propagation();
            return;
        }
        if let Some(picker) = &mut self.theme_picker {
            match key {
                "escape" => self.cancel_theme(cx),
                "up" => picker.navigate(-1),
                "down" => picker.navigate(1),
                "enter" => {
                    if let Some(theme) = picker.selected() {
                        self.apply_theme(theme, cx);
                    }
                }
                "backspace" => {
                    picker.query.pop();
                    picker.index = 0;
                }
                _ => {
                    if !mods.platform
                        && !mods.control
                        && !mods.alt
                        && let Some(chars) = &event.keystroke.key_char
                        && !chars.chars().any(char::is_control)
                        && picker.query.len() < 80
                    {
                        picker.query.push_str(chars);
                        picker.index = 0;
                    }
                }
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if self.shading_pie.is_some() {
            if key == "escape" {
                self.close_shading_pie(cx);
            } else if !mods.platform && !mods.control && !mods.alt {
                if let Some(mode) = ShadingPie::mode_for_key(key) {
                    if matches!(key, "up" | "down" | "left" | "right") {
                        self.shading_pie.as_mut().unwrap().hovered = Some(mode);
                        cx.notify();
                    } else {
                        self.execute(Command::SetMode(mode), window, cx);
                    }
                } else if key == "enter"
                    && let Some(mode) = self.shading_pie.as_ref().and_then(|p| p.hovered)
                {
                    self.execute(Command::SetMode(mode), window, cx);
                }
            }
            cx.stop_propagation();
            return;
        }
        if self.shader_editor.is_some() {
            return;
        }
        if self.help_open {
            if matches!(key, "escape" | "h" | "?") {
                self.help_open = false;
                cx.notify();
            }
            cx.stop_propagation();
            return;
        }
        if self.palette_open {
            let commands = palette_commands(&self.palette_query);
            match key {
                "escape" => self.palette_open = false,
                "k" if command_modifier(mods) => self.palette_open = false,
                "up" => self.palette_index = self.palette_index.saturating_sub(1),
                "down" => {
                    self.palette_index =
                        (self.palette_index + 1).min(commands.len().saturating_sub(1))
                }
                "enter" => {
                    if let Some((_, _, command)) = commands.get(self.palette_index) {
                        self.palette_open = false;
                        self.execute(*command, window, cx);
                    }
                }
                "backspace" => {
                    self.palette_query.pop();
                    self.palette_index = 0;
                }
                _ => {
                    if !mods.platform
                        && !mods.control
                        && let Some(chars) = &event.keystroke.key_char
                        && !chars.chars().any(char::is_control)
                        && self.palette_query.len() < 80
                    {
                        self.palette_query.push_str(chars);
                        self.palette_index = 0;
                    }
                }
            }
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if self.active_field.is_some() {
            match key {
                "enter" => self.commit_field(cx),
                "escape" => {
                    self.active_field = None;
                    cx.notify();
                }
                "backspace" => {
                    let (_, text) = self.active_field.as_mut().unwrap();
                    if self.field_replace {
                        text.clear();
                    } else {
                        text.pop();
                    }
                    self.field_replace = false;
                    cx.notify();
                }
                _ => {
                    if let Some(chars) = &event.keystroke.key_char {
                        let is_name = self
                            .active_field
                            .as_ref()
                            .is_some_and(|(field, _)| *field == Field::Name);
                        if chars.chars().all(|c| {
                            (is_name && !c.is_control())
                                || c.is_ascii_digit()
                                || ".-+eE".contains(c)
                        }) {
                            let (_, text) = self.active_field.as_mut().unwrap();
                            if self.field_replace {
                                text.clear();
                                self.field_replace = false;
                            }
                            if text.len() < if is_name { 128 } else { 24 } {
                                text.push_str(chars);
                            }
                            cx.notify();
                        }
                    }
                }
            }
            cx.stop_propagation();
            return;
        }
        if self.preview_open {
            match key {
                "escape" => {
                    self.preview_open = false;
                    cx.notify();
                }
                "z" if !mods.platform && !mods.control && !mods.alt && !mods.shift => {
                    if !event.is_held {
                        let pointer = window.mouse_position();
                        self.open_shading_pie(Vec2::new(pointer.x.into(), pointer.y.into()), cx);
                    }
                }
                "k" if command_modifier(mods) => self.execute(Command::TogglePalette, window, cx),
                _ => {}
            }
            cx.stop_propagation();
            return;
        }
        if self.transform_drag.is_some() {
            match key {
                "escape" => self.finish_transform(true, cx),
                "enter" => self.finish_transform(false, cx),
                "x" | "y" | "z" => {
                    self.transform_drag.as_mut().unwrap().axis = Some(match key {
                        "x" => 0,
                        "y" => 1,
                        _ => 2,
                    });
                    self.update_transform(self.last_mouse, mods.shift, cx);
                }
                "backspace" => {
                    self.transform_drag.as_mut().unwrap().numeric.pop();
                    self.update_transform(self.last_mouse, mods.shift, cx);
                }
                _ => {
                    if let Some(chars) = &event.keystroke.key_char
                        && chars
                            .chars()
                            .all(|c| c.is_ascii_digit() || ".-".contains(c))
                    {
                        let drag = self.transform_drag.as_mut().unwrap();
                        if drag.numeric.len() < 16 {
                            drag.numeric.push_str(chars);
                        }
                        self.update_transform(self.last_mouse, mods.shift, cx);
                    }
                }
            }
            cx.stop_propagation();
            return;
        }
        let command = if command_modifier(mods) {
            match key {
                "s" if mods.shift => Some(Command::SaveAs),
                "s" => Some(Command::Save),
                "o" => Some(Command::Open),
                "n" => Some(Command::New),
                "z" if mods.shift => Some(Command::Redo),
                "z" => Some(Command::Undo),
                "y" if cfg!(not(target_os = "macos")) => Some(Command::Redo),
                "d" => Some(Command::Duplicate),
                "k" => Some(Command::TogglePalette),
                _ => None,
            }
        } else if mods.platform || mods.control {
            None
        } else {
            match key {
                "escape" => {
                    self.palette_open = false;
                    self.help_open = false;
                    self.preview_open = false;
                    cx.notify();
                    None
                }
                "g" | "r" | "s" => {
                    self.start_transform(
                        match key {
                            "g" => Tool::Move,
                            "r" => Tool::Rotate,
                            _ => Tool::Scale,
                        },
                        None,
                        true,
                        cx,
                    );
                    None
                }
                "tab" => Some(Command::ToggleEdit),
                "e" => Some(Command::Extrude),
                "d" if mods.shift => Some(Command::Duplicate),
                "a" if mods.shift => Some(Command::TogglePalette),
                "backspace" | "delete" => Some(Command::Delete),
                "f" => Some(Command::FrameSelected),
                "1" => Some(Command::ViewFront),
                "3" => Some(Command::ViewRight),
                "7" => Some(Command::ViewTop),
                "5" => Some(Command::ToggleProjection),
                "0" => Some(Command::ViewPerspective),
                "z" if !mods.alt && !mods.shift => {
                    if !event.is_held {
                        let pointer = window.mouse_position();
                        self.open_shading_pie(Vec2::new(pointer.x.into(), pointer.y.into()), cx);
                    }
                    cx.stop_propagation();
                    None
                }
                "x" => Some(Command::SetMode(RenderMode::Solid)),
                "c" => Some(Command::SetMode(RenderMode::MaterialPreview)),
                "v" => Some(Command::SetMode(RenderMode::Rendered)),
                "h" | "?" => Some(Command::ToggleHelp),
                "space" => Some(Command::TogglePalette),
                "q" => Some(Command::SetTool(Tool::Select)),
                _ => None,
            }
        };
        if let Some(command) = command {
            self.execute(command, window, cx);
            cx.stop_propagation();
        }
    }
}

impl Render for Studio {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = self.viewport(cx);
        let chrome = crate::ui::render(self, viewport, cx);
        div()
            .id("forma-root")
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_key_up(cx.listener(Self::on_key_up))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|s, _, _, cx| {
                    if s.transform_drag.as_ref().is_some_and(|d| !d.modal) {
                        s.finish_transform(false, cx);
                    }
                    s.navigation = None;
                }),
            )
            .child(chrome)
    }
}

fn file_stem(path: &std::path::Path) -> String {
    path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn linear_to_srgb(value: f32) -> f32 {
    if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1. / 2.4) - 0.055
    }
}
fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

pub(crate) fn palette_commands(query: &str) -> Vec<(&'static str, &'static str, Command)> {
    let query = query.to_lowercase();
    [
        (
            "New project",
            platform_shortcut("⌘ N", "Ctrl+N"),
            Command::New,
        ),
        (
            "Open project…",
            platform_shortcut("⌘ O", "Ctrl+O"),
            Command::Open,
        ),
        (
            "Save project",
            platform_shortcut("⌘ S", "Ctrl+S"),
            Command::Save,
        ),
        (
            "Save project as…",
            platform_shortcut("⇧ ⌘ S", "Ctrl+Shift+S"),
            Command::SaveAs,
        ),
        ("Add cube", "", Command::Add(Primitive::Cube)),
        ("Add sphere", "", Command::Add(Primitive::Sphere)),
        ("Add cylinder", "", Command::Add(Primitive::Cylinder)),
        ("Add torus", "", Command::Add(Primitive::Torus)),
        ("Add plane", "", Command::Add(Primitive::Plane)),
        ("Import Wavefront OBJ…", "", Command::ImportObj),
        ("Export Wavefront OBJ…", "", Command::ExportObj),
        ("Export image…", "", Command::ExportImage),
        ("Duplicate selection", "⇧ D", Command::Duplicate),
        ("Delete selection", "⌫", Command::Delete),
        ("Frame selected", "F", Command::FrameSelected),
        ("Subdivide mesh", "", Command::Subdivide),
        ("Extrude selected face", "E", Command::Extrude),
        (
            "Wireframe viewport",
            "Z, 4",
            Command::SetMode(RenderMode::Wireframe),
        ),
        (
            "PBR surface shader",
            "",
            Command::SetShader(ShaderKind::Pbr),
        ),
        (
            "Glass surface shader",
            "",
            Command::SetShader(ShaderKind::Glass),
        ),
        ("Custom surface shader", "", Command::EditShader),
        ("Solid viewport", "X", Command::SetMode(RenderMode::Solid)),
        (
            "Material preview",
            "C",
            Command::SetMode(RenderMode::MaterialPreview),
        ),
        (
            "Rendered viewport",
            "V",
            Command::SetMode(RenderMode::Rendered),
        ),
        ("Perspective view", "0", Command::ViewPerspective),
        ("Front view", "1", Command::ViewFront),
        ("Right view", "3", Command::ViewRight),
        ("Top view", "7", Command::ViewTop),
        ("Toggle grid", "", Command::ToggleGrid),
        (
            "Toggle viewport AI denoising",
            "",
            Command::ToggleViewportDenoise,
        ),
        (
            "Toggle render export AI denoising",
            "",
            Command::ToggleRenderDenoise,
        ),
        ("Keyboard shortcuts", "?", Command::ToggleHelp),
        (
            "Preferences: Color Theme",
            platform_shortcut("⇧ ⌘ T", "Ctrl+Shift+T"),
            Command::ToggleTheme,
        ),
        (
            "Material preview lighting",
            "",
            Command::TogglePreviewSettings,
        ),
    ]
    .into_iter()
    .filter(|(label, _, _)| {
        query
            .split_whitespace()
            .all(|word| label.to_lowercase().contains(word))
    })
    .collect()
}

impl Studio {
    fn selected_material_id(&self) -> Option<u64> {
        self.scene
            .object(self.selected?)?
            .data
            .material_ids()
            .first()
            .copied()
    }
    fn edit_material(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut Material)) {
        let Some(id) = self.selected_material_id() else {
            return;
        };
        let mut material = self.scene.material(id).unwrap().material.clone();
        edit(&mut material);
        if let Err(error) = material.validate() {
            self.status = error.to_string();
            return;
        }
        if self.scene.material(id).unwrap().material == material {
            return;
        }
        self.history.checkpoint(&self.scene);
        self.scene.material_mut(id).unwrap().material = material;
        self.dirty = true;
        self.status = "Material updated".into();
        self.invalidate(true, cx);
    }
    fn open_shader_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected_material_id() else {
            return;
        };
        let material = &self.scene.material(id).unwrap().material;
        let language = self.renderer_backend.shader_language();
        let mismatch = material.custom_language != language;
        let code = if mismatch && material.custom_code == material.custom_language.default_code() {
            language.default_code().to_owned()
        } else {
            material.custom_code.clone()
        };
        let language_message = mismatch.then(|| format!(
            "This renderer uses {}. Applying replaces the saved {} body; adapt any existing code before applying.",
            language.label(), material.custom_language.label()));
        let editor = cx.new(|cx| CodeEditor::new(code, self.theme_colors(), cx));
        cx.subscribe_in(&editor, window, |s, _, event, w, cx| {
            s.execute(
                match event {
                    EditorEvent::Apply => Command::ApplyShader,
                    EditorEvent::Cancel => Command::CloseShader,
                },
                w,
                cx,
            );
        })
        .detach();
        window.focus(&editor.read(cx).focus);
        self.shader_editor = Some(editor);
        self.shader_target = Some(id);
        self.shader_message = self
            .frame
            .as_ref()
            .and_then(|frame| frame.shader_error.clone())
            .or(language_message);
        self.shader_compiling = false;
        self.shader_request_id += 1;
        self.preview_open = false;
        cx.notify();
    }
    fn apply_shader(&mut self, cx: &mut Context<Self>) {
        if self.shader_compiling {
            return;
        }
        let (Some(editor), Some(id)) = (&self.shader_editor, self.shader_target) else {
            return;
        };
        let code = editor.read(cx).text.clone();
        let Some(data) = self.scene.material(id) else {
            return;
        };
        let original = data.material.clone();
        let mut scene = self.scene.clone();
        let material = &mut scene.material_mut(id).unwrap().material;
        let backend = self.renderer_backend;
        let language = backend.shader_language();
        material.custom_code = code.clone();
        material.custom_language = language;
        material.shader = ShaderKind::Custom;
        if let Err(error) = scene.validate() {
            self.shader_message = Some(format!("{error:#}"));
            cx.notify();
            return;
        }
        self.shader_compiling = true;
        self.shader_message = Some("Compiling Preview and Rendered shaders…".into());
        self.shader_request_id += 1;
        let request = self.shader_request_id;
        let epoch = self.project_epoch;
        let compile = cx
            .background_executor()
            .spawn(async move { backend.validate_custom_shaders(&scene) });
        cx.spawn(async move |this, cx| {
            let result = compile.await;
            let _ = this.update(cx, |s, cx| {
                if s.shader_request_id != request || s.project_epoch != epoch {
                    return;
                }
                s.shader_compiling = false;
                if s.shader_editor
                    .as_ref()
                    .is_none_or(|editor| editor.read(cx).text != code)
                {
                    s.shader_message = Some(
                        "Code changed during compilation. Apply again to compile this draft."
                            .into(),
                    );
                    cx.notify();
                    return;
                }
                if s.scene
                    .material(id)
                    .is_none_or(|data| data.material != original)
                {
                    s.shader_message =
                        Some("The material changed during compilation. Apply again.".into());
                    cx.notify();
                    return;
                }
                match result {
                    Ok(()) => {
                        if original.shader != ShaderKind::Custom
                            || original.custom_code != code
                            || original.custom_language != language
                        {
                            s.history.checkpoint(&s.scene);
                            let material = &mut s.scene.material_mut(id).unwrap().material;
                            material.custom_code = code;
                            material.custom_language = language;
                            material.shader = ShaderKind::Custom;
                            s.dirty = true;
                            s.invalidate(true, cx);
                        }
                        s.shader_message =
                            Some("Compiled and applied · Save the project to keep changes".into());
                        s.status = "Custom shader applied".into();
                    }
                    Err(error) => {
                        s.shader_message = Some(format!(
                            "Compilation failed — current material retained.\n{error:#}"
                        ))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn choose_texture(&mut self, slot: TextureSlot, cx: &mut Context<Self>) {
        if self.texture_loading {
            return;
        }
        let Some(id) = self.selected_material_id() else {
            return;
        };
        self.texture_request_id += 1;
        let request = self.texture_request_id;
        let epoch = self.project_epoch;
        let original = self.scene.material(id).unwrap().material.textures[slot as usize].clone();
        self.texture_loading = true;
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(format!("{} texture · PNG or JPEG", slot.label()).into()),
        });
        cx.spawn(async move |this, cx| {
            let result = match receiver.await {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.into_iter().next() {
                        Some(
                            cx.background_executor()
                                .spawn(async move { forma_render::load_texture(&path) })
                                .await,
                        )
                    } else {
                        None
                    }
                }
                Ok(Err(error)) => Some(Err(error)),
                _ => None,
            };
            let _ = this.update(cx, |s, cx| {
                if s.texture_request_id != request {
                    return;
                }
                s.texture_loading = false;
                if s.project_epoch != epoch {
                    cx.notify();
                    return;
                }
                let Some(data) = s.scene.material(id) else {
                    cx.notify();
                    return;
                };
                if data.material.textures[slot as usize] != original {
                    cx.notify();
                    return;
                }
                match result {
                    Some(Ok(image)) => {
                        let name = image.name.clone();
                        let mut candidate = s.scene.clone();
                        candidate.material_mut(id).unwrap().material.textures[slot as usize] =
                            Some(Arc::new(image));
                        match candidate.validate() {
                            Ok(()) => {
                                s.history.checkpoint(&s.scene);
                                s.scene = candidate;
                                s.dirty = true;
                                s.status = format!("{} texture · {name}", slot.label());
                                s.invalidate(true, cx);
                            }
                            Err(error) => s.status = format!("Texture: {error:#}"),
                        }
                    }
                    Some(Err(error)) => s.status = format!("Texture: {error:#}"),
                    None => {}
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
