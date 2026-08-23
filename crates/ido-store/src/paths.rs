//! Pure path helpers — well-relative id math and name validation. No I/O, so
//! these are the easiest things to unit-test (see the bottom of the file).
//!
//! An *id* is a note/folder path relative to the well root, always
//! `/`-separated and, for notes, without the `.md` extension.

use std::fs;
use std::path::{Path, PathBuf};

use crate::model::{Section, WellRef};

/// Build a [`WellRef`] from an absolute well path (name = the folder's own name).
///
/// `pub` (not `pub(crate)`, unlike its neighbours here): `src-tauri`'s
/// `registry` module builds a `WellRef` for each recent well and needs this
/// across the crate boundary.
pub fn well_ref(path: &str) -> WellRef {
    let name = Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_string();
    WellRef {
        path: path.to_string(),
        name,
    }
}

/// Absolute path of the folder backing `section` within `well`.
pub(crate) fn section_dir(well: &str, section: Section) -> PathBuf {
    well_join(well, section.dir())
}

/// `well/rel`, treating an empty `rel` as the well root.
pub(crate) fn well_join(well: &str, rel: &str) -> PathBuf {
    let base = Path::new(well);
    if rel.is_empty() {
        base.to_path_buf()
    } else {
        base.join(rel)
    }
}

