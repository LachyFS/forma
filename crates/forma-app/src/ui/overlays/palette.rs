//! Command palette. With no query it opens on commands for the current
//! selection and recently run commands, then lists every group; a query ranks
//! matches across all groups and highlights the matched characters.
use super::super::assets::{Icon, icon};
use super::super::commands::{
    adds_part, command_group, command_icon, command_info, command_keywords, group_rank,
};
use super::super::components::*;
use super::super::tokens::*;
use super::{
    animate_in, backdrop, contain, deep_scrim, float_border, float_shadow, heading, hover_fill,
    selected_fill, selection_bar,
};
use crate::app::{Command, EditMode, Studio};
use crate::cad::{CadCommand, PartShape};
use crate::cad_tools::ToolCommand;
use crate::sketch::DrawCommand;
use crate::theme::Colors;
use forma_core::{FeatureOperation, ObjectData, Primitive};
use gpui::{prelude::*, *};
use std::{cell::RefCell, ops::Range};

const WIDTH: f32 = 600.;
const ROW: f32 = 36.;
const SEARCH: f32 = 52.;
const FOOTER: f32 = 36.;
/// Section headings in the unfiltered list, and the space above each one
/// after the first.
const HEADING: f32 = 28.;
const HEADING_GAP: f32 = SP_1_5;
/// Padding above the first row and below the last.
const LIST_TOP: f32 = SP_1_5;
const LIST_BOTTOM: f32 = SP_3;
/// While rows continue below, the list stops this far under its last whole
/// row: the next row's top shows through the fade there, so the cut reads
/// as "more below" and never as a row crushed against the footer.
const PEEK: f32 = 6.;
/// The fade laid over the list's lower edge while more rows sit below.
const FADE: f32 = 12.;
/// With an empty query, the Add group shows this many rows and then a
/// "More…" row, so it never crowds out the commands for the selection.
const ADD_PREVIEW: usize = 3;
const ADD: &str = "Add";
/// The whole card grows to this share of the window before its list scrolls.
const CARD_SHARE: f32 = 0.6;
/// Room for the highlighted command's description beside the footer keys.
const DETAIL_WIDTH: f32 = 380.;

/// Heading for commands that act on the current selection.
const FOR_SELECTION: &str = "For selection";
/// Heading for ways to start when nothing is selected.
const SUGGESTED: &str = "Suggested";
const RECENT: &str = "Recent";

/// One palette row.
pub(crate) struct PaletteEntry {
    pub label: &'static str,
    pub shortcut: &'static str,
    pub command: Command,
    /// Heading of the section the row belongs to in the unfiltered list.
    pub section: &'static str,
    /// What a row pinned under "For selection" acts on ("Part"). Every row
    /// in that section carries the same tag.
    pub scope: Option<&'static str>,
    /// Byte ranges of `label` that matched the query.
    pub matches: Vec<Range<usize>>,
    /// A "More…" row standing in for this many hidden rows of its section.
    /// Choosing it shows them in place; its `command` never runs.
    pub more: Option<usize>,
}

fn is_export(command: Command) -> bool {
    matches!(
        command,
        Command::ExportObj
            | Command::ExportStl
            | Command::ExportImage
            | Command::Cad(CadCommand::ExportStep)
    )
}

/// Where `word` occurs in `haystack`, preferring the start of a word.
/// Both are ASCII-lowercased, so byte offsets match the original label.
fn find_word(haystack: &str, word: &str) -> Option<(usize, bool)> {
    let mut first = None;
    for (position, _) in haystack.match_indices(word) {
        let word_start = haystack[..position]
            .chars()
            .next_back()
            .is_none_or(|previous| !previous.is_alphanumeric());
        if word_start {
            return Some((position, true));
        }
        first.get_or_insert((position, false));
    }
    first
}

