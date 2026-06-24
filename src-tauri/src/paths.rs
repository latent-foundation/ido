//! Pure path helpers — well-relative id math and name validation. No I/O, so
//! these are the easiest things to unit-test (see the bottom of the file).
//!
//! An *id* is a note/folder path relative to the well root, always
//! `/`-separated and, for notes, without the `.md` extension.

use std::path::{Path, PathBuf};

use crate::model::WellRef;

/// Build a [`WellRef`] from an absolute well path (name = the folder's own name).
pub(crate) fn well_ref(path: &str) -> WellRef {
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

/// Absolute path of the markdown file backing note id `id` in `well`.
pub(crate) fn note_path(well: &str, id: &str) -> PathBuf {
    Path::new(well).join(format!("{id}.md"))
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
