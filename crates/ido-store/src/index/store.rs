//! The on-disk vector index under `<well>/.ido/index/`, its manifest-driven
//! incremental rebuild, and brute-force cosine top-k (docs/mcp-server.md
//! §6.4, §6.6).
//!
//! Format:
//! - `manifest.json` — schema version, model id, dim, per-file
//!   `{sha256, mtime, size, chunk ids}`
//! - `vectors.bin` — raw little-endian f32, row-major, one row per
//!   `chunks.jsonl` line, in line order
//! - `chunks.jsonl` — one serialized [`Chunk`] per line
//! - `.lock` — advisory lock around rebuilds (stale after a timeout); a
//!   reader with a stale index answers from what it has rather than blocking
//!
//! No ANN structure, deliberately: a personal well is two orders of magnitude
//! too small for HNSW to earn its complexity (§6.4's napkin math) — a dot
//! product sweep over an in-memory f32 matrix answers in well under a
//! millisecond. Revisit only past ~100k chunks; `IndexStatus::chunks` makes
//! the trigger observable.
//!
//! Incremental strategy: the expensive stage is embedding, not IO. A rebuild
//! re-embeds only files whose `(mtime, size)` — then sha256 — changed,
//! reuses every other file's existing vectors, and rewrites all three files
//! atomically (temp + rename), so chunk ids stay dense and the trio is never
//! mutually inconsistent.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::model::Section;
use crate::paths::{rel_path, section_dir};

use super::IndexStatus;
use super::chunk::{Chunk, chunk_markdown, chunk_task};
use super::embed::{Embedder, Role};

/// The index directory, well-relative. Lives under `.ido/` because it is
/// rebuildable cache — never index `.ido/` or `assets/` themselves.
pub const INDEX_DIR: &str = ".ido/index";

/// Schema version stamped in the manifest; bump on any format change to force
/// a clean rebuild instead of a misread.
pub const SCHEMA_VERSION: u32 = 1;

/// How long a `.lock` file is trusted before a rebuild treats it as
/// abandoned (a crashed process, an interrupted build) and breaks it.
const LOCK_STALE_MS: u64 = 10 * 60 * 1000;

/// One indexed file's fingerprint + its rows.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileEntry {
    /// Content hash — the change detector of record.
    pub sha256: String,
    /// Modified time (epoch ms) at index time — the cheap first-pass filter.
    pub mtime_ms: u64,
    /// File size in bytes — second cheap filter.
    pub size: u64,
    /// Row indices (into `chunks.jsonl` / `vectors.bin`) holding this file's
    /// chunks.
    pub chunks: Vec<u32>,
}

/// `manifest.json`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IndexManifest {
    /// [`SCHEMA_VERSION`] at write time.
    pub schema: u32,
    /// The embedder id that built every vector in this index.
    pub model: String,
    /// Vector width.
    pub dim: usize,
    /// Last (re)build, epoch ms.
    pub built_ms: u64,
    /// Well-relative file path → fingerprint, sorted for stable diffs.
    pub files: BTreeMap<String, FileEntry>,
}

/// A loaded index: the manifest, every chunk, and the vector matrix
/// (row-major, `manifest.dim` wide, one row per chunk).
pub struct VectorIndex {
    pub manifest: IndexManifest,
    pub chunks: Vec<Chunk>,
    pub vectors: Vec<f32>,
}

impl VectorIndex {
    /// Brute-force cosine top-k: `query` is a unit vector (so dot product =
    /// cosine); returns `(chunk index, score)` best-first, at most `top_k`.
    pub fn query(&self, query: &[f32], top_k: usize) -> Vec<(usize, f32)> {
        let dim = self.manifest.dim;
        if dim == 0 || query.len() != dim {
            return Vec::new();
        }
        let mut scored: Vec<(usize, f32)> = self
            .vectors
            .chunks_exact(dim)
            .enumerate()
            .map(|(i, row)| (i, dot(row, query)))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        scored
    }

    /// The chunk behind a [`VectorIndex::query`] hit.
    pub fn chunk(&self, i: usize) -> &Chunk {
        &self.chunks[i]
    }
}

/// Dot product of two equal-length slices (cosine, since vectors are unit
/// norm — see the module doc).
fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

// --- the corpus walk --------------------------------------------------

