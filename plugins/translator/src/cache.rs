//! The last answers, by request, so swapping back, a tile and the pad cost no request.

use crate::google::{Request, Translation};

/// How many answers are kept.
pub const SIZE: usize = 64;

/// Least recently used first.
#[derive(Default)]
pub struct Cache {
    entries: Vec<(Request, Translation)>,
}

impl Cache {
    /// The answer for `r`, now the most recently used.
    pub fn get(&mut self, r: &Request) -> Option<&Translation> {
        let at = self.entries.iter().position(|(k, _)| k == r)?;
        let entry = self.entries.remove(at);
        self.entries.push(entry);
        self.entries.last().map(|(_, t)| t)
    }

    /// The answer for `r`, without touching the order (for drawing).
    pub fn peek(&self, r: &Request) -> Option<&Translation> {
        self.entries.iter().find(|(k, _)| k == r).map(|(_, t)| t)
    }

    /// Whether `r` has an answer, without touching the order.
    pub fn has(&self, r: &Request) -> bool {
        self.entries.iter().any(|(k, _)| k == r)
    }

    /// Keeps `t` for `r`; the least recently used goes past [`SIZE`].
    pub fn put(&mut self, r: Request, t: Translation) {
        self.entries.retain(|(k, _)| *k != r);
        self.entries.push((r, t));
        if self.entries.len() > SIZE {
            self.entries.remove(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(n: usize) -> Request {
        Request {
            text: n.to_string(),
            from: None,
            to: "ru".into(),
        }
    }

    fn answer(n: usize) -> Translation {
        Translation {
            text: format!("t{n}"),
            ..Translation::default()
        }
    }

    #[test]
    fn the_least_recently_used_goes() {
        let mut cache = Cache::default();
        for n in 0..SIZE {
            cache.put(req(n), answer(n));
        }
        assert_eq!(cache.get(&req(0)).unwrap().text, "t0");
        cache.put(req(SIZE), answer(SIZE));
        assert!(cache.has(&req(0)), "used last, kept");
        assert!(!cache.has(&req(1)), "least recently used, gone");
        cache.put(req(5), answer(55));
        assert_eq!(cache.get(&req(5)).unwrap().text, "t55");
    }
}
