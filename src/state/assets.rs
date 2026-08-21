//! Image attachments: resolving a well-relative asset id to a renderable URL
//! for `markdown::render`, backed by the `assets` / `assets_pending` cache.
//! `impl State` block — see [`super`].

use std::collections::{HashMap, HashSet};

use leptos::task::spawn_local;

use super::*;

impl State {
    /// Resolve a well-relative asset id (`assets/foo.png`, as embedded by
    /// `![](…)`) to a renderable `data:` URL, for `markdown::render`'s image
    /// substitution.
    ///
    /// Returns `None` the first time an id is seen (the `<img>` shows a broken
    /// icon until the resolved URL lands and this note re-renders); the actual
    /// read is kicked off here as a side effect and cached in `self.assets`, so
    /// every other reference to the same id resolves synchronously from then on.
    pub fn resolve_asset(self, id: &str) -> Option<String> {
        if let Some(url) = self.assets.get().get(id) {
            return Some(url.clone());
        }
        let well = self.well.get_untracked()?;
        if self.assets_pending.get_untracked().contains(id) {
            return None;
        }
        let id = id.to_string();
        self.assets_pending.update(|p| {
            p.insert(id.clone());
        });
        spawn_local(async move {
            if let Some(url) = ipc::read_asset(well.path, id.clone()).await {
                self.assets.update(|m| {
                    m.insert(id.clone(), url);
                });
            }
            self.assets_pending.update(|p| {
                p.remove(&id);
            });
        });
        None
    }

    /// Clear the asset cache — called on well switch so a previous well's
    /// resolved ids (well-relative paths, not globally unique) never leak into
    /// the next one's renders.
    pub(super) fn clear_assets(self) {
        self.assets.set(HashMap::new());
        self.assets_pending.set(HashSet::new());
    }
}
