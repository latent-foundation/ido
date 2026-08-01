//! Wiki section: opening pages, folder + page CRUD over the wiki tree, inline
//! rename, drag-to-move, and routing link clicks. `impl State` block — see
//! [`super`].
//!
//! A page's identity is its **slug** (globally unique); folders are purely
//! organisational. So page tabs/links key on the slug and survive any folder
//! move unchanged — only a *rename* (which changes the slug) re-points tabs and
//! rewrites links. Folder ops are path-based, mirroring the notes tree.

use leptos::prelude::*;
use leptos::task::spawn_local;

use super::*;

use crate::ipc;

/// The slug of a wiki page node — the last segment of its wiki-relative path.
fn slug_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

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
    /// unresolved `[[wikilink]]` target), create it (at the wiki root) first,
    /// then refresh the tree.
    pub fn open_wiki(self, slug: String) {
        if !self.wiki.get_untracked().iter().any(|s| s == &slug) {
            if let Some(w) = self.well.get_untracked() {
                let slug = slug.clone();
                spawn_local(async move {
                    ipc::ensure_page(w.path.clone(), slug).await;
                    self.set_wiki(ipc::list_wiki(w.path).await);
                });
            }
        }
        self.open_tab(TabTarget::WikiPage(slug));
    }

    /// Create a new page inside `folder` (`""` = wiki root), open it, and start
    /// an inline rename in the tree.
    pub fn add_page(self, folder: String) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            if let Some(slug) = ipc::create_page(w.path.clone(), folder.clone()).await {
                self.expand_wiki_parent(&folder);
                self.set_wiki(ipc::list_wiki(w.path).await);
                self.open_tab(TabTarget::WikiPage(slug.clone()));
                let path = if folder.is_empty() {
                    slug
                } else {
                    format!("{folder}/{slug}")
                };
                self.wiki_renaming.set(Some((path, false)));
            }
        });
    }

    /// Create an organisational folder in `parent` and start an inline rename.
    pub fn add_wiki_folder(self, parent: String) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            if let Some(id) = ipc::create_wiki_folder(w.path.clone(), parent.clone()).await {
                self.expand_wiki_parent(&parent);
                self.set_wiki(ipc::list_wiki(w.path).await);
                self.wiki_renaming.set(Some((id, true)));
            }
        });
    }

    /// Commit an inline wiki rename (no-op if it isn't the one renaming, or the
    /// name is blank). A folder renames purely on disk; a page renames by its
    /// slug — the backend rewrites inbound `[[links]]` across every section and
    /// re-points open tabs.
    pub fn commit_wiki_rename(self, path: String, is_dir: bool, name: String) {
        if self.wiki_renaming.get_untracked() != Some((path.clone(), is_dir)) {
            return;
        }
        self.wiki_renaming.set(None);
        if name.trim().is_empty() {
            return;
        }
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        if is_dir {
            spawn_local(async move {
                match ipc::rename_wiki_folder(w.path.clone(), path, name).await {
                    Ok(_) => self.set_wiki(ipc::list_wiki(w.path).await),
                    Err(e) => self.show_toast(e, None),
                }
            });
        } else {
            let slug = slug_of(&path).to_string();
            spawn_local(async move {
                match ipc::rename_page(w.path.clone(), slug.clone(), name).await {
                    Ok(new_slug) => {
                        self.sync_page_id(&slug, &new_slug);
                        self.set_wiki(ipc::list_wiki(w.path).await);
                        // Inbound `[[links]]` were rewritten on disk — refresh each
                        // pane's open buffer (and its backlinks) so it shows at once.
                        self.reload_panes();
                    }
                    Err(e) => self.show_toast(e, None),
                }
            });
        }
    }

    /// Delete a wiki page (with an undo toast), or an *empty* folder. Closes any
    /// tab showing a deleted page.
    pub fn remove_wiki_entry(self, path: String, is_dir: bool) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        if is_dir {
            let name = slug_of(&path).to_string();
            spawn_local(async move {
                if ipc::delete_wiki_folder(w.path.clone(), path).await {
                    self.set_wiki(ipc::list_wiki(w.path).await);
                    self.show_toast(format!("deleted {name}"), None);
                }
            });
            return;
        }
        let slug = slug_of(&path).to_string();
        spawn_local(async move {
            // Capture the body first so the delete can be undone.
            let content = ipc::read_page(w.path.clone(), slug.clone()).await;
            if ipc::delete_page(w.path.clone(), slug.clone()).await {
                self.close_page_tabs(&slug);
                self.set_wiki(ipc::list_wiki(w.path).await);
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

    /// Move the currently-dragged wiki entry into `dest` (`""` = wiki root). A
    /// page keeps its slug, so open tabs and links need no re-pointing.
    pub fn move_wiki_into(self, dest: String) {
        let Some((path, is_dir)) = self.wiki_dragging.get_untracked() else {
            return;
        };
        self.wiki_dragging.set(None);
        if parent_of(&path) == dest {
            return;
        }
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            if ipc::move_wiki_entry(w.path.clone(), path, is_dir, dest)
                .await
                .is_some()
            {
                self.set_wiki(ipc::list_wiki(w.path).await);
            }
        });
    }

    /// Expand or collapse a wiki folder.
    pub fn toggle_wiki_expand(self, path: String) {
        self.wiki_expanded.update(|set| {
            if !set.remove(&path) {
                set.insert(path);
            }
        });
    }

    /// Keep a non-root `parent` folder expanded so a freshly-created child shows.
    fn expand_wiki_parent(self, parent: &str) {
        if !parent.is_empty() {
            self.wiki_expanded.update(|set| {
                set.insert(parent.to_string());
            });
        }
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
