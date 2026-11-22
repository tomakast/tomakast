//! The translator pad: a compose pad holding the user's text, its translation below.
//!
//! [`Pad`] is pure: it says which request to send and what the pad shows; `lib.rs` does the
//! fetching, the timers and the storage.

use tomakast_plugin::{Action, Icon, ScratchpadView, ToolbarButton, View};

use crate::google::{self, Failure, Request, Translation};
use crate::pair::{self, Choice, Pair};
use crate::picker::{self, Side};
use crate::render;
use crate::{flags, langs};

/// The pad's view id. One pad at a time: pushing it again returns to it.
pub const VIEW: &str = "pad";

/// The pad's actions, the first run by Ctrl/Cmd+Enter.
pub const COPY: &str = "copy";
pub const COPY_SOURCE: &str = "copy-source";
pub const RECENT: &str = "recent";
/// The toolbar's swap button; the language buttons are [`Side::button_id`]'s.
pub const SWAP: &str = "swap";

pub struct Pad {
    pub text: String,
    pub revision: u64,
    /// Languages fixed for this pad only (swap, a tile, a recent row); over the stored ones.
    pub choice: Choice,
    /// The source Google detected for this pad's last answered text.
    last_src: Option<String>,
    /// The last answer shown, for whatever text it was.
    shown: Option<(Request, Result<Translation, Failure>)>,
    /// The request out for the current text.
    asked: Option<Request>,
    /// The current text was asked again once already.
    retried: bool,
    /// The last answer has no target to go to (English text on an English-only pair).
    no_target: bool,
    /// The languages changed since the last answer: it is shown until the next one, but
    /// counts for nothing.
    stale: bool,
}

impl Pad {
    pub fn new(text: String, choice: Choice) -> Pad {
        Pad {
            text,
            revision: 0,
            choice,
            last_src: None,
            shown: None,
            asked: None,
            retried: false,
            no_target: false,
            stale: false,
        }
    }

    /// The languages changed: the current text is asked again, and the answer shown so far
    /// is no longer its translation.
    pub fn invalidate(&mut self) {
        self.stale = true;
        self.asked = None;
        self.retried = false;
    }

    /// The user typed: `text` at `revision`.
    pub fn edited(&mut self, revision: u64, text: String) {
        self.revision = revision;
        if text != self.text {
            self.text = text;
            self.retried = false;
            self.asked = None;
        }
    }

    /// What to ask for the current text: `None` for an empty text, a text with no target, or
    /// one already asked or answered.
    pub fn request(&self, pair: &Pair, stored: &Choice) -> Option<Request> {
        if self.text.trim().is_empty() || self.asked.is_some() || self.answered_now() {
            return None;
        }
        let choice = self.choice.over(stored);
        let to = pair::first_target(pair, &choice, &self.text, self.last_src.as_deref())?;
        Some(Request {
            text: self.text.clone(),
            from: choice.from,
            to,
        })
    }

    /// `r` is on its way.
    pub fn asking(&mut self, r: Request) {
        self.asked = Some(r);
    }

    /// `r` was not sent after all (the host's requests are all out): it is asked again later.
    pub fn unask(&mut self, r: &Request) {
        if self.asked.as_ref() == Some(r) {
            self.asked = None;
        }
    }

    /// The answer to `r`. Returns the request to send next when the rule wants another
    /// target for the detected source (once per text). An answer to a request this pad is not
    /// waiting for (an older text, or a target since changed) changes nothing.
    pub fn answered(
        &mut self,
        r: &Request,
        answer: Result<Translation, Failure>,
        pair: &Pair,
        stored: &Choice,
    ) -> Option<Request> {
        if self.asked.as_ref() != Some(r) {
            return None;
        }
        self.asked = None;
        self.no_target = false;
        if let Ok(t) = &answer {
            self.last_src = Some(t.src.clone());
            let choice = self.choice.over(stored);
            if !self.retried
                && let Some(to) = pair::retry_target(pair, &choice, &r.to, &t.src)
            {
                self.retried = true;
                let again = Request { to, ..r.clone() };
                self.asked = Some(again.clone());
                return Some(again);
            }
            self.no_target = choice.to.is_none() && pair::target_for(pair, &t.src).is_none();
        }
        // Only an answer shown replaces the one from before a change of languages.
        self.stale = false;
        self.shown = Some((r.clone(), answer));
        None
    }

