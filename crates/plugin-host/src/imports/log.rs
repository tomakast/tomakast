//! `log`: plugin messages into `tracing` with target `plugin`, truncated to 4 KiB and at most
//! 100 lines per second.

use std::time::{Duration, Instant};

use super::HostState;
use crate::bindings::tomakast::plugin::log::{Host, Level};

/// Longest kept message, in bytes (escapes included).
pub const MAX_LOG_BYTES: usize = 4 * 1024;
/// Lines kept per [`LOG_WINDOW`].
pub const MAX_LOG_LINES: u32 = 100;
const LOG_WINDOW: Duration = Duration::from_secs(1);

/// Counts lines per one-second window.
#[derive(Default)]
pub(crate) struct LogLimiter {
    window: Option<Instant>,
    lines: u32,
    dropped: u64,
}

impl LogLimiter {
    /// Whether a line at `now` is kept, and how many lines were dropped in the window before
    /// it (reported once, with the first line of the next window).
    fn admit(&mut self, now: Instant) -> (bool, u64) {
        let expired = self
            .window
            .is_none_or(|start| now.duration_since(start) >= LOG_WINDOW);
        let mut dropped = 0;
        if expired {
            self.window = Some(now);
            self.lines = 0;
            dropped = std::mem::take(&mut self.dropped);
        }
        if self.lines < MAX_LOG_LINES {
            self.lines += 1;
            (true, dropped)
        } else {
            self.dropped += 1;
            (false, dropped)
        }
    }
}

/// `message` cut to [`MAX_LOG_BYTES`] on a character boundary, with control characters
/// escaped so that a plugin cannot forge log lines.
pub(crate) fn clean(message: &str) -> String {
    sanitize(message, MAX_LOG_BYTES)
}

/// Plugin-controlled text made safe to log or show: control characters escaped, so it stays
/// one line, and the result cut on a character boundary to at most `max` bytes plus the `…`
/// that marks the cut. Escapes count towards `max`.
pub(crate) fn sanitize(message: &str, max: usize) -> String {
    let mut out = String::with_capacity(message.len().min(max));
    for c in message.chars() {
        let start = out.len();
        if c.is_control() {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
        if out.len() > max {
            out.truncate(start);
            out.push('…');
            break;
        }
    }
    out
}

impl Host for HostState {
    fn log(&mut self, level: Level, message: String) {
        self.record_import("log.log");
        let (keep, dropped) = self.log.admit(Instant::now());
        let plugin = self.plugin.as_str();
        if dropped > 0 {
            tracing::warn!(target: "plugin", plugin, "dropped {dropped} log lines (over {MAX_LOG_LINES} per s)");
        }
        if !keep {
            return;
        }
        let message = clean(&message);
        match level {
            Level::Debug => tracing::debug!(target: "plugin", plugin, "{message}"),
            Level::Info => tracing::info!(target: "plugin", plugin, "{message}"),
            Level::Warn => tracing::warn!(target: "plugin", plugin, "{message}"),
            Level::Error => tracing::error!(target: "plugin", plugin, "{message}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_messages_are_cut_on_a_char_boundary() {
        let ascii = "a".repeat(MAX_LOG_BYTES + 10);
        let cleaned = clean(&ascii);
        assert_eq!(cleaned.len(), MAX_LOG_BYTES + '…'.len_utf8());
        // A two-byte character straddling the limit is dropped whole, without a panic.
        let straddling = format!("{}é tail", "a".repeat(MAX_LOG_BYTES - 1));
        let cleaned = clean(&straddling);
        assert_eq!(cleaned, format!("{}…", "a".repeat(MAX_LOG_BYTES - 1)));
        assert_eq!(clean("short"), "short");
    }

    #[test]
    fn escapes_count_towards_the_limit() {
        let lines = "\n".repeat(MAX_LOG_BYTES);
        let cleaned = clean(&lines);
        assert!(
            cleaned.len() <= MAX_LOG_BYTES + '…'.len_utf8(),
            "{}",
            cleaned.len()
        );
        assert!(cleaned.starts_with("\\n\\n") && cleaned.ends_with('…'));
    }

    #[test]
    fn control_characters_are_escaped() {
        assert_eq!(clean("one\ntwo\r\u{1b}[31m"), "one\\ntwo\\r\\u{1b}[31m");
    }

    #[test]
    fn at_most_100_lines_per_second() {
        let mut limiter = LogLimiter::default();
        let start = Instant::now();
        let kept = (0..150)
            .filter(|i| limiter.admit(start + Duration::from_millis(*i)).0)
            .count();
        assert_eq!(kept, 100);
        // The next window keeps lines again and reports the 50 dropped ones once.
        let next = start + LOG_WINDOW + Duration::from_millis(1);
        assert_eq!(limiter.admit(next), (true, 50));
        assert_eq!(limiter.admit(next), (true, 0));
    }
}
