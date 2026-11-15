//! `--ui-capture` states for the part timeline and feature inspector.
use crate::app::{Command, Studio};
use crate::capture::{Scenario, leave_parts, parts, resize, restore_size};
use crate::theme::Theme;
use crate::timeline::TimelineCommand;
use crate::ui::PanelSection;
use forma_core::{FaceRef, FeatureKind, Vec2, Vec3};
use forma_render::RenderMode;
use gpui::{Context, Window};

fn object_named(s: &Studio, name: &str) -> Option<u64> {
    s.scene
        .objects
        .iter()
        .find(|object| object.name == name)
        .map(|object| object.id)
}

/// The demo scene with `object` active and its feature `feature` selected.
fn feature(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>, object: &str, feature: &str) {
    parts(s, w, cx, RenderMode::MaterialPreview, Some(object));
    select(s, w, cx, object, feature);
}

fn select(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>, object: &str, feature: &str) {
    let Some(id) = object_named(s, object) else {
        return;
    };
    let target = s
        .scene
        .object_part(id)
        .and_then(|part| part.features.iter().find(|f| f.name == feature))
        .map(|f| f.id);
    if let Some(target) = target {
        s.execute(Command::Timeline(TimelineCommand::Select(target)), w, cx);
    }
}

/// Edit the active part's history directly (capture setup, not undoable).
fn edit_part(s: &mut Studio, object: &str, edit: impl FnOnce(&mut forma_core::PartData)) {
    if let Some(part) = object_named(s, object).and_then(|id| s.scene.object_part_mut(id)) {
        edit(part);
        part.restamp();
    }
}

fn leave_theme(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    s.apply_theme(Theme::Forma, cx);
    leave_parts(s, w, cx);
}

fn leave_narrow(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    leave_parts(s, w, cx);
    restore_size(s, w, cx);
}

fn leave_draw(s: &mut Studio, w: &mut Window, cx: &mut Context<Studio>) {
    s.abandon_draw();
    leave_parts(s, w, cx);
}

