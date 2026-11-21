//! Calculator plugin: answers math typed in the launcher search, and keeps scratchpad sheets
//! (with a history of them) in the plugin's storage.
//!
//! - Search: a query that is clearly math gets one double tile, the widget `answer`: the
//!   input on the left and the result on the right, each with a caption. Its params `result`
//!   and `input` are what search computed; `copy` copies the result and `copy-both` the input
//!   and the result (`2+2 = 4`). Added to Home it evaluates its input afresh on each render
//!   (currencies and times move on), falling back to the stored result when it cannot.
//! - The "Calculator" command opens the current sheet as a scratchpad, `sheet:<id>`, with one
//!   annotation per non-empty line. Edits are evaluated on every `text-changed` and saved
//!   (see [`sheets`] for when); `closed` saves what is left.
//! - "Calculator History" (and the pad's History action) lists the sheets, newest first.
//! - Exchange rates are fetched at `init` and on a 10-minute timer, in the background (see
//!   [`rates`]), and once more a minute after a failed fetch of a feed calc has none of;
//!   search and sheets read what is in memory. New rates, and a new `timezone`, update the
//!   open pads unasked.
//! - The zone is the `timezone` preference, else the computer's (read on every evaluation,
//!   so a change of the computer's zone is followed), else UTC.

pub mod engine;
pub mod json;
pub mod rates;
#[cfg(test)]
mod sheet_parity;
mod sheets;
mod views;

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use tomakast_plugin::{
    ActionChosen, Effect, Event, Item, Plugin, ScratchpadView, SearchAnswer, TextChanged, Toast,
    View, ViewEvent, ViewTarget, WidgetAction, WidgetContext, WidgetEvent, WidgetFrame,
    WidgetTarget, export_plugin, locale, log, preferences, timers,
};

use engine::zones::ZoneId;
use engine::{Context, Used, zones};
use views::{ANSWER, HISTORY, SHEET_PREFIX};

struct Calc;

thread_local! {
    /// The zone the `timezone` preference names, or `None` to follow the computer's.
    static ZONE: Cell<Option<ZoneId>> = const { Cell::new(None) };
    /// The sheets whose pad is on screen, and the revision the launcher last sent for each:
    /// an update that no edit asked for must carry it, or the launcher drops it.
    static OPEN: RefCell<BTreeMap<String, u64>> = RefCell::default();
}

/// Reads the `timezone` preference into [`ZONE`]. An empty one follows the computer's zone;
/// a value that names no zone is UTC.
fn load_zone() {
    let zone = match preferences::get("timezone") {
        Some(value) if !value.trim().is_empty() => Some(zones::find(&value).unwrap_or_else(|| {
            log::warn(&format!(
                "timezone '{value}' is not a known time zone; using UTC"
            ));
            zones::utc()
        })),
        _ => None,
    };
    ZONE.set(zone);
}

/// The user's zone: the preference's, else the computer's (asked on every call), else UTC.
fn zone() -> ZoneId {
    ZONE.get()
        .unwrap_or_else(|| zones::find(&locale::local_time_zone()).unwrap_or(zones::utc()))
}

impl Plugin for Calc {
    fn init() -> Result<Vec<Effect>, String> {
        load_zone();
        rates::with(rates::Feeds::load);
        match timers::start(rates::TICK_MS, true) {
            Ok(tick) => rates::with(|feeds| feeds.tick = Some(tick)),
            Err(e) => log::warn(&format!("starting the rates timer failed: {e}")),
        }
        rates::with(|feeds| feeds.refresh(sheets::unix_now(), true));
        Ok(Vec::new())
    }

    fn search(query: String) -> Result<SearchAnswer, String> {
        // Called on every keystroke in the launcher: an edit still waiting (after `closed`
        // with the write budget spent) is saved here, without waiting for a user action,
        // unless saving it needs a sheet dropped (that waits for the next other call).
        sheets::with(sheets::Sheets::flush_quietly);
        let (answer, used) = with_context(|ctx| views::answer(&query, ctx));
        note_rates_used(used);
        Ok(SearchAnswer {
            items: Vec::new(),
            widgets: answer?.into_iter().collect(),
        })
    }

    fn run(item: Item, action_id: String) -> Result<Vec<Effect>, String> {
        match (item.id.as_str(), action_id.as_str()) {
            ("calc/calculator", "run") => with_sheets(|s| {
                let mut effects = s.flush();
                let pushed = s.open_current().and_then(|id| push_sheet(s, &id));
                effects.push(pushed.unwrap_or_else(failed));
                Ok(effects)
            }),
            ("calc/history", "run") => with_sheets(|s| {
                let mut effects = s.flush();
                effects.push(push_history(s));
                Ok(effects)
            }),
            _ => Ok(Vec::new()),
        }
    }