/// Score `label` against the query `words` (lowercase). Every word must occur
/// in the label, the command's group or its search keywords; label matches
/// rank higher, earlier and at word starts higher still. Returns the score
/// and the matched label ranges, merged and in order.
pub(crate) fn match_command(
    label: &str,
    command: Command,
    words: &[String],
) -> Option<(u32, Vec<Range<usize>>)> {
    let lower = label.to_ascii_lowercase();
    let extra =
        format!("{} {}", command_group(command), command_keywords(command)).to_ascii_lowercase();
    let mut score = 0;
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for word in words {
        if let Some((position, word_start)) = find_word(&lower, word) {
            score += match (position, word_start) {
                (0, _) => 8,
                (_, true) => 5,
                _ => 2,
            };
            ranges.push(position..position + word.len());
        } else if find_word(&extra, word).is_some() {
            score += 1;
        } else {
            return None;
        }
    }
    if lower.starts_with(&words.join(" ")) {
        score += 10;
    }
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    Some((score, merged))
}

impl Studio {
    /// Every palette row in display order. Drawing and keyboard dispatch both
    /// read this list, so the highlighted row is always the one Return runs.
    pub(crate) fn palette_entries(&self) -> Vec<PaletteEntry> {
        let mut all = crate::app::palette_commands("");
        all.sort_by_key(|(_, _, command)| group_rank(command_group(*command)));
        if self.palette_exports_only {
            all.retain(|(_, _, command)| is_export(*command));
        }
        let words: Vec<String> = self
            .palette_query
            .split_whitespace()
            .map(str::to_ascii_lowercase)
            .collect();
        if !words.is_empty() {
            let mut ranked: Vec<(u32, PaletteEntry)> = all
                .into_iter()
                .filter_map(|(label, shortcut, command)| {
                    let (score, matches) = match_command(label, command, &words)?;
                    Some((
                        score,
                        PaletteEntry {
                            label,
                            shortcut,
                            command,
                            section: command_group(command),
                            scope: None,
                            matches,
                            more: None,
                        },
                    ))
                })
                .collect();
            ranked.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
            return ranked.into_iter().map(|(_, entry)| entry).collect();
        }
        let entry = |(label, shortcut, command): (&'static str, &'static str, Command),
                     section: &'static str| PaletteEntry {
            label,
            shortcut,
            command,
            section,
            scope: None,
            matches: Vec::new(),
            more: None,
        };
        if self.palette_exports_only {
            return all.into_iter().map(|item| entry(item, "Export")).collect();
        }
        let mut entries = Vec::new();
        let (context, suggested) = self.palette_context();
        let scope = (context == FOR_SELECTION).then(|| self.palette_scope());
        let pinned = suggested.iter().map(|command| (context, *command)).chain(
            self.panels
                .recent_commands()
                .iter()
                .map(|command| (RECENT, *command)),
        );
        for (section, command) in pinned {
            if let Some(position) = all.iter().position(|(_, _, c)| *c == command) {
                let mut pinned = entry(all.remove(position), section);
                if section == FOR_SELECTION {
                    pinned.scope = scope;
                }
                entries.push(pinned);
            }
        }
        let expanded = LIST.with(|list| list.borrow().expanded);
        let adds = all
            .iter()
            .filter(|(_, _, command)| command_group(*command) == ADD)
            .count();
        let fold = !expanded && adds > ADD_PREVIEW + 1;
        let mut shown = 0;
        for item in all {
            let section = command_group(item.2);
            if fold && section == ADD {
                shown += 1;
                if shown > ADD_PREVIEW {
                    if shown == ADD_PREVIEW + 1 {
                        entries.push(PaletteEntry {
                            label: "More…",
                            shortcut: "",
                            command: Command::TogglePalette,
                            section,
                            scope: None,
                            matches: Vec::new(),
                            more: Some(adds - ADD_PREVIEW),
                        });
                    }
                    continue;
                }
            }
            entries.push(entry(item, section));
        }
        entries
    }

    /// Choose the palette's "More…" row at `index`, if that is what it is:
    /// its hidden rows appear in its place, and the highlight stays on the
    /// first of them. Returns whether it was one.
    pub(crate) fn palette_expand_more(&mut self, index: usize) -> bool {
        let is_more = self
            .palette_entries()
            .get(index)
            .is_some_and(|entry| entry.more.is_some());
        if is_more {
            LIST.with(|list| list.borrow_mut().expanded = true);
            self.palette_index = index;
        }
        is_more
    }

    /// What the "For selection" commands act on, as one tag for the section.
    fn palette_scope(&self) -> &'static str {
        if self.is_drawing() {
            return "Sketch";
        }
        if self.part_editing().is_some() {
            return "Part";
        }
        if self.edit_mode.is_component() {
            return "Mesh";
        }
        let ids = self.selected_ids();
        let mut objects = ids.iter().filter_map(|id| self.scene.object(*id));
        match (objects.next(), objects.next()) {
            (Some(object), None) => super::super::outliner::kind_label(
                super::super::outliner::object_kind(&self.scene, object),
            ),
            _ => "Objects",
        }
    }

    /// Commands that act on what is selected or being edited; with nothing
    /// selected, ways to start.
    fn palette_context(&self) -> (&'static str, Vec<Command>) {
        if self.is_drawing() {
            return (FOR_SELECTION, vec![Command::Draw(DrawCommand::Finish)]);
        }
        if self.part_editing().is_some() {
            return (
                FOR_SELECTION,
                vec![
                    Command::CadTool(ToolCommand::Chamfer),
                    Command::CadTool(ToolCommand::PushPull),
                    Command::CadTool(ToolCommand::SketchOnFace),
                    Command::SetEditMode(EditMode::Object),
                ],
            );
        }
        if self.edit_mode.is_component() {
            return (
                FOR_SELECTION,
                vec![
                    Command::Extrude,
                    Command::Inset,
                    Command::SelectLinked,
                    Command::SetEditMode(EditMode::Object),
                ],
            );
        }
        let ids = self.selected_ids();
        let mut objects = ids.iter().filter_map(|id| self.scene.object(*id));
        match (objects.next(), objects.next()) {
            (None, _) if self.scene.objects.is_empty() => (
                SUGGESTED,
                vec![
                    Command::Draw(DrawCommand::NewSketch),
                    Command::Cad(CadCommand::AddPart(PartShape::Box)),
                    Command::Add(Primitive::Cube),
                    Command::Cad(CadCommand::ImportStep),
                ],
            ),
            (None, _) => (
                SUGGESTED,
                vec![
                    Command::Draw(DrawCommand::NewSketch),
                    Command::Cad(CadCommand::AddPart(PartShape::Box)),
                    Command::Add(Primitive::Cube),
                    Command::FrameAll,
                ],
            ),
            (Some(_), Some(_)) => (
                FOR_SELECTION,
                vec![
                    Command::Cad(CadCommand::Combine(FeatureOperation::Join)),
                    Command::Cad(CadCommand::Combine(FeatureOperation::Cut)),
                    Command::Cad(CadCommand::Combine(FeatureOperation::Intersect)),
                    Command::Duplicate,
                ],
            ),
            (Some(object), None) => (
                FOR_SELECTION,
                match &object.data {
                    ObjectData::Part { .. } => vec![
                        Command::CadTool(ToolCommand::Chamfer),
                        Command::CadTool(ToolCommand::PushPull),
                        Command::Cad(CadCommand::ConvertToMesh),
                        Command::Cad(CadCommand::ExportStep),
                    ],
                    ObjectData::Mesh { .. } => vec![
                        Command::SetEditMode(EditMode::Face),
                        Command::Subdivide,
                        Command::Cad(CadCommand::ConvertToPart),
                        Command::ExportStl,
                    ],
                    ObjectData::Sketch { .. } => vec![
                        Command::CadTool(ToolCommand::Extrude),
                        Command::CadTool(ToolCommand::Revolve),
                        Command::Draw(DrawCommand::EditSketch),
                    ],
                    _ => vec![Command::FrameSelected, Command::Duplicate, Command::Delete],
                },
            ),
        }
    }
}

