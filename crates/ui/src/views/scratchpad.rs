//! The scratchpad view: a multi-line editor with a column of per-line
//! annotations from the module beside it.
//!
//! The editor does not wrap, so each line of text is one row and the
//! annotation of a line sits on that row; long lines scroll horizontally.
//! The column takes its rows' positions from the editor's own layout of the
//! same frame, so it follows the editor's vertical scroll.
//!
//! The view model ([`ScratchpadView`]) holds the module's annotations; the
//! typed text lives in the editor. Edits are sent as `TextChanged` once the
//! user pauses for [`TEXT_DEBOUNCE`].

use std::collections::BTreeMap;
use std::time::Duration;

use gpui_kit::base::input::RopeExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::{
    AnyElement, App, AppContext as _, AvailableSpace, Bounds, ContentMask, Context, Element,
    ElementId, Entity, GlobalElementId, Hsla, InspectorElementId, InteractiveElement as _,
    IntoElement, LayoutId, ParentElement as _, Pixels, Role, ScrollDelta, ScrollEvent,
    SharedString, StatefulInteractiveElement as _, Style, Styled as _, Subscription, Task,
    TestSupportExt as _, Window, div, point, px, relative, rgba, size,
};
use tomakast_core::{LineAnnotation, ScratchpadView};

use crate::launcher_view::LauncherView;
use crate::theme;

/// The colour square of an annotation that carries one: its side, corner radius and the
/// gap to the text, in logical pixels.
const SWATCH_SIZE: f32 = 12.;
const SWATCH_RADIUS: f32 = 3.;
const SWATCH_GAP: f32 = 6.;

/// How long typing must pause before the text is sent to the module.
pub(crate) const TEXT_DEBOUNCE: Duration = Duration::from_millis(30);

pub(crate) struct ScratchpadState {
    editor: Entity<TextareaState>,
    /// Sends the text when it fires; replaced, and so cancelled, by every
    /// edit. Dropped with the view.
    debounce: Option<Task<()>>,
    /// An edit has not been sent yet: the debounce has not fired. The view
    /// sends it before `Closed` when it goes.
    pending: bool,
    _subscription: Subscription,
}

impl ScratchpadState {
    /// `instance`: the stack entry this state belongs to. The editor takes
    /// the focus.
    pub(crate) fn new(
        instance: u64,
        view: &ScratchpadView,
        window: &mut Window,
        cx: &mut Context<LauncherView>,
    ) -> Self {
        let editor = cx.new(|cx| {
            let mut editor = TextareaState::new(window, cx).default_value(view.text.clone());
            editor.set_soft_wrap(false, window, cx);
            editor
        });
        let subscription = cx.subscribe(&editor, move |host, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                host.scratchpad_edited(instance, cx);
            }
        });
        let state = ScratchpadState {
            editor,
            debounce: None,
            pending: false,
            _subscription: subscription,
        };
        state.focus(window, cx);
        state
    }

    pub(crate) fn focus(&self, window: &mut Window, cx: &mut Context<LauncherView>) {
        self.editor
            .update(cx, |editor, cx| editor.focus(window, cx));
    }

    pub(crate) fn text(&self, cx: &App) -> String {
        self.editor.read(cx).value().to_string()
    }

    /// Replaces the pending send, if any.
    pub(crate) fn debounce(&mut self, task: Task<()>) {
        self.debounce = Some(task);
        self.pending = true;
    }

    /// Whether an edit is unsent; it counts as sent from now on.
    pub(crate) fn take_pending(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }

    /// Scrolls the editor vertically, for the wheel over the annotation column.
    pub(crate) fn scroll(&self, delta: ScrollDelta, cx: &mut App) {
        self.editor.update(cx, |editor, cx| {
            let line_height = editor.line_height().unwrap_or(px(20.));
            let mut offset = editor.scroll_offset();
            offset.y += delta.pixel_delta(line_height).y;
            // Clamped by the editor at its next layout.
            editor.set_scroll_offset(offset, cx);
        });
    }

    /// Where the editor laid out the start of `line` in the last frame;
    /// `None` when the line is not laid out (scrolled away, or past the end).
    pub(crate) fn line_bounds(&self, line: usize, cx: &App) -> Option<Bounds<Pixels>> {
        let editor = self.editor.read(cx);
        let text = editor.text();
        if line >= text.lines_len() {
            return None;
        }
        let start = text.line_start_offset(line);
        editor.range_to_bounds(&(start..start))
    }
}

