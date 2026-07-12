//! The wiki section: a flat namespace of linked markdown pages under `wiki/`.
//!
//! A page's id is its **slug** (its file stem, no `.md`), kept flat — no folders.
//! Pages link to each other with `[[slug]]` / `[[slug|label]]`; [`backlinks`]
//! answers "what links here" by scanning the wiki, notes, **and** task/goal
//! bodies, so the link graph spans every section. A rename rewrites inbound
//! links across all of them, too.

use std::fs;
use std::path::{Path, PathBuf};

use crate::model::{LinkRef, Section};
use crate::paths::{rel_path, section_dir, slugify, unique_name};

/// Absolute path of the markdown file backing wiki page `slug` in `well`.
fn page_path(well: &str, slug: &str) -> PathBuf {
    section_dir(well, Section::Wiki).join(format!("{slug}.md"))
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
/// (recursing into subfolders when `recurse`, for foldered notes). Best-effort:
/// unreadable / unwritable files are skipped.
fn rewrite_dir_links(dir: &Path, old: &str, new: &str, recurse: bool) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if recurse && path.is_dir() {
            rewrite_dir_links(&path, old, new, true);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Some(updated) = rewrite_links(&content, old, new) {
                    let _ = fs::write(&path, updated);
                }
            }
        }
    }
}

/// The wiki's pages, by slug, alphabetical. Hidden / non-`.md` files are skipped.
#[tauri::command]
pub fn list_wiki(well: String) -> Vec<String> {
    let Ok(entries) = fs::read_dir(section_dir(&well, Section::Wiki)) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            if name.starts_with('.') || path.extension().and_then(|x| x.to_str()) != Some("md") {
                return None;
            }
            path.file_stem().and_then(|s| s.to_str()).map(String::from)
        })
        .collect();
    out.sort_by_key(|s| s.to_lowercase());
    out
}

/// A wiki page's markdown body — empty when the page doesn't exist yet (so a
/// freshly-clicked `[[wikilink]]` opens a blank, editable page).
#[tauri::command]
pub fn read_page(well: String, slug: String) -> String {
    fs::read_to_string(page_path(&well, &slug)).unwrap_or_default()
}

/// Write a wiki page's body, creating `wiki/` if needed.
#[tauri::command]
pub fn write_page(well: String, slug: String, content: String) -> Result<(), String> {
    let dir = section_dir(&well, Section::Wiki);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::write(dir.join(format!("{slug}.md")), content).map_err(|e| e.to_string())
}

/// Create a uniquely-named empty page (`untitled`, `untitled-2`, …). Returns its slug.
#[tauri::command]
pub fn create_page(well: String) -> Result<String, String> {
    let dir = section_dir(&well, Section::Wiki);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let slug = unique_name(&dir, "untitled", "md");
    fs::write(dir.join(format!("{slug}.md")), "").map_err(|e| e.to_string())?;
    Ok(slug)
}

