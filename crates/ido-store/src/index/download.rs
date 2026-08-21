//! The one-time model download and the per-machine model cache
//! (docs/mcp-server.md §6.3 "model download and cache").
//!
//! This is the **only** network event in the system's entire life (§9): three
//! files from one public HuggingFace repo, fetched with ureq + rustls into
//! `<data_dir>/latent.ido/models/{repo}/` — beside the recent-wells registry,
//! *not* `.ido/` (models are per-machine and shared across wells; `.ido/` is
//! per-well rebuildable cache). Each file downloads to a temp path, is
//! sha256-verified against the pin, then renamed into place — a partial
//! download is treated as absent.
//!
//! Both halves of the pin live on the [`ModelSpec`] so the registry stays the
//! single source of truth: `revision` (a commit sha, so the URL can't drift
//! when the repo's `main` moves) and `files` (a sha256 per file, so the bytes
//! can't drift either). §6.3 "honest costs" is the reason to bother — candle
//! reimplements architectures, and a config key it doesn't parse shifts
//! behavior silently, so the known-vector test is only meaningful against
//! known bytes.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::embed::ModelSpec;

/// Overrides the cache **root** (tests, evals, a machine with a small `C:`).
/// The flattened repo directory is still appended, so one directory holds one
/// model under the override exactly as it does by default.
const MODEL_DIR_ENV: &str = "IDO_MODEL_DIR";

/// Hard ceiling on a single downloaded file. ureq 3 reads a body unbounded by
/// default; the largest file we ask for is ~133 MB, so this bounds a hostile
/// or misrouted response without being in the way of a legitimate model.
const MAX_FILE_BYTES: u64 = 1024 * 1024 * 1024;

/// Suffix for the in-progress copy. It is deliberately *not* one of
/// `ModelSpec::files`, so [`model_present`] reads a half-written download as
/// absent and the next attempt overwrites it.
const PART_SUFFIX: &str = ".part";

/// Where `spec`'s files live (or would live) on this machine. Honors an
/// `IDO_MODEL_DIR` override (tests, evals); the repo's `/` is flattened so
/// one directory holds one model.
pub fn model_dir(spec: &ModelSpec) -> Result<PathBuf, String> {
    let root = match std::env::var(MODEL_DIR_ENV) {
        Ok(v) if !v.trim().is_empty() => PathBuf::from(v.trim()),
        _ => dirs::data_dir()
            .ok_or_else(|| "this platform has no user data directory".to_string())?
            .join("latent.ido")
            .join("models"),
    };
    Ok(root.join(spec.repo.replace('/', "--")))
}

/// True when every required file is present (existence + size — the sha256
/// was verified at download time).
pub fn model_present(spec: &ModelSpec) -> bool {
    let Ok(dir) = model_dir(spec) else {
        return false;
    };
    spec.files.iter().all(|(name, _)| complete(&dir.join(name)))
}

/// Ensure `spec`'s files are on disk, downloading whatever is missing.
/// Returns the model directory. User-triggered only — never called on a
/// server's startup path.
pub fn ensure_model(spec: &'static ModelSpec) -> Result<PathBuf, String> {
    let dir = model_dir(spec)?;
    fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    for (name, sha) in spec.files {
        check_plain_name(name)?;
        let dest = dir.join(name);
        if complete(&dest) {
            continue;
        }
        fetch_verified(spec, name, sha, &dir, &dest)?;
    }
    Ok(dir)
}

/// A file that exists, is a file, and has content. Size alone is a weak test —
/// which is why the sha256 runs before the file ever reaches this name.
fn complete(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() > 0)
}

/// Reject anything that isn't a bare file name. `ModelSpec::files` is our own
/// data, not user input, but the strings end up joined onto a real path — and
/// a future sentence-transformers-style model with `2_Dense/model.safetensors`
/// would otherwise fail confusingly at `File::create` instead of here.
fn check_plain_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.contains(['/', '\\']) || name.contains("..") {
        return Err(format!("model file `{name}` is not a plain file name"));
    }
    Ok(())
}

/// The pinned URL for one file. `resolve/{revision}/` takes a commit sha, so
/// this is reproducible in a way `resolve/main/` is not.
fn file_url(spec: &ModelSpec, name: &str) -> String {
    format!(
        "https://huggingface.co/{}/resolve/{}/{}",
        spec.repo, spec.revision, name
    )
}