pub(crate) fn render(
    instance: u64,
    view: &ScratchpadView,
    state: &ScratchpadState,
    cx: &mut Context<LauncherView>,
) -> AnyElement {
    let palette = theme::palette(cx);
    // A sheet reads like code: its text and its results are monospace, so
    // numbers line up in columns.
    let mono = cx.theme().mono_font_family.clone();
    let editor = div()
        .id("scratchpad-editor")
        .test_support()
        .flex_1()
        .min_w_0()
        .h_full()
        .child(
            // Borderless, like the rest of the panel.
            Textarea::new(&state.editor)
                .appearance(false)
                .font_family(mono.clone())
                .font_features(theme::mono_features())
                .text_size(px(theme::BODY_TEXT_SIZE))
                .h(relative(1.))
                .aria_label(view.title.clone()),
        );
    let column = div()
        .id("scratchpad-annotations")
        .role(Role::Group)
        .aria_label("Results")
        .test_support()
        .flex_none()
        .w(relative(theme::COLUMN_WIDTH))
        .h_full()
        .overflow_hidden()
        // The column has no scroll of its own: the wheel scrolls the editor.
        .on_scroll(cx.listener(move |host, event: &ScrollEvent, _, cx| {
            host.scratchpad_scroll(instance, event.delta, cx);
        }))
        .child(AnnotationColumn {
            editor: state.editor.clone(),
            annotations: view.annotations.clone(),
            font: mono,
            muted: palette.muted,
            error: palette.error,
            border: palette.border,
        });
    div()
        .flex()
        .gap_2()
        .flex_1()
        .min_h_0()
        .p(px(theme::VIEW_PADDING))
        .pl(px(theme::VIEW_PADDING - theme::EDITOR_TEXT_INSET.0))
        .pt(px(theme::VIEW_PADDING - theme::EDITOR_TEXT_INSET.1))
        .child(editor)
        .child(column)
        .into_any_element()
}

/// The annotation rows. They are placed in prepaint, after the editor (laid
/// out before it in the same row) has laid out this frame: each annotation
/// goes where the editor put the start of its line.
struct AnnotationColumn {
    editor: Entity<TextareaState>,
    annotations: Vec<LineAnnotation>,
    font: SharedString,
    muted: Hsla,
    error: Hsla,
    border: Hsla,
}

impl IntoElement for AnnotationColumn {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for AnnotationColumn {
    type RequestLayoutState = ();
    type PrepaintState = Vec<AnyElement>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let style = Style {
            size: size(relative(1.).into(), relative(1.).into()),
            ..Style::default()
        };
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
    ) -> Vec<AnyElement> {
        let Some((line_height, placed)) = self.place(cx) else {
            return Vec::new();
        };
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            placed
                .into_iter()
                .map(|(line, y, annotation)| {
                    let color = if annotation.is_error {
                        self.error
                    } else {
                        self.muted
                    };
                    let text = one_line(&annotation.text);
                    let mut row = div()
                        .id(("scratchpad-annotation", line))
                        .role(Role::Label)
                        .aria_label(text.clone())
                        .test_support()
                        .h(line_height)
                        .w(bounds.size.width)
                        // Muted size, on the editor's line height, so each
                        // note stays level with its line.
                        .font_family(self.font.clone())
                        .font_features(theme::mono_features())
                        .text_size(px(theme::MUTED_TEXT_SIZE))
                        .line_height(line_height)
                        .overflow_hidden()
                        .flex()
                        .items_center()
                        // The swatch sits right before the right-aligned text.
                        .justify_end()
                        .gap(px(SWATCH_GAP));
                    if let Some(fill) = annotation
                        .swatch
                        .as_deref()
                        .and_then(tomakast_core::view::swatch_rgba)
                    {
                        row = row.child(
                            div()
                                .id(("scratchpad-annotation-swatch", line))
                                .test_support()
                                .flex_none()
                                .size(px(SWATCH_SIZE))
                                .rounded(px(SWATCH_RADIUS))
                                .bg(rgba(fill))
                                .border_1()
                                .border_color(self.border),
                        );
                    }
                    let mut row = row
                        .child(
                            div()
                                .id(("scratchpad-annotation-text", line))
                                .test_support()
                                .min_w_0()
                                .truncate()
                                .text_right()
                                .text_color(color)
                                .child(text),
                        )
                        .into_any_element();
                    row.prepaint_as_root(
                        point(bounds.origin.x, y),
                        size(
                            AvailableSpace::Definite(bounds.size.width),
                            AvailableSpace::Definite(line_height),
                        ),
                        window,
                        cx,
                    );
                    row
                })
                .collect()
        })
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        rows: &mut Vec<AnyElement>,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for row in rows {
                row.paint(window, cx);
            }
        });
    }
}

impl AnnotationColumn {
    /// The editor's line height and, for each annotated line it laid out,
    /// the line, the window y of its start and its annotation (the last
    /// one for the line). `None` before the editor's first layout.
    #[allow(clippy::type_complexity)]
    fn place(&self, cx: &App) -> Option<(Pixels, Vec<(usize, Pixels, &LineAnnotation)>)> {
        let editor = self.editor.read(cx);
        let line_height = editor.line_height()?;
        let visible = editor.visible_row_range()?;
        let text = editor.text();
        let lines = text.lines_len();
        // Module data: lines past the text are dropped.
        let mut shown = BTreeMap::new();
        for annotation in &self.annotations {
            let line = annotation.line as usize;
            if line < lines && visible.contains(&line) {
                shown.insert(line, annotation);
            }
        }
        let placed = shown
            .into_iter()
            .filter_map(|(line, annotation)| {
                let start = text.line_start_offset(line);
                let at = editor.range_to_bounds(&(start..start))?;
                Some((line, at.origin.y, annotation))
            })
            .collect();
        Some((line_height, placed))
    }
}

/// `text` on one row: line breaks and other control characters, which the
/// text system would lay out as more lines, become spaces.
fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}