    fn on_event(event: Event) -> Result<Vec<Effect>, String> {
        let (view_id, event) = match event {
            Event::View(ViewTarget { view_id, event }) => (view_id, event),
            Event::PreferencesChanged => {
                load_zone();
                return Ok(open_pad_updates());
            }
            Event::TimerFired(timer) => {
                rates::with(|feeds| feeds.timer_fired(timer, sheets::unix_now()));
                return Ok(Vec::new());
            }
            Event::Widget(target) => return Ok(answer_event(target)),
            // Calc asks for no location.
            Event::Location(_) => return Ok(Vec::new()),
            // A tile draws from its params alone: nothing to keep fresh while hidden.
            Event::Home(_) => return Ok(Vec::new()),
            Event::HttpDone(done) => {
                // `answered` writes through the sheets: it returns before the pads are drawn.
                let stored = rates::with(|feeds| feeds.answered(done, sheets::unix_now()));
                return Ok(if stored {
                    open_pad_updates()
                } else {
                    Vec::new()
                });
            }
        };
        if let Some(id) = view_id.strip_prefix(SHEET_PREFIX) {
            if !views::is_sheet_id(id) {
                return Ok(Vec::new());
            }
            return with_sheets(|s| sheet_event(s, id, event));
        }
        match event {
            ViewEvent::ActionChosen(ActionChosen { row_id, action_id }) if view_id == HISTORY => {
                with_sheets(|s| history_action(s, &row_id, &action_id))
            }
            _ => Ok(Vec::new()),
        }
    }

    fn render_widget(id: String, context: WidgetContext) -> Result<WidgetFrame, String> {
        if id != ANSWER {
            return Err(format!("no widget {id}"));
        }
        with_context(|ctx| views::answer_tile(&context, ctx)).0
    }
}

export_plugin!(Calc);

/// An action on the answer tile: `copy` copies the result of its params, `copy-both` the
/// input and the result; on a Home tile `copy-live` and `copy-both-live` copy the same of
/// what it shows now.
fn answer_event(target: WidgetTarget) -> Vec<Effect> {
    let WidgetTarget {
        widget_id,
        params,
        event: WidgetEvent::Action(WidgetAction { action_id, .. }),
    } = target
    else {
        return Vec::new();
    };
    let Some((result, input)) = views::tile_params(&params) else {
        return Vec::new();
    };
    if widget_id != ANSWER {
        return Vec::new();
    }
    // A Home tile's actions copy what it shows now.
    let live = || with_context(|ctx| views::home_parts(result, input, ctx)).0;
    match action_id.as_str() {
        "copy" => vec![Effect::copy_text(result)],
        "copy-both" => vec![Effect::copy_text(format!("{input} = {result}"))],
        "copy-live" => vec![Effect::copy_text(live().result)],
        "copy-both-live" => {
            let parts = live();
            vec![Effect::copy_text(format!(
                "{} = {}",
                parts.input, parts.result
            ))]
        }
        _ => Vec::new(),
    }
}

/// Runs `f`, a call that may save, on the sheets, and puts the toast naming the sheets
/// dropped to make room (by this call or by searches before it) after `f`'s own effects,
/// so that no toast of `f` replaces it. A call that fails tells them in the next one.
fn with_sheets(
    f: impl FnOnce(&mut sheets::Sheets) -> Result<Vec<Effect>, String>,
) -> Result<Vec<Effect>, String> {
    sheets::with(|s| {
        let mut effects = f(s)?;
        effects.extend(s.drop_notice());
        Ok(effects)
    })
}

/// An event of the pad of sheet `id`.
fn sheet_event(s: &mut sheets::Sheets, id: &str, event: ViewEvent) -> Result<Vec<Effect>, String> {
    match event {
        ViewEvent::TextChanged(TextChanged { revision, text }) => {
            OPEN.with_borrow_mut(|open| open.insert(id.to_owned(), revision));
            let (update, used) = with_context(|ctx| views::pad_update(id, &text, revision, ctx));
            note_rates_used(used);
            let mut effects = vec![Effect::update_view(update?)];
            effects.extend(s.edited(id, text));
            Ok(effects)
        }
        ViewEvent::Closed => {
            OPEN.with_borrow_mut(|open| open.remove(id));
            Ok(s.closed(id))
        }
        ViewEvent::ActionChosen(ActionChosen { action_id, .. }) => match action_id.as_str() {
            "copy-last" => {
                let mut effects = s.flush();
                let last = s
                    .text(id)
                    .map(|text| with_context(|ctx| views::last_result(&text, ctx)).0);
                effects.push(match last {
                    Ok(Some(result)) => Effect::copy_text(result),
                    Ok(None) => Effect::toast(Toast::error("No result to copy")),
                    Err(e) => failed(e),
                });
                Ok(effects)
            }
            "new" => {
                let mut effects = s.flush();
                match s.create().and_then(|new| push_sheet(s, &new)) {
                    // The new sheet takes the old one's place on the stack.
                    Ok(pad) => effects.extend([Effect::pop_view(), pad]),
                    Err(e) => effects.push(failed(e)),
                }
                Ok(effects)
            }
            "history" => {
                // Pushed above the pad. Opening this sheet from it does not stack a second
                // pad: the launcher returns to this one (a push of a view id already on the
                // stack, see `push-view` in the WIT).
                let mut effects = s.flush();
                effects.push(push_history(s));
                Ok(effects)
            }
            _ => Ok(Vec::new()),
        },
        ViewEvent::QueryChanged(_) | ViewEvent::FormSubmitted(_) => Ok(Vec::new()),
    }
}

