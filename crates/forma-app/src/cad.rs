//! CAD parts in the editor: part commands, the background CAD host, and
//! applying regeneration results to the scene.
//!
//! Part edits change `PartData` immediately (one undo step, like any edit)
//! and restamp it. `sync_parts` then asks the worker to regenerate every part
//! whose cached tessellation is stale; results land through
//! `Scene::set_part_tessellation`, which ignores stamps that no longer match.
//! Nothing here blocks the UI thread: the kernel runs in `forma --cad-worker`.
use crate::app::{Command, EditMode, Studio};
use forma_cad::host::{CadHost, HostConfig, HostEvent, Job, WorkerMode};
use forma_cad::{FaceInfo, RegenStats, StepPart};
use forma_core::{
    CombineFeature, ExtrudeExtent, ExtrudeFeature, Feature, FeatureKind, FeatureOperation, Mesh,
    ObjectData, PartData, Placement, PrimitiveFeature, PrimitiveShape, Scene, ToolBody,
};
use glam::{Mat4, Vec3};
use gpui::{Context, PathPromptOptions};
use std::collections::HashMap;
use std::path::PathBuf;

/// Parametric primitives offered as new parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PartShape {
    Box,
    Cylinder,
    Sphere,
}

impl PartShape {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Box => "Box",
            Self::Cylinder => "Cylinder",
            Self::Sphere => "Sphere",
        }
    }

    /// Same proportions as the mesh primitives, so swapping is familiar.
    fn shape(self) -> PrimitiveShape {
        match self {
            Self::Box => PrimitiveShape::Box {
                size: Vec3::splat(2.0),
            },
            Self::Cylinder => PrimitiveShape::Cylinder {
                radius: 1.0,
                height: 2.0,
            },
            Self::Sphere => PrimitiveShape::Sphere { radius: 1.0 },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CadCommand {
    AddPart(PartShape),
    ConvertToPart,
    ConvertToMesh,
    /// Combine the other selected objects into the active one.
    Combine(FeatureOperation),
    /// Extrude the selected sketch's outer regions into a new part (1 m).
    ExtrudeSketch,
    ImportStep,
    ExportStep,
}

/// What to do when a ticketed job finishes.
enum Pending {
    ToMesh {
        object: u64,
        part: u64,
        stamp: u64,
        epoch: u64,
    },
    Export {
        path: PathBuf,
    },
    Import {
        epoch: u64,
    },
}

/// Editor-side CAD state. The worker starts on first use.
pub(crate) struct CadState {
    host: Option<CadHost>,
    events: async_channel::Sender<HostEvent>,
    /// Stamp requested per part, so unchanged parts are not resubmitted.
    requested: HashMap<u64, u64>,
    pending: HashMap<u64, Pending>,
    next_ticket: u64,
    /// Face groups of each part's latest tessellation, for face picking.
    pub(crate) faces: HashMap<u64, Vec<FaceInfo>>,
    pub(crate) last_stats: Option<(u64, RegenStats)>,
    /// Parts whose regeneration is running or queued.
    pub(crate) busy: usize,
    /// Show the next regeneration's outcome in the status line.
    pub(crate) announce: bool,
}

impl CadState {
    pub(crate) fn new(cx: &mut Context<Studio>) -> Self {
        let (events, receiver) = async_channel::unbounded::<HostEvent>();
        cx.spawn(async move |this, cx| {
            while let Ok(event) = receiver.recv().await {
                if this.update(cx, |s, cx| s.receive_cad(event, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            host: None,
            events,
            requested: HashMap::new(),
            pending: HashMap::new(),
            next_ticket: 0,
            faces: HashMap::new(),
            last_stats: None,
            busy: 0,
            announce: false,
        }
    }

    fn host(&mut self) -> Result<&CadHost, String> {
        if self.host.is_none() {
            let mode = WorkerMode::current_exe().map_err(|error| {
                format!("Could not locate Forma to start the CAD worker: {error}")
            })?;
            let events = self.events.clone();
            self.host = Some(CadHost::new(HostConfig::new(mode), move |event| {
                let _ = events.try_send(event);
            }));
        }
        Ok(self.host.as_ref().unwrap())
    }

    fn ticket(&mut self, pending: Pending) -> u64 {
        self.next_ticket += 1;
        self.pending.insert(self.next_ticket, pending);
        self.next_ticket
    }

    /// Regenerate a candidate part for display under a preview job id (see
    /// `cad_tools::preview_job`); results come back through `receive_cad`.
    pub(crate) fn submit_preview(&mut self, job: u64, part: PartData) -> Result<(), String> {
        self.host()?.regenerate(job, part);
        Ok(())
    }

    /// Stop the worker process; called before the app exits.
    pub(crate) fn shutdown(&self) {
        if let Some(host) = &self.host {
            host.shutdown();
        }
    }
}

/// One-line description of a feature's parameters, for the inspector.
pub(crate) fn feature_summary(feature: &Feature) -> String {
    let length = |value: f32| crate::transform::format_precision_value(value);
    match &feature.kind {
        FeatureKind::Primitive(primitive) => match primitive.shape {
            PrimitiveShape::Box { size } => format!(
                "Box {} × {} × {} m",
                length(size.x),
                length(size.y),
                length(size.z)
            ),
            PrimitiveShape::Cylinder { radius, height } => {
                format!("Cylinder r {} · h {} m", length(radius), length(height))
            }
            PrimitiveShape::Sphere { radius } => format!("Sphere r {} m", length(radius)),
            PrimitiveShape::Cone {
                bottom_radius,
                height,
                ..
            } => format!("Cone r {} · h {} m", length(bottom_radius), length(height)),
            PrimitiveShape::Torus {
                major_radius,
                minor_radius,
            } => format!(
                "Torus {} / {} m",
                length(major_radius),
                length(minor_radius)
            ),
        },
        FeatureKind::Extrude(extrude) => format!(
            "Extrude {} m · {}",
            length(extrude.distance),
            extrude.operation.label()
        ),
        FeatureKind::Revolve(revolve) => format!(
            "Revolve {:.0}° · {}",
            revolve.angle.to_degrees(),
            revolve.operation.label()
        ),
        FeatureKind::Combine(combine) => format!(
            "{} · {} tool{}",
            combine.operation.label(),
            combine.tools.len(),
            if combine.tools.len() == 1 { "" } else { "s" }
        ),
        FeatureKind::PushPull(push) => format!("Push/pull {} m", length(push.distance)),
        FeatureKind::Chamfer(chamfer) => format!(
            "Chamfer {} m · {} edge{}",
            length(chamfer.distance),
            chamfer.edges.len(),
            if chamfer.edges.len() == 1 { "" } else { "s" }
        ),
        FeatureKind::BaseBody(base) => match &base.body {
            forma_core::BaseBody::Mesh { mesh } => {
                format!("Mesh body · {} polygons", mesh.faces.len())
            }
            forma_core::BaseBody::Step { text } => {
                format!("STEP body · {:.1} MB", text.len() as f64 / 1.0e6)
            }
        },
    }
}

/// The world transform of `tool` expressed in `target`'s local space.
fn relative_transform(scene: &Scene, target: u64, tool: u64) -> Option<Mat4> {
    let target = scene.world_transform(target)?;
    let tool = scene.world_transform(tool)?;
    let relative = target.inverse() * tool;
    relative.is_finite().then_some(relative)
}

/// Give every feature in `features` (recursively) a fresh id from `part`.
fn reassign_ids(features: &mut [Feature], part: &mut PartData) {
    for feature in features {
        feature.id = part.allocate_feature_id();
        if let FeatureKind::Combine(combine) = &mut feature.kind {
            for tool in &mut combine.tools {
                reassign_ids(&mut tool.features, part);
            }
        }
    }
}

impl Studio {
    /// Stops the CAD worker process before the app exits.
    pub(crate) fn shutdown_cad(&self) {
        self.cad.shutdown();
    }

    /// Commands that would edit a part's cached tessellation as polygons.
    /// Returns true (and explains) when the command must not run.
    pub(crate) fn cad_blocks(&mut self, command: Command) -> bool {
        let component = matches!(
            command,
            Command::ToggleEdit
                | Command::SetEditMode(EditMode::Face | EditMode::Edge | EditMode::Vertex)
                | Command::Extrude
                | Command::Inset
                | Command::Bevel
                | Command::Subdivide
                | Command::Triangulate
                | Command::FlipNormals
                | Command::RecalculateNormals
                | Command::ApplyScale
                | Command::ApplyRotationScale
                | Command::CenterOrigin
        );
        if !component || (command == Command::ToggleEdit && self.edit_mode.is_component()) {
            return false;
        }
        let parts = self
            .selected_ids()
            .into_iter()
            .chain(self.selected)
            .any(|id| self.scene.object(id).is_some_and(|o| o.is_part()));
        if parts {
            self.status = "This is a CAD part · Convert to mesh to edit polygons".into();
        }
        parts
    }

    pub(crate) fn execute_cad(&mut self, command: CadCommand, cx: &mut Context<Self>) {
        match command {
            CadCommand::AddPart(shape) => self.add_part(shape, cx),
            CadCommand::ConvertToPart => self.convert_to_part(cx),
            CadCommand::ConvertToMesh => self.convert_to_mesh(cx),
            CadCommand::Combine(operation) => self.combine(operation, cx),
            CadCommand::ExtrudeSketch => self.extrude_sketch(cx),
            CadCommand::ImportStep => self.choose_step_import(cx),
            CadCommand::ExportStep => self.choose_step_export(cx),
        }
    }

    fn add_part(&mut self, shape: PartShape, cx: &mut Context<Self>) {
        let mut part = PartData::new(0, shape.label());
        part.push_feature(
            shape.label(),
            FeatureKind::Primitive(PrimitiveFeature {
                shape: shape.shape(),
                placement: Placement::default(),
                operation: FeatureOperation::Join,
            }),
        );
        let before = self.scene.clone();
        match self.scene.add_part(shape.label(), part) {
            Ok(id) => {
                self.history.checkpoint(&before);
                self.select_objects([id].into_iter().collect(), Some(id));
                self.dirty = true;
                self.cad.announce = true;
                self.status = format!("Added part {} · Regenerating…", shape.label());
                self.invalidate(true, cx);
            }
            Err(error) => self.status = format!("Add part: {error:#}"),
        }
    }

    /// Make a part from each selected closed mesh, in place.
    fn convert_to_part(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<u64> = self
            .selected_ids()
            .into_iter()
            .filter(|id| {
                self.scene
                    .object(*id)
                    .is_some_and(|o| o.data.is_editable_mesh())
            })
            .collect();
        if ids.is_empty() {
            self.status = "Select a closed mesh to convert to a part".into();
            return;
        }
        let mut edited = self.scene.clone();
        for &id in &ids {
            if let Err(message) = mesh_to_part_in_place(&mut edited, id) {
                let name = self
                    .scene
                    .object(id)
                    .map(|o| o.name.clone())
                    .unwrap_or_default();
                self.status = format!("Convert {name} to part: {message}");
                return;
            }
        }
        self.history.checkpoint(&self.scene);
        self.scene = edited;
        self.clear_components();
        self.dirty = true;
        self.cad.announce = true;
        self.status = format!(
            "Converted {} to part{} · Regenerating…",
            ids.len(),
            if ids.len() == 1 { "" } else { "s" }
        );
        self.invalidate(true, cx);
    }

    fn convert_to_mesh(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<u64> = self
            .selected_ids()
            .into_iter()
            .filter(|id| self.scene.object(*id).is_some_and(|o| o.is_part()))
            .collect();
        if ids.is_empty() {
            self.status = "Select a part to convert to a mesh".into();
            return;
        }
        for id in ids {
            let Some(part) = self.scene.object_part(id).cloned() else {
                continue;
            };
            let ticket = self.cad.ticket(Pending::ToMesh {
                object: id,
                part: part.id,
                stamp: part.stamp,
                epoch: self.project_epoch,
            });
            match self.cad.host() {
                Ok(host) => host.submit(Job::PartToMesh { ticket, part }),
                Err(error) => {
                    self.status = error;
                    return;
                }
            }
        }
        self.status = "Converting to mesh…".into();
        cx.notify();
    }

    fn combine(&mut self, operation: FeatureOperation, cx: &mut Context<Self>) {
        let verb = operation.label();
        let Some(target) = self.selected else {
            self.status = format!("{verb}: select the tools, then the target last");
            return;
        };
        let tools: Vec<u64> = self
            .selected_ids()
            .into_iter()
            .filter(|id| *id != target)
            .filter(|id| {
                self.scene
                    .object(*id)
                    .is_some_and(|o| o.is_part() || o.data.is_editable_mesh())
            })
            .collect();
        if tools.is_empty() {
            self.status = format!(
                "{verb}: select the target last, with at least one other part or mesh as a tool"
            );
            return;
        }
        let mut edited = self.scene.clone();
        let target_is_part = edited.object(target).is_some_and(|o| o.is_part());
        if !target_is_part && let Err(message) = mesh_to_part_in_place(&mut edited, target) {
            self.status = format!("{verb}: the target {message}");
            return;
        }
        let mut bodies = Vec::new();
        for &tool in &tools {
            let object = edited.object(tool).unwrap().clone();
            let features = if let Some(part) = edited.object_part(tool) {
                part.features.clone()
            } else {
                let mesh = edited.object_mesh(tool).cloned().unwrap_or_default();
                if let Err(message) = forma_cad::check_closed_manifold(&mesh) {
                    self.status = format!("{verb}: {} {message}", object.name);
                    return;
                }
                forma_cad::part_from_mesh(&object.name, &mesh, Some(tool)).features
            };
            let Some(transform) = relative_transform(&edited, target, tool) else {
                self.status = format!("{verb}: {} has an invalid transform", object.name);
                return;
            };
            bodies.push((object.name, transform, features));
        }
        let part = edited.object_part_mut(target).unwrap();
        let mut tool_bodies = Vec::new();
        for (name, transform, mut features) in bodies {
            reassign_ids(&mut features, part);
            tool_bodies.push(ToolBody {
                name,
                transform,
                features,
            });
        }
        let count = tool_bodies.len();
        let feature = part.allocate_feature_id();
        let mut combine = Feature::new(
            feature,
            verb,
            FeatureKind::Combine(CombineFeature {
                operation,
                tools: tool_bodies,
            }),
        );
        combine.source = tools.first().copied();
        part.features.push(combine);
        part.restamp();
        if let Err(error) = part.validate() {
            self.status = format!("{verb}: {error:#}");
            return;
        }
        for tool in &tools {
            edited.remove(*tool);
        }
        edited.purge_orphans();
        self.history.checkpoint(&self.scene);
        self.scene = edited;
        self.select_objects([target].into_iter().collect(), Some(target));
        self.dirty = true;
        self.cad.announce = true;
        self.status = format!(
            "{verb} with {count} tool{} · Regenerating…",
            if count == 1 { "" } else { "s" }
        );
        self.invalidate(true, cx);
    }

    fn extrude_sketch(&mut self, cx: &mut Context<Self>) {
        let sketch_id = self
            .selected
            .into_iter()
            .chain(self.selected_ids())
            .find(|id| self.scene.object_sketch(*id).is_some());
        let Some(sketch_id) = sketch_id else {
            self.status = "Select a sketch to extrude".into();
            return;
        };
        let sketch = self.scene.object_sketch(sketch_id).unwrap().clone();
        let regions = forma_cad::regions::default_region_points(&sketch);
        if regions.is_empty() {
            self.status = "The sketch has no closed regions · Close its outline first".into();
            return;
        }
        let source = self.scene.object(sketch_id).unwrap().clone();
        let mut part = PartData::new(0, "Extrude");
        let id = part.push_feature(
            "Extrude",
            FeatureKind::Extrude(ExtrudeFeature {
                sketch,
                profiles: regions,
                distance: 1.0,
                extent: ExtrudeExtent::OneSide,
                flip: false,
                operation: FeatureOperation::Join,
            }),
        );
        part.edit_feature(id, |feature| feature.source = Some(sketch_id));
        let mut edited = self.scene.clone();
        let object = match edited.add_part(format!("{} extrusion", source.name), part) {
            Ok(object) => object,
            Err(error) => {
                self.status = format!("Extrude sketch: {error:#}");
                return;
            }
        };
        // Part space is the sketch object's space, where its plane lives.
        if let Some(created) = edited.object_mut(object) {
            created.transform = source.transform;
            created.parent = source.parent;
            created.parent_inverse = source.parent_inverse;
            created.collections = source.collections.clone();
        }
        self.history.checkpoint(&self.scene);
        self.scene = edited;
        self.select_objects([object].into_iter().collect(), Some(object));
        self.dirty = true;
        self.cad.announce = true;
        self.status = "Extruded sketch 1 m · Regenerating…".into();
        self.invalidate(true, cx);
    }

    fn choose_step_import(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import STEP (.step, .stp)".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |s, cx| s.import_step_from(path, cx));
        })
        .detach();
    }

    /// Read a STEP file in the background and add it as a new part.
    pub(crate) fn import_step_from(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let epoch = self.project_epoch;
        self.status = "Importing STEP…".into();
        let read = cx.background_executor().spawn({
            let path = path.clone();
            async move { read_step(&path) }
        });
        cx.spawn(async move |this, cx| {
            let read = read.await;
            let _ = this.update(cx, |s, cx| {
                match read {
                    Ok(text) => {
                        let name = path
                            .file_stem()
                            .and_then(|n| n.to_str())
                            .unwrap_or("STEP part")
                            .chars()
                            .filter(|c| !c.is_control())
                            .take(100)
                            .collect::<String>();
                        let ticket = s.cad.ticket(Pending::Import { epoch });
                        match s.cad.host() {
                            Ok(host) => host.submit(Job::ImportStep { ticket, name, text }),
                            Err(error) => s.status = error,
                        }
                    }
                    Err(error) => s.status = format!("STEP import: {error}"),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn choose_step_export(&mut self, cx: &mut Context<Self>) {
        let selected: Vec<u64> = self
            .selected_ids()
            .into_iter()
            .filter(|id| self.scene.object(*id).is_some_and(|o| o.is_part()))
            .collect();
        let ids: Vec<u64> = if selected.is_empty() {
            self.scene
                .objects
                .iter()
                .filter(|o| o.is_part() && self.scene.is_effectively_visible(o.id))
                .map(|o| o.id)
                .collect()
        } else {
            selected
        };
        let parts: Vec<StepPart> = ids
            .iter()
            .filter_map(|id| {
                let object = self.scene.object(*id)?;
                Some(StepPart {
                    name: object.name.clone(),
                    part: self.scene.object_part(*id)?.clone(),
                    transform: self.scene.world_transform(*id)?,
                })
            })
            .collect();
        if parts.is_empty() {
            self.status = "There are no parts to export as STEP".into();
            return;
        }
        let directory = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let name = format!("{}.step", parts[0].name);
        let receiver = cx.prompt_for_new_path(&directory, Some(&name));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(mut path))) = receiver.await else {
                return;
            };
            if path.extension().is_none() {
                path.set_extension("step");
            }
            let _ = this.update(cx, |s, cx| {
                s.export_step_to(path, parts);
                cx.notify();
            });
        })
        .detach();
    }

    /// Export placed parts to `path` in the background.
    pub(crate) fn export_step_to(&mut self, path: PathBuf, parts: Vec<StepPart>) {
        let ticket = self.cad.ticket(Pending::Export { path });
        match self.cad.host() {
            Ok(host) => {
                host.submit(Job::ExportStep { ticket, parts });
                self.status = "Exporting STEP…".into();
            }
            Err(error) => self.status = error,
        }
    }

    /// Request regeneration of every part whose tessellation is stale.
    pub(crate) fn sync_parts(&mut self) {
        let stale: Vec<(u64, u64)> = self
            .scene
            .parts
            .iter()
            .filter(|part| part.is_pending() && self.scene.part_users(part.id) > 0)
            .filter(|part| self.cad.requested.get(&part.id) != Some(&part.stamp))
            .map(|part| (part.id, part.stamp))
            .collect();
        if stale.is_empty() {
            return;
        }
        for (id, stamp) in stale {
            let part = self.scene.part(id).unwrap().clone();
            match self.cad.host() {
                Ok(host) => {
                    host.regenerate(id, part);
                    self.cad.requested.insert(id, stamp);
                }
                Err(error) => {
                    self.status = error;
                    return;
                }
            }
        }
        self.cad.busy = self.cad.requested.len();
    }

    fn receive_cad(&mut self, event: HostEvent, cx: &mut Context<Self>) {
        match event {
            HostEvent::Started { .. } => {}
            HostEvent::Regenerated(result) if crate::cad_tools::is_preview_job(result.part_id) => {
                let outcome = crate::cad_tools::PreviewOutcome {
                    mesh: result.mesh,
                    issues: result.issues,
                    failed: false,
                };
                self.receive_preview(result.part_id, result.stamp, outcome, cx);
            }
            HostEvent::RegenFailed {
                part_id,
                stamp,
                issue,
            } if crate::cad_tools::is_preview_job(part_id) => {
                let outcome = crate::cad_tools::PreviewOutcome {
                    mesh: None,
                    issues: vec![issue],
                    failed: true,
                };
                self.receive_preview(part_id, stamp, outcome, cx);
            }
            HostEvent::Regenerated(result) => {
                if self.cad.requested.get(&result.part_id) == Some(&result.stamp) {
                    self.cad.requested.remove(&result.part_id);
                }
                let name = self
                    .scene
                    .part(result.part_id)
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                let issues = result.issues.clone();
                match install_regeneration(
                    &mut self.scene,
                    result.part_id,
                    result.stamp,
                    result.mesh,
                    result.issues,
                ) {
                    Ok(true) => {
                        self.cad.faces.insert(result.part_id, result.faces);
                        self.cad.last_stats = Some((result.part_id, result.stats.clone()));
                        if let Some(issue) = issues.first() {
                            self.status = if issue.suggestion.is_empty() {
                                format!("{name}: {}", issue.message)
                            } else {
                                format!("{name}: {} · {}", issue.message, issue.suggestion)
                            };
                        } else if self.cad.announce {
                            self.status = format!(
                                "{name} regenerated in {:.0} ms · {} faces",
                                result.stats.total_ms, result.stats.brep_faces
                            );
                        }
                        self.cad.announce = false;
                        self.invalidate(true, cx);
                    }
                    Ok(false) => {}
                    Err(issue) => {
                        self.status = format!("{name}: {} · {}", issue.message, issue.suggestion)
                    }
                }
            }
            HostEvent::RegenFailed {
                part_id,
                stamp,
                issue,
            } => {
                if self.cad.requested.get(&part_id) == Some(&stamp) {
                    self.cad.requested.remove(&part_id);
                }
                if self
                    .scene
                    .mark_part_failed(part_id, stamp, vec![issue.clone()])
                {
                    self.status = format!(
                        "{} · {} · Showing the previous result",
                        issue.message, issue.suggestion
                    );
                }
            }
            HostEvent::WorkerRestarted { reason } => {
                self.status = format!("{reason} · The CAD worker restarted");
            }
            HostEvent::Mesh { ticket, result } => {
                if let Some(Pending::ToMesh {
                    object,
                    part,
                    stamp,
                    epoch,
                }) = self.cad.pending.remove(&ticket)
                    // Another project was opened meanwhile: its ids mean
                    // other objects.
                    && epoch == self.project_epoch
                {
                    self.finish_convert_to_mesh(object, part, stamp, result, cx);
                }
            }
            HostEvent::Step { ticket, result } => {
                if let Some(Pending::Export { path }) = self.cad.pending.remove(&ticket) {
                    match result {
                        Ok(text) => {
                            let destination = path.clone();
                            let write = cx.background_executor().spawn(async move {
                                std::fs::write(&destination, text.as_bytes())
                            });
                            cx.spawn(async move |this, cx| {
                                let result = write.await;
                                let _ = this.update(cx, |s, cx| {
                                    s.status = match result {
                                        Ok(()) => format!("STEP exported · {}", path.display()),
                                        Err(error) => format!("STEP export failed: {error}"),
                                    };
                                    cx.notify();
                                });
                            })
                            .detach();
                        }
                        Err(error) => self.status = format!("STEP export failed: {error}"),
                    }
                }
            }
            HostEvent::Imported { ticket, result } => {
                if let Some(Pending::Import { epoch }) = self.cad.pending.remove(&ticket) {
                    if epoch != self.project_epoch {
                        return;
                    }
                    match result {
                        Ok(part) => {
                            let name = part.name.clone();
                            let before = self.scene.clone();
                            match self.scene.add_part(name.clone(), part) {
                                Ok(id) => {
                                    self.history.checkpoint(&before);
                                    self.select_objects([id].into_iter().collect(), Some(id));
                                    self.dirty = true;
                                    self.cad.announce = true;
                                    self.status = format!("Imported STEP {name}");
                                    self.invalidate(true, cx);
                                }
                                Err(error) => self.status = format!("STEP import: {error:#}"),
                            }
                        }
                        Err(error) => self.status = format!("STEP import: {error}"),
                    }
                }
            }
            HostEvent::Echo { .. } => {}
        }
        self.cad.busy = self.cad.requested.len();
        cx.notify();
    }

    fn finish_convert_to_mesh(
        &mut self,
        object: u64,
        part: u64,
        stamp: u64,
        result: Result<Mesh, String>,
        cx: &mut Context<Self>,
    ) {
        let mesh = match result {
            Ok(mesh) => mesh,
            Err(error) => {
                self.status = format!("Convert to mesh: {error}");
                return;
            }
        };
        let current = self.scene.object(object).and_then(|o| o.part_id());
        if current != Some(part) || self.scene.part(part).is_none_or(|p| p.stamp != stamp) {
            self.status = "Convert to mesh skipped because the part changed · Try again".into();
            return;
        }
        let mut edited = self.scene.clone();
        let name = edited.object(object).unwrap().name.clone();
        let materials = edited.object(object).unwrap().data.material_ids().to_vec();
        let mesh_id = match edited.create_mesh_data(name.clone(), mesh) {
            Ok(id) => id,
            Err(error) => {
                self.status = format!("Convert to mesh: {error:#}");
                return;
            }
        };
        edited.object_mut(object).unwrap().data = ObjectData::Mesh {
            mesh: mesh_id,
            materials,
        };
        edited.purge_orphans();
        if let Err(error) = edited.validate() {
            self.status = format!("Convert to mesh: {error:#}");
            return;
        }
        self.history.checkpoint(&self.scene);
        self.scene = edited;
        self.dirty = true;
        self.status = format!("{name} is now an editable mesh · Tab to edit polygons");
        self.invalidate(true, cx);
    }
}

/// Install a regeneration result. A tessellation the scene rejects (over
/// the scene's geometry budget, say) marks the part failed instead of
/// leaving it "regenerating" and resubmitting on every change. Returns
/// `Ok(false)` for stale results and the issue shown for rejected ones.
pub(crate) fn install_regeneration(
    scene: &mut Scene,
    part_id: u64,
    stamp: u64,
    mesh: Option<Mesh>,
    issues: Vec<forma_core::FeatureIssue>,
) -> Result<bool, forma_core::FeatureIssue> {
    scene
        .set_part_tessellation(part_id, stamp, mesh, issues)
        .map_err(|error| {
            let issue = forma_core::FeatureIssue {
                feature: 0,
                message: format!("The regenerated part was not accepted ({error:#})"),
                suggestion: "Choose a coarser tessellation quality or simplify the scene".into(),
                warning: false,
            };
            scene.mark_part_failed(part_id, stamp, vec![issue.clone()]);
            issue
        })
}

/// Replace a mesh object's data with a part made from its mesh. The mesh
/// keeps displaying (in a private copy) until the first regeneration.
pub(crate) fn mesh_to_part_in_place(scene: &mut Scene, object: u64) -> Result<(), String> {
    let source = scene.object(object).ok_or("does not exist")?.clone();
    let mesh = scene.object_mesh(object).cloned().ok_or("is not a mesh")?;
    forma_cad::check_closed_manifold(&mesh)?;
    let part = forma_cad::part_from_mesh(&source.name, &mesh, Some(object));
    let part_id = scene.create_part_data(part).map_err(|e| format!("{e:#}"))?;
    let display = scene
        .create_mesh_data(source.name.clone(), mesh)
        .map_err(|e| format!("{e:#}"))?;
    let materials = source.data.material_ids().to_vec();
    scene.object_mut(object).unwrap().data = ObjectData::Part {
        part: part_id,
        mesh: Some(display),
        materials,
    };
    scene.purge_orphans();
    Ok(())
}

fn read_step(path: &std::path::Path) -> Result<String, String> {
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if metadata.len() > forma_core::MAX_PART_STEP_BYTES as u64 {
        return Err("the file is larger than 32 MiB".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|_| "the file is not text STEP (ISO 10303-21)".into())
}

/// A showcase scene of B-rep parts on the studio plinth, used by UI
/// captures and the smoke test: a drilled plate with a boss and chamfered
/// corners, a revolved knob, and a rounded, cross-drilled block.
pub(crate) fn demo_scene() -> Scene {
    use forma_core::{
        Material, RevolveFeature, Sketch, SketchCurve, SketchEntity, SketchPlane, Vec2,
    };
    let mut scene = Scene::default();
    let showcase: Vec<u64> = scene
        .objects
        .iter()
        .filter(|o| ["Torus", "Sphere", "Rounded cube"].contains(&o.name.as_str()))
        .map(|o| o.id)
        .collect();
    for id in showcase {
        scene.remove(id);
    }
    scene.purge_orphans();
    let circle = |x: f32, y: f32, radius: f32| -> SketchEntity {
        SketchCurve::Circle {
            center: Vec2::new(x, y),
            radius,
        }
        .into()
    };
    let at_height = |height: f32| SketchPlane {
        origin: Vec3::new(0.0, height, 0.0),
        ..SketchPlane::ground()
    };
    let extrude = |sketch: Sketch, distance: f32, flip: bool, operation| {
        FeatureKind::Extrude(ExtrudeFeature {
            sketch,
            profiles: Vec::new(),
            distance,
            extent: ExtrudeExtent::OneSide,
            flip,
            operation,
        })
    };
    let primitive = |shape, origin: Vec3, rotation: Vec3, operation| {
        FeatureKind::Primitive(PrimitiveFeature {
            shape,
            placement: Placement { origin, rotation },
            operation,
        })
    };

    // Plate: sketched holes and boss on its top face, then corner chamfers.
    let mut plate = PartData::new(0, "Bracket");
    plate.push_feature(
        "Plate",
        primitive(
            PrimitiveShape::Box {
                size: Vec3::new(2.2, 0.3, 1.5),
            },
            Vec3::new(0.0, 0.15, 0.0),
            Vec3::ZERO,
            FeatureOperation::Join,
        ),
    );
    let mut holes = Sketch::new(at_height(0.3));
    for (x, y) in [(-0.8, -0.48), (0.8, -0.48), (-0.8, 0.48), (0.8, 0.48)] {
        holes.entities.push(circle(x, y, 0.12));
    }
    plate.push_feature("Holes", extrude(holes, 0.6, true, FeatureOperation::Cut));
    let mut boss = Sketch::new(at_height(0.3));
    boss.entities.push(circle(0.0, 0.0, 0.38));
    plate.push_feature("Boss", extrude(boss, 0.45, false, FeatureOperation::Join));
    let mut bore = Sketch::new(at_height(0.75));
    bore.entities.push(circle(0.0, 0.0, 0.17));
    plate.push_feature("Bore", extrude(bore, 1.2, true, FeatureOperation::Cut));
    let corners = [(-1.1, -0.75), (1.1, -0.75), (-1.1, 0.75), (1.1, 0.75)];
    plate.push_feature(
        "Chamfer",
        FeatureKind::Chamfer(forma_core::ChamferFeature {
            edges: corners
                .iter()
                .map(|&(x, z)| forma_core::EdgeRef {
                    start: Vec3::new(x, 0.0, z),
                    end: Vec3::new(x, 0.3, z),
                })
                .collect(),
            distance: 0.14,
        }),
    );

    // Knob: a profile touching the axis, revolved a full turn.
    let mut profile = Sketch::new(SketchPlane::front());
    let p = Vec2::new;
    let line = |a: Vec2, b: Vec2| -> SketchEntity { SketchCurve::Line { start: a, end: b }.into() };
    profile.entities = vec![
        line(p(0.0, 0.0), p(0.55, 0.0)),
        line(p(0.55, 0.0), p(0.55, 0.12)),
        SketchCurve::arc_through(p(0.55, 0.12), p(0.47, 0.42), p(0.3, 0.6))
            .unwrap()
            .into(),
        line(p(0.3, 0.6), p(0.2, 0.66)),
        line(p(0.2, 0.66), p(0.2, 0.95)),
        line(p(0.2, 0.95), p(0.0, 0.95)),
        line(p(0.0, 0.95), p(0.0, 0.0)),
    ];
    let mut knob = PartData::new(0, "Knob");
    knob.push_feature(
        "Revolve",
        FeatureKind::Revolve(RevolveFeature {
            sketch: profile,
            profiles: Vec::new(),
            axis_origin: Vec2::ZERO,
            axis_direction: Vec2::Y,
            angle: std::f32::consts::TAU,
            operation: FeatureOperation::Join,
        }),
    );

    // Classic CSG: (box ∩ cylinder) − two crossing bores. (truck 0.4 cannot
    // yet intersect spheres, cones or tori with other solids.)
    let mut block = PartData::new(0, "Block");
    let center = Vec3::new(0.0, 0.65, 0.0);
    block.push_feature(
        "Cube",
        primitive(
            PrimitiveShape::Box {
                size: Vec3::splat(1.3),
            },
            center,
            Vec3::ZERO,
            FeatureOperation::Join,
        ),
    );
    block.push_feature(
        "Round",
        primitive(
            PrimitiveShape::Cylinder {
                radius: 0.8,
                height: 2.0,
            },
            center,
            // Turning a cylinder about its own axis leaves it unchanged but
            // moves truck's patch seams off the cube's symmetry planes, where
            // the intersection only succeeds approximately.
            Vec3::new(0.0, std::f32::consts::FRAC_PI_4, 0.0),
            FeatureOperation::Intersect,
        ),
    );
    let quarter = std::f32::consts::FRAC_PI_2;
    for (name, rotation) in [
        ("Bore X", Vec3::new(0.0, 0.0, quarter)),
        ("Bore Z", Vec3::new(quarter, 0.0, 0.0)),
    ] {
        block.push_feature(
            name,
            primitive(
                PrimitiveShape::Cylinder {
                    radius: 0.3,
                    height: 2.0,
                },
                center,
                rotation,
                FeatureOperation::Cut,
            ),
        );
    }

    let looks = [
        (
            plate,
            Vec3::new(-2.0, 0.0, 0.25),
            Material {
                base_color: Vec3::new(0.80, 0.82, 0.85),
                metallic: 0.9,
                roughness: 0.32,
                ..Material::default()
            },
        ),
        (
            knob,
            Vec3::new(0.1, 0.0, -0.35),
            Material {
                base_color: Vec3::new(0.86, 0.84, 0.80),
                metallic: 0.0,
                roughness: 0.42,
                ..Material::default()
            },
        ),
        (
            block,
            Vec3::new(2.0, 0.0, 0.25),
            Material {
                base_color: Vec3::new(0.85, 0.36, 0.12),
                metallic: 0.0,
                roughness: 0.3,
                ..Material::default()
            },
        ),
    ];
    for (part, translation, material) in looks {
        let name = part.name.clone();
        let id = scene.add_part(name, part).expect("demo part");
        scene.object_mut(id).unwrap().transform.translation = translation;
        *scene.object_material_mut(id).unwrap() = material;
    }
    scene.camera.target = Vec3::new(0.0, 0.4, 0.0);
    scene.camera.yaw = 0.5;
    scene.camera.pitch = 0.42;
    scene.camera.distance = 7.6;
    scene
}

impl Studio {
    /// No part is stale, queued or regenerating.
    pub(crate) fn cad_idle(&self) -> bool {
        self.cad.requested.is_empty()
            && self.cad.pending.is_empty()
            && !self.preview_pending()
            && self
                .scene
                .parts
                .iter()
                .all(|part| !part.is_pending() || self.scene.part_users(part.id) == 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_demo_scene_is_valid_and_regenerates_without_issues() {
        let scene = demo_scene();
        scene.validate().unwrap();
        assert_eq!(scene.parts.len(), 3);
        let mut cache = forma_cad::RegenCache::default();
        for part in &scene.parts {
            let result = forma_cad::regenerate(
                &forma_cad::RegenRequest {
                    part_id: part.id,
                    part: part.clone(),
                    skip: Vec::new(),
                },
                &mut cache,
                &mut |_| {},
            );
            assert!(
                result.issues.is_empty(),
                "{}: {:?}",
                part.name,
                result.issues
            );
            let mesh = result.mesh.expect("tessellation");
            assert!(mesh.has_groups());
            if part.name == "Bracket" {
                // The bore through the boss is open: nothing covers its axis
                // at the boss top or the plate bottom.
                for height in [0.75, 0.0] {
                    let covered = mesh.triangles().iter().any(|t| {
                        let [a, b, c] = t.map(|i| mesh.positions[i as usize]);
                        let flat = [a, b, c].iter().all(|p| (p.y - height).abs() < 1.0e-4);
                        let side = |p: Vec3, q: Vec3| (q.x - p.x) * -p.z - (q.z - p.z) * -p.x;
                        let (d0, d1, d2) = (side(a, b), side(b, c), side(c, a));
                        flat && ((d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0)
                            || (d0 <= 0.0 && d1 <= 0.0 && d2 <= 0.0))
                    });
                    assert!(!covered, "the bore is closed at y = {height}");
                }
            }
            println!(
                "{}: {:.1} ms (evaluate {:.1}, tessellate {:.1}) · {} B-rep faces · {} triangles · {} face groups",
                part.name,
                result.stats.total_ms,
                result.stats.evaluate_ms,
                result.stats.tessellate_ms,
                result.stats.brep_faces,
                result.stats.triangles,
                result.faces.len()
            );
        }
    }

    #[test]
    fn rejected_tessellations_mark_the_part_failed() {
        // Review case: a tessellation the scene refused left the part
        // "Regenerating…" forever and resubmitted it on every change.
        let mut scene = Scene::empty();
        let mut part = PartData::new(0, "Box");
        part.push_feature(
            "Box",
            FeatureKind::Primitive(PrimitiveFeature {
                shape: PrimitiveShape::Box { size: Vec3::ONE },
                placement: Placement::default(),
                operation: FeatureOperation::Join,
            }),
        );
        let object = scene.add_part("Box", part).unwrap();
        let part_id = scene.object(object).unwrap().part_id().unwrap();
        let stamp = scene.part(part_id).unwrap().stamp;
        let broken = Mesh::new(vec![Vec3::ZERO], vec![vec![0, 1, 2]]);
        let issue =
            install_regeneration(&mut scene, part_id, stamp, Some(broken), Vec::new()).unwrap_err();
        assert!(issue.message.contains("not accepted"), "{}", issue.message);
        let part = scene.part(part_id).unwrap();
        assert!(part.regeneration_failed() && !part.is_pending());
        assert_eq!(part.issues(), &[issue]);
        assert!(scene.object(object).unwrap().mesh_id().is_none());
        // Stale results are dropped without marking anything.
        assert_eq!(
            install_regeneration(&mut scene, part_id, stamp ^ 1, None, Vec::new()),
            Ok(false)
        );
    }

    #[test]
    fn feature_ids_stay_unique_when_tools_are_merged() {
        let mut target = PartData::new(1, "Target");
        target.push_feature(
            "Box",
            FeatureKind::Primitive(PrimitiveFeature {
                shape: PrimitiveShape::Box { size: Vec3::ONE },
                placement: Placement::default(),
                operation: FeatureOperation::Join,
            }),
        );
        let mut tool = target.features.clone();
        reassign_ids(&mut tool, &mut target);
        assert_ne!(tool[0].id, target.features[0].id);
        let id = target.allocate_feature_id();
        target.features.push(Feature::new(
            id,
            "Join",
            FeatureKind::Combine(CombineFeature {
                operation: FeatureOperation::Join,
                tools: vec![ToolBody {
                    name: "Tool".into(),
                    transform: Mat4::IDENTITY,
                    features: tool,
                }],
            }),
        ));
        target.restamp();
        target.validate().unwrap();
        assert!(feature_summary(&target.features[1]).contains("1 tool"));
    }
}
