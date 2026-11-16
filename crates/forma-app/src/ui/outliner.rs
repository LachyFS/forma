//! Scene outliner: collections and objects as a tree, with kind icons,
//! selection, visibility and inline rename.
use super::assets::{Icon, icon};
use super::components::*;
use super::tokens::*;
use super::{Anchor, MenuKind};
use crate::app::{Command, EditMode, Field, Studio};
use crate::theme::Colors;
use forma_core::{Object, Scene};
use gpui::{prelude::*, *};
use std::collections::{BTreeSet, HashMap};

pub(super) const ROW: f32 = 26.;
pub(super) const HEADER: f32 = 36.;
const INDENT: f32 = 14.;

/// Icon for an object's kind. Unknown kinds (for example a future CAD part)
/// fall back on their kind name.
pub(crate) fn kind_icon(kind: &str) -> Icon {
    match kind {
        "mesh" => Icon::Mesh,
        "part" => Icon::Part,
        "sketch" => Icon::Sketch,
        "light" => Icon::Light,
        "camera" => Icon::Camera,
        _ => Icon::Empty,
    }
}

/// Human name for an object kind, for badges and tooltips.
pub(crate) fn kind_label(kind: &str) -> &'static str {
    match kind {
        "mesh" => "Mesh",
        "part" => "Part",
        "sketch" => "Sketch",
        "light" => "Light",
        "camera" => "Camera",
        "empty" => "Empty",
        _ => "Object",
    }
}

fn kind_ink(t: Colors, kind: &str) -> u32 {
    match kind {
        "light" => t.warning,
        "sketch" => t.accent_text,
        _ => t.text_muted,
    }
}

struct Tree<'a> {
    scene: &'a Scene,
    selected: BTreeSet<u64>,
    active: Option<u64>,
    /// Children listed under each parent (those sharing its collection),
    /// built once so drawing the tree stays linear in the object count.
    children: HashMap<u64, Vec<&'a Object>>,
}

impl<'a> Tree<'a> {
    fn new(scene: &'a Scene, selected: BTreeSet<u64>, active: Option<u64>) -> Self {
        let mut tree = Self {
            scene,
            selected,
            active,
            children: HashMap::new(),
        };
        let mut children: HashMap<u64, Vec<&'a Object>> = HashMap::new();
        for object in &scene.objects {
            if let Some(parent) = object.parent.and_then(|id| scene.object(id))
                && tree.home(parent) == tree.home(object)
            {
                children.entry(parent.id).or_default().push(object);
            }
        }
        tree.children = children;
        tree
    }
}

impl Tree<'_> {
    /// The collection an object is listed under: its first link, or the root.
    fn home(&self, object: &Object) -> u64 {
        object
            .collections
            .first()
            .copied()
            .unwrap_or(self.scene.root_collection)
    }

    /// Objects listed directly under `collection`: members whose parent is
    /// not listed in the same collection.
    fn roots(&self, collection: u64) -> impl Iterator<Item = &Object> {
        self.scene.objects.iter().filter(move |object| {
            self.home(object) == collection
                && object.parent.is_none_or(|parent| {
                    self.scene
                        .object(parent)
                        .is_none_or(|parent| self.home(parent) != collection)
                })
        })
    }

    /// Children nested under `parent`, when they share its collection.
    fn children(&self, parent: &Object) -> impl Iterator<Item = &Object> {
        self.children.get(&parent.id).into_iter().flatten().copied()
    }

    fn count(&self, collection: u64) -> usize {
        self.scene
            .objects
            .iter()
            .filter(|object| self.home(object) == collection)
            .count()
    }
}