/// Recursively list `.md` files under `dir` (skipping dotfiles/dirs and
/// symlinks — never follow a path out of the well), as `(backing-file key,
/// absolute path)` pairs. `key` is `"<section>/<relative-path>"`,
/// forward-slash separated, `.md` included — the manifest key.
fn walk_md_files(root: &Path, dir: &Path, section: &str, out: &mut Vec<(String, PathBuf)>) {
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
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            walk_md_files(root, &path, section, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let rel = rel_path(root, &path);
            out.push((format!("{section}/{rel}"), path));
        }
    }
}

/// Chunk every note under `notes/` (recursive). `entry_id` is the note's id
/// (well-relative path sans `.md`); `title` is the file stem.
fn collect_notes(well: &str, out: &mut Vec<(String, Vec<Chunk>)>) {
    let root = section_dir(well, Section::Notes);
    let mut files = Vec::new();
    walk_md_files(&root, &root, "notes", &mut files);
    for (key, path) in files {
        let Ok(body) = fs::read_to_string(&path) else {
            continue;
        };
        let id = rel_path(&root, &path.with_extension(""));
        let title = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        out.push((key, chunk_markdown("note", &id, &title, &body)));
    }
}

/// Chunk every wiki page under `wiki/` (recursive — folders are cosmetic but
/// real directories). `entry_id` and `title` are both the page's slug
/// (globally unique regardless of folder); the backing-file key still
/// carries the folder, since that's the real path being stat-ed.
fn collect_wiki(well: &str, out: &mut Vec<(String, Vec<Chunk>)>) {
    let root = section_dir(well, Section::Wiki);
    let mut files = Vec::new();
    walk_md_files(&root, &root, "wiki", &mut files);
    for (key, path) in files {
        let Ok(body) = fs::read_to_string(&path) else {
            continue;
        };
        let Some(slug) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        out.push((key, chunk_markdown("wiki", slug, slug, &body)));
    }
}

/// Chunk every non-archived task and goal via [`crate::tasks`], so titles,
/// tags, goal links and the archived flag all resolve the same way the board
/// and the lexical scan see them.
fn collect_tasks(well: &str, out: &mut Vec<(String, Vec<Chunk>)>) {
    let goals = crate::tasks::list_goals(well.to_string());
    let goal_title = |id: &str| -> Option<String> {
        if id.is_empty() {
            return None;
        }
        goals.iter().find(|g| g.id == id).map(|g| g.title.clone())
    };
    for task in crate::tasks::list_tasks(well.to_string()) {
        if task.archived {
            continue;
        }
        let chunks = chunk_task(
            "task",
            &task.id,
            &task.title,
            &task.tags,
            goal_title(&task.goal).as_deref(),
            &task.body,
        );
        out.push((format!("tasks/{}.md", task.id), chunks));
    }
    for goal in &goals {
        if goal.archived {
            continue;
        }
        let chunks = chunk_task("goal", &goal.id, &goal.title, &[], None, &goal.body);
        out.push((format!("tasks/goals/{}.md", goal.id), chunks));
    }
}