/// The list's scroll position. It lives outside `Studio` because only the
/// view needs it; `rest` returns it to the top whenever the palette closes.
struct ListState {
    scroll: ScrollHandle,
    /// The selection (and query) the list last scrolled to, so the wheel can
    /// scroll freely until the keyboard moves the selection again.
    followed: Option<(usize, String, bool)>,
    /// The Add group's "More…" row was chosen; it stays open until the
    /// palette closes.
    expanded: bool,
}

thread_local! {
    static LIST: RefCell<ListState> = RefCell::new(ListState {
        scroll: ScrollHandle::new(),
        followed: None,
        expanded: false,
    });
}

/// Return the list to the top for the next time the palette opens.
pub(super) fn rest() {
    LIST.with(|list| {
        let mut list = list.borrow_mut();
        list.expanded = false;
        if list.followed.take().is_some() {
            list.scroll.set_offset(point(px(0.), px(0.)));
        }
    });
}

/// Keep the keyboard selection in view. GPUI drops a scroll request made
/// before the list's first layout, so until then this asks for another frame
/// and tries again.
fn follow(selected: usize, query: &str, exports: bool, cx: &mut Context<Studio>) -> ScrollHandle {
    LIST.with(|list| {
        let mut list = list.borrow_mut();
        let key = (selected, query.to_owned(), exports);
        if list.followed.as_ref() != Some(&key) {
            if selected == 0 {
                list.scroll.set_offset(point(px(0.), px(0.)));
                list.followed = Some(key);
            } else if list.scroll.bounds().size.height > px(0.) {
                list.scroll.scroll_to_item(selected);
                list.followed = Some(key);
            } else {
                cx.notify();
            }
        }
        list.scroll.clone()
    })
}

