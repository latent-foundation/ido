//! The wiki section: linked markdown pages under `wiki/`, optionally organised
//! into folders.
//!
//! A page's id is its **slug** (its file stem, no `.md`) and is **globally
//! unique** across the whole section, regardless of which folder the file sits
//! in — folders are *purely organisational*. Because identity is the slug (never
//! the path), the link graph is folder-agnostic: pages link with `[[slug]]` /
//! `[[slug|label]]`, [`backlinks`] answers "what links here" by scanning the
//! wiki, notes, **and** task/goal bodies, and a rename rewrites inbound links
//! across all of them — none of which cares where the file lives. Moving a page
//! between folders keeps its slug, so no link ever breaks.
//!
//! Folder create/rename/delete/move are path-based and mirror the notes store
//! (they reuse [`crate::notes::build_tree`] / [`crate::notes::same_entry`]); the
//! page operations stay slug-based, resolving a slug to its file anywhere in the
//! tree via [`find_page`].

use std::fs;
use std::path::{Path, PathBuf};

use crate::model::{LinkRef, Section, TreeNode};
use crate::notes::{build_tree, same_entry};
use crate::paths::{
    append_text, join_rel, parent_of, rel_path, section_dir, slugify, unique_name, valid_name,
};

/// The absolute `wiki/` folder for `well`.
fn wiki_root(well: &str) -> PathBuf {
    section_dir(well, Section::Wiki)
}

/// Find the file backing page `slug` anywhere under `root` (slugs are globally
/// unique, so there is at most one). Recurses folders; skips hidden entries.
fn find_page(root: &Path, slug: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            if let Some(found) = find_page(&path, slug) {
                return Some(found);
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("md")
            && path.file_stem().and_then(|s| s.to_str()) == Some(slug)
        {
            return Some(path);
        }
    }
    None
}

/// A slug like `stem`, `stem-2`, … not used by any page anywhere in the tree
/// (uniqueness is section-wide, not per-folder).
fn unique_slug(root: &Path, stem: &str) -> String {
    if find_page(root, stem).is_none() {
        return stem.to_string();
    }
    let mut n = 2;
    loop {
        let candidate = format!("{stem}-{n}");
        if find_page(root, &candidate).is_none() {
            return candidate;
        }
        n += 1;
    }
}

/// Every slug in `[[…]]` references within `content`, slugified (so `[[Auth
/// System]]` and `[[auth-system]]` both resolve to `auth-system`). The label
/// after a `|` is ignored. Used by [`backlinks`].
fn wikilink_targets(content: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut rest = content;
    while let Some(i) = rest.find("[[") {
        let after = &rest[i + 2..];
        let Some(j) = after.find("]]") else {
            break;
        };
        let inner = &after[..j];
        if !inner.contains('[') && !inner.contains('\n') {
            let target = inner.split_once('|').map(|(s, _)| s).unwrap_or(inner);
            let slug = slugify(target);
            if !slug.is_empty() {
                targets.push(slug);
            }
        }
        rest = &after[j + 2..];
    }
    targets
}

/// Rewrite every `[[link]]` in `content` whose target resolves to `old` so it
/// points at `new`, preserving any explicit `|label`. Returns the updated text
/// only if something changed (so unaffected pages aren't rewritten).
///
/// `[[old]]` → `[[new]]`, `[[old|alias]]` → `[[new|alias]]`, and
/// `[[Old Title]]` (which slugs to `old`) → `[[new]]`.
fn rewrite_links(content: &str, old: &str, new: &str) -> Option<String> {
    if !content.contains("[[") {
        return None;
    }
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    let mut changed = false;
    while let Some(i) = rest.find("[[") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let Some(j) = after.find("]]") else {
            // Unterminated — keep the literal "[[" and move past it.
            out.push_str("[[");
            rest = after;
            continue;
        };
        let inner = &after[..j];
        let (target, label) = match inner.split_once('|') {
            Some((t, l)) => (t, Some(l)),
            None => (inner, None),
        };
        if !inner.contains('[') && !inner.contains('\n') && slugify(target.trim()) == old {
            out.push_str("[[");
            out.push_str(new);
            if let Some(label) = label {
                out.push('|');
                out.push_str(label);
            }
            out.push_str("]]");
            changed = true;
        } else {
            out.push_str("[[");
            out.push_str(inner);
            out.push_str("]]");
        }
        rest = &after[j + 2..];
    }
    out.push_str(rest);
    changed.then_some(out)
}