/// Express `p` as an id relative to `well` (`/`-separated, backslashes folded).
pub(crate) fn rel_path(well: &Path, p: &Path) -> String {
    p.strip_prefix(well)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// The parent portion of an id (`"a/b" -> "a"`, `"a" -> ""`).
pub(crate) fn parent_of(id: &str) -> String {
    match id.rsplit_once('/') {
        Some((parent, _)) => parent.to_string(),
        None => String::new(),
    }
}

/// Join a parent id and a name (`("", "x") -> "x"`, `("a", "x") -> "a/x"`).
pub(crate) fn join_rel(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

/// Slugify a title into a flat wiki page id: lowercase, runs of non-alphanumeric
/// characters collapse to a single `-`, leading/trailing dashes trimmed. Unicode
/// letters/digits are kept (so non-latin titles still slug). May return `""` for
/// punctuation-only input — callers fall back to a default.
pub(crate) fn slugify(title: &str) -> String {
    let mut out = String::new();
    for ch in title.trim().chars() {
        if ch.is_alphanumeric() {
            out.extend(ch.to_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// Trim and validate an entry name: must be non-empty and contain no path
/// separators (so it stays a single tree level).
pub(crate) fn valid_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("name is empty".into());
    }
    if name.contains('/') || name.contains('\\') {
        return Err("name can't contain slashes".into());
    }
    Ok(name)
}

/// Absolute path of the markdown file backing note id `id`, under `base`
/// (a section's folder, e.g. `well/notes`).
pub(crate) fn note_path(base: &Path, id: &str) -> PathBuf {
    base.join(format!("{id}.md"))
}

/// A name like `stem`, `stem-2`, … not yet taken in `dir` (with `.ext` if given).
pub(crate) fn unique_name(dir: &Path, stem: &str, ext: &str) -> String {
    let exists = |candidate: &str| {
        let file = if ext.is_empty() {
            candidate.to_string()
        } else {
            format!("{candidate}.{ext}")
        };
        dir.join(file).exists()
    };
    if !exists(stem) {
        return stem.to_string();
    }
    let mut n = 2;
    loop {
        let candidate = format!("{stem}-{n}");
        if !exists(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Join `existing` content with an appended `addition` — the one separator
/// rule shared by every MCP append tool (notes, wiki pages, task/goal
/// bodies): a blank line (`\n\n`) separates the two, unless `existing`
/// already ends with one (nothing to separate, or nothing to separate
/// *from* — empty/absent content appends with no leading blank line at all).
/// `addition`'s own trailing newlines are trimmed first, so the result always
/// ends in exactly one `\n`, never a ragged run of them.
pub(crate) fn append_text(existing: &str, addition: &str) -> String {
    let addition = addition.trim_end_matches('\n');
    if existing.is_empty() {
        return format!("{addition}\n");
    }
    let sep = if existing.ends_with("\n\n") {
        ""
    } else if existing.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    format!("{existing}{sep}{addition}\n")
}

/// Resolve `relative` under `well`'s `section` and confirm it really lands
/// inside the well after canonicalisation — the symlink-proof half of id
/// validation, which lexical `..`-rejection cannot provide on its own: a
/// symlink *inside* the well can point anywhere on disk, and nothing about
/// the id string itself gives that away.
///
/// `relative` is rejected outright if it's absolute, or — the Windows-only
/// trap `is_absolute()` alone misses — *rooted without a drive prefix*
/// (`\evil`, which `PathBuf::push` resolves against the base's own drive
/// rather than treating as a no-op), or lexically contains a `..` component.
/// That's cheap, and catches the common case before touching disk.
///
/// The target usually doesn't exist yet (`create_*_at` / `append_*` call
/// this *before* writing anything), and canonicalising a path that doesn't
/// exist fails on Windows — so this walks up to the target's **nearest
/// existing ancestor**, canonicalises *that* (resolving any symlink along
/// the way to its real location), and re-appends the non-existent remainder
/// verbatim. If the reassembled path no longer starts with the well's own
/// canonical root, it escaped through a symlink and is rejected.
///
/// `pub`, unlike its neighbours here: `ido-mcp`'s write tools call this
/// directly, across the crate boundary, to validate an incoming id before it
/// ever reaches a store write.
pub fn confined(well: &str, section: Section, relative: &str) -> Result<PathBuf, String> {
    let rel = Path::new(relative);
    if rel.is_absolute() || rel.has_root() {
        return Err("path must be relative".into());
    }
    if rel
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("path can't contain '..'".into());
    }

    let canonical_well =
        fs::canonicalize(well).map_err(|e| format!("well root doesn't resolve: {e}"))?;

    let target = if relative.is_empty() {
        section_dir(well, section)
    } else {
        section_dir(well, section).join(relative)
    };

    // Walk up to the nearest existing ancestor — at worst the well root
    // itself, which we already know canonicalises — collecting the
    // non-existent tail as we go.
    let mut existing: &Path = &target;
    let mut remainder: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name() else {
            return Err("path escapes the well".into());
        };
        remainder.push(name.to_os_string());
        let Some(parent) = existing.parent() else {
            return Err("path escapes the well".into());
        };
        existing = parent;
    }

    let mut resolved = fs::canonicalize(existing).map_err(|e| e.to_string())?;
    for part in remainder.into_iter().rev() {
        resolved.push(part);
    }

    if !resolved.starts_with(&canonical_well) {
        return Err("path escapes the well".into());
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn ids_and_names() {
        assert_eq!(parent_of("a/b/c"), "a/b");
        assert_eq!(parent_of("a"), "");
        assert_eq!(join_rel("", "x"), "x");
        assert_eq!(join_rel("a", "x"), "a/x");
        assert_eq!(valid_name("  hi "), Ok("hi"));
        assert!(valid_name("").is_err());
        assert!(valid_name("a/b").is_err());
        assert!(valid_name("a\\b").is_err());
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("Auth System"), "auth-system");
        assert_eq!(slugify("  Hello, World!  "), "hello-world");
        assert_eq!(slugify("C++ Notes"), "c-notes");
        assert_eq!(slugify("foo/bar"), "foo-bar");
        assert_eq!(slugify("already-slug"), "already-slug");
        assert_eq!(slugify("!!!"), "");
    }

    #[test]
    fn append_text_separator_rules() {
        // Empty/absent content: no leading blank line.
        assert_eq!(append_text("", "hello"), "hello\n");
        // No trailing newline at all: a full blank line separates.
        assert_eq!(append_text("existing", "more"), "existing\n\nmore\n");
        // One trailing newline: one more closes the blank line.
        assert_eq!(append_text("existing\n", "more"), "existing\n\nmore\n");
        // Already ends with a blank line: no extra separator is added.
        assert_eq!(append_text("existing\n\n", "more"), "existing\n\nmore\n");
        // The addition's own trailing newlines are trimmed to exactly one.
        assert_eq!(append_text("x", "more\n\n\n"), "x\n\nmore\n");
    }

    #[test]
    fn confined_accepts_a_normal_relative_id() {
        let dir = tempdir().unwrap();
        let well = dir.path().to_string_lossy().into_owned();
        fs::create_dir_all(Path::new(&well).join("notes")).unwrap();

        // The target need not exist yet — a nested, not-yet-created folder
        // still resolves, since the nearest existing ancestor is the well
        // itself.
        let resolved = confined(&well, Section::Notes, "sub/note.md").unwrap();
        let canonical_well = fs::canonicalize(&well).unwrap();
        assert!(resolved.starts_with(&canonical_well));
        assert!(resolved.ends_with("sub/note.md") || resolved.ends_with("sub\\note.md"));

        // An empty relative path resolves to the section root itself.
        let root = confined(&well, Section::Notes, "").unwrap();
        assert_eq!(
            root,
            fs::canonicalize(Path::new(&well).join("notes")).unwrap()
        );
    }

    #[test]
    fn confined_rejects_dotdot_and_absolute_paths() {
        let dir = tempdir().unwrap();
        let well = dir.path().to_string_lossy().into_owned();
        fs::create_dir_all(Path::new(&well).join("notes")).unwrap();

        assert!(confined(&well, Section::Notes, "../escape.md").is_err());
        assert!(confined(&well, Section::Notes, "sub/../../escape.md").is_err());

        #[cfg(windows)]
        assert!(confined(&well, Section::Notes, "C:\\Windows\\evil.md").is_err());
        #[cfg(windows)]
        // Rooted-but-no-prefix: `PathBuf::push` resolves this against the
        // base's own drive rather than treating it as relative — the trap
        // `is_absolute()` alone would miss.
        assert!(confined(&well, Section::Notes, "\\evil.md").is_err());
        #[cfg(not(windows))]
        assert!(confined(&well, Section::Notes, "/etc/evil.md").is_err());
    }

    #[test]
    fn confined_rejects_symlink_escaping_the_well() {
        let well_dir = tempdir().unwrap();
        let well = well_dir.path().to_string_lossy().into_owned();
        fs::create_dir_all(Path::new(&well).join("notes")).unwrap();
        let outside = tempdir().unwrap();
        fs::create_dir_all(outside.path().join("secret")).unwrap();
        let link = Path::new(&well).join("notes").join("escape");

        #[cfg(windows)]
        let created = std::os::windows::fs::symlink_dir(outside.path().join("secret"), &link);
        #[cfg(unix)]
        let created = std::os::unix::fs::symlink(outside.path().join("secret"), &link);

        match created {
            Ok(()) => {
                let result = confined(&well, Section::Notes, "escape/pwned.md");
                assert!(
                    result.is_err(),
                    "a symlink inside the well pointing outside it must be rejected"
                );
            }
            // Creating a symlink can need an elevated privilege on Windows
            // (SeCreateSymbolicLinkPrivilege / Developer Mode). Rather than
            // fail the whole suite on a machine that lacks it, skip just
            // this assertion — the lexical `..` and absolute-path cases
            // above still cover the rest of `confined`.
            Err(e) => {
                eprintln!(
                    "skipping confined_rejects_symlink_escaping_the_well: couldn't create a \
                     symlink ({e}) — likely missing privilege on this machine"
                );
            }
        }
    }
}