    fn answered_now(&self) -> bool {
        !self.stale
            && self
                .shown
                .as_ref()
                .is_some_and(|(r, _)| r.text == self.text)
    }

    /// The translation of the current text, if it is in and went somewhere.
    pub fn current(&self) -> Option<&Translation> {
        match &self.shown {
            Some((r, Ok(t))) if r.text == self.text && !self.no_target && !self.stale => Some(t),
            _ => None,
        }
    }

    /// The request [`current`](Self::current) answers.
    pub fn shown_request(&self) -> Option<&Request> {
        self.current()?;
        self.shown.as_ref().map(|(r, _)| r)
    }

    /// The current text's answer had nowhere to go: a target must be picked.
    pub fn needs_target(&self) -> bool {
        self.no_target && self.answered_now()
    }

    /// The pad as the launcher shows it: title, output and actions at the pad's revision.
    pub fn view(&self, pair: &Pair, stored: &Choice) -> Result<View, String> {
        let (busy, output, detail) = self.face(pair, stored);
        let pad: ScratchpadView = View::scratchpad(VIEW, render::TITLE, self.text.clone())?
            .revision(self.revision)
            .output(output)
            .busy(busy)
            .placeholder(render::PLACEHOLDER);
        let pad = match detail {
            Some(detail) => pad.detail(detail),
            None => pad,
        };
        let pad = self.toolbar(pad, pair, stored)?;
        Ok(actions(pad)?.into())
    }

    /// The language bar: the source's menu, swap, the target's menu.
    fn toolbar(
        &self,
        pad: ScratchpadView,
        pair: &Pair,
        stored: &Choice,
    ) -> Result<ScratchpadView, String> {
        let choice = self.choice.over(stored);
        let languages = || Icon::Named("languages".into());
        // The source last detected, while there is text: an empty field detects nothing.
        let shown_src = match &self.shown {
            Some((_, Ok(t))) if !t.src.is_empty() && !self.text.trim().is_empty() => {
                Some(t.src.as_str())
            }
            _ => None,
        };
        let (from_label, from_selected, from_icon) = match (&choice.from, shown_src) {
            (Some(from), _) => (langs::name(from), from.clone(), flags::icon(from)),
            (None, Some(src)) => (
                format!("{} (detected)", langs::name(src)),
                picker::DETECT.to_string(),
                flags::icon(src),
            ),
            (None, None) => (
                "Detect language".to_string(),
                picker::DETECT.to_string(),
                languages(),
            ),
        };
        let target = match (&choice.to, &self.shown) {
            (Some(to), _) => Some(to.clone()),
            _ if self.text.trim().is_empty() => None,
            (None, Some((r, _))) if r.text == self.text && !self.stale => Some(r.to.clone()),
            (None, _) => pair::first_target(pair, &choice, &self.text, self.last_src.as_deref()),
        };
        let (to_label, to_icon) = match &target {
            Some(to) => (langs::name(to), flags::icon(to)),
            None if self.text.trim().is_empty() => ("Automatic".to_string(), languages()),
            None => ("Choose language".to_string(), languages()),
        };
        let to_selected = choice
            .to
            .clone()
            .unwrap_or_else(|| picker::AUTO.to_string());
        let from = ToolbarButton::new(Side::Source.button_id(), from_label)?
            .icon(from_icon)
            .shortcut("ctrl+shift+p")
            .options(picker::options(Side::Source, pair)?)
            .selected(from_selected);
        let swap = ToolbarButton::new(SWAP, "⇄")?.shortcut("ctrl+s");
        let to = ToolbarButton::new(Side::Target.button_id(), to_label)?
            .icon(to_icon)
            .shortcut("ctrl+p")
            .options(picker::options(Side::Target, pair)?)
            .selected(to_selected);
        Ok(pad.button(from).button(swap).button(to))
    }

