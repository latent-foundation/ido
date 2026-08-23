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

use std::ops::Range;

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
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
pub fn chunk_markdown(kind: &str, entry_id: &str, title: &str, source: &str) -> Vec<Chunk> {
    if source.trim().is_empty() {
        return Vec::new();
    }

    let blocks = top_level_blocks(source);
    let sections = sections_from_blocks(title, &blocks);

    let mut out = Vec::new();
    for section in &sections {
        if section.blocks.is_empty() {
            continue; // no content directly under this heading (e.g. two headings back to back)
        }
        for (raw_start, raw_end) in split_blocks(source, &section.blocks) {
            // A block's own span (from pulldown-cmark) can carry a single
            // trailing newline; trim it (and any other leading/trailing
            // blank-line whitespace) so `text` never opens or closes on one.
            let Some((start, end)) = trim_range(source, raw_start, raw_end) else {
                continue; // defensive: blocks are never blank, but never emit a blank chunk
            };
            let text = source[start..end].to_string();
            out.push(Chunk {
                kind: kind.to_string(),
                entry_id: entry_id.to_string(),
                heading_path: section.heading_path.clone(),
                byte_range: Some((start, end)),
                text,
            });
        }
    }
    out
}

/// Chunk a task or goal: title + tags + linked-goal title + body composed as
/// one chunk (they're short — §6.2). `byte_range` is `None`: the text is
/// composed, not a file slice.
pub fn chunk_task(
    kind: &str,
    entry_id: &str,
    title: &str,
    tags: &[String],
    goal_title: Option<&str>,
    body: &str,
) -> Vec<Chunk> {
    // Named per §6.2's contract: an otherwise-empty task (no title, no body,
    // no tags) carries nothing worth embedding, even if it happens to name a
    // goal — in practice this never fires, since a real task's title always
    // falls back to its id.
    if title.trim().is_empty() && body.trim().is_empty() && tags.is_empty() {
        return Vec::new();
    }

    let mut lines = vec![title.to_string()];
    if !tags.is_empty() {
        lines.push(format!("tags: {}", tags.join(", ")));
    }
    if let Some(goal) = goal_title {
        lines.push(format!("goal: {goal}"));
    }
    let body = body.trim();
    if !body.is_empty() {
        lines.push(String::new()); // blank separator before the body
        lines.push(body.to_string());
    }

    vec![Chunk {
        kind: kind.to_string(),
        entry_id: entry_id.to_string(),
        heading_path: title.to_string(),
        byte_range: None,
        text: lines.join("\n"),
    }]
}

// --- markdown segmentation --------------------------------------------------
//
// Everything below is pure and private: one pass over pulldown-cmark's offset
// iterator builds `TopBlock`s (§6.2's "block-level event offsets"), which two
// more pure passes turn into heading-scoped `Section`s and then char-capped,
// overlapping byte ranges. No step re-parses or line-scans, and a fenced code
// block is exactly one `TopBlock` — so it can never be mistaken for headings
// or split, the same instinct `tasks::count_checks` uses for `- [ ]`.

/// One top-level (depth-0) block of the document: a paragraph, list, fenced
/// code block, table, thematic break, etc. — plus, when it's a heading, the
/// heading's own level and text (for [`Chunk::heading_path`] chaining).
/// `range` is the block's exact byte span in `source`: pulldown-cmark's
/// offset iterator gives the *same*, whole-element range for a container's
/// `Start` and `End` events (verified against 0.13's tree-based
/// implementation — the span is fixed once at tree-build time), so capturing
/// it at `End` is exact.
struct TopBlock {
    range: Range<usize>,
    heading: Option<(HeadingLevel, String)>,
}

/// Walk `source` once via [`Parser::into_offset_iter`], tracking container
/// nesting depth so only depth-0 (top-level) elements become [`TopBlock`]s.
/// That's what makes a fenced code block one atomic unit no matter how many
/// blank lines or `#`-prefixed lines it contains, and what keeps a heading
/// nested inside a blockquote or list item from being mistaken for document
/// structure (only a genuinely top-level heading opens a new section). A
/// top-level heading's inline text (`Text`/`Code`, breaks folded to a space)
/// is collected while it's open, to become part of `heading_path`.
fn top_level_blocks(source: &str) -> Vec<TopBlock> {
    // Mirrors the frontend's rendering options (`src/markdown.rs`) minus
    // smart punctuation — heading text feeds search relevance, so it should
    // stay exactly what the user typed rather than typographically rewritten.
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_MATH;

    let mut blocks = Vec::new();
    let mut depth: i32 = 0;
    let mut in_heading = false;
    let mut heading_text = String::new();

    for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                if depth == 0 && matches!(tag, Tag::Heading { .. }) {
                    in_heading = true;
                    heading_text.clear();
                }
                depth += 1;
            }
            Event::End(tag_end) => {
                depth -= 1;
                if depth == 0 {
                    let heading = match tag_end {
                        TagEnd::Heading(level) => {
                            in_heading = false;
                            Some((level, heading_text.trim().to_string()))
                        }
                        _ => None,
                    };
                    blocks.push(TopBlock { range, heading });
                }
            }
            Event::Text(t) | Event::Code(t) if in_heading => heading_text.push_str(&t),
            Event::SoftBreak | Event::HardBreak if in_heading => heading_text.push(' '),
            _ => {
                // A leaf at depth 0 (a thematic break is the practical case)
                // is a top-level block all by itself.
                if depth == 0 {
                    blocks.push(TopBlock {
                        range,
                        heading: None,
                    });
                }
            }
        }
    }
    blocks
}

