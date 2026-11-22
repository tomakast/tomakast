//! Translator plugin: translates texts with Google Translate (the no-key endpoint, see
//! [`google`]), with a dictionary for words. Design:
//! `docs/superpowers/specs/2026-11-22-translator-design.md`.
//!
//! - The "Translate" command opens the pad ([`pad`]): a compose pad whose text is translated
//!   once it rests [`PAD_REST_MS`], the translation and the dictionary below it.
//! - The target follows the user's pair ([`pair`]): the `languages` preference, else the
//!   system's languages. The pickers ([`picker`]) fix a target or a source, kept in storage
//!   (`target`, `source`).
//! - Recent translations ([`recent`]) are kept in storage (`recent`).
//! - Answers are cached in memory ([`cache`]); requests out are remembered by id.

pub mod cache;
pub mod flags;
pub mod google;
pub mod json;
pub mod langs;
pub mod pad;
pub mod pair;
pub mod phrase;
pub mod picker;
pub mod recent;
pub mod render;
pub mod tile;

use std::cell::RefCell;

use tomakast_plugin::{
    ActionChosen, Effect, Event, HomeTile, HttpDone, Item, Plugin, QueryReading, RequestId,
    SearchAnswer, TextChanged, TimerId, Toast, ViewEvent, ViewTarget, WidgetAction, WidgetAnswer,
    WidgetContext, WidgetEvent, WidgetFrame, WidgetParam, WidgetTarget, export_plugin, http,
    locale, log, param, preferences, storage, timers,
};

use cache::Cache;
use google::{Failure, Request, Translation};
use pad::Pad;
use pair::{Choice, Pair};
use picker::Side;
use recent::Recent;

/// How long the pad's text must rest before it is translated.
pub const PAD_REST_MS: u64 = 400;
/// How long a root-search phrase must rest before its tile is translated.
pub const SEARCH_REST_MS: u64 = 300;
/// The storage key listing the Home tiles' stored translations.
const HOME_INDEX: &str = "home";
/// The command's item id.
const COMMAND: &str = "translator/translate";
/// Storage keys of the picked target and source.
const TARGET_KEY: &str = "target";
const SOURCE_KEY: &str = "source";
const STILL_TRANSLATING: &str = "Still translating";
const PICK_TARGET: &str = "Pick a target language first: Ctrl+P";

/// What a request out was for.
#[derive(Clone, Debug, PartialEq)]
enum Purpose {
    Pad,
    /// The search tile's, or a Home tile's (whose answer is also stored).
    Tile {
        home: bool,
    },
}

#[derive(Default)]
struct State {
    pad: Option<Pad>,
    pad_rest: Option<TimerId>,
    cache: Cache,
    out: Vec<(RequestId, Request, Purpose)>,
    /// The search tile's request, fetched once the query rests [`SEARCH_REST_MS`].
    search: Option<Request>,
    search_rest: Option<TimerId>,
    /// The Home tiles' requests, kept fresh by the `home` event.
    home: Vec<Request>,
    /// The tiles' failed requests, for drawing.
    failures: Vec<(Request, Failure)>,
    /// The Home tiles' translations, apart from the cache so they are never evicted; also in
    /// storage under [`tile::home_key`].
    home_answers: Vec<(Request, Translation)>,
    /// Tile requests the host refused because its requests were all out; sent when one ends.
    waiting: Vec<(Request, bool)>,
    /// The pad's request waits for a free request (one pad request is out at a time).
    pad_waiting: bool,
    stored: Choice,
    recent: Vec<Recent>,
    /// A run of failures was logged; cleared by the next answer.
    failing: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with_borrow_mut(f)
}

/// The pair: the `languages` preference, else the system's languages.
fn current_pair() -> Pair {
    if let Some(value) = preferences::get("languages").filter(|v| !v.trim().is_empty()) {
        match pair::from_preference(&value) {
            Some(pair) => return pair,
            None => log::warn(&format!(
                "languages = {value:?} is not two known languages; using the system's"
            )),
        }
    }
    pair::automatic(&locale::languages())
}