/// Download one file to `<dest>.part`, verify its sha256, and only then move
/// it to `dest`. Every failure path removes the partial file, so "the model is
/// present" never means "some of the model is present".
fn fetch_verified(
    spec: &ModelSpec,
    name: &str,
    want: &str,
    dir: &Path,
    dest: &Path,
) -> Result<(), String> {
    let url = file_url(spec, name);
    let tmp = dir.join(format!("{name}{PART_SUFFIX}"));

    let result = stream_to_file(&url, &tmp).and_then(|_| {
        let got = sha256_file(&tmp)?;
        if got == want {
            Ok(())
        } else {
            Err(format!(
                "{name} failed its integrity check (expected {want}, got {got}) — \
                 the pinned revision may have been re-uploaded"
            ))
        }
    });
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }

    // Windows' `rename` fails onto an existing file, unlike POSIX's. Nothing
    // *should* be here (the caller checked), but a zero-byte leftover would
    // otherwise make the download un-completable forever.
    if dest.exists() {
        let _ = fs::remove_file(dest);
    }
    fs::rename(&tmp, dest).map_err(|e| format!("installing {name}: {e}"))
}

/// GET `url` into `path`, streaming — a 133 MB safetensors file has no reason
/// to pass through memory. ureq errors on a non-2xx status by default, so a
/// 404 from a bad revision surfaces as a download error rather than an HTML
/// error page written to disk.
fn stream_to_file(url: &str, path: &Path) -> Result<(), String> {
    let response = ureq::get(url)
        .call()
        .map_err(|e| format!("downloading {url}: {e}"))?;
    let mut reader = response
        .into_body()
        .into_with_config()
        .limit(MAX_FILE_BYTES)
        .reader();
    let mut file = File::create(path).map_err(|e| format!("creating {}: {e}", path.display()))?;
    std::io::copy(&mut reader, &mut file)
        .map_err(|e| format!("writing {}: {e}", path.display()))?;
    file.flush()
        .map_err(|e| format!("writing {}: {e}", path.display()))
}

/// The file's sha256 as lowercase hex, read in chunks so hashing a large
/// checkpoint costs one buffer rather than its whole size in RAM.
fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// Lowercase hex, to compare against the pins as written.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::embed::DEFAULT_MODEL;

    #[test]
    fn model_dir_flattens_the_repo_under_the_override() {
        // Set + read in one test: `IDO_MODEL_DIR` is process-global, and the
        // suite runs threaded.
        unsafe { std::env::set_var(MODEL_DIR_ENV, "C:/tmp/models") };
        let dir = model_dir(DEFAULT_MODEL).unwrap();
        unsafe { std::env::remove_var(MODEL_DIR_ENV) };
        assert!(
            dir.ends_with("BAAI--bge-small-en-v1.5"),
            "one directory per model, `/` flattened: {}",
            dir.display()
        );
        assert!(dir.parent().unwrap().ends_with("models"));
    }

    #[test]
    fn pinned_urls_name_a_commit_not_main() {
        let url = file_url(DEFAULT_MODEL, "config.json");
        assert_eq!(
            url,
            "https://huggingface.co/BAAI/bge-small-en-v1.5/resolve/\
             5c38ec7c405ec4b44b94cc5a9bb96e735b38267a/config.json"
        );
        assert!(!url.contains("/main/"), "a moving ref defeats the sha pins");
    }

    #[test]
    fn every_pin_is_a_plain_name_and_a_sha256() {
        assert_eq!(DEFAULT_MODEL.files.len(), 3, "config + tokenizer + weights");
        for (name, sha) in DEFAULT_MODEL.files {
            check_plain_name(name).unwrap();
            assert_eq!(sha.len(), 64, "{name}: sha256 is 64 hex chars");
            assert!(
                sha.chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
                "{name}: pins are lowercase hex, as `hex` emits them"
            );
        }
    }

    #[test]
    fn path_traversal_names_are_refused() {
        assert!(check_plain_name("../../evil").is_err());
        assert!(check_plain_name("2_Dense/model.safetensors").is_err());
        assert!(check_plain_name("").is_err());
        assert!(check_plain_name("model.safetensors").is_ok());
    }

    #[test]
    fn sha256_matches_a_known_digest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("abc.txt");
        fs::write(&path, b"abc").unwrap();
        assert_eq!(
            sha256_file(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_partial_download_reads_as_absent() {
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join(format!("model.safetensors{PART_SUFFIX}"));
        fs::write(&part, b"half a model").unwrap();
        assert!(
            !complete(&dir.path().join("model.safetensors")),
            "a `.part` file must not satisfy the real name"
        );
        fs::write(dir.path().join("empty.bin"), b"").unwrap();
        assert!(
            !complete(&dir.path().join("empty.bin")),
            "zero bytes is not a file we have"
        );
    }
}