    /// The output, and whether a translation of the text is on its way.
    fn face(&self, pair: &Pair, stored: &Choice) -> (bool, String, Option<String>) {
        if self.text.trim().is_empty() {
            return (false, String::new(), None);
        }
        let busy = !self.answered_now();
        if busy && self.asked.is_none() && self.request(pair, stored).is_none() {
            return (false, String::new(), Some(render::pick_target_hint()));
        }
        let (output, detail) = match &self.shown {
            None => (String::new(), None),
            Some((_, Ok(_))) if !busy && self.no_target => {
                (String::new(), Some(render::pick_target_hint()))
            }
            Some((r, Ok(t))) => render::pane(t, google::cut(&r.text).1),
            Some((_, Err(f))) => (String::new(), Some(render::failure(f))),
        };
        (busy, output, detail)
    }

    /// A pad for the other direction: the translation as its text, the languages swapped and
    /// fixed for that pad.
    pub fn swapped(&self) -> Option<Pad> {
        let t = self.current()?;
        let r = self.shown_request()?;
        let from = r.from.clone().unwrap_or_else(|| t.src.clone());
        Some(Pad::new(
            t.text.clone(),
            Choice {
                from: Some(r.to.clone()),
                to: Some(from),
            },
        ))
    }
}

fn actions(pad: ScratchpadView) -> Result<ScratchpadView, String> {
    // The language actions are named as their toolbar buttons: the launcher opens the
    // button's menu for them.
    let pad = pad.action(COPY, "Copy Translation")?;
    Ok(pad
        .with_action(Action::new(COPY_SOURCE, "Copy Source Text")?.shortcut("ctrl+shift+c"))
        .with_action(Action::new(SWAP, "Swap Languages")?.shortcut("ctrl+s"))
        .with_action(
            Action::new(Side::Target.button_id(), "Change Target Language…")?.shortcut("ctrl+p"),
        )
        .with_action(
            Action::new(Side::Source.button_id(), "Change Source Language…")?
                .shortcut("ctrl+shift+p"),
        )
        .with_action(Action::new(RECENT, "Recent Translations")?.shortcut("ctrl+shift+h")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google::tests::{RUN, answer};

    fn ru_en() -> Pair {
        Pair {
            first: "ru".into(),
            second: Some("en".into()),
        }
    }

    fn translation(text: &str, src: &str) -> Translation {
        Translation {
            text: text.into(),
            src: src.into(),
            ..Translation::default()
        }
    }

    fn output(pad: &Pad, pair: &Pair) -> (String, String) {
        let View::ScratchpadView(view) = pad.view(pair, &Choice::default()).unwrap() else {
            panic!("a pad");
        };
        assert_eq!(view.title, "Translate", "never a status in the title");
        let status = if view.busy { "busy" } else { "idle" };
        // The output, then the detail (a failure, a hint, the dictionary).
        let shown = view.output.unwrap() + &view.detail.unwrap_or_default();
        (status.into(), shown)
    }

    fn typed(text: &str) -> Pad {
        let mut pad = Pad::new(String::new(), Choice::default());
        pad.edited(1, text.into());
        pad
    }

    #[test]
    fn a_text_is_asked_once_and_shown() {
        let none = Choice::default();
        let mut pad = typed("hello");
        let r = pad.request(&ru_en(), &none).unwrap();
        assert_eq!((r.to.as_str(), r.from.as_deref()), ("ru", None));
        pad.asking(r.clone());
        assert_eq!(pad.request(&ru_en(), &none), None, "already out");
        assert_eq!(output(&pad, &ru_en()).0, "busy");
        assert_eq!(
            pad.answered(&r, Ok(translation("привет", "en")), &ru_en(), &none),
            None
        );
        assert_eq!(pad.current().unwrap().text, "привет");
        let (status, out) = output(&pad, &ru_en());
        assert_eq!(status, "idle");
        assert!(out.starts_with("привет"));
        assert_eq!(pad.request(&ru_en(), &none), None, "answered");
    }

    #[test]
    fn a_detected_source_asks_again_once() {
        let none = Choice::default();
        let mut pad = typed("привіт");
        let r = pad.request(&ru_en(), &none).unwrap();
        assert_eq!(r.to, "en", "Cyrillic is guessed Russian");
        pad.asking(r.clone());
        let again = pad
            .answered(&r, Ok(translation("hi", "uk")), &ru_en(), &none)
            .expect("Ukrainian goes to Russian");
        assert_eq!(again.to, "ru");
        assert!(pad.current().is_none(), "the first answer is not shown");
        assert_eq!(
            pad.answered(&again, Ok(translation("привет", "uk")), &ru_en(), &none),
            None
        );
        assert_eq!(pad.current().unwrap().text, "привет");
    }

    /// Review Focus 5: an answer for an older text changes nothing.
    #[test]
    fn late_answers_are_ignored() {
        let none = Choice::default();
        let mut pad = typed("one");
        let first = pad.request(&ru_en(), &none).unwrap();
        pad.asking(first.clone());
        pad.answered(&first, Ok(translation("один", "en")), &ru_en(), &none);
        pad.edited(2, "two".into());
        let second = pad.request(&ru_en(), &none).unwrap();
        pad.asking(second);
        let stale = Request {
            text: "one".into(),
            ..first
        };
        pad.answered(&stale, Ok(translation("СТАРОЕ", "en")), &ru_en(), &none);
        let (title, out) = output(&pad, &ru_en());
        assert!(out.starts_with("один"), "keeps the last pane: {out}");
        assert_eq!(title, "busy");
        // Review Focus 1: nothing to copy for the new text yet.
        assert!(pad.current().is_none());
    }

    #[test]
    fn hints() {
        let pad = Pad::new(String::new(), Choice::default());
        let (status, out) = output(&pad, &ru_en());
        assert_eq!(status, "idle");
        assert_eq!(out, "", "the hint is the editor's placeholder");
        let View::ScratchpadView(view) = pad.view(&ru_en(), &Choice::default()).unwrap() else {
            panic!();
        };
        assert_eq!(view.placeholder, "Type or paste text to translate.");

        // Review Focus 4: an English-only pair and English text.
        let en = Pair {
            first: "en".into(),
            second: None,
        };
        let none = Choice::default();
        let mut pad = typed("hello");
        let r = pad.request(&en, &none).unwrap();
        pad.asking(r.clone());
        pad.answered(&r, Ok(translation("hello", "en")), &en, &none);
        assert!(output(&pad, &en).1.starts_with("This text is in English"));
    }

    /// Review finding 1: an answer to a request no longer asked (the user picked another
    /// target for the same text) changes nothing.
    #[test]
    fn an_answer_no_longer_asked_is_ignored() {
        let none = Choice::default();
        let mut pad = typed("hello");
        let to_ru = pad.request(&ru_en(), &none).unwrap();
        pad.asking(to_ru.clone());
        // Ctrl+P, German: the pad is opened again for the same text.
        let german = Choice {
            from: None,
            to: Some("de".into()),
        };
        let mut pad = Pad::new("hello".into(), german);
        let to_de = pad.request(&ru_en(), &none).unwrap();
        assert_eq!(to_de.to, "de");
        pad.asking(to_de.clone());
        pad.answered(&to_ru, Ok(translation("привет", "en")), &ru_en(), &none);
        assert!(
            pad.current().is_none(),
            "the Russian answer is not this pad's"
        );
        pad.answered(&to_de, Ok(translation("hallo", "en")), &ru_en(), &none);
        assert_eq!(pad.current().unwrap().text, "hallo");
    }

    /// Review findings 3 and 4: on an English-only pair, English text has no target: nothing
    /// to copy or swap; and a later text in another script is still translated.
    #[test]
    fn an_english_only_pair_recovers() {
        let en = Pair {
            first: "en".into(),
            second: None,
        };
        let none = Choice::default();
        let mut pad = typed("hello");
        let r = pad.request(&en, &none).unwrap();
        pad.asking(r.clone());
        pad.answered(&r, Ok(translation("hello", "en")), &en, &none);
        assert!(pad.needs_target());
        assert!(pad.current().is_none(), "nothing to copy");
        assert!(pad.swapped().is_none(), "nothing to swap");
        pad.edited(2, "привет".into());
        let r = pad.request(&en, &none).expect("Cyrillic goes to English");
        assert_eq!(r.to, "en");
        pad.asking(r.clone());
        pad.answered(&r, Ok(translation("hi", "ru")), &en, &none);
        assert!(!pad.needs_target());
        assert_eq!(pad.current().unwrap().text, "hi");
        assert_eq!(pad.shown_request().unwrap().to, "en");
    }

    #[test]
    fn failures_show_one_line() {
        let none = Choice::default();
        let mut pad = typed("hello");
        let r = pad.request(&ru_en(), &none).unwrap();
        pad.asking(r.clone());
        pad.answered(&r, Err(Failure::Offline), &ru_en(), &none);
        assert_eq!(output(&pad, &ru_en()).1, "No connection.");
        assert!(pad.current().is_none());
    }

    #[test]
    fn swap_and_choices() {
        let none = Choice::default();
        let mut pad = typed("run");
        let r = pad.request(&ru_en(), &none).unwrap();
        pad.asking(r.clone());
        pad.answered(&r, Ok(answer(RUN)), &ru_en(), &none);
        let swapped = pad.swapped().unwrap();
        assert_eq!(swapped.text, "бегать");
        assert_eq!(
            swapped.choice,
            Choice {
                from: Some("ru".into()),
                to: Some("en".into())
            }
        );
        let stored = Choice {
            from: None,
            to: Some("de".into()),
        };
        assert_eq!(typed("hello").request(&ru_en(), &stored).unwrap().to, "de");
        let mut fixed = Pad::new(
            String::new(),
            Choice {
                from: None,
                to: Some("fr".into()),
            },
        );
        fixed.edited(1, "hello".into());
        assert_eq!(
            fixed.request(&ru_en(), &stored).unwrap().to,
            "fr",
            "the pad's own wins"
        );
    }

    #[test]
    fn the_actions() {
        let View::ScratchpadView(view) = typed("x").view(&ru_en(), &Choice::default()).unwrap()
        else {
            panic!();
        };
        let ids: Vec<&str> = view.actions.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(
            ids,
            [COPY, COPY_SOURCE, SWAP, "to-lang", "from-lang", RECENT]
        );
        let keys: Vec<Option<&str>> = view.actions.iter().map(|a| a.shortcut.as_deref()).collect();
        assert_eq!(
            keys,
            [
                None,
                Some("ctrl+shift+c"),
                Some("ctrl+s"),
                Some("ctrl+p"),
                Some("ctrl+shift+p"),
                Some("ctrl+shift+h")
            ]
        );
        assert_eq!(view.revision, 1);
    }

    fn bar(pad: &Pad, stored: &Choice) -> Vec<(String, String, Option<String>)> {
        let View::ScratchpadView(view) = pad.view(&ru_en(), stored).unwrap() else {
            panic!();
        };
        view.toolbar
            .into_iter()
            .map(|b| (b.id, b.label, b.selected))
            .collect()
    }

    #[test]
    fn the_language_bar() {
        let none = Choice::default();
        // Before any answer: the source is detected, the target automatic.
        let pad = typed("hello");
        assert_eq!(
            bar(&pad, &none),
            [
                (
                    "from-lang".into(),
                    "Detect language".into(),
                    Some("detect".into())
                ),
                ("swap".into(), "⇄".into(), None),
                ("to-lang".into(), "Russian".into(), Some("auto".into())),
            ]
        );
        // After an answer: the detected source, and the target it went to.
        let mut pad = typed("hello");
        let r = pad.request(&ru_en(), &none).unwrap();
        pad.asking(r.clone());
        pad.answered(&r, Ok(translation("привет", "en")), &ru_en(), &none);
        let shown = bar(&pad, &none);
        assert_eq!(shown[0].1, "English (detected)");
        assert_eq!(shown[2].1, "Russian");
        // The field emptied: nothing is detected any more.
        let mut emptied = pad;
        emptied.edited(2, String::new());
        assert_eq!(bar(&emptied, &none)[0].1, "Detect language");
        let mut pad = typed("hello");
        let r = pad.request(&ru_en(), &none).unwrap();
        pad.asking(r.clone());
        pad.answered(&r, Ok(translation("привет", "en")), &ru_en(), &none);
        // A picked target and source show as picked.
        let picked = Choice {
            from: Some("de".into()),
            to: Some("fr".into()),
        };
        let shown = bar(&typed("hallo"), &picked);
        assert_eq!(
            shown[0],
            ("from-lang".into(), "German".into(), Some("de".into()))
        );
        assert_eq!(
            shown[2],
            ("to-lang".into(), "French".into(), Some("fr".into()))
        );
        // An empty pad says what the pair does.
        assert_eq!(bar(&typed(""), &none)[2].1, "Automatic");
        let View::ScratchpadView(view) = pad.view(&ru_en(), &none).unwrap() else {
            panic!();
        };
        assert_eq!(view.toolbar[2].shortcut.as_deref(), Some("ctrl+p"));
        assert_eq!(view.toolbar[0].shortcut.as_deref(), Some("ctrl+shift+p"));
        assert_eq!(view.toolbar[1].shortcut.as_deref(), Some("ctrl+s"));
        assert!(view.toolbar[2].options.len() > 100);
    }

    /// Review finding 5: after a language change, while a retry is out, the answer from
    /// before the change still counts for nothing.
    #[test]
    fn the_old_answer_stays_stale_during_a_retry() {
        let from_en = Choice {
            from: Some("en".into()),
            to: None,
        };
        let none = Choice::default();
        // The source fixed to English: "привет" asked into Russian, answered.
        let mut pad = typed("привет");
        let r = pad.request(&ru_en(), &from_en).unwrap();
        assert_eq!(r.to, "ru");
        pad.asking(r.clone());
        pad.answered(&r, Ok(translation("привет", "en")), &ru_en(), &from_en);
        assert!(pad.current().is_some());
        // "Detect language" chosen: asked again into Russian (the last source was English)…
        pad.invalidate();
        let again = pad.request(&ru_en(), &none).unwrap();
        assert_eq!(again.to, "ru");
        pad.asking(again.clone());
        // …Google says Russian: asked once more, into English.
        let retry = pad.answered(&again, Ok(translation("привет", "ru")), &ru_en(), &none);
        assert_eq!(retry.map(|r| r.to).as_deref(), Some("en"));
        assert!(
            pad.current().is_none(),
            "the answer from before the change is stale"
        );
    }

    /// A language picked from the bar asks the same text again.
    #[test]
    fn a_new_choice_asks_again() {
        let none = Choice::default();
        let mut pad = typed("hello");
        let r = pad.request(&ru_en(), &none).unwrap();
        pad.asking(r.clone());
        pad.answered(&r, Ok(translation("привет", "en")), &ru_en(), &none);
        assert_eq!(pad.request(&ru_en(), &none), None, "answered");
        let german = Choice {
            from: None,
            to: Some("de".into()),
        };
        pad.invalidate();
        let again = pad.request(&ru_en(), &german).expect("asked again");
        assert_eq!(again.to, "de");
        assert!(
            pad.current().is_none(),
            "the Russian answer is not for German"
        );
    }
}
