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

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
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
use crate::markdown::{self, LinkOpener};
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
    /// Where a compose pad's toolbar buttons were painted last, for their menus to hang
    /// under them.
    toolbar_bounds: Rc<RefCell<Vec<Option<Bounds<Pixels>>>>>,
    /// A compose pad's two columns as last painted (the editor, the divider, the pane), for
    /// fitting its text size.
    columns: Rc<RefCell<Vec<Bounds<Pixels>>>>,
    /// The toolbar's row in the header as last painted, to place its buttons over the
    /// columns.
    toolbar_row: Rc<std::cell::Cell<Option<Bounds<Pixels>>>>,
    /// The last fitted text size and what it was fitted for (see [`fitted_size`]).
    fit: std::cell::Cell<Option<(u64, f32)>>,
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
            if !view.placeholder.is_empty() {
                editor = editor.placeholder(view.placeholder.clone());
            }
            // A sheet keeps one row per line so its annotations line up; a compose pad
            // (output set) wraps like prose. Set at the push: an update does not switch it.
            editor.set_soft_wrap(view.output.is_some(), window, cx);
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
            toolbar_bounds: Rc::default(),
            columns: Rc::default(),
            toolbar_row: Rc::default(),
            fit: Default::default(),
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

    /// Where toolbar button `ix` was painted last, in window coordinates.
    pub(crate) fn toolbar_bounds(&self, ix: usize) -> Option<Bounds<Pixels>> {
        self.toolbar_bounds.borrow().get(ix).copied().flatten()
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

/// `links` opens the links of a compose pad's output.
pub(crate) fn render(
    instance: u64,
    view: &ScratchpadView,
    state: &ScratchpadState,
    links: Rc<dyn LinkOpener>,
    window: &mut Window,
    cx: &mut Context<LauncherView>,
) -> AnyElement {
    if let Some(output) = &view.output {
        return render_compose(view, output, state, links, window, cx);
    }
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

/// Two columns, each the whole height and scrolling on its own: the editor, wrapping, in
/// the UI font, then a divider and the output as markdown and the detail under it. The text
/// and the output share the size [`fitted_size`] picks, and one line height; the detail is
/// at the body size. The toolbar is in the header ([`header_toolbar`]).
fn render_compose(
    view: &ScratchpadView,
    output: &str,
    state: &ScratchpadState,
    links: Rc<dyn LinkOpener>,
    window: &mut Window,
    cx: &mut Context<LauncherView>,
) -> AnyElement {
    let palette = theme::palette(cx);
    let size = fitted_size(view, state, window, cx);
    let editor = div()
        .id("scratchpad-editor")
        .test_support()
        .flex_1()
        .min_w_0()
        .h_full()
        .line_height(relative(COMPOSE_LINE_HEIGHT))
        .pl(px(theme::VIEW_PADDING - theme::EDITOR_TEXT_INSET.0))
        .pr(px(theme::VIEW_PADDING - theme::EDITOR_TEXT_INSET.0))
        .pt(px(theme::VIEW_PADDING - theme::EDITOR_TEXT_INSET.1))
        .child(
            Textarea::new(&state.editor)
                .appearance(false)
                .text_size(px(size))
                // The kit fixes its inputs' line height in rems: the output's, in sizes.
                .line_height(relative(COMPOSE_LINE_HEIGHT))
                .h(relative(1.))
                .aria_label(view.title.clone()),
        );
    let detail = view.detail.as_ref().map(|detail| {
        div()
            .id("scratchpad-detail")
            .test_support()
            .pt(px(theme::VIEW_PADDING))
            .text_size(px(theme::BODY_TEXT_SIZE))
            .line_height(relative(DETAIL_LINE_HEIGHT))
            .child(markdown::render_with(detail, links.clone(), cx))
    });
    let label = match &view.detail {
        Some(detail) => format!(
            "{}\n{}",
            markdown::plain_text(output),
            markdown::plain_text(detail)
        ),
        None => markdown::plain_text(output),
    };
    let pane = div()
        .id("scratchpad-output")
        .role(Role::Document)
        .aria_label(label)
        .test_support()
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_y_scroll()
        .p(px(theme::VIEW_PADDING))
        .child(
            div()
                .id("scratchpad-output-text")
                .test_support()
                .text_size(px(size))
                .line_height(relative(COMPOSE_LINE_HEIGHT))
                .child(markdown::render_with(output, links, cx)),
        )
        .children(detail);
    let columns = state.columns.clone();
    div()
        .on_children_prepainted(move |children, _, _| *columns.borrow_mut() = children)
        .flex()
        .flex_1()
        .min_h_0()
        .child(editor)
        .child(div().flex_none().w(px(1.)).h_full().bg(palette.divider))
        .child(pane)
        .into_any_element()
}

/// The text size of a compose pad's text and output: the largest of [`COMPOSE_SIZES`] at
/// which the text wraps within its column's height, as the text system shapes it; the
/// smallest when none does, or the text is too long to be worth shaping. Only the text
/// counts, not the output: an answer arriving does not resize what the user is reading. An
/// empty editor fits its placeholder on one line, as the editor draws it. Kept until the
/// text or the column changes.
fn fitted_size(
    view: &ScratchpadView,
    state: &ScratchpadState,
    window: &mut Window,
    cx: &App,
) -> f32 {
    let typed = state.text(cx);
    let (text, one_line) = if typed.is_empty() {
        (view.placeholder.clone(), true)
    } else {
        (typed, false)
    };
    // The editor's text box, as last painted; before that, half of the window.
    let painted = state.columns.borrow().first().map(|b| b.size);
    let column = painted.unwrap_or_else(|| {
        let viewport = window.viewport_size();
        let margin = px(2. * theme::shadow_margin());
        size(
            (viewport.width - margin) / 2.,
            viewport.height - margin - px(theme::HEADER_HEIGHT + theme::FOOTER_HEIGHT),
        )
    });
    let room = size(
        (column.width - px(2. * theme::VIEW_PADDING)).max(px(1.)),
        (column.height - px(2. * theme::VIEW_PADDING)).max(px(1.)),
    );
    let key = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (&text, one_line).hash(&mut hasher);
        (f32::from(room.width) as u32, f32::from(room.height) as u32).hash(&mut hasher);
        hasher.finish()
    };
    if let Some((known, size)) = state.fit.get()
        && known == key
    {
        return size;
    }
    // The launcher's UI font: while views are built the window's text style is still the
    // default one, which the launcher does not draw with.
    let mut style = window.text_style();
    style.font_family = gpui_kit::component::Theme::global(cx).font_family.clone();
    let fits = |size: f32, window: &mut Window| {
        if text.len() > FIT_MAX_BYTES {
            return false;
        }
        let run = style.to_run(text.len());
        let wrap = (!one_line).then_some(room.width);
        let Ok(lines) = window.text_system().shape_text(
            SharedString::from(text.clone()),
            px(size),
            &[run],
            wrap,
            None,
        ) else {
            return false;
        };
        if one_line {
            return lines.iter().all(|l| l.width() <= room.width);
        }
        let count: usize = lines.iter().map(|l| l.wrap_boundaries.len() + 1).sum();
        px(size * COMPOSE_LINE_HEIGHT) * count as f32 <= room.height
    };
    let fitted = COMPOSE_SIZES
        .iter()
        .copied()
        .find(|&size| size <= COMPOSE_MIN_SIZE || fits(size, window))
        .unwrap_or(COMPOSE_MIN_SIZE);
    state.fit.set(Some((key, fitted)));
    fitted
}

/// A compose pad's text sizes, largest first: a few steps, so typing does not shake the
/// text a pixel at a time. The last is the body size.
const COMPOSE_SIZES: [f32; 4] = [26., 21., 17., COMPOSE_MIN_SIZE];
const COMPOSE_MIN_SIZE: f32 = theme::BODY_TEXT_SIZE;
/// The line height of a compose pad's text and output, and of its detail, in text sizes.
const COMPOSE_LINE_HEIGHT: f32 = 1.45;
const DETAIL_LINE_HEIGHT: f32 = 1.45;
/// A text longer than this is not shaped to fit: it takes the smallest size.
const FIT_MAX_BYTES: usize = 4096;

/// A compose pad's toolbar, as the header's content: the first half of the buttons over
/// the editor, the second half over the output, a middle one (of an odd count) between
/// them. A click runs a button ([`LauncherView::toolbar_clicked`]); the buttons' bounds are
/// kept for their menus. `None` for a pad with no toolbar, or a sheet.
pub(crate) fn header_toolbar(
    instance: u64,
    view: &ScratchpadView,
    state: &ScratchpadState,
    cx: &mut Context<LauncherView>,
) -> Option<AnyElement> {
    if view.output.is_none() || view.toolbar.is_empty() {
        return None;
    }
    let palette = theme::palette(cx);
    let n = view.toolbar.len();
    // Kept across frames: a menu open now hangs under its button as last painted.
    state.toolbar_bounds.borrow_mut().resize(n, None);
    let mut button = |ix: usize| {
        let button = &view.toolbar[ix];
        let icon = button.icon.as_ref().map(|icon| {
            crate::views::icon_element(
                crate::icons::IconSpec {
                    icon: Some(icon),
                    monogram: &button.label,
                },
                TOOLBAR_ICON_SIZE,
                &palette,
            )
        });
        let chevron = (!button.options.is_empty()).then(|| {
            div()
                .flex_none()
                .text_color(palette.muted)
                .child("\u{25be}")
        });
        div()
            .id(("toolbar-button", ix))
            .role(Role::Button)
            .aria_label(button.label.clone())
            .test_support()
            .flex_none()
            .h(px(TOOLBAR_BUTTON_HEIGHT))
            .px(px(TOOLBAR_BUTTON_PADDING))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(theme::ACTION_RADIUS))
            .text_size(px(theme::BODY_TEXT_SIZE))
            .cursor_pointer()
            .hover(|el| el.bg(palette.selected))
            .on_click(cx.listener(move |host, _, window, cx| {
                host.toolbar_clicked(instance, ix, window, cx);
            }))
            .children(icon)
            .child(div().min_w_0().truncate().child(button.label.clone()))
            .children(chevron)
            .into_any_element()
    };
    // A group of buttons `range`, its painted bounds kept by button.
    let mut group = |range: std::ops::Range<usize>| {
        let bounds = state.toolbar_bounds.clone();
        let first = range.start;
        let buttons: Vec<AnyElement> = range.map(&mut button).collect();
        div()
            .on_children_prepainted(move |children, _, _| {
                let mut kept = bounds.borrow_mut();
                for (i, b) in children.into_iter().enumerate() {
                    if let Some(slot) = kept.get_mut(first + i) {
                        *slot = Some(b);
                    }
                }
            })
            .flex()
            .items_center()
            .gap(px(4.))
            .children(buttons)
    };
    let half = n / 2;
    // Over the columns, as painted last: the middle button centred on the line between
    // them, the second half starting where the output's text does. Before that, in a row.
    let divider = match state.columns.borrow().as_slice() {
        [_, divider, _] => Some(divider.center().x),
        _ => None,
    };
    let placed = divider.zip(state.toolbar_row.get()).map(|(divider, row)| {
        let middle_width = (n % 2 == 1)
            .then(|| state.toolbar_bounds(half))
            .flatten()
            .map_or(px(0.), |b| b.size.width);
        // The second half clear of the middle button, or where the output's text starts.
        let text = px(theme::VIEW_PADDING - TOOLBAR_BUTTON_PADDING);
        let clear = middle_width / 2. + px(TOOLBAR_GAP);
        (
            divider - row.left() - middle_width / 2.,
            divider - row.left() + text.max(clear),
        )
    });
    let middle = (n % 2 == 1).then(|| group(half..half + 1).flex_none());
    let left = group(0..half).flex_1().min_w_0();
    let right = group(half + n % 2..n).flex_1().min_w_0();
    let row = div()
        .id("scratchpad-toolbar")
        .role(Role::Toolbar)
        .aria_label("Toolbar")
        .test_support()
        .relative()
        .w_full()
        .h(px(TOOLBAR_BUTTON_HEIGHT))
        .flex()
        .items_center();
    let row = match placed {
        Some((middle_left, right_left)) => row
            .child(left)
            .children(middle.map(|m| m.absolute().top_0().left(middle_left)))
            .child(right.absolute().top_0().left(right_left)),
        None => row.child(left).children(middle).child(right),
    };
    let kept = state.toolbar_row.clone();
    Some(
        div()
            .on_children_prepainted(move |children, _, _| kept.set(children.first().copied()))
            .w_full()
            .child(row)
            .into_any_element(),
    )
}

/// The room between the middle button and the second half.
const TOOLBAR_GAP: f32 = 12.;
/// A toolbar button's own horizontal padding: its icon starts this far in.
const TOOLBAR_BUTTON_PADDING: f32 = 8.;

/// The height of a toolbar button, and of its icon.
const TOOLBAR_BUTTON_HEIGHT: f32 = 30.;
const TOOLBAR_ICON_SIZE: f32 = 18.;