/// The label with matched characters in the accent ink.
fn label_text(t: Colors, entry: &PaletteEntry, enabled: bool) -> impl IntoElement {
    let ink = if enabled { t.text } else { t.text_faint };
    let highlight = HighlightStyle {
        color: Some(hex(if enabled { t.accent_text } else { t.text_muted })),
        font_weight: Some(WEIGHT_SEMIBOLD),
        ..Default::default()
    };
    div()
        .min_w(px(0.))
        .flex_shrink()
        .truncate()
        .text_size(px(TEXT_LG))
        .text_color(rgb(ink))
        .child(
            StyledText::new(entry.label)
                .with_highlights(entry.matches.iter().map(|range| (range.clone(), highlight))),
        )
}

fn command_row(
    t: Colors,
    s: &Studio,
    index: usize,
    entry: &PaletteEntry,
    active: bool,
    show_category: bool,
    cx: &mut Context<Studio>,
) -> Stateful<Div> {
    let command = entry.command;
    if let Some(hidden) = entry.more {
        return more_row(t, index, hidden, active, cx);
    }
    // Unavailable commands stay listed, dimmed, with the reason in place of
    // their shortcut: a format never silently disappears.
    let unavailable = s.command_unavailable(command);
    let enabled = unavailable.is_none();
    let glyph_ink = if !enabled {
        t.text_faint
    } else if active || adds_part(command) {
        t.accent_text
    } else {
        t.text_muted
    };
    row()
        .id(("palette-command", index))
        .relative()
        .h(px(ROW))
        .flex_shrink_0()
        .pl(px(SP_3))
        .pr(px(SP_2 + 2.))
        .gap(px(SP_2 + 2.))
        .rounded(px(R_MD))
        .when(enabled, |d| d.cursor_pointer())
        .bg(if active { selected_fill(t) } else { clear() })
        .when(!active && enabled, |d| {
            d.hover(move |d| d.bg(hover_fill(t)))
        })
        .when(active, |d| {
            d.child(selection_bar(
                if enabled { t.accent } else { t.text_faint },
                9.,
            ))
        })
        .child(match command_icon(command) {
            Some(glyph) => icon(glyph, ICON_MD, glyph_ink).into_any_element(),
            None => div().w(px(ICON_MD)).flex_shrink_0().into_any_element(),
        })
        .child(label_text(t, entry, enabled))
        // The tag reads as the label's subtitle, right beside it. Rows for
        // the selection all name what they act on; elsewhere it is the
        // command's category, left out where the label already starts with
        // it ("Export STEP…" under Export).
        .when_some(
            entry.scope.or_else(|| {
                let group = command_group(command);
                (show_category && !entry.label.starts_with(group)).then_some(group)
            }),
            |d, tag| {
                d.when(enabled, |d| {
                    d.child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(TEXT_MD))
                            .text_color(rgb(if active { t.text_muted } else { t.text_faint }))
                            .child(tag),
                    )
                })
            },
        )
        .child(spacer())
        // The reason a command cannot run takes the shortcut's place.
        .child(
            row()
                .flex_shrink_0()
                .when(enabled && !entry.shortcut.is_empty(), |d| {
                    d.child(keycaps(t, entry.shortcut))
                })
                .when_some(unavailable, |d, reason| {
                    d.text_size(px(TEXT_MD))
                        .text_color(rgb(t.text_muted))
                        .child(reason)
                }),
        )
        .when(enabled, |d| {
            d.on_click(cx.listener(move |s, _, w, cx| {
                s.palette_open = false;
                s.panels.remember_command(command);
                s.execute(command, w, cx);
            }))
        })
}

