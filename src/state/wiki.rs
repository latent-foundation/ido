//! Wiki section: opening/creating/renaming/deleting pages, and routing link
//! clicks. `impl State` block — see [`super`].

use leptos::prelude::*;
use leptos::task::spawn_local;

use super::*;

use crate::ipc;

impl State {
    /// Open a wiki page in the reusable **preview** tab (sidebar single-click).
    pub fn preview_wiki(self, slug: String) {
        self.open_preview(TabTarget::WikiPage(slug));
    }

    /// Route a clicked link: internal `[[wikilinks]]` open the page in a tab;
    /// everything else opens in the OS browser. Never navigates the webview.
    pub fn open_link(self, url: String) {
        if let Some(slug) = url.strip_prefix(crate::markdown::WIKI_HREF) {
            self.open_wiki(slug.to_string());
        } else {
            spawn_local(async move { ipc::open_external(url).await });
        }
    }

    // --- wiki -------------------------------------------------------------

    /// Open wiki page `slug` in a tab. If the page doesn't exist yet (an
    /// unresolved `[[wikilink]]` target), create it first, then refresh the list.
    pub fn open_wiki(self, slug: String) {
        if !self.wiki.get_untracked().iter().any(|s| s == &slug) {
            if let Some(w) = self.well.get_untracked() {
                let slug = slug.clone();
                spawn_local(async move {
                    ipc::ensure_page(w.path.clone(), slug).await;
                    self.wiki.set(ipc::list_wiki(w.path).await);
                });
            }
        }
        self.open_tab(TabTarget::WikiPage(slug));
    }

    /// Create a new wiki page, open it, and start an inline rename in the list.
    pub fn add_page(self) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            if let Some(slug) = ipc::create_page(w.path.clone()).await {
                self.wiki.set(ipc::list_wiki(w.path).await);
                self.open_tab(TabTarget::WikiPage(slug.clone()));
                self.page_renaming.set(Some(slug));
            }
        });
    }

    /// Commit an inline wiki-page rename (no-op if it isn't the one renaming, or
    /// the name is blank). The backend renames the file and rewrites inbound
    /// `[[links]]` across every section.
    pub fn commit_page_rename(self, slug: String, name: String) {
        if self.page_renaming.get_untracked().as_deref() != Some(slug.as_str()) {
            return;
        }
        self.page_renaming.set(None);
        if name.trim().is_empty() {
            return;
        }
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            match ipc::rename_page(w.path.clone(), slug.clone(), name).await {
                Ok(new_slug) => {
                    self.sync_page_id(&slug, &new_slug);
                    self.wiki.set(ipc::list_wiki(w.path).await);
                    // Inbound `[[links]]` were rewritten on disk — refresh each
                    // pane's open buffer (and its backlinks) so it shows at once.
                    self.reload_panes();
                }
                Err(e) => self.show_toast(e, None),
            }
        });
    }

    /// Delete wiki page `slug`; close any tab showing it.
    pub fn remove_page(self, slug: String) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            // Capture the body first so the delete can be undone.
            let content = ipc::read_page(w.path.clone(), slug.clone()).await;
            if ipc::delete_page(w.path.clone(), slug.clone()).await {
                self.close_page_tabs(&slug);
                self.wiki.set(ipc::list_wiki(w.path).await);
                self.show_toast(
                    format!("deleted {slug}"),
                    Some(UndoEntry {
                        kind: EntryKind::WikiPage,
                        id: slug,
                        content,
                    }),
                );
            }
        });
    }

    /// After a wiki rename from `old` slug to `new`, re-point matching page tabs
    /// across both panes.
    fn sync_page_id(self, old: &str, new: &str) {
        for p in self.panes {
            p.tabs.update(|tabs| {
                for tab in tabs.iter_mut() {
                    if let Some(slug) = tab.target.page_slug_mut() {
                        if slug == old {
                            *slug = new.to_string();
                        }
                    }
                }
            });
        }
        self.persist_session();
    }

    /// Close every tab showing wiki page `slug`.
    fn close_page_tabs(self, slug: &str) {
        self.close_tabs(|t| t.page_slug() == Some(slug));
    }
}