/// One heading-scoped section: everything up to the next heading (of any
/// level), plus the [`Chunk::heading_path`] that applies to it. `blocks`
/// holds the section's own content blocks in source order — never the
/// heading that opens it, since that's already folded into `heading_path`.
struct Section {
    heading_path: String,
    blocks: Vec<Range<usize>>,
}

/// Partition `blocks` into [`Section`]s on heading boundaries, maintaining a
/// heading stack by level so a deeper heading's path chains through its
/// ancestors and a sibling-or-shallower heading pops back to the right point
/// first (§6.2: `"Auth rewrite › Storage › Session tokens"`). Content before
/// the first heading gets `heading_path = title` alone.
fn sections_from_blocks(title: &str, blocks: &[TopBlock]) -> Vec<Section> {
    let mut sections = Vec::new();
    let mut stack: Vec<(HeadingLevel, String)> = Vec::new();
    let mut path = title.to_string();
    let mut pending: Vec<Range<usize>> = Vec::new();

    for block in blocks {
        match &block.heading {
            Some((level, text)) => {
                sections.push(Section {
                    heading_path: path.clone(),
                    blocks: std::mem::take(&mut pending),
                });
                while stack.last().is_some_and(|(top, _)| *top >= *level) {
                    stack.pop();
                }
                stack.push((*level, text.clone()));
                path = std::iter::once(title)
                    .chain(stack.iter().map(|(_, t)| t.as_str()))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" › ");
            }
            None => pending.push(block.range.clone()),
        }
    }
    sections.push(Section {
        heading_path: path,
        blocks: pending,
    });
    sections
}

/// Split one section's content `blocks` into char-capped byte ranges,
/// greedily packing whole blocks under [`MAX_CHUNK_CHARS`] and overlapping
/// consecutive splits by roughly [`OVERLAP_CHARS`]. A block is always kept
/// whole — a single block already over the cap becomes its own oversized
/// chunk — so the only sub-block cut this function ever makes is the overlap
/// restart point when a chunk has no interior block boundary to snap back
/// to (the char-boundary fallback described in §6.2).
fn split_blocks(source: &str, blocks: &[Range<usize>]) -> Vec<(usize, usize)> {
    debug_assert!(!blocks.is_empty());
    let mut out = Vec::new();
    let mut cursor = blocks[0].start;

    loop {
        let k = blocks
            .iter()
            .position(|b| b.end > cursor)
            .expect("cursor is always within the section's block span");
        let chunk_start = cursor;
        let mut m = k;
        let mut chunk_end = blocks[k].end;
        while m + 1 < blocks.len()
            && char_len(source, chunk_start, blocks[m + 1].end) <= MAX_CHUNK_CHARS
        {
            m += 1;
            chunk_end = blocks[m].end;
        }
        out.push((chunk_start, chunk_end));

        if m + 1 >= blocks.len() {
            break;
        }

        // Overlap: repeat roughly the last OVERLAP_CHARS of this chunk,
        // snapped back to the latest interior block boundary at or before
        // the target — or, if this chunk was a single (oversized) block with
        // no interior boundary, fall back to a raw char-safe offset.
        let target = back_n_chars(source, chunk_start, chunk_end, OVERLAP_CHARS);
        let mut next_start = target;
        for b in &blocks[k + 1..m + 1] {
            if b.start <= target {
                next_start = b.start;
            }
        }
        if next_start <= chunk_start {
            // The whole chunk was shorter than the overlap window (or the
            // fallback landed exactly on its start) — force at least one
            // char of forward progress so the loop always terminates.
            next_start = source[chunk_start..chunk_end]
                .char_indices()
                .nth(1)
                .map(|(i, _)| chunk_start + i)
                .unwrap_or(chunk_end);
        }
        cursor = next_start;
    }
    out
}