/// Walk the well and chunk every indexable entry, grouped by the
/// well-relative path of the backing file (the manifest key). Covers notes
/// (recursive), wiki pages (recursive — folders are cosmetic but real
/// directories), tasks and goals (via the task store, so titles/tags/goal
/// links resolve); skips hidden entries, `.ido/`, `assets/`, and archived
/// tasks/goals (lexical search excludes them too — the two halves must see
/// the same corpus). Never follows a path out of the well.
pub fn collect_chunks(well: &str) -> Vec<(String, Vec<Chunk>)> {
    let mut out = Vec::new();
    collect_notes(well, &mut out);
    collect_wiki(well, &mut out);
    collect_tasks(well, &mut out);
    // Deterministic order, so chunk ids are stable across identical states.
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

// --- fingerprints -------------------------------------------------------

/// A file's `(size, mtime_ms)` — the two cheap fingerprints behind the
/// fast-path diff and the staleness sweep. `None` when the file can't be
/// stat-ed (a race with a delete, a permissions error).
fn stat_file(path: &Path) -> Option<(u64, u64)> {
    let meta = fs::metadata(path).ok()?;
    let mtime_ms = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    Some((meta.len(), mtime_ms))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn sha256_of_file(path: &Path) -> String {
    sha256_hex(&fs::read(path).unwrap_or_default())
}

fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// --- the advisory lock ---------------------------------------------------

/// A held advisory lock at `.ido/index/.lock`; removed on drop — including
/// every early-return error path in [`update_index`] — so the RAII guard is
/// the only place lock cleanup has to be right.
struct LockGuard {
    path: PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Whether the lock at `path` is younger than [`LOCK_STALE_MS`]. An
/// unreadable or unparseable lock is treated as stale (broken, not trusted).
fn lock_is_fresh(path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    let Some(ms) = content
        .rsplit("at ")
        .next()
        .and_then(|s| s.trim().parse::<u64>().ok())
    else {
        return false;
    };
    epoch_ms().saturating_sub(ms) < LOCK_STALE_MS
}

/// Take the advisory rebuild lock, breaking it first if it's older than
/// [`LOCK_STALE_MS`]. A fresh lock held by another process returns `Err`
/// immediately, before any of `manifest.json` / `chunks.jsonl` /
/// `vectors.bin` is touched.
fn acquire_lock(well: &str) -> Result<LockGuard, String> {
    let dir = Path::new(well).join(INDEX_DIR);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let lock_path = dir.join(".lock");
    loop {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut f) => {
                let _ = write!(f, "pid {} at {}", std::process::id(), epoch_ms());
                return Ok(LockGuard { path: lock_path });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if lock_is_fresh(&lock_path) {
                    return Err("index locked by another process".into());
                }
                // Stale (or unreadable) — break it and retake.
                let _ = fs::remove_file(&lock_path);
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

// --- atomic write ---------------------------------------------------------

/// Write `content` to a `.tmp` sibling of `path`, then rename into place.
/// Windows refuses to rename onto an existing destination, so the dest is
/// removed first.
fn atomic_write(path: &Path, content: &[u8]) -> Result<(), String> {
    let tmp_name = format!(
        "{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("index")
    );
    let tmp = path.with_file_name(tmp_name);
    fs::write(&tmp, content).map_err(|e| e.to_string())?;
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Rewrite all three index files. `manifest.json` is written **last** — it's
/// the commit point: a reader never sees chunks/vectors without a manifest
/// that describes them.
fn write_index(
    well: &str,
    manifest: &IndexManifest,
    chunks: &[Chunk],
    vectors: &[f32],
) -> Result<(), String> {
    let dir = Path::new(well).join(INDEX_DIR);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let mut jsonl = String::new();
    for chunk in chunks {
        jsonl.push_str(&serde_json::to_string(chunk).map_err(|e| e.to_string())?);
        jsonl.push('\n');
    }
    atomic_write(&dir.join("chunks.jsonl"), jsonl.as_bytes())?;

    let mut bytes = Vec::with_capacity(vectors.len() * 4);
    for v in vectors {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    atomic_write(&dir.join("vectors.bin"), &bytes)?;

    let json = serde_json::to_string_pretty(manifest).map_err(|e| e.to_string())?;
    atomic_write(&dir.join("manifest.json"), json.as_bytes())?;

    Ok(())
}

// --- the incremental rebuild ----------------------------------------------

/// What a single file needs, decided by diffing it against the old manifest.
enum Plan {
    /// `(mtime, size)` matched, or the content hash matched — reuse the old
    /// rows verbatim rather than re-embedding.
    Reuse {
        row_ids: Vec<u32>,
        sha256: String,
        mtime_ms: u64,
        size: u64,
    },
    /// New file, or content actually changed — needs (re-)embedding.
    Fresh {
        chunks: Vec<Chunk>,
        sha256: String,
        mtime_ms: u64,
        size: u64,
    },
}

/// Bring `.ido/index/` up to date against the well, re-embedding only what
/// changed (see the module doc). Creates the index when absent; a manifest
/// whose `model`/`dim`/`schema` disagree with `embedder` forces a full
/// rebuild. Takes the advisory lock; if another process holds a fresh lock,
/// returns `Err` without touching anything (callers answer from the stale
/// index instead).
pub fn update_index(well: &str, embedder: &dyn Embedder) -> Result<IndexStatus, String> {
    let _guard = acquire_lock(well)?;

    let walked = collect_chunks(well);

    // A schema/model/dim mismatch forces a full rebuild: every file below is
    // treated as new, and the old vectors are never read.
    let old = load_index(well).filter(|idx| {
        idx.manifest.schema == SCHEMA_VERSION
            && idx.manifest.model == embedder.id()
            && idx.manifest.dim == embedder.dim()
    });

    let mut plans: Vec<(String, Plan)> = Vec::with_capacity(walked.len());
    let mut embed_queue: Vec<String> = Vec::new();

    for (path, chunks) in walked {
        let abs = Path::new(well).join(&path);
        let Some((size, mtime_ms)) = stat_file(&abs) else {
            // Gone since the walk (a rare race) — drop it, like any other
            // file that disappeared.
            continue;
        };
        let old_entry = old.as_ref().and_then(|idx| idx.manifest.files.get(&path));
        let plan = if let Some(entry) = old_entry {
            if entry.mtime_ms == mtime_ms && entry.size == size {
                Plan::Reuse {
                    row_ids: entry.chunks.clone(),
                    sha256: entry.sha256.clone(),
                    mtime_ms,
                    size,
                }
            } else {
                let sha256 = sha256_of_file(&abs);
                if sha256 == entry.sha256 {
                    // Content unchanged (a touch, or a copy with a fresh
                    // mtime) — reuse the rows, just refresh the fingerprint.
                    Plan::Reuse {
                        row_ids: entry.chunks.clone(),
                        sha256,
                        mtime_ms,
                        size,
                    }
                } else {
                    embed_queue.extend(chunks.iter().map(Chunk::embedding_text));
                    Plan::Fresh {
                        chunks,
                        sha256,
                        mtime_ms,
                        size,
                    }
                }
            }
        } else {
            let sha256 = sha256_of_file(&abs);
            embed_queue.extend(chunks.iter().map(Chunk::embedding_text));
            Plan::Fresh {
                chunks,
                sha256,
                mtime_ms,
                size,
            }
        };
        plans.push((path, plan));
    }

    // One batch call for everything that needs embedding — the embedder
    // batches internally.
    let embedded = if embed_queue.is_empty() {
        Vec::new()
    } else {
        embedder.embed(&embed_queue, Role::Document)?
    };
    let mut embedded = embedded.into_iter();

    let dim = embedder.dim();
    let mut chunks_out: Vec<Chunk> = Vec::new();
    let mut vectors_out: Vec<f32> = Vec::new();
    let mut files: BTreeMap<String, FileEntry> = BTreeMap::new();

    for (path, plan) in plans {
        let entry = match plan {
            Plan::Reuse {
                row_ids,
                sha256,
                mtime_ms,
                size,
            } => {
                let idx = old
                    .as_ref()
                    .expect("a Reuse plan only exists when an old index matched");
                let mut new_rows = Vec::with_capacity(row_ids.len());
                for row in row_ids {
                    let row = row as usize;
                    new_rows.push(chunks_out.len() as u32);
                    chunks_out.push(idx.chunks[row].clone());
                    let start = row * dim;
                    vectors_out.extend_from_slice(&idx.vectors[start..start + dim]);
                }
                FileEntry {
                    sha256,
                    mtime_ms,
                    size,
                    chunks: new_rows,
                }
            }
            Plan::Fresh {
                chunks,
                sha256,
                mtime_ms,
                size,
            } => {
                let mut new_rows = Vec::with_capacity(chunks.len());
                for chunk in chunks {
                    let vector = embedded
                        .next()
                        .ok_or("embedder returned fewer vectors than requested")?;
                    if vector.len() != dim {
                        return Err(format!(
                            "embedder returned a {}-dim vector, expected {dim}",
                            vector.len()
                        ));
                    }
                    new_rows.push(chunks_out.len() as u32);
                    chunks_out.push(chunk);
                    vectors_out.extend_from_slice(&vector);
                }
                FileEntry {
                    sha256,
                    mtime_ms,
                    size,
                    chunks: new_rows,
                }
            }
        };
        files.insert(path, entry);
    }

    let manifest = IndexManifest {
        schema: SCHEMA_VERSION,
        model: embedder.id().to_string(),
        dim,
        built_ms: epoch_ms(),
        files,
    };

    write_index(well, &manifest, &chunks_out, &vectors_out)?;

    Ok(IndexStatus {
        model: manifest.model,
        dim: manifest.dim,
        chunks: chunks_out.len(),
        files: manifest.files.len(),
        built_ms: manifest.built_ms,
        stale: false,
    })
}

/// Load the index for querying. `None` when absent or unreadable (wrong
/// schema, truncated vectors, dim mismatch) — callers degrade to keyword
/// search, never error.
pub fn load_index(well: &str) -> Option<VectorIndex> {
    let dir = Path::new(well).join(INDEX_DIR);
    let manifest_raw = fs::read_to_string(dir.join("manifest.json")).ok()?;
    let manifest: IndexManifest = serde_json::from_str(&manifest_raw).ok()?;
    if manifest.schema != SCHEMA_VERSION {
        return None;
    }

    let chunks_raw = fs::read_to_string(dir.join("chunks.jsonl")).ok()?;
    let mut chunks = Vec::new();
    for line in chunks_raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        chunks.push(serde_json::from_str::<Chunk>(line).ok()?);
    }

    let bytes = fs::read(dir.join("vectors.bin")).ok()?;
    if bytes.len() % 4 != 0 {
        return None;
    }
    let vectors: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    if vectors.len() != chunks.len() * manifest.dim {
        return None;
    }

    Some(VectorIndex {
        manifest,
        chunks,
        vectors,
    })
}

// --- staleness --------------------------------------------------------

/// Whether `path` (whose manifest key is `key`) is newer than what the
/// manifest recorded, or missing from it entirely. Stat-only — no read.
fn entry_is_newer(path: &Path, key: &str, manifest: &IndexManifest) -> bool {
    let Some((_, mtime_ms)) = stat_file(path) else {
        return false;
    };
    match manifest.files.get(key) {
        None => true,
        Some(fe) => mtime_ms > fe.mtime_ms,
    }
}

/// A cheap staleness sweep: true when at least one indexable file is missing
/// from the manifest, present-but-deleted, or newer than recorded. Notes and
/// wiki pages are only ever stat-ed, never read. Tasks/goals are the one
/// exception — telling an archived entry (excluded from the corpus, see
/// [`collect_tasks`]) apart from "never indexed" needs the frontmatter, so
/// this reads task/goal files via [`crate::tasks`] rather than only stat-ing
/// them; that's a small, bounded cost (a well's task count, not its whole
/// corpus) traded for never reporting a permanently-archived task as stale.
fn is_stale(well: &str, manifest: &IndexManifest) -> bool {
    let mut seen: HashSet<String> = HashSet::new();

    let notes_root = section_dir(well, Section::Notes);
    let wiki_root = section_dir(well, Section::Wiki);
    let mut md_files = Vec::new();
    walk_md_files(&notes_root, &notes_root, "notes", &mut md_files);
    walk_md_files(&wiki_root, &wiki_root, "wiki", &mut md_files);
    for (key, path) in &md_files {
        seen.insert(key.clone());
        if entry_is_newer(path, key, manifest) {
            return true;
        }
    }

    for task in crate::tasks::list_tasks(well.to_string()) {
        if task.archived {
            continue;
        }
        let key = format!("tasks/{}.md", task.id);
        seen.insert(key.clone());
        if entry_is_newer(&Path::new(well).join(&key), &key, manifest) {
            return true;
        }
    }
    for goal in crate::tasks::list_goals(well.to_string()) {
        if goal.archived {
            continue;
        }
        let key = format!("tasks/goals/{}.md", goal.id);
        seen.insert(key.clone());
        if entry_is_newer(&Path::new(well).join(&key), &key, manifest) {
            return true;
        }
    }

    // Anything the manifest still remembers but the walk never saw = deleted.
    manifest.files.keys().any(|k| !seen.contains(k))
}

/// The manifest's summary + a cheap staleness sweep (stat every indexed and
/// indexable file; no hashing, no reads). `None` when no index exists.
pub fn index_status(well: &str) -> Option<IndexStatus> {
    let dir = Path::new(well).join(INDEX_DIR);
    let manifest_raw = fs::read_to_string(dir.join("manifest.json")).ok()?;
    let manifest: IndexManifest = serde_json::from_str(&manifest_raw).ok()?;

    let stale = is_stale(well, &manifest);
    let chunks = manifest.files.values().map(|f| f.chunks.len()).sum();

    Some(IndexStatus {
        model: manifest.model,
        dim: manifest.dim,
        chunks,
        files: manifest.files.len(),
        built_ms: manifest.built_ms,
        stale,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::embed::FakeEmbedder;
    use tempfile::{TempDir, tempdir};

    /// A well with the three section folders scaffolded (mirrors
    /// `search.rs`'s test helper).
    fn well() -> (TempDir, String) {
        let dir = tempdir().unwrap();
        for s in ["notes", "wiki", "tasks"] {
            fs::create_dir_all(dir.path().join(s)).unwrap();
        }
        let path = dir.path().to_string_lossy().into_owned();
        (dir, path)
    }

    fn write(well: &str, rel: &str, body: &str) {
        let path = Path::new(well).join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    fn mk_chunk(kind: &str, id: &str, heading: &str, text: &str) -> Chunk {
        Chunk {
            kind: kind.to_string(),
            entry_id: id.to_string(),
            heading_path: heading.to_string(),
            byte_range: None,
            text: text.to_string(),
        }
    }

    /// A [`FakeEmbedder`] under a different model id (and optionally a
    /// different dim) — simulates a model swap without needing a second real
    /// embedder implementation.
    struct RenamedFake {
        id: &'static str,
        inner: FakeEmbedder,
    }

    impl Embedder for RenamedFake {
        fn id(&self) -> &str {
            self.id
        }
        fn dim(&self) -> usize {
            self.inner.dim()
        }
        fn embed(&self, texts: &[String], role: Role) -> Result<Vec<Vec<f32>>, String> {
            self.inner.embed(texts, role)
        }
    }

    /// Wraps an [`Embedder`] and records every text it was asked to embed —
    /// proves *which* chunks a rebuild actually re-embedded.
    struct CountingEmbedder<E: Embedder> {
        inner: E,
        embedded: std::cell::RefCell<Vec<String>>,
    }

    impl<E: Embedder> CountingEmbedder<E> {
        fn wrap(inner: E) -> Self {
            Self {
                inner,
                embedded: std::cell::RefCell::new(Vec::new()),
            }
        }
        fn count(&self) -> usize {
            self.embedded.borrow().len()
        }
        fn reset(&self) {
            self.embedded.borrow_mut().clear();
        }
        fn texts(&self) -> Vec<String> {
            self.embedded.borrow().clone()
        }
    }

    impl<E: Embedder> Embedder for CountingEmbedder<E> {
        fn id(&self) -> &str {
            self.inner.id()
        }
        fn dim(&self) -> usize {
            self.inner.dim()
        }
        fn embed(&self, texts: &[String], role: Role) -> Result<Vec<Vec<f32>>, String> {
            self.embedded.borrow_mut().extend(texts.iter().cloned());
            self.inner.embed(texts, role)
        }
    }

    // --- pure logic: query, manifest round-trip, load validation ------

    #[test]
    fn query_ranks_best_first_and_caps_top_k() {
        let manifest = IndexManifest {
            schema: SCHEMA_VERSION,
            model: "fake".into(),
            dim: 3,
            built_ms: 0,
            files: BTreeMap::new(),
        };
        let chunks = vec![
            mk_chunk("note", "a", "A", "x"),
            mk_chunk("note", "b", "B", "y"),
            mk_chunk("note", "c", "C", "z"),
        ];
        #[rustfmt::skip]
        let vectors = vec![
            1.0, 0.0, 0.0,
            0.0, 1.0, 0.0,
            0.9, 0.1, 0.0_f32,
        ];
        let idx = VectorIndex {
            manifest,
            chunks,
            vectors,
        };
        let hits = idx.query(&[1.0, 0.0, 0.0], 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].0, 0, "exact match ranks first");
        assert_eq!(
            hits[1].0, 2,
            "near match ranks second, over the orthogonal one"
        );
    }

    #[test]
    fn query_on_empty_index_returns_nothing() {
        let manifest = IndexManifest {
            schema: SCHEMA_VERSION,
            model: "fake".into(),
            dim: 3,
            built_ms: 0,
            files: BTreeMap::new(),
        };
        let idx = VectorIndex {
            manifest,
            chunks: Vec::new(),
            vectors: Vec::new(),
        };
        assert!(idx.query(&[1.0, 0.0, 0.0], 5).is_empty());
    }

    #[test]
    fn write_and_load_index_round_trip() {
        let (_d, w) = well();
        let chunks = vec![mk_chunk("note", "a", "A", "hello")];
        let vectors = vec![1.0_f32, 0.0, 0.0];
        let mut files = BTreeMap::new();
        files.insert(
            "notes/a.md".to_string(),
            FileEntry {
                sha256: "deadbeef".into(),
                mtime_ms: 123,
                size: 5,
                chunks: vec![0],
            },
        );
        let manifest = IndexManifest {
            schema: SCHEMA_VERSION,
            model: "fake-bow-v1".into(),
            dim: 3,
            built_ms: 999,
            files,
        };
        write_index(&w, &manifest, &chunks, &vectors).unwrap();

        let loaded = load_index(&w).expect("a freshly written index must load");
        assert_eq!(loaded.manifest.model, "fake-bow-v1");
        assert_eq!(loaded.manifest.dim, 3);
        assert_eq!(loaded.chunks, chunks);
        assert_eq!(loaded.vectors, vectors);
        assert_eq!(
            loaded.manifest.files.get("notes/a.md").unwrap().mtime_ms,
            123
        );
    }

    #[test]
    fn load_index_missing_returns_none() {
        let (_d, w) = well();
        assert!(load_index(&w).is_none());
    }

    #[test]
    fn load_index_rejects_wrong_schema() {
        let (_d, w) = well();
        let chunks = vec![mk_chunk("note", "a", "A", "hi")];
        let vectors = vec![1.0_f32, 0.0];
        let manifest = IndexManifest {
            schema: SCHEMA_VERSION + 1,
            model: "fake".into(),
            dim: 2,
            built_ms: 0,
            files: BTreeMap::new(),
        };
        write_index(&w, &manifest, &chunks, &vectors).unwrap();
        assert!(load_index(&w).is_none());
    }

    #[test]
    fn load_index_rejects_truncated_vectors() {
        let (_d, w) = well();
        // Two chunks at dim=2 need 4 floats; only give it 2.
        let chunks = vec![
            mk_chunk("note", "a", "A", "hi"),
            mk_chunk("note", "b", "B", "yo"),
        ];
        let vectors = vec![1.0_f32, 0.0];
        let manifest = IndexManifest {
            schema: SCHEMA_VERSION,
            model: "fake".into(),
            dim: 2,
            built_ms: 0,
            files: BTreeMap::new(),
        };
        write_index(&w, &manifest, &chunks, &vectors).unwrap();
        assert!(load_index(&w).is_none());
    }

    // --- lock semantics + full lifecycle on an empty well --------------
    // (an empty well has nothing to chunk, so these never reach the chunker)

    #[test]
    fn fresh_lock_blocks_update_without_touching_the_index() {
        let (_d, w) = well();
        let dir = Path::new(&w).join(INDEX_DIR);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(".lock"), format!("pid 999999 at {}", epoch_ms())).unwrap();

        let result = update_index(&w, &FakeEmbedder::default());
        assert!(result.is_err());
        assert!(
            !dir.join("manifest.json").exists(),
            "a fresh lock must block the rebuild entirely"
        );
        assert!(
            dir.join(".lock").exists(),
            "the other process's lock must survive untouched"
        );
    }

    #[test]
    fn stale_lock_is_broken_and_rebuild_proceeds() {
        let (_d, w) = well();
        let dir = Path::new(&w).join(INDEX_DIR);
        fs::create_dir_all(&dir).unwrap();
        let old_ts = epoch_ms().saturating_sub(LOCK_STALE_MS + 1_000);
        fs::write(dir.join(".lock"), format!("pid 999999 at {old_ts}")).unwrap();

        let status = update_index(&w, &FakeEmbedder::default()).expect("stale lock must break");
        assert_eq!(status.files, 0);
        assert_eq!(status.chunks, 0);
        assert!(
            !dir.join(".lock").exists(),
            "lock released after the rebuild"
        );
        assert!(dir.join("manifest.json").exists());
    }

    #[test]
    fn model_mismatch_forces_a_fresh_manifest() {
        let (_d, w) = well();
        let first = update_index(&w, &FakeEmbedder::default()).unwrap();
        assert_eq!(first.model, "fake-bow-v1");

        let second = update_index(
            &w,
            &RenamedFake {
                id: "fake-bow-v2",
                inner: FakeEmbedder::default(),
            },
        )
        .unwrap();
        assert_eq!(second.model, "fake-bow-v2");
    }

    #[test]
    fn index_status_none_without_an_index() {
        let (_d, w) = well();
        assert!(index_status(&w).is_none());
    }

    #[test]
    fn index_status_flips_stale_after_a_new_file_appears() {
        let (_d, w) = well();
        update_index(&w, &FakeEmbedder::default()).unwrap();
        assert!(!index_status(&w).unwrap().stale, "nothing changed yet");

        write(&w, "notes/new.md", "content doesn't matter for staleness");
        assert!(
            index_status(&w).unwrap().stale,
            "a file the manifest never saw must flip staleness"
        );
    }

    // --- end-to-end (needs the wave-1 chunker) --------------------------

    #[test]
    fn end_to_end_semantic_query_finds_the_right_note() {
        let (_d, w) = well();
        write(
            &w,
            "notes/auth.md",
            "# Auth\n\nsession keys live in the OS keychain",
        );
        write(
            &w,
            "notes/board.md",
            "# Board\n\nkanban drag indicator styling",
        );

        update_index(&w, &FakeEmbedder::default()).unwrap();
        let idx = load_index(&w).unwrap();
        assert_eq!(idx.chunks.len(), 2);

        let query_vec = FakeEmbedder::default()
            .embed(&["where are session keys stored".to_string()], Role::Query)
            .unwrap()
            .remove(0);
        let hits = idx.query(&query_vec, 1);
        assert_eq!(hits.len(), 1);
        assert_eq!(idx.chunk(hits[0].0).entry_id, "auth");
    }

    #[test]
    fn incremental_reindex_only_reembeds_the_changed_file() {
        let (_d, w) = well();
        write(&w, "notes/a.md", "alpha content one");
        write(&w, "notes/b.md", "beta content two");
        let embedder = CountingEmbedder::wrap(FakeEmbedder::default());
        update_index(&w, &embedder).unwrap();
        assert!(
            embedder.count() >= 2,
            "both files embedded on the first build"
        );

        std::thread::sleep(std::time::Duration::from_millis(50));
        write(&w, "notes/b.md", "beta content two, revised");
        embedder.reset();
        let status = update_index(&w, &embedder).unwrap();
        assert_eq!(status.files, 2);
        assert_eq!(embedder.count(), 1, "only b's one chunk should re-embed");
        assert!(embedder.texts()[0].contains("revised"));
    }

    #[test]
    fn deleted_file_chunks_vanish_from_the_index() {
        let (_d, w) = well();
        write(&w, "notes/a.md", "alpha content");
        write(&w, "notes/b.md", "beta content");
        update_index(&w, &FakeEmbedder::default()).unwrap();
        assert_eq!(load_index(&w).unwrap().manifest.files.len(), 2);

        fs::remove_file(Path::new(&w).join("notes/b.md")).unwrap();
        let status = update_index(&w, &FakeEmbedder::default()).unwrap();
        assert_eq!(status.files, 1);
        let idx = load_index(&w).unwrap();
        assert!(idx.manifest.files.contains_key("notes/a.md"));
        assert!(!idx.manifest.files.contains_key("notes/b.md"));
        assert!(idx.chunks.iter().all(|c| c.entry_id != "b"));
    }

    #[test]
    fn different_embedder_id_forces_a_full_reembed() {
        let (_d, w) = well();
        write(&w, "notes/a.md", "alpha content");
        write(&w, "notes/b.md", "beta content");
        update_index(&w, &FakeEmbedder::default()).unwrap();
        let total_chunks = load_index(&w).unwrap().chunks.len();
        assert!(total_chunks >= 2);

        let embedder = CountingEmbedder::wrap(RenamedFake {
            id: "fake-bow-v2",
            inner: FakeEmbedder::default(),
        });
        let status = update_index(&w, &embedder).unwrap();
        assert_eq!(status.chunks, total_chunks, "same corpus, different model");
        assert_eq!(
            embedder.count(),
            total_chunks,
            "a model swap re-embeds every chunk, not just changed files"
        );
    }

    #[test]
    fn collect_chunks_is_sorted_by_backing_file_path() {
        let (_d, w) = well();
        write(&w, "notes/z.md", "z");
        write(&w, "notes/a.md", "a");
        write(&w, "wiki/m.md", "m");
        let out = collect_chunks(&w);
        let paths: Vec<&str> = out.iter().map(|(p, _)| p.as_str()).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted);
    }

    #[test]
    fn collect_chunks_covers_all_sections_and_skips_archived() {
        let (_d, w) = well();
        write(&w, "notes/n.md", "note body");
        write(&w, "wiki/pg.md", "wiki body");
        crate::tasks::create_task(w.clone(), "todo".into(), "Do thing".into(), None).unwrap();
        let archived_id =
            crate::tasks::create_task(w.clone(), "todo".into(), "Archived thing".into(), None)
                .unwrap();
        crate::tasks::set_task_field(w.clone(), archived_id, "archived".into(), "true".into())
            .unwrap();
        fs::create_dir_all(Path::new(&w).join("tasks/goals")).unwrap();
        crate::tasks::create_goal(w.clone()).unwrap();

        let out = collect_chunks(&w);
        let paths: Vec<&str> = out.iter().map(|(p, _)| p.as_str()).collect();
        assert!(paths.contains(&"notes/n.md"));
        assert!(paths.contains(&"wiki/pg.md"));
        assert!(paths.contains(&"tasks/do-thing.md"));
        assert!(paths.iter().any(|p| p.starts_with("tasks/goals/")));
        assert!(!paths.iter().any(|p| p.contains("archived-thing")));
    }
}
