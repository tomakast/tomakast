//! The plugin catalog built from the plugins folder alone, for when the plugin runtime is down.

use std::path::Path;

use tomakast_core::{
    CatalogCommand, CatalogPlugin, ParamTitle, PluginCatalog, PluginPreference, PluginState,
};

use crate::manager::{plugin_folder_names, sort_catalog, unreadable_entry};
use crate::manifest::Manifest;
use crate::package::Package;

/// The catalog entry of a plugin with what its manifest declares, in `state`.
pub fn catalog_entry(manifest: &Manifest, state: PluginState) -> CatalogPlugin {
    CatalogPlugin {
        id: manifest.id.clone(),
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        author: manifest.author.clone(),
        state,
        preferences: manifest
            .preferences
            .iter()
            .map(|p| PluginPreference {
                key: p.key.clone(),
                title: p.title.clone(),
                description: p.description.clone(),
                kind: p.value_kind(),
                default: p.default.clone(),
                reported_default: None,
            })
            .collect(),
        commands: manifest
            .commands
            .iter()
            .map(|c| CatalogCommand {
                id: c.id.clone(),
                title: c.title.clone(),
                icon: c.icon.clone(),
            })
            .collect(),
        widgets: manifest.widgets.iter().map(|w| w.decl()).collect(),
        param_titles: manifest
            .widgets
            .iter()
            .flat_map(|w| {
                w.params.iter().map(|p| ParamTitle {
                    widget: w.id.clone(),
                    key: p.key.clone(),
                    title: p.title.clone(),
                })
            })
            .collect(),
    }
}

/// Every plugin folder in `plugins_dir`, read for what its manifest declares and marked
/// `NotRunning(reason)`. A folder that cannot be read as a plugin is a `ManifestError` entry; a
/// missing plugins folder is an empty list; one that cannot be listed is `unavailable`.
pub fn list_catalog(plugins_dir: &Path, reason: &str) -> PluginCatalog {
    let names = match plugin_folder_names(plugins_dir) {
        Ok(names) => names,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            return PluginCatalog {
                plugins: Vec::new(),
                unavailable: Some(format!("cannot list {}: {e}", plugins_dir.display())),
            };
        }
    };
    let mut plugins = Vec::new();
    for name in names {
        let dir = plugins_dir.join(&name);
        // A stray file (a README) is no plugin.
        if std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_file()) {
            continue;
        }
        plugins.push(match Package::read(&dir) {
            Ok(package) => catalog_entry(
                &package.manifest,
                PluginState::NotRunning(reason.to_owned()),
            ),
            Err(e) => unreadable_entry(&name, &format!("cannot read the plugin: {e}")),
        });
    }
    sort_catalog(&mut plugins);
    PluginCatalog {
        plugins,
        unavailable: None,
    }
}