/// Trim leading/trailing blank-line whitespace from `source[start..end]`,
/// adjusting the byte range to match exactly (never just from the returned
/// `&str`, since the caller needs the range too). `None` when the slice is
/// entirely blank. `str::trim*` only ever moves inward to a char boundary,
/// so this is UTF-8 safe by construction.
fn trim_range(source: &str, start: usize, end: usize) -> Option<(usize, usize)> {
    let slice = &source[start..end];
    if slice.trim().is_empty() {
        return None;
    }
    let lead = slice.len() - slice.trim_start().len();
    let trail = slice.len() - slice.trim_end().len();
    Some((start + lead, end - trail))
}

/// Character count (not byte count — [`MAX_CHUNK_CHARS`] is a char budget)
/// of `source[start..end]`.
fn char_len(source: &str, start: usize, end: usize) -> usize {
    source[start..end].chars().count()
}

/// The byte offset `n` chars back from `end` within `source`, clamped to
/// `floor` and always landing on a char boundary. Fewer than `n` chars
/// available between `floor` and `end` clamps to `floor`.
fn back_n_chars(source: &str, floor: usize, end: usize, n: usize) -> usize {
    if n == 0 {
        return end;
    }
    source[floor..end]
        .char_indices()
        .rev()
        .nth(n - 1)
        .map(|(i, _)| floor + i)
        .unwrap_or(floor)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every markdown chunk's `text` must be exactly the `source` slice its
    /// `byte_range` names — the contract that makes `get_entry`'s `offset`
    /// meaningful. Panics (via the slice index) if a range isn't even
    /// char-boundary-safe.
    fn assert_ranges_are_exact(source: &str, chunks: &[Chunk]) {
        for c in chunks {
            let (s, e) = c.byte_range.expect("markdown chunks carry byte ranges");
            assert_eq!(
                &source[s..e],
                c.text,
                "byte_range must slice out text exactly"
            );
        }
    }

    #[test]
    fn whitespace_only_source_yields_no_chunks() {
        assert!(chunk_markdown("note", "e", "T", "").is_empty());
        assert!(chunk_markdown("note", "e", "T", "   \n\n\t \n").is_empty());
    }

    #[test]
    fn no_headings_gives_a_single_title_only_chunk() {
        let chunks = chunk_markdown("wiki", "page", "Standalone page", "just some body text.");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].kind, "wiki");
        assert_eq!(chunks[0].entry_id, "page");
        assert_eq!(chunks[0].heading_path, "Standalone page");
        assert_eq!(chunks[0].text, "just some body text.");
    }

    #[test]
    fn heading_path_chains_through_nested_levels_and_resets_on_siblings() {
        let source = "\
intro paragraph before any heading

## Storage

storage notes

### Session tokens

token notes

## Networking

networking notes
";
        let chunks = chunk_markdown("note", "auth", "Auth rewrite", source);
        let paths: Vec<&str> = chunks.iter().map(|c| c.heading_path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "Auth rewrite",
                "Auth rewrite › Storage",
                "Auth rewrite › Storage › Session tokens",
                "Auth rewrite › Networking",
            ]
        );
        let texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "intro paragraph before any heading",
                "storage notes",
                "token notes",
                "networking notes",
            ],
            "section text excludes the heading line itself"
        );
        assert_ranges_are_exact(source, &chunks);
    }

    #[test]
    fn fenced_code_is_never_treated_as_structure_or_split() {
        let source = "\
# Real heading

before fence

```text
# not a heading

still inside the fence
```

after fence
";
        let chunks = chunk_markdown("note", "e", "T", source);
        // If the `#` inside the fence (or its blank lines) were mistaken for
        // structure, this would come out as more than one chunk/section.
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].heading_path, "T › Real heading");
        assert!(chunks[0].text.contains("# not a heading"));
        assert!(chunks[0].text.contains("still inside the fence"));
        assert_ranges_are_exact(source, &chunks);
    }

    #[test]
    fn long_section_splits_with_overlap_and_stays_under_cap() {
        // Many small paragraphs, each well under the cap, totalling
        // comfortably over 2x MAX_CHUNK_CHARS so both grouping and overlap
        // get exercised.
        let mut body = String::new();
        for i in 0..40 {
            body.push_str(&format!(
                "Paragraph number {i} with enough padding text to add up over many repeats, padding padding padding padding.\n\n"
            ));
        }
        assert!(
            body.chars().count() > 2 * MAX_CHUNK_CHARS,
            "test fixture too small"
        );

        let source = format!("# Notes\n\n{body}");
        let chunks = chunk_markdown("note", "e", "Doc", &source);
        assert!(chunks.len() > 1, "expected the section to split");

        for c in &chunks {
            assert!(
                c.text.chars().count() <= MAX_CHUNK_CHARS,
                "chunk exceeds MAX_CHUNK_CHARS: {} chars",
                c.text.chars().count()
            );
            assert_eq!(c.heading_path, "Doc › Notes");
        }
        assert_ranges_are_exact(&source, &chunks);

        for w in chunks.windows(2) {
            let (_, end0) = w[0].byte_range.unwrap();
            let (start1, _) = w[1].byte_range.unwrap();
            assert!(
                start1 < end0,
                "consecutive chunks must overlap in byte space"
            );
            let overlap = &source[start1..end0];
            assert!(
                !overlap.trim().is_empty(),
                "overlap region must carry real content"
            );
            assert!(
                w[0].text.ends_with(overlap),
                "overlap must be the previous chunk's tail"
            );
            assert!(
                w[1].text.starts_with(overlap),
                "overlap must be the next chunk's head"
            );
        }
    }

    #[test]
    fn oversized_atomic_fence_stays_one_chunk() {
        let mut fence_body = String::new();
        while fence_body.chars().count() < MAX_CHUNK_CHARS * 2 {
            fence_body.push_str("let x = 1; // padding line to inflate this fence past the cap\n");
        }
        let source = format!("# Big fence\n\n```text\n{fence_body}```\n");

        let chunks = chunk_markdown("note", "e", "Doc", &source);
        assert_eq!(
            chunks.len(),
            1,
            "an oversized atomic block must stay a single, whole chunk"
        );
        assert!(
            chunks[0].text.chars().count() > MAX_CHUNK_CHARS,
            "the whole fence must be kept, never cut"
        );
        assert!(chunks[0].text.contains("```"));
        assert_ranges_are_exact(&source, &chunks);
    }

    #[test]
    fn utf8_safe_near_split_boundaries() {
        // Accents, CJK, and an emoji ZWJ sequence packed densely so a naive
        // byte-offset split would very likely land mid-codepoint.
        let unit = "café 世界 🎉🏳️‍🌈 naïve résumé 日本語 ";
        let mut body = String::new();
        while body.chars().count() < MAX_CHUNK_CHARS * 3 {
            body.push_str(unit);
        }
        let source = format!("# Ünïcödé\n\n{body}\n");

        // The real assertion is that this doesn't panic; the loop below just
        // confirms why (every range lands on a char boundary).
        let chunks = chunk_markdown("note", "e", "Dôc", &source);
        assert!(!chunks.is_empty());
        for c in &chunks {
            let (s, e) = c.byte_range.unwrap();
            assert!(
                source.is_char_boundary(s),
                "range start must be a char boundary"
            );
            assert!(
                source.is_char_boundary(e),
                "range end must be a char boundary"
            );
        }
        assert_ranges_are_exact(&source, &chunks);
    }

    #[test]
    fn chunk_task_composes_title_tags_goal_and_body() {
        let tags = vec!["urgent".to_string(), "backend".to_string()];
        let chunks = chunk_task(
            "task",
            "fix-auth",
            "Fix auth bug",
            &tags,
            Some("Ship v2"),
            "steps:\n1. repro\n2. fix\n",
        );
        assert_eq!(chunks.len(), 1);
        let c = &chunks[0];
        assert_eq!(c.kind, "task");
        assert_eq!(c.entry_id, "fix-auth");
        assert_eq!(c.heading_path, "Fix auth bug");
        assert_eq!(c.byte_range, None);
        assert_eq!(
            c.text,
            "Fix auth bug\ntags: urgent, backend\ngoal: Ship v2\n\nsteps:\n1. repro\n2. fix"
        );
    }

    #[test]
    fn chunk_task_title_only_when_tags_goal_body_all_absent() {
        let chunks = chunk_task("task", "id", "Just a title", &[], None, "");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "Just a title");
        assert_eq!(chunks[0].heading_path, "Just a title");
        assert_eq!(chunks[0].byte_range, None);
    }

    #[test]
    fn chunk_task_empty_everything_yields_no_chunk() {
        assert!(chunk_task("task", "id", "", &[], None, "").is_empty());
        assert!(chunk_task("task", "id", "   ", &[], None, "  \n").is_empty());
    }

    #[test]
    fn chunk_task_goal_alone_does_not_rescue_an_otherwise_empty_task() {
        // §6.2's contract names title/body/tags for the empty check; a lone
        // goal_title with everything else blank still yields nothing under
        // that literal reading. Never actually happens in practice — a real
        // task's title always falls back to its id — but the behavior is
        // deliberate, not an oversight.
        assert!(chunk_task("task", "id", "", &[], Some("Some goal"), "").is_empty());
    }

    #[test]
    fn embedding_text_is_heading_path_then_blank_line_then_text() {
        let c = Chunk {
            kind: "note".into(),
            entry_id: "x".into(),
            heading_path: "Doc › Section".into(),
            byte_range: None,
            text: "body text".into(),
        };
        assert_eq!(c.embedding_text(), "Doc › Section\n\nbody text");
    }
}