/// The row that stands in for the rest of a folded group.
fn more_row(
    t: Colors,
    index: usize,
    hidden: usize,
    active: bool,
    cx: &mut Context<Studio>,
) -> Stateful<Div> {
    row()
        .id(("palette-command", index))
        .relative()
        .h(px(ROW))
        .flex_shrink_0()
        .pl(px(SP_3))
        .pr(px(SP_2 + 2.))
        .gap(px(SP_2 + 2.))
        .rounded(px(R_MD))
        .cursor_pointer()
        .bg(if active { selected_fill(t) } else { clear() })
        .when(!active, |d| d.hover(move |d| d.bg(hover_fill(t))))
        .when(active, |d| d.child(selection_bar(t.accent, 9.)))
        .child(icon(
            Icon::More,
            ICON_MD,
            if active { t.accent_text } else { t.text_faint },
        ))
        .child(
            div()
                .text_size(px(TEXT_LG))
                .text_color(rgb(if active { t.text } else { t.text_muted }))
                .child("More…"),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(TEXT_MD))
                .text_color(rgb(t.text_faint))
                .child(format!("{hidden} more to add")),
        )
        .child(spacer())
        .on_click(cx.listener(move |s, _, _, cx| {
            cx.stop_propagation();
            s.palette_expand_more(index);
            cx.notify();
        }))
}