/// Rewrite inbound `[[links]]` in every `.md` under `dir` after a slug change
/// (recursing into subfolders when `recurse`). Best-effort: unreadable /
/// unwritable files are skipped.
fn rewrite_dir_links(dir: &Path, old: &str, new: &str, recurse: bool) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if recurse && path.is_dir() {
            rewrite_dir_links(&path, old, new, true);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md")
            && let Ok(content) = fs::read_to_string(&path)
            && let Some(updated) = rewrite_links(&content, old, new)
        {
            let _ = fs::write(&path, updated);
        }
    }
}

/// The wiki's tree of folders and pages (folders first, then pages, each
/// alphabetical). A page node's `name` is its slug; folder nodes carry their
/// wiki-relative path. Folders are display-only — a page's slug is its identity
/// wherever the file lives.
pub fn list_wiki(well: String) -> Vec<TreeNode> {
    let root = wiki_root(&well);
    build_tree(&root, &root)
}

/// A wiki page's markdown body — empty when the page doesn't exist yet (so a
/// freshly-clicked `[[wikilink]]` opens a blank, editable page).
pub fn read_page(well: String, slug: String) -> String {
    find_page(&wiki_root(&well), &slug)
        .and_then(|p| fs::read_to_string(p).ok())
        .unwrap_or_default()
}

/// Write a wiki page's body. An existing page is written where it lives (in
/// whatever folder); a brand-new slug lands at the wiki root.
pub fn write_page(well: String, slug: String, content: String) -> Result<(), String> {
    let root = wiki_root(&well);
    let path = find_page(&root, &slug).unwrap_or_else(|| root.join(format!("{slug}.md")));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(path, content).map_err(|e| e.to_string())
}

/// Create a uniquely-named empty page (`untitled`, `untitled-2`, …) inside
/// `folder` (`""` = the wiki root). The slug is unique **section-wide**, not just
/// within the folder. Returns its slug.
pub fn create_page(well: String, folder: String) -> Result<String, String> {
    let root = wiki_root(&well);
    let dir = if folder.is_empty() {
        root.clone()
    } else {
        root.join(&folder)
    };
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let slug = unique_slug(&root, "untitled");
    fs::write(dir.join(format!("{slug}.md")), "").map_err(|e| e.to_string())?;
    Ok(slug)
}

/// Create a wiki page at `slug` with `content`, never overwriting an
/// existing page: where [`write_page`] always overwrites (fine for the app,
/// where the user is looking at the page they're editing), this is the
/// MCP-facing create-or-uniquify half (see `docs/mcp-server.md` §8). A taken
/// slug uniquifies with `-N` via [`unique_slug`] — the same section-wide
/// scheme [`create_page`] uses. `slug` must be a single path segment (no
/// `/`; see [`valid_name`]) and, since a wiki slug never carries folders,
/// always lands at the wiki **root** regardless of where a colliding slug
/// happens to live. Returns the slug actually written.
pub fn create_page_at(well: String, slug: String, content: String) -> Result<String, String> {
    let slug = valid_name(&slug)?;
    let root = wiki_root(&well);
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let final_slug = unique_slug(&root, slug);
    fs::write(root.join(format!("{final_slug}.md")), content).map_err(|e| e.to_string())?;
    Ok(final_slug)
}

/// Append `text` to page `slug`, creating it (at the wiki root) when it
/// doesn't exist yet. An existing page is found wherever it lives — folders
/// are organisational only, see [`find_page`] — and appended in place; see
/// [`crate::paths::append_text`] for the exact separator rule. Returns
/// `slug` unchanged (identity is the slug, never the path). `slug` must be a
/// single path segment (no `/`; see [`valid_name`]).
pub fn append_page(well: String, slug: String, text: String) -> Result<String, String> {
    valid_name(&slug)?;
    let root = wiki_root(&well);
    let path = find_page(&root, &slug).unwrap_or_else(|| root.join(format!("{slug}.md")));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let existing = fs::read_to_string(&path).unwrap_or_default();
    fs::write(&path, append_text(&existing, &text)).map_err(|e| e.to_string())?;
    Ok(slug)
}