fn read(key: &str) -> Option<String> {
    storage::get(key).ok().flatten()
}

fn write(key: &str, value: Option<&str>) {
    let result = match value {
        Some(value) => storage::set(key, value),
        None => storage::remove(key),
    };
    if let Err(e) = result {
        log::warn(&format!("storage {key}: {e}"));
    }
}

struct TranslatorPlugin;

impl Plugin for TranslatorPlugin {
    fn init() -> Result<Vec<Effect>, String> {
        with(|s| {
            s.stored = Choice {
                from: read(SOURCE_KEY),
                to: read(TARGET_KEY),
            };
            s.recent = read(recent::KEY)
                .map(|raw| recent::load(&raw))
                .unwrap_or_default();
        });
        Ok(Vec::new())
    }

    fn search_with(query: String, readings: Vec<QueryReading>) -> Result<SearchAnswer, String> {
        let Some(found) = phrase::parse_any(&query, &readings) else {
            with(|s| {
                s.search = None;
                if let Some(t) = s.search_rest.take() {
                    timers::cancel(t);
                }
            });
            return Ok(Vec::new().into());
        };
        let r = Request {
            text: found.text,
            from: None,
            to: found.to.to_string(),
        };
        let mut answer = WidgetAnswer::new(tile::WIDGET)?
            .param(tile::TEXT, r.text.clone())
            .param(tile::FROM, "")
            .param(tile::TO, r.to.clone());
        for (id, title) in tile::ACTIONS {
            answer = answer.action(id, title)?;
        }
        with(|s| {
            if s.search.as_ref() != Some(&r) {
                if let Some(t) = s.search_rest.take() {
                    timers::cancel(t);
                }
                if !s.cache.has(&r) {
                    s.search_rest = timers::start(SEARCH_REST_MS, false).ok();
                }
                s.search = Some(r);
            }
        });
        Ok(SearchAnswer {
            items: Vec::new(),
            widgets: vec![answer],
        })
    }

    fn render_widget(id: String, context: WidgetContext) -> Result<WidgetFrame, String> {
        if id != tile::WIDGET {
            return Err(format!("no widget {id}"));
        }
        let r = tile_request(&context.params).ok_or("a translation tile needs text and to")?;
        with(|s| {
            let state = match (tile_answer(s, &r), s.failures.iter().find(|(f, _)| *f == r)) {
                (Some(t), _) => tile::TileState::Done(t),
                (None, Some((_, f))) => tile::TileState::Failed(f),
                (None, None) => tile::TileState::Busy,
            };
            tile::frame(&context, &r.text, r.from.as_deref(), &r.to, state)
        })
    }

    fn run(item: Item, action_id: String) -> Result<Vec<Effect>, String> {
        if item.id == COMMAND && action_id == "run" {
            return open_pad(Pad::new(String::new(), Choice::default()));
        }
        Ok(Vec::new())
    }

    fn on_event(event: Event) -> Result<Vec<Effect>, String> {
        match event {
            Event::View(ViewTarget { view_id, event }) => view_event(&view_id, event),
            Event::TimerFired(timer) => {
                let (pad_due, search_due) = with(|s| {
                    let pad_due = s.pad_rest == Some(timer);
                    if pad_due {
                        s.pad_rest = None;
                    }
                    let search_due = s.search_rest == Some(timer);
                    if search_due {
                        s.search_rest = None;
                        if let Some(r) = s.search.clone() {
                            fetch_tile(s, r, false);
                        }
                    }
                    (pad_due, search_due)
                });
                let mut effects = if pad_due { translate_pad() } else { Vec::new() };
                if search_due {
                    effects.push(Effect::redraw_widget(tile::WIDGET));
                }
                Ok(effects)
            }
            Event::HttpDone(done) => Ok(answered(done)),
            Event::PreferencesChanged => Ok(refresh_pad()),
            Event::Widget(target) => tile_event(target),
            Event::Home(tiles) => Ok(home(tiles)),
            Event::Location(_) => Ok(Vec::new()),
        }
    }
}

