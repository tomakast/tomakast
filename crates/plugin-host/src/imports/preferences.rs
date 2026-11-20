//! `preferences`: the values the user set in `[plugins.<id>]` of `config.toml`, falling back
//! to the defaults in `plugin.toml`. Only declared preferences have values.

use std::collections::BTreeMap;

use super::HostState;
use crate::bindings::tomakast::plugin::preferences::Host;
use crate::manifest::PreferenceDecl;

/// Every declared preference that has a value: the user's, else the manifest default.
pub(crate) fn effective(
    decls: &[PreferenceDecl],
    mut user: BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    decls
        .iter()
        .filter_map(|decl| {
            let value = user.remove(&decl.key).or_else(|| decl.default.clone())?;
            Some((decl.key.clone(), value))
        })
        .collect()
}

impl Host for HostState {
    fn get(&mut self, key: String) -> Option<String> {
        self.record_import("preferences.get");
        self.prefs.get(&key).cloned()
    }
}
