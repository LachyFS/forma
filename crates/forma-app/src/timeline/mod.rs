//! The parametric history workflow: the feature timeline's state and
//! commands, feature parameter editing, and sketch associativity.
//!
//! - [`TimelineState`] is session-only UI state (selection, context menu,
//!   rollback marker drag); it never enters the document or undo history.
//!   A feature selection belongs to one part object and is resolved lazily
//!   against the scene, so undo, redo and deletions can never leave it
//!   pointing at something that no longer exists.
//! - Every document change is one undo step: a committed field edit, one
//!   scrub, one marker drag, one delete, suppress or rollback.
//! - [`params`] reads and writes feature values; [`links`] keeps features in
//!   step with the sketch objects they were made from.
//!
//! The dock and inspector UI live in `ui::timeline` and
//! `ui::feature_inspector`; this module owns behaviour.
pub(crate) mod capture;
pub(crate) mod links;
pub(crate) mod params;
pub(crate) mod smoke;

use crate::app::{Command, Field, Studio};
use crate::sketch::DrawTarget;
use forma_core::{
    Feature, FeatureIssue, FeatureKind, FeatureOperation, PartData, SKETCH_TOLERANCE, Sketch,
};
use gpui::{
    Context, KeyDownEvent, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    ScrollHandle, Window,
};
pub(crate) use params::{ExtentMode, FeatureField};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

/// Commands on the active part's history. Commands without an id act on the
/// selected feature, so they also work from the palette and the keyboard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TimelineCommand {
    /// Select a feature of the active part.
    Select(u64),
    Previous,
    Next,
    Deselect,
    /// Edit the selected feature: its sketch, or its first value.
    Edit,
    EditSketch,
    Rename,
    ToggleSuppress,
    Delete,
    /// Move the rollback marker to just after the selected feature.
    RollBackHere,
    RollToEnd,
    /// Move the rollback marker by whole features.
    StepMarker(i32),
    SetOperation(FeatureOperation),
    SetExtent(ExtentMode),
    ToggleFlip,
}

/// A feature picked in the timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Selection {
    pub object: u64,
    pub feature: u64,
}

/// An open feature context menu.
#[derive(Clone, Debug)]
pub(crate) struct ContextMenu {
    pub selection: Selection,
    pub position: Point<Pixels>,
    pub index: usize,
}

/// A rollback marker drag: live marker moves, one undo step on release.
pub(crate) struct MarkerDrag {
    pub object: u64,
    pub before: Box<forma_core::Scene>,
    pub moved: bool,
}

/// Where to return after editing a feature's sketch from the timeline.
#[derive(Clone, Copy, Debug)]
struct SketchReturn {
    selection: Selection,
    sketch: u64,
}

/// Per-feature cautions: `(feature id, message)`.
pub(crate) type Warnings = Rc<Vec<(u64, String)>>;

/// A mesh object's closed-manifold check at a scene revision.
type ManifoldCheck = (u64, u64, Result<(), String>);

/// Session-only timeline state, kept in `PanelState`.
#[derive(Default)]
pub(crate) struct TimelineState {
    selection: Option<Selection>,
    pub(crate) menu: Option<ContextMenu>,
    pub(crate) marker: Option<MarkerDrag>,
    /// Horizontal scroll of the chip strip.
    pub(crate) scroll: ScrollHandle,
    /// Scroll the selected chip into view on the next frame.
    pub(crate) reveal: std::cell::Cell<bool>,
    /// The selection and part state last scrolled into view.
    pub(crate) revealed: std::cell::Cell<Option<(u64, u64, bool, bool)>>,
    sketch_return: Option<SketchReturn>,
    /// Region-pick warnings per part, keyed by its stamp.
    warnings: RefCell<HashMap<u64, (u64, Warnings)>>,
    /// Closed-manifold check of a mesh object at a scene revision.
    manifold: RefCell<Option<ManifoldCheck>>,
}

impl TimelineState {
    /// Forget the feature selection (the object selection changed).
    pub(crate) fn clear_selection(&mut self) {
        self.selection = None;
        self.menu = None;
    }

    pub(crate) fn select(&mut self, selection: Selection) {
        self.selection = Some(selection);
        self.reveal.set(true);
    }
}

/// One context menu row; `None` entries are separators.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MenuEntry {
    pub command: TimelineCommand,
    pub label: &'static str,
    pub shortcut: Option<&'static str>,
    pub danger: bool,
}

pub(crate) fn is_sketch_feature(feature: &Feature) -> bool {
    matches!(
        feature.kind,
        FeatureKind::Extrude(_) | FeatureKind::Revolve(_)
    )
}

/// The embedded sketch of a sketch-based feature.
pub(crate) fn feature_sketch(feature: &Feature) -> Option<&Sketch> {
    match &feature.kind {
        FeatureKind::Extrude(extrude) => Some(&extrude.sketch),
        FeatureKind::Revolve(revolve) => Some(&revolve.sketch),
        _ => None,
    }
}

/// The context menu for a feature, in display order.
pub(crate) fn menu_entries(feature: &Feature) -> Vec<Option<MenuEntry>> {
    let entry = |command, label, shortcut, danger| {
        Some(MenuEntry {
            command,
            label,
            shortcut,
            danger,
        })
    };
    let mut entries = Vec::new();
    if is_sketch_feature(feature) {
        entries.push(entry(
            TimelineCommand::EditSketch,
            "Edit sketch",
            Some("Return"),
            false,
        ));
    }
    entries.push(entry(TimelineCommand::Rename, "Rename", Some("F2"), false));
    entries.push(entry(
        TimelineCommand::ToggleSuppress,
        if feature.suppressed {
            "Unsuppress"
        } else {
            "Suppress"
        },
        None,
        false,
    ));
    entries.push(entry(
        TimelineCommand::RollBackHere,
        "Roll back to here",
        None,
        false,
    ));
    entries.push(None);
    entries.push(entry(
        TimelineCommand::Delete,
        "Delete",
        Some("Delete"),
        true,
    ));
    entries
}