/// Pushes `pad` and translates its text at once.
fn open_pad(pad: Pad) -> Result<Vec<Effect>, String> {
    let pair = current_pair();
    let view = with(|s| {
        if let Some(t) = s.pad_rest.take() {
            timers::cancel(t);
        }
        let next = pad.request(&pair, &s.stored);
        s.pad = Some(pad);
        // A cached answer shows in the pushed view itself; a fetch updates it later.
        drive(s, &pair, next);
        s.pad.as_ref().map(|pad| pad.view(&pair, &s.stored))
    });
    match view {
        Some(view) => Ok(vec![Effect::push_view(view?)]),
        None => Ok(Vec::new()),
    }
}

fn view_event(view_id: &str, event: ViewEvent) -> Result<Vec<Effect>, String> {
    if view_id == pad::VIEW {
        return pad_event(event);
    }
    if view_id == recent::VIEW
        && let ViewEvent::ActionChosen(ActionChosen { row_id, .. }) = event
    {
        let Some(entry) = with(|s| recent::row(&s.recent, &row_id)) else {
            return Ok(Vec::new());
        };
        let choice = Choice {
            from: entry.from,
            to: Some(entry.to),
        };
        return open_pad(Pad::new(entry.text, choice));
    }
    Ok(Vec::new())
}

fn pad_event(event: ViewEvent) -> Result<Vec<Effect>, String> {
    match event {
        ViewEvent::TextChanged(TextChanged { revision, text }) => {
            let pair = current_pair();
            with(|s| {
                let Some(pad) = s.pad.as_mut() else {
                    return Ok(Vec::new());
                };
                pad.edited(revision, text);
                if let Some(t) = s.pad_rest.take() {
                    timers::cancel(t);
                }
                if pad.request(&pair, &s.stored).is_some() {
                    s.pad_rest = timers::start(PAD_REST_MS, false).ok();
                }
                Ok(vec![Effect::update_view(pad.view(&pair, &s.stored)?)])
            })
        }
        ViewEvent::ActionChosen(ActionChosen { row_id, action_id }) => {
            match Side::of_button(&action_id) {
                Some(side) => choose_language(side, &row_id),
                None => pad_action(&action_id),
            }
        }
        ViewEvent::Closed => {
            with(|s| {
                if let Some(t) = s.pad_rest.take() {
                    timers::cancel(t);
                }
                if let Some(pad) = s.pad.take() {
                    remember(s, &pad);
                }
            });
            Ok(Vec::new())
        }
        ViewEvent::QueryChanged(_) | ViewEvent::FormSubmitted(_) => Ok(Vec::new()),
    }
}

/// Copies `text`; the launcher says so itself ("Copied to clipboard").
fn copied(text: String) -> Vec<Effect> {
    vec![Effect::copy_text(text)]
}

/// Keeps the pad's text in the recent list when it has a translation.
fn remember(s: &mut State, pad: &Pad) {
    let (Some(t), Some(r)) = (pad.current(), pad.shown_request()) else {
        return;
    };
    if t.text.trim().is_empty() {
        return;
    }
    recent::remember(
        &mut s.recent,
        Recent {
            text: pad.text.clone(),
            from: r.from.clone(),
            to: r.to.clone(),
        },
    );
    write(recent::KEY, Some(&recent::save(&s.recent)));
}

