//! The generic list view: a filter field over rows with optional section
//! headers and an optional detail pane.

use std::rc::Rc;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Role, ScrollStrategy, StatefulInteractiveElement as _, Styled as _,
    Subscription, TestSupportExt as _, UniformListScrollHandle, Window, div, px, uniform_list,
};
use tomakast_core::{ListRow, ListView, Matcher, Pattern, SearchMode, View};

use crate::chrome;
use crate::icons::IconSpec;
use crate::launcher_view::LauncherView;
use crate::markdown::{self, LinkOpener};
use crate::theme::{self, Palette};
use crate::views::{self, ViewState};

/// One rendered line: a section header or a row.
enum Line {
    Section(String),
    /// Index into `ListState::visible`.
    Row(usize),
}

pub(crate) struct ListState {
    input: Entity<InputState>,
    /// Indices of the rows that pass the filter, in the module's order.
    visible: Vec<usize>,
    lines: Vec<Line>,
    /// Index into `visible`.
    selected: usize,
    scroll: UniformListScrollHandle,
    matcher: Matcher,
    _input_events: Subscription,
}

impl ListState {
    /// The input starts with the view's `query`; the view is filtered by it.
    /// `instance`: the stack entry this state belongs to.
    pub(crate) fn new(
        instance: u64,
        view: &ListView,
        window: &mut Window,
        cx: &mut Context<LauncherView>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(view.placeholder.clone())
                .default_value(view.query.clone())
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        let input_events = cx.subscribe(&input, move |host, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                host.list_query_changed(instance, cx);
            }
        });
        let mut state = ListState {
            input,
            visible: Vec::new(),
            lines: Vec::new(),
            selected: 0,
            scroll: UniformListScrollHandle::new(),
            matcher: Matcher::new(),
            _input_events: input_events,
        };
        state.refilter(view, &view.query, None);
        state
    }

    pub(crate) fn focus(&self, window: &mut Window, cx: &mut Context<LauncherView>) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    pub(crate) fn query(&self, cx: &Context<LauncherView>) -> String {
        self.input.read(cx).value().to_string()
    }

    /// Recomputes the visible rows. A `CoreFilter` list keeps the rows that
    /// match `query`, in the module's order (so sections stay together); a
    /// `Module` list shows all rows, which the module filtered. The row with
    /// id `keep` stays selected when it is still visible; otherwise the selection
    /// stays at its index, clamped to the new length (the first row when `keep` is `None`).
    pub(crate) fn refilter(&mut self, view: &ListView, query: &str, keep: Option<&str>) {
        let pattern = Pattern::parse(match view.search {
            SearchMode::CoreFilter => query,
            SearchMode::Module => "",
        });
        let matcher = &mut self.matcher;
        self.visible = view
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| matcher.score_text(&pattern, &row.title).is_some())
            .map(|(ix, _)| ix)
            .collect();
        self.lines.clear();
        let mut section = None;
        for (visible_ix, &row_ix) in self.visible.iter().enumerate() {
            let row = &view.rows[row_ix];
            if row.section.is_some() && row.section != section {
                section.clone_from(&row.section);
                self.lines
                    .push(Line::Section(row.section.clone().unwrap_or_default()));
            }
            self.lines.push(Line::Row(visible_ix));
        }
        let previous = self.selected;
        self.selected = match keep {
            // The kept row is gone: stay at the same place, clamped to the new length.
            Some(id) => self
                .visible
                .iter()
                .position(|&ix| view.rows[ix].id == id)
                .unwrap_or_else(|| previous.min(self.visible.len().saturating_sub(1))),
            None => 0,
        };
        self.scroll_to_selected(ScrollStrategy::Top);
    }

    pub(crate) fn selected_row<'v>(&self, view: &'v ListView) -> Option<&'v ListRow> {
        let ix = *self.visible.get(self.selected)?;
        view.rows.get(ix)
    }

    pub(crate) fn select_prev(&mut self) {
        self.select(self.selected.saturating_sub(1));
    }

    pub(crate) fn select_next(&mut self) {
        let last = self.visible.len().saturating_sub(1);
        self.select((self.selected + 1).min(last));
    }

    fn select(&mut self, ix: usize) {
        self.selected = ix;
        self.scroll_to_selected(ScrollStrategy::Nearest);
    }

    fn scroll_to_selected(&self, strategy: ScrollStrategy) {
        let line = self
            .lines
            .iter()
            .position(|line| matches!(line, Line::Row(ix) if *ix == self.selected));
        // Keep the section header of the first row in view too.
        let line = match strategy {
            ScrollStrategy::Top if self.selected == 0 => Some(0),
            _ => line,
        };
        if let Some(line) = line {
            self.scroll.scroll_to_item(line, strategy);
        }
    }

    fn line(&self, view: &ListView, ix: usize, palette: &Palette) -> Option<AnyElement> {
        match self.lines.get(ix)? {
            Line::Section(name) => {
                Some(views::section_label(("section", ix).into(), name, palette))
            }
            Line::Row(visible_ix) => {
                let row = view.rows.get(*self.visible.get(*visible_ix)?)?;
                Some(views::row(
                    ("row", ix).into(),
                    &row.title,
                    row.subtitle.as_deref(),
                    IconSpec {
                        icon: row.icon.as_ref(),
                        monogram: &row.title,
                    },
                    &row.accessories,
                    *visible_ix == self.selected,
                    palette,
                ))
            }
        }
    }
}

/// The list's filter field, for the header: borderless, like the root search.
pub(crate) fn filter_input(view: &ListView, state: &ListState) -> AnyElement {
    chrome::header_input(&state.input, "list-input", view.title.clone())
}

/// The body of a list view: its rows and detail pane.
/// `links` opens the links of the detail pane's markdown.
pub(crate) fn render(
    instance: u64,
    view: &ListView,
    state: &ListState,
    links: Rc<dyn LinkOpener>,
    cx: &mut Context<LauncherView>,
) -> AnyElement {
    let palette = theme::palette(cx);
    let rows = uniform_list(
        "list-rows",
        state.lines.len(),
        cx.processor(move |host, range: std::ops::Range<usize>, _, cx| {
            let palette = theme::palette(cx);
            let Some(entry) = host.stack().get(instance) else {
                return Vec::new();
            };
            let (View::List(view), ViewState::List(state)) = (&entry.view, &entry.state) else {
                return Vec::new();
            };
            range
                .filter_map(|ix| state.line(view, ix, &palette))
                .collect()
        }),
    )
    .track_scroll(&state.scroll)
    .flex_1()
    .min_h_0();
    let main = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .child(rows);
    if !view.show_detail {
        return main.into_any_element();
    }
    let source = state
        .selected_row(view)
        .and_then(|row| row.detail_markdown.clone())
        .unwrap_or_default();
    let detail = div()
        .id("list-detail")
        .role(Role::Document)
        .aria_label(markdown::plain_text(&source))
        .test_support()
        .w(px(theme::DETAIL_WIDTH))
        .flex_none()
        .p(px(theme::VIEW_PADDING))
        .text_size(px(theme::BODY_TEXT_SIZE))
        .border_l_1()
        .border_color(palette.divider)
        .overflow_hidden()
        .child(markdown::render_with(&source, links, cx));
    div()
        .flex()
        .flex_row()
        .flex_1()
        .min_h_0()
        .child(main)
        .child(detail)
        .into_any_element()
}
