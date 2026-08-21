//! Markdown → heading-scoped chunks (docs/mcp-server.md §6.2).
//!
//! The useful embedding unit for markdown is a heading-scoped section, capped:
//! whole notes give one mushy vector, single lines give context-free
//! fragments. Segmentation runs on pulldown-cmark's offset iterator — parser
//! events, never line scanning — so a `#` inside a fenced code block is never
//! a heading and a fence is never split.
//!
//! Every chunk knows its `heading_path` (`"Entry title › H2 › H3"`); the
//! index embeds [`Chunk::embedding_text`] — path prefixed to body — which is
//! the single highest-leverage trick in the pipeline: it gives an isolated
//! chunk the context the reader had.

use serde::{Deserialize, Serialize};

/// Cap on one chunk's body, in characters — ~450 tokens at ~4 chars/token.
/// Sections longer than this split at paragraph boundaries.
pub const MAX_CHUNK_CHARS: usize = 1800;
/// Overlap carried between consecutive splits of one long section (~15% of
/// [`MAX_CHUNK_CHARS`]), so a thought straddling a split lands whole in at
/// least one chunk.
pub const OVERLAP_CHARS: usize = 270;

/// One embeddable unit of a well. Serialized verbatim as a `chunks.jsonl`
/// line, so a search hit can point back at an exact place in a source file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chunk {
    /// The entry's kind: `"note"`, `"wiki"`, `"task"`, or `"goal"` — the same
    /// strings `SearchHit` uses, so ids round-trip.
    pub kind: String,
    /// The entry's id (note path sans `.md`, wiki slug, task/goal slug).
    pub entry_id: String,
    /// `"Entry title › H2 › H3"` — the entry's display title, then the
    /// enclosing heading chain. Never empty: a chunk before any heading is
    /// just the title.
    pub heading_path: String,
    /// Byte offsets of `text` within the source file, when the chunk is a
    /// literal slice of it (markdown sections). `None` for composed chunks
    /// (tasks/goals, whose text interleaves frontmatter fields with the body).
    pub byte_range: Option<(usize, usize)>,
    /// The chunk body — the raw section text, without the heading-path prefix.
    pub text: String,
}

impl Chunk {
    /// What actually gets embedded: the heading path, a blank line, the body.
    /// The index build path and the query path must agree on this composition,
    /// which is why it lives on the type rather than at a call site.
    pub fn embedding_text(&self) -> String {
        format!("{}\n\n{}", self.heading_path, self.text)
    }
}

/// Chunk a note's or wiki page's markdown `source` into heading-scoped
/// sections, splitting any section over [`MAX_CHUNK_CHARS`] at paragraph
/// boundaries with [`OVERLAP_CHARS`] of overlap. Whitespace-only sources
/// yield no chunks; fenced code blocks are never split and their contents are
/// never treated as structure.
pub fn chunk_markdown(_kind: &str, _entry_id: &str, _title: &str, _source: &str) -> Vec<Chunk> {
    todo!("P2 wave 1: the chunker (see docs/mcp-server.md §6.2)")
}

/// Chunk a task or goal: title + tags + linked-goal title + body composed as
/// one chunk (they're short — §6.2). `byte_range` is `None`: the text is
/// composed, not a file slice.
pub fn chunk_task(
    _kind: &str,
    _entry_id: &str,
    _title: &str,
    _tags: &[String],
    _goal_title: Option<&str>,
    _body: &str,
) -> Vec<Chunk> {
    todo!("P2 wave 1: the chunker (see docs/mcp-server.md §6.2)")
}