fn pad_action(action_id: &str) -> Result<Vec<Effect>, String> {
    match action_id {
        pad::COPY => Ok(with(|s| {
            let Some(pad) = s.pad.as_ref() else {
                return Vec::new();
            };
            if pad.needs_target() {
                return vec![Effect::toast(Toast::error(PICK_TARGET))];
            }
            match pad.current().map(|t| t.text.clone()) {
                Some(text) => {
                    let pad = s.pad.take().expect("checked above");
                    remember(s, &pad);
                    s.pad = Some(pad);
                    copied(text)
                }
                None => vec![Effect::toast(Toast::success(STILL_TRANSLATING))],
            }
        })),
        pad::COPY_SOURCE => Ok(with(|s| {
            s.pad
                .as_ref()
                .filter(|p| !p.text.is_empty())
                .map(|p| copied(p.text.clone()))
                .unwrap_or_default()
        })),
        pad::SWAP => match with(|s| s.pad.as_ref().and_then(Pad::swapped)) {
            Some(swapped) => open_pad(swapped),
            None => Ok(vec![Effect::toast(Toast::success(STILL_TRANSLATING))]),
        },
        pad::RECENT => Ok(vec![Effect::push_view(with(|s| recent::view(&s.recent))?)]),
        _ => Ok(Vec::new()),
    }
}

/// A language chosen from the pad's bar: stored as the picked target or source ("Automatic"
/// and "Detect language" clear it), the pad's own choice for that side gives way, and the
/// text is translated again.
fn choose_language(side: Side, option_id: &str) -> Result<Vec<Effect>, String> {
    let Some(code) = picker::chosen(option_id) else {
        return Ok(Vec::new());
    };
    let pair = current_pair();
    let key = with(|s| {
        let key = match side {
            Side::Target => {
                s.stored.to = code.clone();
                TARGET_KEY
            }
            Side::Source => {
                s.stored.from = code.clone();
                SOURCE_KEY
            }
        };
        if let Some(pad) = s.pad.as_mut() {
            match side {
                Side::Target => pad.choice.to = None,
                Side::Source => pad.choice.from = None,
            }
            pad.invalidate();
        }
        key
    });
    write(key, code.as_deref());
    with(|s| {
        let next = s.pad.as_ref().and_then(|p| p.request(&pair, &s.stored));
        drive(s, &pair, next);
    });
    Ok(pad_update(&pair))
}

/// Asks for the pad's text: from the cache at once, else from Google.
fn translate_pad() -> Vec<Effect> {
    let pair = current_pair();
    with(|s| {
        let next = s.pad.as_ref().and_then(|p| p.request(&pair, &s.stored));
        drive(s, &pair, next);
    });
    pad_update(&pair)
}

/// Asks `next` for the pad, and the retries its answers want: from the cache while it has
/// them, else from Google.
fn drive(s: &mut State, pair: &Pair, mut next: Option<Request>) {
    s.pad_waiting = false;
    while let Some(r) = next.take() {
        let pad_out = s.out.iter().any(|(_, _, p)| *p == Purpose::Pad);
        let Some(pad) = s.pad.as_mut() else {
            return;
        };
        match s.cache.get(&r).cloned() {
            Some(t) => {
                pad.asking(r.clone());
                next = pad.answered(&r, Ok(t), pair, &s.stored);
            }
            // One pad request at a time: the latest text is asked when it is back. Not asked
            // yet, so the pad asks it again then (a retry's target follows from the source
            // the pad saw last).
            None if pad_out => {
                pad.unask(&r);
                s.pad_waiting = true;
            }
            None => {
                pad.asking(r.clone());
                send(s, r, Purpose::Pad);
            }
        }
    }
}