/// Create `slug` as an empty page (at the wiki root) if it doesn't exist anywhere
/// — backs "create on click" for an unresolved `[[wikilink]]`. A no-op when a
/// page with that slug already exists (in any folder).
pub fn ensure_page(well: String, slug: String) -> Result<(), String> {
    let root = wiki_root(&well);
    if find_page(&root, &slug).is_none() {
        fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        fs::write(root.join(format!("{slug}.md")), "").map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Rename page `slug` to the slug of `name` (kept in its current folder), and
/// rewrite every inbound `[[link]]` to follow it — across the wiki, notes, **and**
/// task/goal bodies. The new slug must be free section-wide. Returns the new slug.
pub fn rename_page(well: String, slug: String, name: String) -> Result<String, String> {
    let new_slug = slugify(&name);
    if new_slug.is_empty() {
        return Err("name is empty".into());
    }
    if new_slug == slug {
        return Ok(slug);
    }
    let root = wiki_root(&well);
    let old = find_page(&root, &slug).ok_or("page not found")?;
    if find_page(&root, &new_slug).is_some() {
        return Err("name already taken".into());
    }
    // Keep the file in its folder — only the stem (slug) changes.
    let new = old.with_file_name(format!("{new_slug}.md"));
    fs::rename(&old, &new).map_err(|e| e.to_string())?;
    rewrite_dir_links(&root, &slug, &new_slug, true);
    rewrite_dir_links(&section_dir(&well, Section::Notes), &slug, &new_slug, true);
    // Tasks, recursing into tasks/goals/ (frontmatter never holds `[[`, so only
    // bodies are touched).
    rewrite_dir_links(&section_dir(&well, Section::Tasks), &slug, &new_slug, true);
    Ok(new_slug)
}

/// Delete wiki page `slug` (wherever it lives).
pub fn delete_page(well: String, slug: String) -> Result<(), String> {
    let path = find_page(&wiki_root(&well), &slug).ok_or("page not found")?;
    fs::remove_file(path).map_err(|e| e.to_string())
}

/// Create a uniquely-named organisational folder in `parent` (`""` = the wiki
/// root). Returns its wiki-relative id. Folders hold nothing but pages/subfolders
/// and never affect a page's slug or its links.
pub fn create_wiki_folder(well: String, parent: String) -> Result<String, String> {
    let root = wiki_root(&well);
    let base = if parent.is_empty() {
        root
    } else {
        root.join(&parent)
    };
    let name = unique_name(&base, "new folder", "");
    fs::create_dir_all(base.join(&name)).map_err(|e| e.to_string())?;
    Ok(join_rel(&parent, &name))
}

/// Rename a wiki folder in place (parent unchanged). Purely organisational — no
/// page slug or link is touched. Returns the new id.
pub fn rename_wiki_folder(well: String, path: String, name: String) -> Result<String, String> {
    let name = valid_name(&name)?;
    let new_id = join_rel(&parent_of(&path), name);
    if new_id == path {
        return Ok(path);
    }
    let root = wiki_root(&well);
    let (old, new) = (root.join(&path), root.join(&new_id));
    if new.exists() && !same_entry(&old, &new) {
        return Err("name already taken".into());
    }
    fs::rename(old, new).map_err(|e| e.to_string())?;
    Ok(new_id)
}

/// Delete an *empty* wiki folder. Non-empty folders are refused so pages are
/// never destroyed implicitly (delete the pages first).
pub fn delete_wiki_folder(well: String, path: String) -> Result<(), String> {
    let dir = wiki_root(&well).join(&path);
    if fs::read_dir(&dir)
        .map(|mut e| e.next().is_some())
        .unwrap_or(false)
    {
        return Err("folder isn't empty".into());
    }
    fs::remove_dir(dir).map_err(|e| e.to_string())
}

/// Move a page or folder into the `dest` folder (`""` = the wiki root). Returns
/// the new wiki-relative id. A page keeps its slug (only the file relocates), so
/// its links never change; a folder can't move into itself or a descendant.
pub fn move_wiki_entry(
    well: String,
    path: String,
    is_dir: bool,
    dest: String,
) -> Result<String, String> {
    let base = path.rsplit('/').next().unwrap_or(&path);
    let new_id = join_rel(&dest, base);
    if new_id == path {
        return Ok(path);
    }
    if is_dir && (dest == path || dest.starts_with(&format!("{path}/"))) {
        return Err("can't move a folder into itself".into());
    }
    let root = wiki_root(&well);
    let (old, new) = if is_dir {
        (root.join(&path), root.join(&new_id))
    } else {
        (
            root.join(format!("{path}.md")),
            root.join(format!("{new_id}.md")),
        )
    };
    if new.exists() && !same_entry(&old, &new) {
        return Err("an item with that name already exists there".into());
    }
    if let Some(parent) = new.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::rename(old, new).map_err(|e| e.to_string())?;
    Ok(new_id)
}

/// Whether the markdown at `path` links to `slug` (via `[[slug]]` / `[[slug|…]]`).
fn links_to(path: &Path, slug: &str) -> bool {
    fs::read_to_string(path)
        .map(|c| wikilink_targets(&c).iter().any(|t| t == slug))
        .unwrap_or(false)
}

/// Recursively collect wiki pages under `dir` (any folder) that link to `slug`,
/// as `wiki` refs (id = title = slug); the page itself is excluded.
fn wiki_backlinks(dir: &Path, slug: &str, out: &mut Vec<LinkRef>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            wiki_backlinks(&path, slug, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if stem != slug && links_to(&path, slug) {
                out.push(LinkRef {
                    kind: "wiki".into(),
                    id: stem.to_string(),
                    title: stem.to_string(),
                });
            }
        }
    }
}

/// Recursively collect notes under `dir` that link to `slug` (id = well-relative
/// path without `.md`; hidden entries skipped).
fn note_backlinks(dir: &Path, root: &Path, slug: &str, out: &mut Vec<LinkRef>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            note_backlinks(&path, root, slug, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") && links_to(&path, slug) {
            let title = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            out.push(LinkRef {
                kind: "note".into(),
                id: rel_path(root, &path.with_extension("")),
                title,
            });
        }
    }
}