fn object_row(
    t: Colors,
    s: &Studio,
    tree: &Tree,
    object: &Object,
    depth: usize,
    rows: &mut Vec<AnyElement>,
    cx: &mut Context<Studio>,
) {
    let id = object.id;
    let selected = tree.selected.contains(&id);
    let active = tree.active == Some(id);
    let visible = object.visible;
    let kind = object.kind();
    let renaming = active && s.panels.rename_in_outliner && s.field_is_active(Field::Name);
    let ink = if visible { t.text } else { t.text_faint };
    let glyph_ink = if !visible {
        t.text_faint
    } else if selected {
        t.accent_text
    } else {
        kind_ink(t, kind)
    };
    let group = SharedString::from(format!("outliner-row-{id}"));
    let row_element = row()
        .id(SharedString::from(format!("object-{id}")))
        .relative()
        .h(px(ROW))
        .flex_shrink_0()
        .pl(px(SP_3 + depth as f32 * INDENT))
        .pr(px(SP_1))
        .mx(px(SP_1))
        .gap(px(SP_1_5))
        .rounded(px(R_SM))
        .group(group.clone())
        .bg(if selected {
            hex(t.accent_soft)
        } else {
            clear()
        })
        .cursor_pointer()
        .hover(move |d| {
            d.bg(rgb(if selected {
                t.accent_soft_hover
            } else {
                t.control_hover
            }))
        })
        .when(active, |d| {
            d.child(
                div()
                    .absolute()
                    .left(px(0.))
                    .top(px(6.))
                    .bottom(px(6.))
                    .w(px(2.))
                    .rounded_full()
                    .bg(rgb(t.accent)),
            )
        })
        .child(icon(kind_icon(kind), ICON_SM, glyph_ink))
        .child(if renaming {
            numeric_field(
                t,
                s,
                "outliner-rename",
                Field::Name,
                FieldMark::None,
                "",
                cx,
            )
            .h(px(ROW - 4.))
            .flex_1()
            .min_w(px(0.))
            .into_any_element()
        } else {
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_color(rgb(ink))
                .when(active, |d| d.font_weight(WEIGHT_MEDIUM))
                .child(object.name.clone())
                .into_any_element()
        })
        .child(
            div()
                .flex()
                .when(visible, |d| {
                    d.opacity(0.).group_hover(group.clone(), |d| d.opacity(1.))
                })
                .child(on_command(
                    icon_button(
                        t,
                        SharedString::from(format!("visibility-{id}")),
                        if visible { Icon::Eye } else { Icon::EyeOff },
                        false,
                        H_XS,
                    )
                    .tooltip(tip(
                        t,
                        if visible { "Hide" } else { "Show" },
                        None,
                    )),
                    Command::ToggleVisible(id),
                    cx,
                )),
        )
        .on_click(cx.listener(move |s, event: &ClickEvent, w, cx| {
            if event.modifiers().shift && s.edit_mode == EditMode::Object {
                if s.transform_drag.is_some() {
                    s.finish_transform(false, cx);
                }
                if s.transform_drag.is_some() {
                    return;
                }
                let mut ids = s.selected_ids();
                if !ids.insert(id) {
                    ids.remove(&id);
                }
                s.select_objects(ids, Some(id));
                s.invalidate(false, cx);
            } else {
                s.execute(Command::Select(id), w, cx);
                if event.click_count() == 2 && s.selected == Some(id) {
                    s.begin_field(Field::Name, w, cx);
                    s.panels.rename_in_outliner = true;
                }
            }
        }));
    rows.push(row_element.into_any_element());
    for child in tree.children(object) {
        object_row(t, s, tree, child, depth + 1, rows, cx);
    }
}

fn collection_rows(
    t: Colors,
    s: &Studio,
    tree: &Tree,
    collection: &forma_core::Collection,
    depth: usize,
    rows: &mut Vec<AnyElement>,
    cx: &mut Context<Studio>,
) {
    let id = collection.id;
    let open = !s.panels.collapsed_collections.contains(&id);
    let count = tree.count(id);
    rows.push(
        row()
            .id(SharedString::from(format!("collection-{id}")))
            .h(px(ROW))
            .flex_shrink_0()
            .pl(px(SP_1 + depth as f32 * INDENT))
            .pr(px(SP_2))
            .mx(px(SP_1))
            .gap(px(SP_1_5))
            .rounded(px(R_SM))
            .cursor_pointer()
            .hover(move |d| d.bg(rgb(t.control_hover)))
            .child(icon(
                if open {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                },
                12.,
                t.text_faint,
            ))
            .child(icon(
                if open {
                    Icon::CollectionOpen
                } else {
                    Icon::Collection
                },
                ICON_SM,
                if collection.visible {
                    t.text_muted
                } else {
                    t.text_faint
                },
            ))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .font_weight(WEIGHT_MEDIUM)
                    .text_color(rgb(if collection.visible {
                        t.text
                    } else {
                        t.text_faint
                    }))
                    .child(collection.name.clone()),
            )
            .child(
                caption(t, count.to_string())
                    .tabular()
                    .text_color(rgb(t.text_faint)),
            )
            .tooltip(tip(t, if open { "Collapse" } else { "Expand" }, None))
            .on_click(cx.listener(move |s, _, _, cx| {
                cx.stop_propagation();
                let collapsed = &mut s.panels.collapsed_collections;
                if !collapsed.remove(&id) {
                    collapsed.insert(id);
                }
                cx.notify();
            }))
            .into_any_element(),
    );
    if !open {
        return;
    }
    for object in tree.roots(id) {
        object_row(t, s, tree, object, depth + 1, rows, cx);
    }
    for child in tree
        .scene
        .collections
        .iter()
        .filter(|candidate| candidate.parent == Some(id))
    {
        collection_rows(t, s, tree, child, depth + 1, rows, cx);
    }
}