/// Sends `r` for `purpose`. A request the host refuses is a failure of that request.
fn send(s: &mut State, r: Request, purpose: Purpose) {
    match http::fetch(&google::http_request(&r)) {
        Ok(id) => s.out.push((id, r, purpose)),
        // All the host's requests are out: wait for one to end, it is no failure.
        Err(e) if e.contains("at a time") => match purpose {
            Purpose::Pad => {
                if let Some(pad) = s.pad.as_mut() {
                    pad.unask(&r);
                }
                s.pad_waiting = true;
            }
            Purpose::Tile { home } => {
                if !s.waiting.iter().any(|(w, _)| *w == r) {
                    s.waiting.push((r, home));
                }
            }
        },
        Err(e) => {
            log::warn(&format!("translate request refused: {e}"));
            match purpose {
                Purpose::Pad => {
                    if let Some(pad) = s.pad.as_mut() {
                        pad.answered(&r, Err(Failure::Offline), &current_pair(), &s.stored);
                    }
                }
                Purpose::Tile { .. } => {
                    s.failures.retain(|(f, _)| *f != r);
                    s.failures.push((r, Failure::Offline));
                }
            }
        }
    }
}

fn pad_update(pair: &Pair) -> Vec<Effect> {
    with(|s| {
        let Some(pad) = s.pad.as_ref() else {
            return Vec::new();
        };
        match pad.view(pair, &s.stored) {
            Ok(view) => vec![Effect::update_view(view)],
            Err(e) => {
                log::error(&format!("pad view: {e}"));
                Vec::new()
            }
        }
    })
}

/// The pad again, with the pair as it is now (a preference changed).
fn refresh_pad() -> Vec<Effect> {
    let has_pad = with(|s| s.pad.is_some());
    if has_pad { translate_pad() } else { Vec::new() }
}

fn answered(done: HttpDone) -> Vec<Effect> {
    let Some((r, purpose)) = with(|s| {
        let at = s.out.iter().position(|(id, _, _)| *id == done.request)?;
        let (_, r, purpose) = s.out.remove(at);
        Some((r, purpose))
    }) else {
        return Vec::new();
    };
    let result: Result<Translation, Failure> = match done.response {
        Ok(response) => google::parse(response.status, &response.body),
        Err(_) => Err(Failure::Offline),
    };
    let purpose_was_pad = purpose == Purpose::Pad;
    with(|s| match &result {
        Ok(t) => {
            s.failing = false;
            s.cache.put(r.clone(), t.clone());
            s.failures.retain(|(f, _)| *f != r);
            if s.home.contains(&r) {
                keep_home(s, &r, t);
            }
        }
        Err(f) => {
            if !s.failing {
                log::warn(&format!("translation failed: {}", f.message()));
            }
            s.failing = true;
        }
    });
    let pair = current_pair();
    let mut effects = with(|s| {
        let mut effects = Vec::new();
        match purpose {
            Purpose::Pad => {
                let State { pad, stored, .. } = &mut *s;
                let next = pad.as_mut().and_then(|pad| {
                    // The rule may want another target; else the text typed since, if any.
                    pad.answered(&r, result, &pair, stored)
                        .or_else(|| pad.request(&pair, stored))
                });
                drive(s, &pair, next);
            }
            Purpose::Tile { .. } => {
                if let Err(f) = result {
                    s.failures.retain(|(x, _)| *x != r);
                    s.failures.push((r, f));
                }
                effects.push(Effect::redraw_widget(tile::WIDGET));
            }
        }
        // A request ended: what waited for one goes now, the pad's first.
        if s.pad_waiting {
            let next = s.pad.as_ref().and_then(|p| p.request(&pair, &s.stored));
            drive(s, &pair, next);
        }
        for (r, home) in std::mem::take(&mut s.waiting) {
            fetch_tile(s, r, home);
        }
        effects
    });
    if purpose_was_pad {
        effects.extend(pad_update(&pair));
    }
    effects
}

/// Keeps `t` as the Home tile's translation of `r`, in memory and in storage.
fn keep_home(s: &mut State, r: &Request, t: &Translation) {
    if !s.home_answers.iter().any(|(k, _)| k == r) {
        s.home_answers.push((r.clone(), t.clone()));
    }
    let key = tile::home_key(r.from.as_deref(), &r.to, &r.text);
    write(&key, Some(&tile::home_value(t)));
}