/// Next actionable menu row in `direction`, wrapping over separators.
pub(crate) fn menu_step(entries: &[Option<MenuEntry>], index: usize, direction: isize) -> usize {
    let count = entries.len() as isize;
    if count == 0 {
        return 0;
    }
    let mut next = index as isize;
    for _ in 0..count {
        next = (next + direction).rem_euclid(count);
        if entries[next as usize].is_some() {
            return next as usize;
        }
    }
    index
}

// ---------------------------------------------------------------------------
// Pure history edits, shared by the commands and the unit tests.

/// Remove a feature, keeping the rollback marker on the same features.
/// Returns the neighbour to select next. The only feature cannot be removed.
pub(crate) fn delete_feature(part: &mut PartData, id: u64) -> Result<Option<u64>, String> {
    let index = part
        .features
        .iter()
        .position(|feature| feature.id == id)
        .ok_or("The feature no longer exists")?;
    if part.features.len() == 1 {
        return Err("A part needs at least one feature · Delete the object instead".into());
    }
    part.features.remove(index);
    if let Some(rollback) = part.rollback {
        let rollback = if index < rollback {
            rollback - 1
        } else {
            rollback
        };
        part.rollback = (rollback < part.features.len()).then_some(rollback);
    }
    part.restamp();
    let next = part
        .features
        .get(index)
        .or_else(|| part.features.get(index.wrapping_sub(1)))
        .map(|feature| feature.id);
    Ok(next)
}

/// Flip a feature's suppression. Returns the new state.
pub(crate) fn toggle_suppress(part: &mut PartData, id: u64) -> Result<bool, String> {
    let mut state = false;
    if !part.edit_feature(id, |feature| {
        feature.suppressed = !feature.suppressed;
        state = feature.suppressed;
    }) {
        return Err("The feature no longer exists".into());
    }
    Ok(state)
}

/// Put the rollback marker after `position` features (`len` = the end).
/// Returns whether it moved.
pub(crate) fn set_marker(part: &mut PartData, position: usize) -> bool {
    let position = position.min(part.features.len());
    let rollback = (position < part.features.len()).then_some(position);
    if part.rollback == rollback {
        return false;
    }
    part.rollback = rollback;
    part.restamp();
    true
}

/// Where the marker sits: the number of features before it.
pub(crate) fn marker(part: &PartData) -> usize {
    part.rollback
        .unwrap_or(part.features.len())
        .min(part.features.len())
}

/// The feature `step` places from `id`, clamped to the history.
pub(crate) fn neighbour(part: &PartData, id: u64, step: isize) -> Option<u64> {
    let index = part.features.iter().position(|f| f.id == id)? as isize;
    let next = (index + step).clamp(0, part.features.len() as isize - 1);
    part.features.get(next as usize).map(|f| f.id)
}

/// Rename a feature. Names do not affect geometry, so the part keeps its
/// stamp and nothing regenerates.
pub(crate) fn rename_feature(part: &mut PartData, id: u64, name: &str) -> Result<bool, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
        return Err("Use a single-line name between 1 and 128 bytes".into());
    }
    let feature = part
        .features
        .iter_mut()
        .find(|feature| feature.id == id)
        .ok_or("The feature no longer exists")?;
    if feature.name == name {
        return Ok(false);
    }
    feature.name = name.to_owned();
    Ok(true)
}

/// Per-feature cautions the kernel does not report: region picks that no
/// longer find their region while others still do.
pub(crate) fn pick_warnings(part: &PartData) -> Vec<(u64, String)> {
    let mut warnings = Vec::new();
    for feature in &part.features {
        let (sketch, picks) = match &feature.kind {
            FeatureKind::Extrude(extrude) => (&extrude.sketch, &extrude.profiles),
            FeatureKind::Revolve(revolve) => (&revolve.sketch, &revolve.profiles),
            _ => continue,
        };
        if picks.is_empty() {
            continue;
        }
        let profiles = sketch.profiles(SKETCH_TOLERANCE);
        let found = picks
            .iter()
            .filter(|pick| profiles.iter().any(|profile| profile.contains(**pick)))
            .count();
        if found > 0 && found < picks.len() {
            let lost = picks.len() - found;
            warnings.push((
                feature.id,
                format!(
                    "{lost} of {} picked regions no longer exist{}",
                    picks.len(),
                    if lost == 1 { "s" } else { "" }
                ),
            ));
        }
    }
    warnings
}

/// A feature issue's message without the feature-name prefix the kernel
/// adds (the timeline already shows the name).
pub(crate) fn issue_text(issue: &FeatureIssue) -> String {
    let message = if issue.feature != 0 {
        issue
            .message
            .split_once(": ")
            .map(|(_, rest)| rest)
            .unwrap_or(&issue.message)
    } else {
        &issue.message
    };
    let mut message = message.to_owned();
    if let Some(first) = message.get(..1) {
        let upper = first.to_uppercase();
        message.replace_range(..1, &upper);
    }
    message
}