pub(super) fn outliner(t: Colors, s: &Studio, cx: &mut Context<Studio>) -> Div {
    let tree = Tree::new(&s.scene, s.selected_ids(), s.selected);
    let mut rows = Vec::with_capacity(s.scene.objects.len() + s.scene.collections.len());
    if let Some(root) = s.scene.collection(s.scene.root_collection) {
        collection_rows(t, s, &tree, root, 0, &mut rows, cx);
    }
    let menu_open = s
        .panels
        .menu
        .as_ref()
        .is_some_and(|menu| menu.kind == MenuKind::Add && menu.from == Anchor::Outliner);
    col()
        .min_h(px(0.))
        .child(
            row()
                .h(px(HEADER))
                .flex_shrink_0()
                .pl(px(SP_3))
                .pr(px(SP_1_5))
                .gap(px(SP_2))
                .child(
                    div()
                        .text_size(px(TEXT_MD))
                        .font_weight(WEIGHT_SEMIBOLD)
                        .child("Scene"),
                )
                .child(spacer())
                .child(
                    icon_button(t, "outliner-add", Icon::Plus, menu_open, H_SM)
                        .relative()
                        .child(s.panels.anchor_cell(Anchor::Outliner).element())
                        .tooltip(tip_detail(
                            t,
                            "Add",
                            "Meshes, parts, sketches and imports",
                            None,
                        ))
                        .on_click(cx.listener(|s, _, _, cx| {
                            cx.stop_propagation();
                            s.open_menu(MenuKind::Add, Anchor::Outliner, cx);
                        })),
                ),
        )
        .child(
            col()
                .id("scene-objects")
                .flex_1()
                .min_h(px(0.))
                .pb(px(SP_2))
                .overflow_y_scroll()
                .track_scroll(&s.panels.outliner_scroll)
                .children(rows)
                .when(s.scene.objects.is_empty(), |d| {
                    d.child(
                        div()
                            .pl(px(SP_3 + INDENT + 20.))
                            .py(px(SP_1))
                            .text_color(rgb(t.text_faint))
                            .child("No objects yet"),
                    )
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::Tree;
    use forma_core::{Primitive, Scene};
    use std::collections::BTreeSet;

    #[test]
    fn children_are_grouped_once_and_stay_in_their_collection() {
        let mut scene = Scene::empty();
        let parent = scene.add(Primitive::Cube);
        let kids: Vec<u64> = (0..3).map(|_| scene.add(Primitive::Cube)).collect();
        for &kid in &kids {
            scene.set_parent(kid, Some(parent), false).unwrap();
        }
        // A child linked into another collection is listed there instead.
        let other = scene.add_collection("Other", None).unwrap();
        scene.link_object(kids[2], other).unwrap();
        scene.unlink_object(kids[2], scene.root_collection).unwrap();
        let tree = Tree::new(&scene, BTreeSet::new(), None);
        let listed: Vec<u64> = tree
            .children(scene.object(parent).unwrap())
            .map(|o| o.id)
            .collect();
        assert_eq!(listed, kids[..2]);
        // Large flat scenes build the tree in linear time.
        let mut scene = Scene::empty();
        let root = scene.add(Primitive::Plane);
        for _ in 0..1500 {
            let id = scene.add(Primitive::Plane);
            scene.set_parent(id, Some(root), false).unwrap();
        }
        let started = std::time::Instant::now();
        let tree = Tree::new(&scene, BTreeSet::new(), None);
        let total: usize = scene.objects.iter().map(|o| tree.children(o).count()).sum();
        assert_eq!(total, 1500);
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }
}
