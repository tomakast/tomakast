//! Thin wrappers over the contract's imports. Each needs its permission in `plugin.toml`
//! (`preferences`, `log` and `locale` need none); without it, a call fails.

use crate::bindings::tomakast::plugin as wit;

/// The plugin's own key-value store; needs `storage = true`.
///
/// Quota: 1 MiB in total, counted as the bytes of every key and value plus 32 bytes per
/// entry; keys at most 256 bytes, values at most 256 KiB. At most 20 writes (`set` and
/// `remove` together, failed ones included) per second; the writes beyond fail. Keep data in
/// memory and write it when it changes, not on a timer.
pub mod storage {
    use super::wit::storage as imp;

    /// The value of `key`, or `None` when there is none.
    pub fn get(key: &str) -> Result<Option<String>, String> {
        imp::get(key)
    }

    /// Stores `value` under `key`. Fails when the quota or the write rate would be exceeded;
    /// a quota failure's error starts with `quota: ` (no other error starts so).
    pub fn set(key: &str, value: &str) -> Result<(), String> {
        imp::set(key, value)
    }

    /// Removes `key` (removing a missing key succeeds). Fails when the write rate is
    /// exceeded.
    pub fn remove(key: &str) -> Result<(), String> {
        imp::remove(key)
    }
}

/// The plugin's preferences, declared under `[[preferences]]` in `plugin.toml` and set by the
/// user. A change arrives as [`Event::PreferencesChanged`](crate::Event::PreferencesChanged).
pub mod preferences {
    /// The user's value of `key`, else the manifest default, else `None`; an undeclared key
    /// is `None`. A `bool` preference is the text `"true"` or `"false"`.
    pub fn get(key: &str) -> Option<String> {
        super::wit::preferences::get(key)
    }
}

/// Lines in the launcher's log; needs no permission. Messages are truncated to 4 KiB, and at
/// most 100 lines per second are kept.
pub mod log {
    use super::wit::log::{Level, log};

    /// Logs `message` at debug level.
    pub fn debug(message: &str) {
        log(Level::Debug, message);
    }

    /// Logs `message` at info level.
    pub fn info(message: &str) {
        log(Level::Info, message);
    }

    /// Logs `message` at warn level.
    pub fn warn(message: &str) {
        log(Level::Warn, message);
    }

    /// Logs `message` at error level.
    pub fn error(message: &str) {
        log(Level::Error, message);
    }
}

/// Timers; need `timers = true`. At most 16 active timers per plugin; a repeating timer's
/// interval is at least 250 ms, a one-shot delay at least 10 ms. A firing arrives as
/// [`Event::TimerFired`](crate::Event::TimerFired) with the id `start` returned.
///
/// Timers end with the plugin instance (an unload, a reload, a stop): start them again in
/// [`Plugin::init`](crate::Plugin::init). The launcher may delay, coalesce or suspend timers,
/// especially while its window is hidden.
pub mod timers {
    use crate::TimerId;

    /// Starts a timer that fires after `millis` at the earliest, once or (with `repeat`)
    /// until cancelled. Fails when the quota is exceeded or the interval is too short.
    pub fn start(millis: u64, repeat: bool) -> Result<TimerId, String> {
        super::wit::timers::start(millis, repeat)
    }

    /// Cancels `timer`; cancelling an unknown or finished timer does nothing.
    pub fn cancel(timer: TimerId) {
        super::wit::timers::cancel(timer);
    }
}

/// Web requests to the domains listed under `[permissions] http` in `plugin.toml`.
///
/// `fetch` returns at once; the answer arrives as
/// [`Event::HttpDone`](crate::Event::HttpDone) with the id it returned. At most 4 requests at
/// a time, a 4 MiB response body and 30 s per request. A status other than 2xx is an answer,
/// not an error. The other rules (redirects, headers the launcher sets, local addresses) are
/// on the `http` interface of `plugin.wit`.
pub mod http {
    use crate::{HttpRequest, RequestId};

    /// Starts `request`. Fails when its domain is not allowed, it is not valid, or 4
    /// requests are already running.
    pub fn fetch(request: &HttpRequest) -> Result<RequestId, String> {
        super::wit::http::fetch(request)
    }
}

/// The computer's approximate location; needs `location = true`. The answer arrives later as
/// [`Event::Location`](crate::Event::Location): the coordinates rounded to 0.1°, with
/// `accuracy_km` (never below 11; 500 for a time zone's place), or why there are none. The
/// launcher takes it from the user's city, the OS service, Wi-Fi (when the user turned it on)
/// or the time zone. A request while one waits is answered by the same event, and the
/// launcher keeps its answer for 10 minutes (a coarser one than 50 km, or a failure, for 1),
/// so asking again is cheap. When the user changes the location settings, the event comes
/// unasked. The other rules are on the `location` interface of `plugin.wit`.
pub mod location {
    /// Asks where the computer is. Fails at once without the permission.
    pub fn request() -> Result<(), String> {
        super::wit::location::request()
    }
}

/// Opens links and files with the user's default application; needs `open = true` and a user
/// action: only during [`Plugin::run`](crate::Plugin::run), or
/// [`Plugin::on_event`](crate::Plugin::on_event) for an `action-chosen` or `form-submitted`
/// view event. One call opens at most once: after one success, both functions fail until the
/// call returns.
pub mod open {
    /// Opens an `http`, `https` or `mailto` URL (no spaces, control characters, user name or
    /// password).
    pub fn url(url: &str) -> Result<(), String> {
        super::wit::open::open_url(url)
    }

    /// Opens an existing file or folder by its absolute path. Executables and scripts are
    /// refused, with the error `this path may not be opened`.
    pub fn path(path: &str) -> Result<(), String> {
        super::wit::open::open_path(path)
    }
}

/// The user's locale; needs no permission.
pub mod locale {
    /// The IANA name of the time zone the computer is set to (`Europe/Moscow`), read when
    /// called, so it follows a change; `UTC` when the launcher cannot tell. At most 64 bytes of
    /// `A-Z a-z 0-9 / _ + -`.
    pub fn local_time_zone() -> String {
        super::wit::locale::local_time_zone()
    }
}