/// Create `slug` as an empty page if it doesn't exist — backs "create on click"
/// for an unresolved `[[wikilink]]`. A no-op when the page already exists.
#[tauri::command]
pub fn ensure_page(well: String, slug: String) -> Result<(), String> {
    let dir = section_dir(&well, Section::Wiki);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{slug}.md"));
    if !path.exists() {
        fs::write(path, "").map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Rename page `slug` to the slug of `name`, and rewrite every inbound
/// `[[link]]` to follow it — across the wiki, notes, **and** task/goal bodies,
/// so no section's links dangle. Returns the new slug.
#[tauri::command]
pub fn rename_page(well: String, slug: String, name: String) -> Result<String, String> {
    let new_slug = slugify(&name);
    if new_slug.is_empty() {
        return Err("name is empty".into());
    }
    if new_slug == slug {
        return Ok(slug);
    }
    let dir = section_dir(&well, Section::Wiki);
    let new_path = dir.join(format!("{new_slug}.md"));
    if new_path.exists() {
        return Err("name already taken".into());
    }
    fs::rename(dir.join(format!("{slug}.md")), new_path).map_err(|e| e.to_string())?;
    rewrite_dir_links(&dir, &slug, &new_slug, false);
    rewrite_dir_links(&section_dir(&well, Section::Notes), &slug, &new_slug, true);
    // Tasks, recursing into tasks/goals/ (frontmatter never holds `[[`, so only
    // bodies are touched).
    rewrite_dir_links(&section_dir(&well, Section::Tasks), &slug, &new_slug, true);
    Ok(new_slug)
}

/// Delete wiki page `slug`.
#[tauri::command]
pub fn delete_page(well: String, slug: String) -> Result<(), String> {
    fs::remove_file(page_path(&well, &slug)).map_err(|e| e.to_string())
}

/// Whether the markdown at `path` links to `slug` (via `[[slug]]` / `[[slug|…]]`).
fn links_to(path: &Path, slug: &str) -> bool {
    fs::read_to_string(path)
        .map(|c| wikilink_targets(&c).iter().any(|t| t == slug))
        .unwrap_or(false)
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
/// section. Wiki pages list first, then notes, then tasks and goals; each group
/// alphabetical. Self-references are excluded. An on-demand scan — no cache yet.
#[tauri::command]
pub fn backlinks(well: String, slug: String) -> Vec<LinkRef> {
    let mut out = Vec::new();
    // Wiki pages (excluding the page itself).
    if let Ok(entries) = fs::read_dir(section_dir(&well, Section::Wiki)) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if stem != slug && links_to(&path, &slug) {
                out.push(LinkRef {
                    kind: "wiki".into(),
                    id: stem.to_string(),
                    title: stem.to_string(),
                });
            }
        }
    }
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
    use tempfile::{tempdir, TempDir};

    /// A scaffolded well with an empty `wiki/` dir.
    fn well() -> (TempDir, String) {
        let dir = tempdir().unwrap();
        let path = dir.path().to_string_lossy().into_owned();
        fs::create_dir_all(dir.path().join("wiki")).unwrap();
        (dir, path)
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
        let slug = create_page(w.clone()).unwrap();
        assert_eq!(slug, "untitled");
        write_page(w.clone(), slug.clone(), "# Hi\n\nbody".into()).unwrap();
        assert_eq!(read_page(w.clone(), slug.clone()), "# Hi\n\nbody");
        assert_eq!(list_wiki(w.clone()), vec!["untitled".to_string()]);
        // A missing page reads as empty, not an error.
        assert_eq!(read_page(w, "nope".into()), "");
    }

    #[test]
    fn backlinks_span_wiki_and_notes() {
        let (_d, w) = well();
        fs::create_dir_all(Path::new(&w).join("notes/sub")).unwrap();
        write_page(w.clone(), "auth-system".into(), "the hub".into()).unwrap();
        write_page(w.clone(), "login".into(), "uses [[Auth System]]".into()).unwrap();
        write_page(
            w.clone(),
            "signup".into(),
            "see [[auth-system|auth]]".into(),
        )
        .unwrap();
        write_page(w.clone(), "stray".into(), "no links".into()).unwrap();
        // A note (cross-section) that links to the page, and one that doesn't.
        fs::write(
            Path::new(&w).join("notes/diary.md"),
            "today [[auth-system]]",
        )
        .unwrap();
        fs::write(Path::new(&w).join("notes/sub/plan.md"), "no link here").unwrap();

        let back = backlinks(w, "auth-system".into());
        let got: Vec<(&str, &str)> = back
            .iter()
            .map(|l| (l.kind.as_str(), l.id.as_str()))
            .collect();
        // Wiki pages first (alphabetical), then notes by path.
        assert_eq!(
            got,
            vec![("wiki", "login"), ("wiki", "signup"), ("note", "diary")]
        );
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

        // Both bodies surface as backlinks, titled for display.
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

        // A rename follows the links into both bodies — frontmatter untouched.
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
        // No matching link → no rewrite.
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
        // A note (in a subfolder) that links to the page must follow the rename too.
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
}
