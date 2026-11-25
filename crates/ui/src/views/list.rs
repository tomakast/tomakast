//! The generic list view: a filter field over rows with optional section
//! headers and an optional detail pane, or over a grid of tiles
//! ([`ListLayout::Grid`]), which draws neither.

use std::rc::Rc;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Role, ScrollHandle, ScrollStrategy, StatefulInteractiveElement as _,
    Styled as _, Subscription, TestSupportExt as _, UniformListScrollHandle, Window, div, px,
    uniform_list,
};
use tomakast_core::{
    ListLayout, ListRow, ListView, Matcher, Pattern, Scripts, SearchMode, TextKeys, View,
};

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
    /// The view's layout, as of the last `refilter`.
    layout: ListLayout,
    /// Scrolls the tile rows of a grid, which are its children.
    grid_scroll: ScrollHandle,
    matcher: Matcher,
    /// Match keys of the row titles, rebuilt by `reindex` when the rows change.
    row_keys: Vec<TextKeys>,
    /// The scripts of all row titles together.
    row_scripts: Scripts,
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
        let input_events = watch_query(&input, instance, cx);
        let mut state = ListState {
            input,
            visible: Vec::new(),
            lines: Vec::new(),
            selected: 0,
            scroll: UniformListScrollHandle::new(),
            layout: view.layout,
            grid_scroll: ScrollHandle::new(),
            matcher: Matcher::new(),
            row_keys: Vec::new(),
            row_scripts: Scripts::default(),
            _input_events: input_events,
        };
        state.reindex(view);
        state.refilter(view, &view.query, None);
        state
    }

    pub(crate) fn focus(&self, window: &mut Window, cx: &mut Context<LauncherView>) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    /// Makes the search field again in `window`, with its text and selection (see
    /// [`super::rehome`]).
    pub(crate) fn rehome(
        &mut self,
        instance: u64,
        view: &ListView,
        window: &mut Window,
        cx: &mut Context<LauncherView>,
    ) {
        let placeholder = view.placeholder.clone();
        let input = super::rehome::input(&self.input, |s| s.placeholder(placeholder), window, cx);
        self._input_events = watch_query(&input, instance, cx);
        self.input = input;
    }

    pub(crate) fn query(&self, cx: &Context<LauncherView>) -> String {
        self.input.read(cx).value().to_string()
    }

    /// Rebuilds the match keys of the rows. Call it whenever the rows change,
    /// before `refilter`.
    pub(crate) fn reindex(&mut self, view: &ListView) {
        self.row_keys = view
            .rows
            .iter()
            .map(|row| TextKeys::new(&row.title))
            .collect();
        self.row_scripts = self
            .row_keys
            .iter()
            .fold(Scripts::default(), |all, keys| all.union(keys.scripts()));
    }

    /// Recomputes the visible rows. A `CoreFilter` list keeps the rows that
    /// match `query`, in the module's order (so sections stay together); a
    /// `Module` list shows all rows, which the module filtered. The row with
    /// id `keep` stays selected when it is still visible; otherwise the selection
    /// stays at its index, clamped to the new length (the first row when `keep` is `None`).
    pub(crate) fn refilter(&mut self, view: &ListView, query: &str, keep: Option<&str>) {
        debug_assert_eq!(self.row_keys.len(), view.rows.len());
        // An update may switch the layout.
        self.layout = view.layout;
        let pattern = Pattern::across(
            match view.search {
                SearchMode::CoreFilter => query,
                SearchMode::Module => "",
            },
            self.row_scripts,
            &mut self.matcher,
        );
        let matcher = &mut self.matcher;
        let keys = &self.row_keys;
        self.visible = view
            .rows
            .iter()
            .enumerate()
            .filter(|(ix, row)| {
                matcher
                    .score_text(&pattern, &row.title, &keys[*ix])
                    .is_some()
            })
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

    /// Tiles per row of a grid; `None` for rows.
    fn columns(&self) -> Option<usize> {
        match self.layout {
            ListLayout::Rows => None,
            ListLayout::Grid { columns } => Some(usize::from(columns).max(1)),
        }
    }

    /// Whether the rows are drawn as a grid of tiles.
    pub(crate) fn is_grid(&self) -> bool {
        self.columns().is_some()
    }

    /// Up: the previous row, or the tile above in a grid.
    pub(crate) fn select_prev(&mut self) {
        match self.columns() {
            Some(columns) if self.selected >= columns => self.select(self.selected - columns),
            Some(_) => {}
            None => self.step(false),
        }
    }

    /// Down: the next row, or the tile below in a grid; into a short last row
    /// it lands on that row's last tile.
    pub(crate) fn select_next(&mut self) {
        let last = self.visible.len().saturating_sub(1);
        match self.columns() {
            Some(columns) if self.selected / columns < last / columns => {
                self.select((self.selected + columns).min(last));
            }
            Some(_) => {}
            None => self.step(true),
        }
    }

    /// Left in a grid: the previous tile of the same row; a row's first tile
    /// stays, as in a grid view.
    pub(crate) fn select_left(&mut self) {
        if let Some(columns) = self.columns()
            && !self.selected.is_multiple_of(columns)
        {
            self.select(self.selected - 1);
        }
    }

    /// Right in a grid: the next tile of the same row; a row's last tile, and
    /// the last tile of a short last row, stay.
    pub(crate) fn select_right(&mut self) {
        let last = self.visible.len().saturating_sub(1);
        if let Some(columns) = self.columns()
            && self.selected % columns < columns - 1
            && self.selected < last
        {
            self.select(self.selected + 1);
        }
    }

    /// The previous (`forward`: next) row of a list of rows, clamped to the ends.
    fn step(&mut self, forward: bool) {
        let last = self.visible.len().saturating_sub(1);
        let ix = match forward {
            true => (self.selected + 1).min(last),
            false => self.selected.saturating_sub(1),
        };
        self.select(ix);
    }

    fn select(&mut self, ix: usize) {
        self.selected = ix;
        self.scroll_to_selected(ScrollStrategy::Nearest);
    }

    fn scroll_to_selected(&self, strategy: ScrollStrategy) {
        if let Some(columns) = self.columns() {
            self.grid_scroll.scroll_to_item(self.selected / columns);
            return;
        }
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
                    None,
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

/// The body of a list drawn as a grid: the visible rows as tiles, `columns`
/// per tile row, in a scrolled column. Sections and the detail pane are not
/// drawn.
fn render_grid(
    view: &ListView,
    state: &ListState,
    columns: usize,
    palette: &Palette,
) -> AnyElement {
    let tile_rows = state
        .visible
        .chunks(columns)
        .enumerate()
        .map(|(tile_row, rows)| {
            div()
                .flex()
                .gap_2()
                .children(rows.iter().enumerate().filter_map(|(col, &row_ix)| {
                    let row = view.rows.get(row_ix)?;
                    let ix = tile_row * columns + col;
                    Some(views::grid::tile(
                        ix,
                        row.icon.as_ref(),
                        &row.title,
                        ix == state.selected,
                        palette,
                    ))
                }))
                // The last row is padded, so its tiles are as wide as the others.
                .children((rows.len()..columns).map(|_| views::grid::filler()))
        });
    div()
        .id("list-grid")
        .test_support()
        .track_scroll(&state.grid_scroll)
        .flex()
        .flex_col()
        .gap_2()
        .flex_1()
        .min_h_0()
        .p(px(theme::VIEW_PADDING))
        .text_size(theme::text(theme::BODY_TEXT_SIZE))
        .overflow_y_scroll()
        .children(tile_rows)
        .into_any_element()
}

/// The body of a list view: its rows and detail pane, or its grid of tiles.
/// `links` opens the links of the detail pane's markdown.
pub(crate) fn render(
    instance: u64,
    view: &ListView,
    state: &ListState,
    links: Rc<dyn LinkOpener>,
    cx: &mut Context<LauncherView>,
) -> AnyElement {
    let palette = theme::palette(cx);
    if let Some(columns) = state.columns() {
        return render_grid(view, state, columns, &palette);
    }
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
        .text_size(theme::text(theme::BODY_TEXT_SIZE))
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

/// Tells the launcher view of each edit of list `instance`'s search field.
fn watch_query(
    input: &Entity<InputState>,
    instance: u64,
    cx: &mut Context<LauncherView>,
) -> Subscription {
    cx.subscribe(input, move |host, _, event: &InputEvent, cx| {
        if let InputEvent::Change = event {
            host.list_query_changed(instance, cx);
        }
    })
}