pub(crate) fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            // The startup bracket with a sketch-based cut selected.
            name: "timeline-startup",
            enter: |s, w, cx| select(s, w, cx, crate::startup::HERO_PART, "Upright holes"),
            // Object selection resets the feature selection.
            leave: |s, _, cx| crate::capture::select_hero(s, cx),
        },
        Scenario {
            name: "timeline",
            enter: |s, w, cx| parts(s, w, cx, RenderMode::MaterialPreview, Some("Bracket")),
            leave: leave_parts,
        },
        Scenario {
            name: "timeline-selected",
            enter: |s, w, cx| feature(s, w, cx, "Bracket", "Holes"),
            leave: leave_parts,
        },
        Scenario {
            name: "timeline-parameters",
            enter: |s, w, cx| feature(s, w, cx, "Block", "Round"),
            leave: leave_parts,
        },
        Scenario {
            name: "timeline-issues",
            enter: |s, w, cx| {
                parts(s, w, cx, RenderMode::MaterialPreview, Some("Bracket"));
                edit_part(s, "Bracket", |part| {
                    // One pick that still finds a hole and one that does not.
                    if let Some(holes) = part.features.iter_mut().find(|f| f.name == "Holes")
                        && let FeatureKind::Extrude(extrude) = &mut holes.kind
                    {
                        extrude.profiles = vec![Vec2::new(-0.8, -0.48), Vec2::new(5.0, 5.0)];
                    }
                    // A push/pull whose face no longer exists.
                    part.push_feature(
                        "Lip",
                        FeatureKind::PushPull(forma_core::PushPullFeature {
                            face: FaceRef {
                                point: Vec3::new(4.0, 2.0, 0.0),
                                normal: Vec3::Y,
                            },
                            distance: 0.1,
                        }),
                    );
                });
                s.invalidate(true, cx);
                select(s, w, cx, "Bracket", "Lip");
            },
            leave: leave_parts,
        },
        Scenario {
            name: "timeline-rollback",
            enter: |s, w, cx| {
                parts(s, w, cx, RenderMode::MaterialPreview, Some("Bracket"));
                edit_part(s, "Bracket", |part| part.rollback = Some(3));
                s.invalidate(true, cx);
            },
            leave: leave_parts,
        },
        Scenario {
            name: "timeline-suppressed",
            enter: |s, w, cx| {
                parts(s, w, cx, RenderMode::MaterialPreview, Some("Bracket"));
                edit_part(s, "Bracket", |part| {
                    if let Some(boss) = part.features.iter_mut().find(|f| f.name == "Boss") {
                        boss.suppressed = true;
                    }
                });
                s.invalidate(true, cx);
                select(s, w, cx, "Bracket", "Boss");
            },
            leave: leave_parts,
        },
        Scenario {
            name: "timeline-extrude",
            enter: |s, w, cx| feature(s, w, cx, "Bracket", "Boss"),
            leave: leave_parts,
        },
        Scenario {
            name: "timeline-menu",
            enter: |s, w, cx| {
                feature(s, w, cx, "Bracket", "Holes");
                let bounds = s.bounds.get();
                let position = gpui::point(
                    bounds.center().x - gpui::px(150.),
                    bounds.bottom() - gpui::px(40.),
                );
                if let Some(id) = s.selected_feature().map(|f| f.feature.id) {
                    s.open_feature_menu(id, position, cx);
                }
            },
            leave: |s, w, cx| {
                s.panels.timeline.menu = None;
                leave_parts(s, w, cx);
            },
        },
        Scenario {
            name: "timeline-light",
            enter: |s, w, cx| {
                s.apply_theme(Theme::Paper, cx);
                feature(s, w, cx, "Bracket", "Boss");
            },
            leave: leave_theme,
        },
        Scenario {
            name: "timeline-light-summary",
            enter: |s, w, cx| {
                s.apply_theme(Theme::Paper, cx);
                parts(s, w, cx, RenderMode::MaterialPreview, Some("Block"));
            },
            leave: leave_theme,
        },
        Scenario {
            name: "timeline-narrow",
            enter: |s, w, cx| {
                resize(w, 1000., 720.);
                feature(s, w, cx, "Bracket", "Holes");
            },
            leave: leave_narrow,
        },
        Scenario {
            name: "timeline-overflow",
            enter: |s, w, cx| {
                resize(w, 1000., 720.);
                parts(s, w, cx, RenderMode::MaterialPreview, Some("Block"));
                edit_part(s, "Block", |part| {
                    // Small vertical holes in a ring: a long, realistic history.
                    for index in 0..9 {
                        let angle = index as f32 / 9.0 * std::f32::consts::TAU;
                        part.push_feature(
                            format!("Pin {}", index + 1),
                            FeatureKind::Primitive(forma_core::PrimitiveFeature {
                                shape: forma_core::PrimitiveShape::Cylinder {
                                    radius: 0.05,
                                    height: 2.0,
                                },
                                placement: forma_core::Placement {
                                    origin: Vec3::new(angle.cos() * 0.45, 0.65, angle.sin() * 0.45),
                                    rotation: Vec3::ZERO,
                                },
                                operation: forma_core::FeatureOperation::Cut,
                            }),
                        );
                    }
                });
                s.invalidate(true, cx);
                select(s, w, cx, "Block", "Pin 9");
            },
            leave: leave_narrow,
        },
        Scenario {
            name: "timeline-edit-sketch",
            enter: |s, w, cx| {
                feature(s, w, cx, "Bracket", "Holes");
                s.execute(Command::Timeline(TimelineCommand::EditSketch), w, cx);
                s.complete_camera_animation(cx);
            },
            leave: leave_draw,
        },
        Scenario {
            name: "combine-selection",
            enter: |s, w, cx| {
                parts(s, w, cx, RenderMode::MaterialPreview, Some("Block"));
                let ids: std::collections::BTreeSet<u64> = ["Knob", "Block"]
                    .iter()
                    .filter_map(|name| object_named(s, name))
                    .collect();
                let block = object_named(s, "Block");
                s.select_objects(ids, block);
                cx.notify();
            },
            leave: leave_parts,
        },
        Scenario {
            name: "mesh-convert",
            enter: |s, _, cx| {
                crate::capture::select_named(s, "Torus");
                for section in [
                    PanelSection::Transform,
                    PanelSection::Surface,
                    PanelSection::Textures,
                ] {
                    if s.panels.is_open(section) {
                        s.toggle_section(section, cx);
                    }
                }
                if !s.panels.is_open(PanelSection::Geometry) {
                    s.toggle_section(PanelSection::Geometry, cx);
                }
                cx.notify();
            },
            leave: |s, _, cx| {
                for section in [PanelSection::Transform, PanelSection::Surface] {
                    if !s.panels.is_open(section) {
                        s.toggle_section(section, cx);
                    }
                }
                if s.panels.is_open(PanelSection::Geometry) {
                    s.toggle_section(PanelSection::Geometry, cx);
                }
                crate::capture::select_hero(s, cx);
            },
        },
    ]
}