pub(super) fn overlay(
    t: Colors,
    s: &Studio,
    window: &Window,
    cx: &mut Context<Studio>,
) -> AnyElement {
    let entries = s.palette_entries();
    let selected = s.palette_index.min(entries.len().saturating_sub(1));
    let viewport = f32::from(window.viewport_size().height);
    let top = (viewport * 0.12f32).clamp(48., 140.);
    let exports = s.palette_exports_only;
    let query = s.palette_query.clone();
    let searching = !query.trim().is_empty();
    // Headings group the unfiltered list; search results are ranked across
    // groups instead, so each shows its group on the right.
    let grouped = !searching && !exports;
    let first_of_section = |index: usize| {
        grouped && (index == 0 || entries[index - 1].section != entries[index].section)
    };
    let budget = (viewport * CARD_SHARE)
        .min(viewport - top - SP_6)
        .max(SEARCH + FOOTER + ROW * 3.)
        - SEARCH
        - FOOTER;
    let list_max = list_height(
        (0..entries.len()).map(|index| {
            ROW + if !first_of_section(index) {
                0.
            } else if index == 0 {
                HEADING
            } else {
                HEADING + HEADING_GAP
            }
        }),
        budget,
    );
    let scroll = follow(selected, &query, exports, cx);

    let mut list = div()
        .id("palette-list")
        .flex()
        .flex_col()
        .max_h(px(list_max))
        .overflow_y_scroll()
        .track_scroll(&scroll)
        .px(px(SP_1_5))
        .pt(px(LIST_TOP))
        // Room under the last row, so it never meets the footer's rule.
        .pb(px(LIST_BOTTOM));
    for (index, entry) in entries.iter().enumerate() {
        let first_of_section = first_of_section(index);
        let pinned = matches!(entry.section, FOR_SELECTION | SUGGESTED | RECENT);
        // Each child is one row plus its heading, so scrolling a row into
        // view also reveals the heading above it.
        list = list.child(
            col()
                .flex_shrink_0()
                .when(first_of_section, |d| {
                    d.child(
                        heading(t, entry.section)
                            .h(px(HEADING))
                            .px(px(SP_3))
                            .when(index > 0, |d| d.mt(px(HEADING_GAP))),
                    )
                })
                .child(command_row(
                    t,
                    s,
                    index,
                    entry,
                    index == selected,
                    searching || (pinned && grouped),
                    cx,
                )),
        );
    }
    if entries.is_empty() {
        list = list.child(
            col()
                .items_center()
                .py(px(SP_6))
                .gap(px(SP_1_5))
                .child(icon(Icon::Search, ICON_LG, t.text_faint))
                .child(
                    div()
                        .text_size(px(TEXT_LG))
                        .text_color(rgb(t.text))
                        .child(if exports {
                            "No matching export formats"
                        } else {
                            "No matching commands"
                        }),
                )
                .child(caption(t, "Try a shorter or different word")),
        );
    }

    let search = row()
        .h(px(SEARCH))
        .flex_shrink_0()
        .px(px(SP_4 + 2.))
        .gap(px(SP_3))
        .border_b_1()
        .border_color(rgb(t.hairline))
        .child(icon(
            if exports { Icon::Export } else { Icon::Search },
            ICON_LG,
            t.text_muted,
        ))
        .child(
            row()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(px(TEXT_XL))
                .when(query.is_empty(), |d| {
                    d.child(div().w(px(1.5)).h(px(18.)).mr(px(1.)).bg(rgb(t.focus)))
                        .child(div().text_color(rgb(t.text_faint)).child(if exports {
                            "Export as…"
                        } else {
                            "Search commands"
                        }))
                })
                .when(!query.is_empty(), |d| {
                    d.child(div().text_color(rgb(t.text)).child(query.clone()))
                        .child(div().w(px(1.5)).h(px(18.)).ml(px(1.)).bg(rgb(t.focus)))
                }),
        )
        .when(searching, |d| {
            d.child(
                caption(
                    t,
                    format!(
                        "{} {}",
                        entries.len(),
                        if entries.len() == 1 {
                            "result"
                        } else {
                            "results"
                        }
                    ),
                )
                .tabular()
                .text_color(rgb(t.text_faint)),
            )
        });

    // While rows continue past an edge, a short shadow line under it says
    // so without dimming the row beside it.
    let max_scroll = scroll.max_offset().height;
    let offset = scroll.offset().y;
    let clipped_above = offset < px(-0.5);
    let clipped_below = max_scroll > px(0.5) && offset > -max_scroll + px(0.5);
    let list = div()
        .relative()
        .child(list)
        .when(clipped_above, |d| {
            d.child(
                scroll_shadow(t, false)
                    .absolute()
                    .left_0()
                    .right_0()
                    .top_0(),
            )
        })
        .when(clipped_below, |d| {
            // A fade into the card's own fill over the peeking row, with
            // the shadow line under it at the edge.
            let mut clear = hex(t.overlay);
            clear.a = 0.;
            d.child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(FADE))
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(clear, 0.),
                        linear_color_stop(hex(t.overlay), 0.85),
                    )),
            )
            .child(
                scroll_shadow(t, true)
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0(),
            )
        });

    // The footer explains the highlighted command in plain words; the keys
    // that act on it stay on the right.
    let detail = entries
        .get(selected)
        .filter(|entry| s.command_unavailable(entry.command).is_none())
        .and_then(|entry| command_info(entry.command).detail);
    let footer = row()
        .h(px(FOOTER))
        .flex_shrink_0()
        .px(px(SP_4 + 2.))
        .gap(px(SP_4))
        .border_t_1()
        .border_color(rgb(t.hairline))
        .bg(rgb(t.raised))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(px(TEXT_SM))
                .text_color(rgb(t.text_muted))
                .when_some(detail, |d, detail| {
                    d.child(fit_text(cx, detail.into(), TEXT_SM, DETAIL_WIDTH))
                }),
        )
        .child(hint(t, "Return", "Run"))
        .child(hint(t, "Esc", "Close"));

    backdrop(
        t,
        "command-palette-backdrop",
        false,
        |s, w, cx| s.execute(Command::TogglePalette, w, cx),
        cx,
    )
    .bg(deep_scrim(t))
    .flex()
    .flex_col()
    .items_center()
    .pt(px(top))
    .child(animate_in(
        contain(
            dialog(t)
                .id("command-menu")
                .w(px(WIDTH))
                .max_w(relative(0.92))
                .border_color(float_border(t))
                .shadow(float_shadow(t))
                .child(search)
                .child(list)
                .child(footer),
        ),
        "command-menu-enter",
    ))
    .into_any_element()
}