/// The selected feature, resolved against the current scene.
pub(crate) struct SelectedFeature<'a> {
    pub object: u64,
    pub index: usize,
    pub feature: &'a Feature,
    pub part: &'a PartData,
}

impl Studio {
    /// The part object whose history the timeline shows: the active object,
    /// when it is a part and Draw mode is not running.
    pub(crate) fn timeline_object(&self) -> Option<u64> {
        if self.is_drawing() {
            return None;
        }
        let id = self.selected?;
        self.scene.object(id).filter(|object| object.is_part())?;
        Some(id)
    }

    pub(crate) fn timeline_part(&self) -> Option<(u64, &PartData)> {
        let object = self.timeline_object()?;
        Some((object, self.scene.object_part(object)?))
    }

    pub(crate) fn selected_feature(&self) -> Option<SelectedFeature<'_>> {
        let selection = self.panels.timeline.selection?;
        let (object, part) = self.timeline_part()?;
        if object != selection.object {
            return None;
        }
        let index = part
            .features
            .iter()
            .position(|feature| feature.id == selection.feature)?;
        Some(SelectedFeature {
            object,
            index,
            feature: &part.features[index],
            part,
        })
    }

    fn selected_ids_pair(&self) -> Option<Selection> {
        self.selected_feature().map(|selected| Selection {
            object: selected.object,
            feature: selected.feature.id,
        })
    }

    /// Whether the worker is regenerating (or about to regenerate) `part`.
    pub(crate) fn part_regenerating(&self, part: &PartData) -> bool {
        part.is_pending()
    }

    /// Cached region-pick warnings for a part.
    pub(crate) fn feature_warnings(&self, part: &PartData) -> Warnings {
        let mut cache = self.panels.timeline.warnings.borrow_mut();
        if let Some((stamp, warnings)) = cache.get(&part.id)
            && *stamp == part.stamp
        {
            return warnings.clone();
        }
        let warnings = Rc::new(pick_warnings(part));
        cache.insert(part.id, (part.stamp, warnings.clone()));
        warnings
    }

    /// Closed-manifold check of a mesh object, cached per scene revision.
    pub(crate) fn manifold_check(&self, object: u64) -> Option<Result<(), String>> {
        let mesh = self.scene.object_mesh(object)?;
        if !self.scene.object(object)?.data.is_editable_mesh() {
            return None;
        }
        let mut cache = self.panels.timeline.manifold.borrow_mut();
        // Transform drags invalidate geometry every frame without touching
        // the mesh itself, so keep the last answer for the same object.
        if let Some((id, revision, result)) = cache.as_ref()
            && *id == object
            && (*revision == self.scene_revision || self.transform_drag.is_some())
        {
            return Some(result.clone());
        }
        let result = forma_cad::check_closed_manifold(mesh);
        *cache = Some((object, self.scene_revision, result.clone()));
        Some(result)
    }

    // -----------------------------------------------------------------------
    // Commands.

    pub(crate) fn execute_timeline(
        &mut self,
        command: TimelineCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.panels.timeline.menu = None;
        let Some(object) = self.timeline_object() else {
            self.status = "Select a part to edit its history".into();
            cx.notify();
            return;
        };
        let selected = self.selected_ids_pair();
        let need_feature = |s: &mut Self| {
            s.status = "Select a feature in the timeline first".into();
        };
        match command {
            TimelineCommand::Select(feature) => {
                let exists = self
                    .scene
                    .object_part(object)
                    .is_some_and(|part| part.feature(feature).is_some());
                if exists {
                    self.panels.timeline.select(Selection { object, feature });
                    self.describe_selection();
                }
            }
            TimelineCommand::Previous | TimelineCommand::Next => {
                let step = if command == TimelineCommand::Previous {
                    -1
                } else {
                    1
                };
                let part = self.scene.object_part(object).unwrap();
                let next = match selected {
                    Some(selection) => neighbour(part, selection.feature, step),
                    None if step < 0 => part.features.last().map(|f| f.id),
                    None => part.features.first().map(|f| f.id),
                };
                if let Some(feature) = next {
                    self.panels.timeline.select(Selection { object, feature });
                    self.describe_selection();
                }
            }
            TimelineCommand::Deselect => {
                self.panels.timeline.selection = None;
                self.status = "Feature deselected".into();
            }
            TimelineCommand::Edit => match self.selected_feature() {
                Some(selected) if is_sketch_feature(selected.feature) => {
                    self.edit_feature_sketch(cx);
                    return;
                }
                Some(selected) => {
                    let field = FeatureField::ORDER
                        .into_iter()
                        .skip(1)
                        .find(|field| params::applies(selected.feature, *field));
                    match field {
                        Some(field) => {
                            let field = Field::Feature(field);
                            self.begin_field(field, window, cx);
                            self.panels.scroll_to_active_field(field, window, cx);
                        }
                        None => {
                            self.status = format!(
                                "{} has no values to edit · Use the inspector",
                                selected.feature.name
                            )
                        }
                    }
                }
                None => need_feature(self),
            },
            TimelineCommand::EditSketch => {
                self.edit_feature_sketch(cx);
                return;
            }
            TimelineCommand::Rename => {
                if selected.is_some() {
                    let field = Field::Feature(FeatureField::Name);
                    self.begin_field(field, window, cx);
                    self.panels.scroll_to_active_field(field, window, cx);
                } else {
                    need_feature(self);
                }
            }
            TimelineCommand::ToggleSuppress => match selected {
                Some(selection) => {
                    self.edit_history(object, cx, |part| {
                        let name = part.feature(selection.feature).map(|f| f.name.clone());
                        let suppressed = toggle_suppress(part, selection.feature)?;
                        Ok(format!(
                            "{} {} · Regenerating…",
                            name.unwrap_or_default(),
                            if suppressed {
                                "suppressed"
                            } else {
                                "unsuppressed"
                            }
                        ))
                    });
                }
                None => need_feature(self),
            },
            TimelineCommand::Delete => match selected {
                Some(selection) => {
                    let mut next = None;
                    let deleted = self.edit_history(object, cx, |part| {
                        let name = part
                            .feature(selection.feature)
                            .map(|f| f.name.clone())
                            .unwrap_or_default();
                        next = delete_feature(part, selection.feature)?;
                        Ok(format!("Deleted {name} · Regenerating…"))
                    });
                    if deleted {
                        self.panels.timeline.selection =
                            next.map(|feature| Selection { object, feature });
                        self.panels.timeline.reveal.set(true);
                    }
                }
                None => need_feature(self),
            },
            TimelineCommand::RollBackHere => match self.selected_feature() {
                Some(selected) => {
                    let position = selected.index + 1;
                    let name = selected.feature.name.clone();
                    self.edit_history(object, cx, move |part| {
                        if set_marker(part, position) {
                            Ok(if position >= part.features.len() {
                                "Rolled forward to the end · Regenerating…".into()
                            } else {
                                format!("Rolled back to {name} · Later features are paused")
                            })
                        } else {
                            Err("The marker is already there".into())
                        }
                    });
                }
                None => need_feature(self),
            },
            TimelineCommand::RollToEnd => {
                self.edit_history(object, cx, |part| {
                    if set_marker(part, part.features.len()) {
                        Ok("Rolled forward to the end · Regenerating…".into())
                    } else {
                        Err("The whole history is already evaluated".into())
                    }
                });
            }
            TimelineCommand::StepMarker(step) => {
                self.edit_history(object, cx, |part| {
                    let position = (marker(part) as i64 + i64::from(step))
                        .clamp(0, part.features.len() as i64)
                        as usize;
                    if set_marker(part, position) {
                        Ok(if position == part.features.len() {
                            "Rollback marker at the end".into()
                        } else {
                            format!(
                                "Rolled back · {position} of {} features evaluated",
                                part.features.len()
                            )
                        })
                    } else {
                        Err("The marker cannot move further".into())
                    }
                });
            }
            TimelineCommand::SetOperation(operation) => match selected {
                Some(selection) => {
                    self.edit_history(object, cx, |part| {
                        let mut changed = false;
                        part.edit_feature(selection.feature, |feature| {
                            let target = match &mut feature.kind {
                                FeatureKind::Primitive(f) => &mut f.operation,
                                FeatureKind::Extrude(f) => &mut f.operation,
                                FeatureKind::Revolve(f) => &mut f.operation,
                                FeatureKind::Combine(f) => &mut f.operation,
                                FeatureKind::BaseBody(f) => &mut f.operation,
                                FeatureKind::PushPull(_) | FeatureKind::Chamfer(_) => return,
                            };
                            changed = *target != operation;
                            *target = operation;
                        });
                        if changed {
                            Ok(format!("{} · Regenerating…", operation.label()))
                        } else {
                            Err(String::new())
                        }
                    });
                }
                None => need_feature(self),
            },
            TimelineCommand::SetExtent(mode) => match selected {
                Some(selection) => {
                    self.edit_history(object, cx, |part| {
                        let mut changed = false;
                        part.edit_feature(selection.feature, |feature| {
                            if let FeatureKind::Extrude(extrude) = &mut feature.kind {
                                let extent = mode.apply(extrude.extent, extrude.distance);
                                changed = extent != extrude.extent;
                                extrude.extent = extent;
                            }
                        });
                        if changed {
                            Ok(format!("Extent: {} · Regenerating…", mode.label()))
                        } else {
                            Err(String::new())
                        }
                    });
                }
                None => need_feature(self),
            },
            TimelineCommand::ToggleFlip => match selected {
                Some(selection) => {
                    self.edit_history(object, cx, |part| {
                        let mut flipped = None;
                        part.edit_feature(selection.feature, |feature| {
                            if let FeatureKind::Extrude(extrude) = &mut feature.kind {
                                extrude.flip = !extrude.flip;
                                flipped = Some(extrude.flip);
                            }
                        });
                        match flipped {
                            Some(true) => Ok("Direction reversed · Regenerating…".into()),
                            Some(false) => Ok("Direction restored · Regenerating…".into()),
                            None => Err("Only extrusions can be flipped".into()),
                        }
                    });
                }
                None => need_feature(self),
            },
        }
        cx.notify();
    }

    /// Apply one history edit to a part as one undo step. The edit returns
    /// the status message, or an error (an empty error means "no change").
    fn edit_history(
        &mut self,
        object: u64,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut PartData) -> Result<String, String>,
    ) -> bool {
        let Some(part) = self.scene.object_part(object) else {
            return false;
        };
        let mut candidate = part.clone();
        match edit(&mut candidate) {
            Ok(status) => {
                if let Err(error) = candidate.validate() {
                    self.status = format!("{error:#}");
                    return false;
                }
                self.history.checkpoint(&self.scene);
                *self.scene.object_part_mut(object).unwrap() = candidate;
                self.dirty = true;
                self.status = status;
                self.invalidate(true, cx);
                true
            }
            Err(message) => {
                if !message.is_empty() {
                    self.status = message;
                }
                false
            }
        }
    }

    fn describe_selection(&mut self) {
        // The Part section leads the inspector; bring the values into view.
        self.panels.scroll_inspector_to(0.);
        if let Some(selected) = self.selected_feature() {
            let summary = crate::cad::feature_summary(selected.feature);
            self.status = format!(
                "{} · {} of {} · {summary}",
                selected.feature.name,
                selected.index + 1,
                selected.part.features.len()
            );
        }
    }

    // -----------------------------------------------------------------------
    // Context menu.

    pub(crate) fn open_feature_menu(
        &mut self,
        feature: u64,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(object) = self.timeline_object() else {
            return;
        };
        let Some(entries) = self
            .scene
            .object_part(object)
            .and_then(|part| part.feature(feature))
            .map(menu_entries)
        else {
            return;
        };
        self.active_field = None;
        self.panels.close_menu();
        let selection = Selection { object, feature };
        self.panels.timeline.select(selection);
        self.panels.timeline.menu = Some(ContextMenu {
            selection,
            position,
            index: menu_step(&entries, entries.len().saturating_sub(1), 1),
        });
        self.describe_selection();
        cx.notify();
    }

    // -----------------------------------------------------------------------
    // Rollback marker drag.

    pub(crate) fn begin_marker_drag(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus);
        cx.stop_propagation();
        let Some(object) = self.timeline_object() else {
            return;
        };
        let _ = event;
        self.active_field = None;
        self.panels.timeline.menu = None;
        self.panels.timeline.marker = Some(MarkerDrag {
            object,
            before: Box::new(self.scene.clone()),
            moved: false,
        });
        self.status = "Drag to roll the history back or forward · Release to keep".into();
        cx.notify();
    }

    /// Marker position for a pointer x, from the laid-out chip bounds.
    fn marker_position_at(&self, x: f32, count: usize) -> usize {
        let scroll = &self.panels.timeline.scroll;
        let offset = f32::from(scroll.offset().x);
        (0..count)
            .take_while(|index| {
                scroll
                    .bounds_for_item(2 * index + 1)
                    .is_some_and(|bounds| f32::from(bounds.center().x) + offset < x)
            })
            .count()
    }

    pub(crate) fn move_marker_drag(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.panels.timeline.marker.as_ref() else {
            return;
        };
        cx.stop_propagation();
        if event.pressed_button.is_none() {
            self.finish_marker_drag(cx);
            return;
        }
        let object = drag.object;
        let Some(count) = self.scene.object_part(object).map(|p| p.features.len()) else {
            self.panels.timeline.marker = None;
            return;
        };
        let position = self.marker_position_at(event.position.x.into(), count);
        let part = self.scene.object_part_mut(object).unwrap();
        if set_marker(part, position) {
            if let Some(drag) = self.panels.timeline.marker.as_mut() {
                drag.moved = true;
            }
            self.status = if position == count {
                "Rollback marker at the end · All features evaluated".into()
            } else {
                format!("Rolled back · {position} of {count} features evaluated")
            };
            // Live: each new position regenerates (the worker coalesces).
            self.invalidate(true, cx);
        }
    }

    pub(crate) fn end_marker_drag(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.panels.timeline.marker.is_some() {
            cx.stop_propagation();
            self.finish_marker_drag(cx);
        }
    }

    fn finish_marker_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.panels.timeline.marker.take() else {
            return;
        };
        if drag.moved && *drag.before != self.scene {
            self.history.checkpoint_owned(*drag.before);
            self.dirty = true;
        }
        cx.notify();
    }

    /// Click on a gap between chips: move the marker there (one step).
    pub(crate) fn place_marker(&mut self, position: usize, cx: &mut Context<Self>) {
        let Some(object) = self.timeline_object() else {
            return;
        };
        self.edit_history(object, cx, |part| {
            if set_marker(part, position) {
                Ok(if position >= part.features.len() {
                    "Rollback marker at the end · All features evaluated".into()
                } else {
                    format!(
                        "Rolled back · {position} of {} features evaluated",
                        part.features.len()
                    )
                })
            } else {
                Err(String::new())
            }
        });
        cx.notify();
    }

    // -----------------------------------------------------------------------
    // Keyboard.

    /// Timeline keys: ←/→ select, ⇧←/→ move the marker, Return edit, F2
    /// rename, Delete remove, Esc deselect, and the open context menu.
    pub(crate) fn timeline_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        let mods = event.keystroke.modifiers;
        if let Some(menu) = self.panels.timeline.menu.as_mut() {
            let entries = self
                .scene
                .object_part(menu.selection.object)
                .and_then(|part| part.feature(menu.selection.feature))
                .map(menu_entries)
                .unwrap_or_default();
            match key {
                "escape" => self.panels.timeline.menu = None,
                "up" => menu.index = menu_step(&entries, menu.index, -1),
                "down" => menu.index = menu_step(&entries, menu.index, 1),
                "enter" => {
                    if let Some(Some(entry)) = entries.get(menu.index) {
                        let command = entry.command;
                        self.panels.timeline.menu = None;
                        self.execute(Command::Timeline(command), window, cx);
                    }
                }
                _ => {}
            }
            cx.notify();
            return true;
        }
        if self.timeline_object().is_none() || mods.platform || mods.control || mods.alt {
            return false;
        }
        let selected = self.selected_feature().is_some();
        let command = match key {
            "left" if mods.shift => TimelineCommand::StepMarker(-1),
            "right" if mods.shift => TimelineCommand::StepMarker(1),
            "left" => TimelineCommand::Previous,
            "right" => TimelineCommand::Next,
            "escape" if selected => TimelineCommand::Deselect,
            "delete" | "backspace" if selected => TimelineCommand::Delete,
            "enter" if selected => TimelineCommand::Edit,
            "f2" if selected => TimelineCommand::Rename,
            _ => return false,
        };
        self.execute(Command::Timeline(command), window, cx);
        true
    }

    /// Status bar hints while a feature is selected or its menu is open.
    pub(crate) fn timeline_input_hints(&self) -> Option<Vec<(&'static str, &'static str)>> {
        if self.panels.timeline.menu.is_some() {
            return Some(vec![("↑ ↓", "Choose"), ("Return", "Run"), ("Esc", "Close")]);
        }
        if self.panels.timeline.marker.is_some() {
            return Some(vec![("Drag", "Roll back or forward"), ("Release", "Keep")]);
        }
        let selected = self.selected_feature()?;
        let mut hints = vec![("← →", "Feature"), ("⇧ ← →", "Rollback")];
        hints.push((
            "Return",
            if is_sketch_feature(selected.feature) {
                "Edit sketch"
            } else {
                "Edit value"
            },
        ));
        hints.extend([("F2", "Rename"), ("Delete", "Remove"), ("Esc", "Deselect")]);
        Some(hints)
    }

    // -----------------------------------------------------------------------
    // Feature fields (inspector values).

    fn field_feature(&self) -> Option<(u64, &Feature)> {
        let selected = self.selected_feature()?;
        Some((selected.object, selected.feature))
    }

    pub(crate) fn feature_field_value(&self, field: FeatureField) -> Option<f32> {
        params::value(self.field_feature()?.1, field)
    }

    pub(crate) fn feature_field_text(&self, field: FeatureField) -> Option<String> {
        let (_, feature) = self.field_feature()?;
        match field {
            FeatureField::Name => Some(feature.name.clone()),
            _ => params::value(feature, field).map(|value| value.to_string()),
        }
    }

    pub(crate) fn feature_field_available(&self, field: FeatureField) -> bool {
        self.field_feature()
            .is_some_and(|(_, feature)| params::applies(feature, field))
    }

    /// Commit typed or scrubbed text to the selected feature. Invalid input
    /// keeps the field open with an explanation, like other fields.
    pub(crate) fn commit_feature_field(
        &mut self,
        field: FeatureField,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        let Some((object, feature)) = self.field_feature().map(|(o, f)| (o, f.id)) else {
            self.active_field = None;
            self.status = "Select a feature first".into();
            cx.notify();
            return;
        };
        let mut candidate = self.scene.object_part(object).unwrap().clone();
        let result = if field == FeatureField::Name {
            rename_feature(&mut candidate, feature, text)
                .map(|changed| changed.then(|| format!("Renamed to {}", text.trim())))
        } else {
            match field.parse(text) {
                None => Err(field.example().to_owned()),
                Some(value) => {
                    let mut outcome = Ok(None);
                    candidate.edit_feature(feature, |feature| {
                        let before = params::value(feature, field);
                        outcome = params::set(feature, field, value).map(|()| {
                            (params::value(feature, field) != before)
                                .then(|| "Value updated · Regenerating…".to_owned())
                        });
                    });
                    outcome
                }
            }
        };
        match result {
            Err(message) => {
                self.status = message;
                cx.notify();
            }
            Ok(None) => {
                self.active_field = None;
                self.status = "Value unchanged".into();
                cx.notify();
            }
            Ok(Some(status)) => {
                if let Err(error) = candidate.validate() {
                    self.status = format!("{error:#}");
                    cx.notify();
                    return;
                }
                self.active_field = None;
                // A scrub records one undo step when the drag ends.
                if !self.panels.is_scrubbing() {
                    self.history.checkpoint(&self.scene);
                }
                *self.scene.object_part_mut(object).unwrap() = candidate;
                self.dirty = true;
                self.status = status;
                if field == FeatureField::Name {
                    cx.notify();
                } else {
                    self.invalidate(true, cx);
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Sketch associativity.

    /// Bring every feature linked to the sketch object `sketch` up to date.
    /// Called after each committed sketch edit; part of that edit's undo step.
    pub(crate) fn sync_sketch_links(&mut self, sketch: u64, cx: &mut Context<Self>) {
        if links::propagate(&mut self.scene, sketch).is_empty() {
            return;
        }
        self.dirty = true;
        self.invalidate(true, cx);
    }

    /// Open the selected feature's sketch in Draw mode. A deleted source is
    /// rebuilt from the feature's embedded copy as a hidden sketch and
    /// linked; a hidden source left behind by a moved part is put back on
    /// the part first. Finishing returns to the part with the feature
    /// selected. Hidden sketches stay hidden: Draw shows the sketch it edits
    /// regardless, so no visibility changes enter the undo history.
    pub(crate) fn edit_feature_sketch(&mut self, cx: &mut Context<Self>) {
        let Some(selected) = self.selected_feature() else {
            self.status = "Select a feature in the timeline first".into();
            cx.notify();
            return;
        };
        let object = selected.object;
        let feature_id = selected.feature.id;
        let name = selected.feature.name.clone();
        let Some(embedded) = feature_sketch(selected.feature).cloned() else {
            self.status = format!("{name} is not made from a sketch");
            cx.notify();
            return;
        };
        let source = selected
            .feature
            .source
            .filter(|id| self.scene.object_sketch(*id).is_some());
        let before = self.scene.clone();
        let mut changed = false;
        let sketch = match source {
            Some(id) => {
                if !self.scene.is_effectively_visible(id)
                    && !links::is_aligned(&self.scene, id, object, &embedded)
                {
                    links::align_to_feature(&mut self.scene, id, object, embedded.plane);
                    changed = true;
                }
                id
            }
            None => {
                match links::materialize(
                    &mut self.scene,
                    object,
                    &embedded,
                    &format!("{name} sketch"),
                ) {
                    Ok(id) => {
                        // Provenance only: no geometry change, no regeneration.
                        if let Some(part) = self.scene.object_part_mut(object)
                            && let Some(feature) =
                                part.features.iter_mut().find(|f| f.id == feature_id)
                        {
                            feature.source = Some(id);
                        }
                        changed = true;
                        id
                    }
                    Err(error) => {
                        self.status = format!("Edit sketch: {error:#}");
                        cx.notify();
                        return;
                    }
                }
            }
        };
        if changed {
            self.history.checkpoint_owned(before);
            self.dirty = true;
        }
        let part_name = self
            .scene
            .object(object)
            .map(|o| o.name.clone())
            .unwrap_or_default();
        self.panels.timeline.sketch_return = Some(SketchReturn {
            selection: Selection {
                object,
                feature: feature_id,
            },
            sketch,
        });
        self.enter_draw(DrawTarget::Object(sketch), cx);
        if self.is_drawing() {
            self.status = format!(
                "Editing {name}'s sketch · {part_name} follows each change · Return to finish"
            );
        }
    }

    /// Called when Draw mode finishes: catch up any linked features and, for
    /// a sketch opened from the timeline, return to its part and feature.
    pub(crate) fn timeline_after_draw(&mut self, edited: Option<u64>, cx: &mut Context<Self>) {
        if let Some(sketch) = edited {
            self.sync_sketch_links(sketch, cx);
        }
        let Some(back) = self.panels.timeline.sketch_return.take() else {
            return;
        };
        if edited != Some(back.sketch) || self.scene.object(back.selection.object).is_none() {
            return;
        }
        let object = back.selection.object;
        self.select_objects([object].into_iter().collect(), Some(object));
        let exists = self
            .scene
            .object_part(object)
            .is_some_and(|part| part.feature(back.selection.feature).is_some());
        if exists {
            self.panels.timeline.select(back.selection);
            if let Some(selected) = self.selected_feature() {
                let regenerating = selected.part.is_pending();
                self.status = format!(
                    "{} updated{}",
                    selected.feature.name,
                    if regenerating {
                        " · Regenerating…"
                    } else {
                        ""
                    }
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forma_core::{
        ExtrudeExtent, History, Placement, PrimitiveFeature, PrimitiveShape, Scene, SketchCurve,
        SketchPlane, Vec2, Vec3,
    };

    fn part_with(count: usize) -> PartData {
        let mut part = PartData::new(0, "Part");
        for index in 0..count {
            part.push_feature(
                format!("Box {index}"),
                FeatureKind::Primitive(PrimitiveFeature {
                    shape: PrimitiveShape::Box {
                        size: Vec3::splat(1.0 + index as f32),
                    },
                    placement: Placement::default(),
                    operation: FeatureOperation::Join,
                }),
            );
        }
        part
    }

    /// A minimal editor: the scene, undo history and timeline selection, with
    /// the same one-checkpoint-per-edit discipline as `Studio`.
    struct Editor {
        scene: Scene,
        history: History,
        object: u64,
        selection: Option<u64>,
    }

    impl Editor {
        fn new(count: usize) -> Self {
            let mut scene = Scene::empty();
            let object = scene.add_part("Part", part_with(count)).unwrap();
            Self {
                scene,
                history: History::default(),
                object,
                selection: None,
            }
        }

        fn part(&self) -> &PartData {
            self.scene.object_part(self.object).unwrap()
        }

        fn ids(&self) -> Vec<u64> {
            self.part().features.iter().map(|f| f.id).collect()
        }

        fn edit<T>(&mut self, edit: impl FnOnce(&mut PartData) -> Result<T, String>) -> Option<T> {
            let mut candidate = self.part().clone();
            let value = edit(&mut candidate).ok()?;
            candidate.validate().unwrap();
            self.history.checkpoint(&self.scene);
            *self.scene.object_part_mut(self.object).unwrap() = candidate;
            Some(value)
        }

        /// The selection as the UI resolves it: only if the feature exists.
        fn resolved(&self) -> Option<u64> {
            self.selection
                .filter(|id| self.part().feature(*id).is_some())
        }

        fn undo(&mut self) {
            assert!(self.history.undo(&mut self.scene));
        }

        fn redo(&mut self) {
            assert!(self.history.redo(&mut self.scene));
        }
    }

    #[test]
    fn selection_steps_clamp_and_survive_undo() {
        let mut editor = Editor::new(3);
        let ids = editor.ids();
        assert_eq!(neighbour(editor.part(), ids[0], -1), Some(ids[0]));
        assert_eq!(neighbour(editor.part(), ids[0], 1), Some(ids[1]));
        assert_eq!(neighbour(editor.part(), ids[2], 1), Some(ids[2]));
        editor.selection = Some(ids[2]);
        let next = editor
            .edit(|part| delete_feature(part, ids[2]))
            .expect("delete");
        assert_eq!(
            next,
            Some(ids[1]),
            "deleting the last selects the one before"
        );
        assert_eq!(
            editor.resolved(),
            None,
            "a deleted feature is never selected"
        );
        editor.selection = next;
        editor.undo();
        assert_eq!(editor.ids(), ids, "one undo step restores the feature");
        assert_eq!(editor.resolved(), Some(ids[1]));
        editor.redo();
        assert_eq!(editor.ids(), &ids[..2]);
    }

    #[test]
    fn delete_keeps_the_marker_on_the_same_features() {
        let mut editor = Editor::new(4);
        let ids = editor.ids();
        editor.edit(|part| Ok(set_marker(part, 3))).unwrap();
        assert_eq!(editor.part().rollback, Some(3));
        assert_eq!(editor.part().active_features().count(), 3);
        // Removing a feature before the marker moves the marker with it.
        let next = editor.edit(|part| delete_feature(part, ids[0])).unwrap();
        assert_eq!(next, Some(ids[1]));
        assert_eq!(editor.part().rollback, Some(2));
        let active: Vec<u64> = editor.part().active_features().map(|f| f.id).collect();
        assert_eq!(active, vec![ids[1], ids[2]]);
        // Removing the only feature after the marker returns it to the end.
        editor.edit(|part| delete_feature(part, ids[3])).unwrap();
        assert_eq!(editor.part().rollback, None);
        editor.undo();
        editor.undo();
        assert_eq!(editor.ids(), ids);
        assert_eq!(editor.part().rollback, Some(3));
        // The last remaining feature is protected.
        let mut single = part_with(1);
        let id = single.features[0].id;
        assert!(delete_feature(&mut single, id).is_err());
    }

    #[test]
    fn rollback_and_suppress_restamp_and_undo_as_single_steps() {
        let mut editor = Editor::new(3);
        let ids = editor.ids();
        let stamp = editor.part().stamp;
        assert_eq!(marker(editor.part()), 3);
        assert!(
            !set_marker(&mut editor.part().clone(), 3),
            "already at the end"
        );
        editor.edit(|part| Ok(set_marker(part, 1))).unwrap();
        assert_eq!(marker(editor.part()), 1);
        assert_ne!(editor.part().stamp, stamp);
        let rolled = editor.part().stamp;
        let suppressed = editor.edit(|part| toggle_suppress(part, ids[0])).unwrap();
        assert!(suppressed);
        assert_eq!(editor.part().active_features().count(), 0);
        editor.undo();
        assert_eq!(editor.part().stamp, rolled);
        assert!(!editor.part().features[0].suppressed);
        editor.undo();
        assert_eq!(
            editor.part().stamp,
            stamp,
            "identical content, identical stamp"
        );
        assert_eq!(editor.part().rollback, None);
        // Marker positions past the end clamp to the end.
        let mut part = editor.part().clone();
        assert!(set_marker(&mut part, 0));
        assert!(set_marker(&mut part, 99));
        assert_eq!(part.rollback, None);
    }

    #[test]
    fn renames_keep_the_stamp_and_validate() {
        let mut part = part_with(2);
        let id = part.features[1].id;
        let stamp = part.stamp;
        assert!(rename_feature(&mut part, id, "  Lid  ").unwrap());
        assert_eq!(part.feature(id).unwrap().name, "Lid");
        assert_eq!(part.stamp, stamp, "names never trigger a regeneration");
        assert!(!rename_feature(&mut part, id, "Lid").unwrap());
        assert!(rename_feature(&mut part, id, "").is_err());
        assert!(rename_feature(&mut part, id, "two\nlines").is_err());
        assert!(rename_feature(&mut part, 999, "Ghost").is_err());
    }

    #[test]
    fn menus_offer_sketch_editing_only_for_sketch_features_and_skip_separators() {
        let part = part_with(1);
        let entries = menu_entries(&part.features[0]);
        assert!(
            entries
                .iter()
                .flatten()
                .all(|e| e.command != TimelineCommand::EditSketch)
        );
        let separator = entries.iter().position(Option::is_none).unwrap();
        let before = separator - 1;
        assert_eq!(menu_step(&entries, before, 1), separator + 1);
        assert_eq!(menu_step(&entries, separator + 1, -1), before);
        assert_eq!(menu_step(&entries, entries.len() - 1, 1), 0, "wraps");
        let mut sketch = Sketch::new(SketchPlane::ground());
        sketch.entities.push(
            SketchCurve::Circle {
                center: Vec2::ZERO,
                radius: 0.5,
            }
            .into(),
        );
        let extrude = Feature::new(
            9,
            "Extrude",
            FeatureKind::Extrude(forma_core::ExtrudeFeature {
                sketch,
                profiles: vec![Vec2::ZERO, Vec2::new(5.0, 5.0)],
                distance: 1.0,
                extent: ExtrudeExtent::OneSide,
                flip: false,
                operation: FeatureOperation::Join,
            }),
        );
        assert_eq!(
            menu_entries(&extrude)[0].unwrap().command,
            TimelineCommand::EditSketch
        );
        let mut with_extrude = part_with(1);
        with_extrude.features.push(extrude);
        with_extrude.next_feature_id = 10;
        let warnings = pick_warnings(&with_extrude);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].1.starts_with("1 of 2 picked regions"));
    }

    #[test]
    fn issue_text_drops_the_feature_name_prefix() {
        let issue = FeatureIssue {
            feature: 3,
            message: "Bore: the cut removed nothing".into(),
            suggestion: String::new(),
            warning: false,
        };
        assert_eq!(issue_text(&issue), "The cut removed nothing");
        let part_level = FeatureIssue {
            feature: 0,
            message: "2 faces could not be tessellated".into(),
            suggestion: String::new(),
            warning: false,
        };
        assert_eq!(issue_text(&part_level), "2 faces could not be tessellated");
    }
}
