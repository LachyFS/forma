//! Small native multiline shader editor: UTF-8 selections, IME input, clipboard,
//! local undo/redo and scrolling. Document history is touched only by Apply.
use gpui::{prelude::*, *};
use std::ops::Range;

const ROW: f32 = 20.0;
const GUTTER: f32 = 48.0;
const HEIGHT: f32 = 320.0;
#[derive(Clone)]
struct Snapshot {
    text: String,
    cursor: usize,
    anchor: usize,
}
pub enum EditorEvent {
    Apply,
    Cancel,
}
pub struct CodeEditor {
    pub focus: FocusHandle,
    pub text: String,
    cursor: usize,
    anchor: usize,
    marked: Option<Range<usize>>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    scroll: usize,
    scroll_x: Pixels,
    bounds: Option<Bounds<Pixels>>,
    lines: Vec<(usize, ShapedLine)>,
    selecting: bool,
}
impl EventEmitter<EditorEvent> for CodeEditor {}
impl CodeEditor {
    pub fn new(text: String, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            text,
            cursor: 0,
            anchor: 0,
            marked: None,
            undo: Vec::new(),
            redo: Vec::new(),
            scroll: 0,
            scroll_x: px(0.),
            bounds: None,
            lines: Vec::new(),
            selecting: false,
        }
    }
    fn range(&self) -> Range<usize> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
    }
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
        }
    }
    fn restore(&mut self, state: Snapshot) {
        self.text = state.text;
        self.cursor = state.cursor;
        self.anchor = state.anchor;
        self.marked = None;
        self.reveal();
    }
    pub fn undo(&mut self, redo: bool, cx: &mut Context<Self>) {
        let previous = if redo {
            self.redo.pop()
        } else {
            self.undo.pop()
        };
        if let Some(previous) = previous {
            let current = self.snapshot();
            if redo {
                self.undo.push(current);
            } else {
                self.redo.push(current);
            }
            self.restore(previous);
            cx.notify();
        }
    }
    fn replace(&mut self, range: Range<usize>, value: &str, cx: &mut Context<Self>) {
        let value = value
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\t', "    ");
        if value.contains('\0')
            || self.text.len() - range.len() + value.len() > forma_core::MAX_SHADER_BYTES
        {
            return;
        }
        if self.text[range.clone()] == value {
            return;
        }
        self.undo.push(self.snapshot());
        if self.undo.len() > 64 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.text.replace_range(range.clone(), &value);
        self.cursor = range.start + value.len();
        self.anchor = self.cursor;
        self.marked = None;
        self.reveal();
        cx.notify();
    }
    fn reveal(&mut self) {
        let line = self.text[..self.cursor]
            .bytes()
            .filter(|b| *b == b'\n')
            .count();
        self.scroll = self
            .scroll
            .min(line)
            .max(line.saturating_sub((HEIGHT / ROW) as usize - 2));
        // The next prepaint adjusts horizontal scrolling using shaped glyphs.
    }
    fn move_to(&mut self, offset: usize, select: bool, cx: &mut Context<Self>) {
        self.cursor = offset.min(self.text.len());
        if !select {
            self.anchor = self.cursor;
        }
        self.marked = None;
        self.reveal();
        cx.notify();
    }
    fn line_start(&self) -> usize {
        self.text[..self.cursor].rfind('\n').map_or(0, |n| n + 1)
    }
    fn line_end(&self) -> usize {
        self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |n| self.cursor + n)
    }
    fn previous(&self) -> usize {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }
    fn next(&self) -> usize {
        self.text[self.cursor..]
            .chars()
            .next()
            .map_or(self.text.len(), |c| self.cursor + c.len_utf8())
    }
    fn vertical(&self, down: bool) -> usize {
        let start = self.line_start();
        let col = self.text[start..self.cursor].chars().count();
        let target = if down {
            (self.line_end() + 1).min(self.text.len())
        } else if start == 0 {
            0
        } else {
            self.text[..start - 1].rfind('\n').map_or(0, |i| i + 1)
        };
        let line = self.text[target..].split('\n').next().unwrap_or("");
        target + line.char_indices().nth(col).map_or(line.len(), |(i, _)| i)
    }
    fn key(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let key = e.keystroke.key.as_str();
        let m = e.keystroke.modifiers;
        let range = self.range();
        match key {
            "escape" => cx.emit(EditorEvent::Cancel),
            "enter" if m.platform || m.control => cx.emit(EditorEvent::Apply),
            "enter" => {
                let indent: String = self.text[self.line_start()..self.cursor]
                    .chars()
                    .take_while(|c| *c == ' ')
                    .collect();
                self.replace(range, &format!("\n{indent}"), cx);
            }
            "tab" => self.replace(range, "    ", cx),
            "backspace" => self.replace(
                if range.is_empty() {
                    self.previous()..self.cursor
                } else {
                    range
                },
                "",
                cx,
            ),
            "delete" => self.replace(
                if range.is_empty() {
                    self.cursor..self.next()
                } else {
                    range
                },
                "",
                cx,
            ),
            "left" => self.move_to(
                if m.platform {
                    self.line_start()
                } else if !m.shift && !range.is_empty() {
                    range.start
                } else {
                    self.previous()
                },
                m.shift,
                cx,
            ),
            "right" => self.move_to(
                if m.platform {
                    self.line_end()
                } else if !m.shift && !range.is_empty() {
                    range.end
                } else {
                    self.next()
                },
                m.shift,
                cx,
            ),
            "home" => self.move_to(self.line_start(), m.shift, cx),
            "end" => self.move_to(self.line_end(), m.shift, cx),
            "up" => self.move_to(
                if m.platform { 0 } else { self.vertical(false) },
                m.shift,
                cx,
            ),
            "down" => self.move_to(
                if m.platform {
                    self.text.len()
                } else {
                    self.vertical(true)
                },
                m.shift,
                cx,
            ),
            "a" if m.platform => {
                self.anchor = 0;
                self.cursor = self.text.len();
                self.reveal();
                cx.notify();
            }
            "c" | "x" if m.platform => {
                if !range.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        self.text[range.clone()].into(),
                    ));
                }
                if key == "x" {
                    self.replace(range, "", cx);
                }
            }
            "v" if m.platform => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.replace(range, &text, cx);
                }
            }
            "z" if m.platform => self.undo(m.shift, cx),
            _ => return, // Text input and IME are delivered by EntityInputHandler.
        }
        cx.stop_propagation();
    }
    fn mouse_index(&self, point: Point<Pixels>) -> usize {
        let Some(bounds) = self.bounds else {
            return 0;
        };
        let y: f32 = (point.y - bounds.top()).into();
        let row = ((y / ROW).floor().max(0.) as usize).min(self.lines.len().saturating_sub(1));
        self.lines
            .get(row)
            .map_or(self.text.len(), |(start, line)| {
                start
                    + line.closest_index_for_x(point.x - bounds.left() - px(GUTTER) + self.scroll_x)
            })
    }
    fn utf8(&self, utf16: usize) -> usize {
        let mut count = 0;
        for (i, c) in self.text.char_indices() {
            if count >= utf16 {
                return i;
            }
            count += c.len_utf16();
        }
        self.text.len()
    }
    fn utf16(&self, utf8: usize) -> usize {
        self.text[..utf8].encode_utf16().count()
    }
    fn utf8_range(&self, r: Range<usize>) -> Range<usize> {
        self.utf8(r.start)..self.utf8(r.end)
    }
}
impl EntityInputHandler for CodeEditor {
    fn text_for_range(
        &mut self,
        r: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let r = self.utf8_range(r);
        *actual = Some(self.utf16(r.start)..self.utf16(r.end));
        Some(self.text[r].into())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let r = self.range();
        Some(UTF16Selection {
            range: self.utf16(r.start)..self.utf16(r.end),
            reversed: self.cursor < self.anchor,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked
            .as_ref()
            .map(|r| self.utf16(r.start)..self.utf16(r.end))
    }
    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }
    fn replace_text_in_range(
        &mut self,
        r: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let r = r
            .map(|r| self.utf8_range(r))
            .or(self.marked.clone())
            .unwrap_or(self.range());
        self.replace(r, text, cx);
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        r: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let r = r
            .map(|r| self.utf8_range(r))
            .or(self.marked.clone())
            .unwrap_or(self.range());
        let start = r.start;
        self.replace(r, text, cx);
        if !text.is_empty() && self.cursor >= start {
            self.marked = Some(start..self.cursor);
        }
        if let Some(selection) = selection {
            let base = self.utf16(start);
            let r = self.utf8_range(base + selection.start..base + selection.end);
            self.anchor = r.start;
            self.cursor = r.end;
        }
    }
    fn bounds_for_range(
        &mut self,
        r: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let index = self.utf8(r.start);
        let bounds = self.bounds?;
        for (row, (start, line)) in self.lines.iter().enumerate() {
            if index >= *start && index <= start + line.text.len() {
                return Some(Bounds::new(
                    point(
                        bounds.left() + px(GUTTER) + line.x_for_index(index - start)
                            - self.scroll_x,
                        bounds.top() + px(row as f32 * ROW),
                    ),
                    size(px(2.), px(ROW)),
                ));
            }
        }
        Some(bounds)
    }
    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.utf16(self.mouse_index(point)))
    }
}
struct CodeElement {
    editor: Entity<CodeEditor>,
}
struct CodePaint {
    lines: Vec<(usize, ShapedLine)>,
    numbers: Vec<ShapedLine>,
}
impl IntoElement for CodeElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for CodeElement {
    type RequestLayoutState = ();
    type PrepaintState = CodePaint;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = px(HEIGHT).into();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> CodePaint {
        let editor = self.editor.read(cx);
        let mut offset = 0;
        let mut lines = Vec::new();
        let mut numbers = Vec::new();
        let style = window.text_style();
        let shape = |text: String, color| {
            let run = TextRun {
                len: text.len(),
                font: style.font(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window
                .text_system()
                .shape_line(text.into(), px(13.), &[run], None)
        };
        let mut scroll_x = editor.scroll_x;
        for (row, text) in editor.text.split('\n').enumerate() {
            if row >= editor.scroll && lines.len() < (HEIGHT / ROW) as usize {
                let line = shape(text.into(), rgb(0xdce2e4).into());
                if editor.cursor >= offset && editor.cursor <= offset + text.len() {
                    let x = line.x_for_index(editor.cursor - offset);
                    let width = (bounds.size.width - px(GUTTER + 12.)).max(px(20.));
                    scroll_x = scroll_x.min(x).max(x - width).max(px(0.));
                }
                lines.push((offset, line));
                numbers.push(shape(format!("{:>4}", row + 1), rgb(0x5c656a).into()));
            }
            offset += text.len() + 1;
        }
        self.editor
            .update(cx, |editor, _| editor.scroll_x = scroll_x);
        CodePaint { lines, numbers }
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        paint: &mut CodePaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let editor = self.editor.read(cx);
        let focus = editor.focus.clone();
        let range = editor.range();
        let cursor = editor.cursor;
        let scroll_x = editor.scroll_x;
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );
        let text_bounds = Bounds::new(
            point(bounds.left() + px(GUTTER), bounds.top()),
            size(
                (bounds.size.width - px(GUTTER)).max(px(0.)),
                bounds.size.height,
            ),
        );
        window.with_content_mask(
            Some(ContentMask {
                bounds: text_bounds,
            }),
            |window| {
                for (row, (start, line)) in paint.lines.iter().enumerate() {
                    let origin = point(
                        text_bounds.left() - scroll_x,
                        bounds.top() + px(row as f32 * ROW),
                    );
                    let end = start + line.text.len();
                    if range.start <= end && range.end > *start {
                        let a = line
                            .x_for_index(range.start.saturating_sub(*start).min(line.text.len()));
                        let b = if range.end > end {
                            line.width + px(7.)
                        } else {
                            line.x_for_index(range.end - start)
                        };
                        window.paint_quad(fill(
                            Bounds::new(
                                point(origin.x + a, origin.y),
                                size((b - a).max(px(1.)), px(ROW)),
                            ),
                            rgb(0x294c43),
                        ));
                    }
                    let _ = line.paint(origin, px(ROW), window, cx);
                    if focus.is_focused(window) && cursor >= *start && cursor <= end {
                        window.paint_quad(fill(
                            Bounds::new(
                                point(origin.x + line.x_for_index(cursor - start), origin.y),
                                size(px(1.5), px(ROW)),
                            ),
                            rgb(0x84cfba),
                        ));
                    }
                }
            },
        );
        for (row, line) in paint.numbers.iter().enumerate() {
            let _ = line.paint(
                point(bounds.left() + px(4.), bounds.top() + px(row as f32 * ROW)),
                px(ROW),
                window,
                cx,
            );
        }
        self.editor.update(cx, |editor, _| {
            editor.bounds = Some(bounds);
            editor.lines = std::mem::take(&mut paint.lines);
        });
    }
}
impl Render for CodeEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("shader-code-input")
            .w_full()
            .h(px(HEIGHT))
            .overflow_hidden()
            .bg(rgb(0x0d0f11))
            .font_family(if cfg!(target_os = "macos") {
                "Menlo"
            } else {
                "DejaVu Sans Mono"
            })
            .cursor(CursorStyle::IBeam)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|s, e: &MouseDownEvent, w, cx| {
                    w.focus(&s.focus);
                    let index = s.mouse_index(e.position);
                    s.move_to(index, e.modifiers.shift, cx);
                    s.selecting = true;
                    cx.stop_propagation();
                }),
            )
            .on_mouse_move(cx.listener(|s, e: &MouseMoveEvent, _, cx| {
                if s.selecting {
                    let i = s.mouse_index(e.position);
                    s.move_to(i, true, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|s, _, _, _| s.selecting = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|s, _, _, _| s.selecting = false),
            )
            .on_scroll_wheel(cx.listener(|s, e: &ScrollWheelEvent, _, cx| {
                let delta: f32 = e.delta.pixel_delta(px(ROW)).y.into();
                let max = s.text.lines().count().saturating_sub(1);
                s.scroll =
                    (s.scroll as i64 - (delta / ROW).round() as i64).clamp(0, max as i64) as usize;
                cx.notify();
                cx.stop_propagation();
            }))
            .child(CodeElement {
                editor: cx.entity(),
            })
    }
}