/// The list's height: everything when it fits in `budget`, otherwise the
/// tallest run of whole rows (each with its heading) that does, so the list
/// never opens on a cut row.
fn list_height(rows: impl IntoIterator<Item = f32>, budget: f32) -> f32 {
    let mut end = LIST_TOP;
    let mut whole = LIST_TOP;
    for height in rows {
        end += height;
        if end + PEEK <= budget {
            whole = end;
        }
    }
    if end + LIST_BOTTOM <= budget {
        end + LIST_BOTTOM
    } else {
        whole.max(LIST_TOP + ROW) + PEEK
    }
}

#[cfg(test)]
mod tests {
    use super::{HEADING, LIST_BOTTOM, LIST_TOP, PEEK, ROW, list_height, match_command};
    use crate::app::Command;
    use forma_render::RenderMode;

    fn words(query: &str) -> Vec<String> {
        query
            .split_whitespace()
            .map(str::to_ascii_lowercase)
            .collect()
    }

    fn spans(ranges: &[std::ops::Range<usize>]) -> Vec<(usize, usize)> {
        ranges
            .iter()
            .map(|range| (range.start, range.end))
            .collect()
    }

    #[test]
    fn matches_prefer_label_starts_and_highlight_label_ranges() {
        let wire = Command::SetMode(RenderMode::Wireframe);
        let (prefix, ranges) = match_command("Wireframe viewport", wire, &words("wire")).unwrap();
        assert_eq!(spans(&ranges), [(0, 4)]);
        let (inner, ranges) = match_command("Wireframe viewport", wire, &words("port")).unwrap();
        assert_eq!(spans(&ranges), [(14, 18)]);
        assert!(prefix > inner);
        // The group and keywords match without highlighting anything.
        let (_, ranges) = match_command("Wireframe viewport", wire, &words("shading")).unwrap();
        assert!(ranges.is_empty());
        assert!(match_command("Wireframe viewport", wire, &words("solid")).is_none());
    }

    #[test]
    fn the_list_ends_on_a_whole_row() {
        let rows = std::iter::once(HEADING + ROW).chain(std::iter::repeat_n(ROW, 30));
        let height = list_height(rows, 480.);
        assert!(height <= 480.);
        // Whole rows, then a few pixels of the next one under the fade.
        let after_heading = height - LIST_TOP - HEADING - PEEK;
        assert_eq!(after_heading % ROW, 0., "{height} cuts a row");
        assert!(
            480. - height < ROW,
            "{height} leaves a whole row's room unused"
        );
        // A short list keeps its bottom padding and never scrolls.
        let short = list_height([HEADING + ROW, ROW], 480.);
        assert_eq!(short, LIST_TOP + HEADING + ROW * 2. + LIST_BOTTOM);
    }

    #[test]
    fn overlapping_word_matches_merge() {
        let (_, ranges) = match_command(
            "Frame selected",
            Command::FrameSelected,
            &words("fra frame"),
        )
        .unwrap();
        assert_eq!(spans(&ranges), [(0, 5)]);
    }
}