/// An action on the history row of sheet `id`.
fn history_action(
    s: &mut sheets::Sheets,
    id: &str,
    action_id: &str,
) -> Result<Vec<Effect>, String> {
    // Rows are calc's own sheet ids; anything else is not acted on.
    if !views::is_sheet_id(id) {
        return Ok(Vec::new());
    }
    let mut effects = s.flush();
    match action_id {
        "open" => {
            let pushed = s.make_current(id).and_then(|()| push_sheet(s, id));
            effects.push(pushed.unwrap_or_else(failed));
        }
        "delete" => {
            // Rows are filtered by the launcher, so the list's revision never moves.
            let updated = s.delete(id).and_then(|()| history(s));
            effects.push(updated.map_or_else(failed, Effect::update_view));
        }
        _ => {}
    }
    Ok(effects)
}

/// The push of sheet `id`'s pad, noted as open at revision 0: a push of a pad already on
/// the stack replaces it, revision and all.
fn push_sheet(s: &sheets::Sheets, id: &str) -> Result<Effect, String> {
    let text = s.text(id)?;
    let pad = sheet_pad(id, &text)?;
    OPEN.with_borrow_mut(|open| open.insert(id.to_owned(), 0));
    Ok(Effect::push_view(pad))
}

/// Updates of every open pad, evaluated again after new rates or a new zone, which change
/// what its lines show. Unasked: nothing is saved, and no feed counts as used (only the
/// user's edits and opens do, or an open pad would keep the crypto feed due forever).
fn open_pad_updates() -> Vec<Effect> {
    let open: Vec<(String, u64)> =
        OPEN.with_borrow(|open| open.iter().map(|(id, rev)| (id.clone(), *rev)).collect());
    sheets::with(|s| {
        open.into_iter()
            .filter_map(|(id, revision)| {
                let text = s
                    .text(&id)
                    .map_err(|e| log::warn(&format!("reading sheet {id} failed: {e}")))
                    .ok()?;
                let (update, _) = with_context(|ctx| views::pad_update(&id, &text, revision, ctx));
                update.ok().map(Effect::update_view)
            })
            .collect()
    })
}

/// The pad of sheet `id` holding `text`. Opening a sheet counts as using the feeds its
/// lines need.
fn sheet_pad(id: &str, text: &str) -> Result<ScratchpadView, String> {
    let (pad, used) = with_context(|ctx| views::pad(id, text, ctx));
    note_rates_used(used);
    Ok(pad?)
}

/// Runs `f` with the evaluation context: the current time, the user's zone and the stored
/// rates. Returns what the evaluation used. `f` must not touch [`rates::with`].
fn with_context<R>(f: impl FnOnce(&Context) -> R) -> (R, Used) {
    rates::with(|feeds| {
        let ctx = Context::new(sheets::unix_now(), zone(), &feeds.rates);
        let result = f(&ctx);
        (result, ctx.used())
    })
}

/// Tells the feeds which of them an evaluation needed (memory only). It runs after
/// [`with_context`] has returned: both borrow the same cell.
fn note_rates_used(used: Used) {
    rates::with(|feeds| feeds.mark_used(used, sheets::unix_now()));
}

fn push_history(s: &sheets::Sheets) -> Effect {
    history(s).map_or_else(failed, Effect::push_view)
}

fn history(s: &sheets::Sheets) -> Result<View, String> {
    Ok(views::history(&s.entries()?, sheets::unix_now())?.into())
}

fn failed(message: String) -> Effect {
    Effect::toast(Toast::error("Could not change the sheets").message(message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::currency::Table;

    #[test]
    fn opening_a_sheet_counts_as_using_its_rates() {
        // A native test cannot ask the computer's zone (the import is the launcher's).
        ZONE.set(Some(zones::utc()));
        let now = sheets::unix_now();
        // Crypto 15 minutes old: due on a tick only when an answer used it lately.
        rates::with(|feeds| {
            feeds.rates.crypto = Some(Table {
                fetched: now - 900,
                values: [("BTC", 67_000.0)].into_iter().collect(),
            });
        });
        let due = || rates::with(|feeds| feeds.due_at(now, false));
        assert!(!due().crypto);
        sheet_pad("0", "2 + 2").unwrap();
        assert!(!due().crypto);
        sheet_pad("0", "1 btc in usd").unwrap();
        assert!(due().crypto);
    }
}