/// Collect the tasks or goals in `dir` (flat) whose body links to `slug`, as
/// `kind` refs titled by their `title:` frontmatter (falling back to the stem).
fn task_backlinks(dir: &Path, kind: &str, slug: &str, out: &mut Vec<LinkRef>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if name.starts_with('.') || path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        if wikilink_targets(&content).iter().any(|t| t == slug) {
            let (fields, _) = crate::frontmatter::parse(&content);
            out.push(LinkRef {
                kind: kind.to_string(),
                id: stem.to_string(),
                title: fields
                    .get("title")
                    .cloned()
                    .unwrap_or_else(|| stem.to_string()),
            });
        }
    }
}

/// Everything that links to wiki page `slug` (via `[[slug]]` / `[[slug|…]]`) —
/// across the wiki, notes, and task/goal bodies, so the link graph spans every
/// section (and every wiki folder). Wiki pages list first, then notes, then tasks
/// and goals; each group alphabetical. Self-references are excluded. An on-demand
/// scan — no cache yet.
pub fn backlinks(well: String, slug: String) -> Vec<LinkRef> {
    let mut out = Vec::new();
    // Wiki pages (any folder, excluding the page itself).
    wiki_backlinks(&wiki_root(&well), &slug, &mut out);
    // Notes (recursive) — the cross-section half.
    let notes_root = section_dir(&well, Section::Notes);
    note_backlinks(&notes_root, &notes_root, &slug, &mut out);
    // Task and goal bodies (each a flat folder).
    let tasks_root = section_dir(&well, Section::Tasks);
    task_backlinks(&tasks_root, "task", &slug, &mut out);
    task_backlinks(&tasks_root.join("goals"), "goal", &slug, &mut out);
    // Wiki, then notes, then tasks/goals; each alphabetical by id.
    out.sort_by(|a, b| {
        let rank = |k: &str| match k {
            "wiki" => 0u8,
            "note" => 1,
            "task" => 2,
            _ => 3,
        };
        rank(&a.kind)
            .cmp(&rank(&b.kind))
            .then(a.id.to_lowercase().cmp(&b.id.to_lowercase()))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{TempDir, tempdir};

    /// A scaffolded well with an empty `wiki/` dir.
    fn well() -> (TempDir, String) {
        let dir = tempdir().unwrap();
        let path = dir.path().to_string_lossy().into_owned();
        fs::create_dir_all(dir.path().join("wiki")).unwrap();
        (dir, path)
    }

    /// Flatten the wiki tree into `(path, is_dir)` pairs for terse assertions.
    fn flat(nodes: &[TreeNode], out: &mut Vec<(String, bool)>) {
        for n in nodes {
            out.push((n.path.clone(), n.is_dir));
            flat(&n.children, out);
        }
    }

    fn slugs(well: &str) -> Vec<(String, bool)> {
        let mut v = Vec::new();
        flat(&list_wiki(well.to_string()), &mut v);
        v
    }

    #[test]
    fn targets_extracted_and_slugified() {
        let t = wikilink_targets("see [[Auth System]] and [[onboarding|here]], not [[a\nb]]");
        assert_eq!(t, vec!["auth-system".to_string(), "onboarding".into()]);
    }

    #[test]
    fn create_list_read_write_page() {
        let (_d, w) = well();
        assert!(list_wiki(w.clone()).is_empty());
        let slug = create_page(w.clone(), String::new()).unwrap();
        assert_eq!(slug, "untitled");
        write_page(w.clone(), slug.clone(), "# Hi\n\nbody".into()).unwrap();
        assert_eq!(read_page(w.clone(), slug.clone()), "# Hi\n\nbody");
        assert_eq!(slugs(&w), vec![("untitled".to_string(), false)]);
        // A missing page reads as empty, not an error.
        assert_eq!(read_page(w, "nope".into()), "");
    }

    #[test]
    fn folders_organise_pages_but_not_identity() {
        let (_d, w) = well();
        let folder = create_wiki_folder(w.clone(), String::new()).unwrap();
        assert_eq!(folder, "new folder");
        // A page created in the folder still opens by its bare slug.
        let slug = create_page(w.clone(), folder.clone()).unwrap();
        assert_eq!(slug, "untitled");
        write_page(w.clone(), slug.clone(), "in a folder".into()).unwrap();
        assert_eq!(read_page(w.clone(), slug.clone()), "in a folder");
        // The tree nests it under the folder (folders first).
        assert_eq!(
            slugs(&w),
            vec![
                ("new folder".to_string(), true),
                ("new folder/untitled".to_string(), false),
            ]
        );
    }

    #[test]
    fn slug_is_unique_section_wide_across_folders() {
        let (_d, w) = well();
        let a = create_wiki_folder(w.clone(), String::new()).unwrap();
        // A second "new folder" uniquifies too.
        let b = create_wiki_folder(w.clone(), String::new()).unwrap();
        assert_eq!(b, "new folder-2");
        let s1 = create_page(w.clone(), a).unwrap();
        let s2 = create_page(w.clone(), b).unwrap();
        assert_eq!(s1, "untitled");
        // Same "untitled" stem in a different folder would collide, so it bumps.
        assert_eq!(s2, "untitled-2");
    }

    #[test]
    fn rename_page_in_folder_rejects_section_wide_collision() {
        let (_d, w) = well();
        let folder = create_wiki_folder(w.clone(), String::new()).unwrap();
        write_page(w.clone(), "alpha".into(), "a".into()).unwrap(); // at root
        // Put a page inside the folder, then try to rename it onto the root slug.
        let inner = create_page(w.clone(), folder.clone()).unwrap();
        assert!(rename_page(w.clone(), inner.clone(), "Alpha".into()).is_err());
        // A free name works and keeps the file in its folder.
        let new = rename_page(w.clone(), inner, "Beta".into()).unwrap();
        assert_eq!(new, "beta");
        assert!(slugs(&w).contains(&("new folder/beta".to_string(), false)));
    }

    #[test]
    fn move_page_keeps_slug_and_links() {
        let (_d, w) = well();
        let folder = create_wiki_folder(w.clone(), String::new()).unwrap();
        write_page(w.clone(), "auth-system".into(), "the hub".into()).unwrap();
        write_page(w.clone(), "login".into(), "uses [[auth-system]]".into()).unwrap();
        // Move the target page into the folder — its slug is unchanged.
        let moved = move_wiki_entry(w.clone(), "auth-system".into(), false, folder).unwrap();
        assert_eq!(moved, "new folder/auth-system");
        // It still reads and is still linked (backlinks are folder-agnostic).
        assert_eq!(read_page(w.clone(), "auth-system".into()), "the hub");
        let back = backlinks(w, "auth-system".into());
        assert!(back.iter().any(|l| l.kind == "wiki" && l.id == "login"));
    }

    #[test]
    fn move_folder_and_delete_empty_only() {
        let (_d, w) = well();
        let parent = create_wiki_folder(w.clone(), String::new()).unwrap();
        let child = create_wiki_folder(w.clone(), String::new()).unwrap();
        assert_eq!(child, "new folder-2");
        // Nest child under parent.
        let moved = move_wiki_entry(w.clone(), child, true, parent.clone()).unwrap();
        assert_eq!(moved, "new folder/new folder-2");
        // Parent isn't empty now → delete refused; the (empty) nested one deletes.
        assert!(delete_wiki_folder(w.clone(), parent.clone()).is_err());
        assert!(delete_wiki_folder(w.clone(), moved).is_ok());
        assert!(delete_wiki_folder(w.clone(), parent).is_ok());
        // A folder can't move into itself.
        let f = create_wiki_folder(w.clone(), String::new()).unwrap();
        assert!(move_wiki_entry(w, f.clone(), true, f).is_err());
    }

    #[test]
    fn backlinks_span_wiki_folders_and_notes() {
        let (_d, w) = well();
        let folder = create_wiki_folder(w.clone(), String::new()).unwrap();
        fs::create_dir_all(Path::new(&w).join("notes/sub")).unwrap();
        write_page(w.clone(), "auth-system".into(), "the hub".into()).unwrap();
        // A linker that lives *inside a folder* must still be found.
        let inner = create_page(w.clone(), folder).unwrap();
        write_page(w.clone(), inner.clone(), "see [[auth-system]]".into()).unwrap();
        let renamed = rename_page(w.clone(), inner, "Login".into()).unwrap();
        assert_eq!(renamed, "login");
        // A note (cross-section) that links to the page.
        fs::write(
            Path::new(&w).join("notes/diary.md"),
            "today [[auth-system]]",
        )
        .unwrap();

        let back = backlinks(w, "auth-system".into());
        let got: Vec<(&str, &str)> = back
            .iter()
            .map(|l| (l.kind.as_str(), l.id.as_str()))
            .collect();
        assert_eq!(got, vec![("wiki", "login"), ("note", "diary")]);
    }

    #[test]
    fn backlinks_and_rename_cover_tasks_and_goals() {
        let (_d, w) = well();
        fs::create_dir_all(Path::new(&w).join("tasks/goals")).unwrap();
        write_page(w.clone(), "auth-system".into(), "the hub".into()).unwrap();
        fs::write(
            Path::new(&w).join("tasks/fix-login.md"),
            "---\nstatus: todo\ntitle: Fix login\n---\nblocked on [[auth-system]]",
        )
        .unwrap();
        fs::write(
            Path::new(&w).join("tasks/goals/ship-v1.md"),
            "---\norder: 0\n---\nships [[Auth System]]",
        )
        .unwrap();

        let back = backlinks(w.clone(), "auth-system".into());
        let got: Vec<(&str, &str, &str)> = back
            .iter()
            .map(|l| (l.kind.as_str(), l.id.as_str(), l.title.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("task", "fix-login", "Fix login"),
                ("goal", "ship-v1", "ship-v1"),
            ]
        );

        rename_page(w.clone(), "auth-system".into(), "Authentication".into()).unwrap();
        let task = fs::read_to_string(Path::new(&w).join("tasks/fix-login.md")).unwrap();
        assert!(task.contains("blocked on [[authentication]]"), "{task}");
        assert!(task.contains("title: Fix login"), "{task}");
        let goal = fs::read_to_string(Path::new(&w).join("tasks/goals/ship-v1.md")).unwrap();
        assert!(goal.contains("ships [[authentication]]"), "{goal}");
    }

    #[test]
    fn rename_and_delete_page() {
        let (_d, w) = well();
        write_page(w.clone(), "untitled".into(), "x".into()).unwrap();
        let new = rename_page(w.clone(), "untitled".into(), "Auth System".into()).unwrap();
        assert_eq!(new, "auth-system");
        assert_eq!(read_page(w.clone(), "auth-system".into()), "x");
        delete_page(w.clone(), "auth-system".into()).unwrap();
        assert!(list_wiki(w).is_empty());
    }

    #[test]
    fn rewrite_links_keeps_labels_and_skips_non_matches() {
        let r = rewrite_links(
            "see [[auth-system]], [[Auth System|the hub]], and [[other]]",
            "auth-system",
            "authentication",
        );
        assert_eq!(
            r.unwrap(),
            "see [[authentication]], [[authentication|the hub]], and [[other]]"
        );
        assert!(rewrite_links("just [[other]] here", "auth-system", "x").is_none());
    }

    #[test]
    fn rename_rewrites_inbound_links() {
        let (_d, w) = well();
        fs::create_dir_all(Path::new(&w).join("notes/sub")).unwrap();
        write_page(w.clone(), "auth-system".into(), "the hub".into()).unwrap();
        write_page(
            w.clone(),
            "login".into(),
            "see [[auth-system]] and [[Auth System|the hub]]".into(),
        )
        .unwrap();
        let note = Path::new(&w).join("notes/sub/diary.md");
        fs::write(&note, "logged in via [[auth-system]]").unwrap();

        let new = rename_page(w.clone(), "auth-system".into(), "Authentication".into()).unwrap();
        assert_eq!(new, "authentication");
        assert_eq!(
            read_page(w, "login".into()),
            "see [[authentication]] and [[authentication|the hub]]"
        );
        assert_eq!(
            fs::read_to_string(&note).unwrap(),
            "logged in via [[authentication]]"
        );
    }

    #[test]
    fn create_page_at_uniquifies_instead_of_overwriting() {
        let (_d, w) = well();
        let slug = create_page_at(w.clone(), "auth-system".into(), "original".into()).unwrap();
        assert_eq!(slug, "auth-system");
        let slug2 = create_page_at(w.clone(), "auth-system".into(), "different".into()).unwrap();
        assert_eq!(slug2, "auth-system-2");
        // The original page's content survived untouched.
        assert_eq!(read_page(w.clone(), "auth-system".into()), "original");
        assert_eq!(read_page(w, "auth-system-2".into()), "different");
    }

    #[test]
    fn create_page_at_collides_section_wide_even_inside_a_folder() {
        let (_d, w) = well();
        let folder = create_wiki_folder(w.clone(), String::new()).unwrap();
        // Put a page called "beta" inside a folder...
        write_page(w.clone(), "beta".into(), "in a folder".into()).unwrap();
        let inside = move_wiki_entry(w.clone(), "beta".into(), false, folder).unwrap();
        assert_eq!(inside, "new folder/beta");
        // ...then create_page_at("beta") must uniquify, not collide, and the
        // new page lands at the wiki root regardless of the folder.
        let slug = create_page_at(w.clone(), "beta".into(), "new one".into()).unwrap();
        assert_eq!(slug, "beta-2");
        assert!(slugs(&w).contains(&("beta-2".to_string(), false)));
        assert_eq!(read_page(w, "beta".into()), "in a folder");
    }

    #[test]
    fn append_page_creates_when_missing_and_appends_when_present() {
        let (_d, w) = well();
        let slug = append_page(w.clone(), "log".into(), "first entry".into()).unwrap();
        assert_eq!(slug, "log");
        assert_eq!(read_page(w.clone(), "log".into()), "first entry\n");

        append_page(w.clone(), "log".into(), "second entry".into()).unwrap();
        assert_eq!(
            read_page(w.clone(), "log".into()),
            "first entry\n\nsecond entry\n"
        );

        // Appending finds a page wherever it lives, not just the root.
        let folder = create_wiki_folder(w.clone(), String::new()).unwrap();
        write_page(w.clone(), "auth-system".into(), "the hub".into()).unwrap();
        move_wiki_entry(w.clone(), "auth-system".into(), false, folder).unwrap();
        append_page(w.clone(), "auth-system".into(), "more detail".into()).unwrap();
        assert_eq!(
            read_page(w, "auth-system".into()),
            "the hub\n\nmore detail\n"
        );
    }
}