/// A tile's translation: the cache's, else Home's.
fn tile_answer<'a>(s: &'a State, r: &Request) -> Option<&'a Translation> {
    s.cache
        .peek(r)
        .or_else(|| s.home_answers.iter().find(|(k, _)| k == r).map(|(_, t)| t))
}

/// The request a tile's params name.
fn tile_request(params: &[WidgetParam]) -> Option<Request> {
    let text = param(params, tile::TEXT).filter(|t| !t.trim().is_empty())?;
    let to = param(params, tile::TO).filter(|t| !t.is_empty())?;
    let from = param(params, tile::FROM).filter(|f| !f.is_empty());
    Some(Request {
        text: text.to_string(),
        from: from.map(String::from),
        to: to.to_string(),
    })
}

/// Fetches a tile's request unless it is answered or out already.
fn fetch_tile(s: &mut State, r: Request, home: bool) {
    let out = s.out.iter().any(|(_, o, _)| *o == r);
    if !out && !s.cache.has(&r) {
        s.failures.retain(|(f, _)| *f != r);
        send(s, r, Purpose::Tile { home });
    }
}

fn tile_event(target: WidgetTarget) -> Result<Vec<Effect>, String> {
    let WidgetTarget {
        widget_id,
        params,
        event: WidgetEvent::Action(WidgetAction { action_id, .. }),
    } = target
    else {
        return Ok(Vec::new());
    };
    let Some(r) = tile_request(&params).filter(|_| widget_id == tile::WIDGET) else {
        return Ok(Vec::new());
    };
    match action_id.as_str() {
        "copy" => Ok(with(|s| match tile_answer(s, &r) {
            Some(t) => copied(t.text.clone()),
            None => {
                // Try again: a failed tile gets a new request.
                fetch_tile(s, r, false);
                vec![
                    Effect::toast(Toast::success(STILL_TRANSLATING)),
                    Effect::redraw_widget(tile::WIDGET),
                ]
            }
        })),
        "open" => open_pad(Pad::new(
            r.text,
            Choice {
                from: r.from,
                to: Some(r.to),
            },
        )),
        _ => Ok(Vec::new()),
    }
}

/// Home's tiles: their translations come from storage, else from Google, once; the stored
/// ones no tile names any more are removed.
fn home(tiles: Vec<HomeTile>) -> Vec<Effect> {
    let wanted: Vec<Request> = tiles
        .iter()
        .filter(|t| t.widget_id == tile::WIDGET)
        .filter_map(|t| tile_request(&t.params))
        .collect();
    let keys: Vec<String> = wanted
        .iter()
        .map(|r| tile::home_key(r.from.as_deref(), &r.to, &r.text))
        .collect();
    let old = read(HOME_INDEX).unwrap_or_default();
    for gone in old.lines().filter(|k| !keys.iter().any(|key| key == k)) {
        write(gone, None);
    }
    let index = keys.join("\n");
    if index != old {
        write(HOME_INDEX, (!index.is_empty()).then_some(index.as_str()));
    }
    with(|s| {
        s.home_answers.retain(|(r, _)| wanted.contains(r));
        s.home = wanted.clone();
        for (r, key) in wanted.into_iter().zip(keys) {
            if s.home_answers.iter().any(|(k, _)| *k == r) {
                continue;
            }
            if let Some(t) = read(&key).and_then(|v| tile::from_home_value(&v)) {
                s.home_answers.push((r, t));
            } else if let Some(t) = s.cache.peek(&r).cloned() {
                // Shown by the search just now: keep it for Home too.
                keep_home(s, &r, &t);
            } else {
                fetch_tile(s, r, true);
            }
        }
    });
    vec![Effect::redraw_widget(tile::WIDGET)]
}

export_plugin!(TranslatorPlugin);
