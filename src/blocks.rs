//! Block segmentation for the live editor.
//!
//! [`segment`] splits a markdown source string into top-level blocks using
//! `pulldown-cmark`'s offset iterator. Each [`Block`] carries its raw source
//! slice and its byte range in the parent document — so editing a single block
//! splices back precisely into the source without touching anything else.
//!
//! ## How it works
//!
//! `pulldown-cmark`'s `into_offset_iter()` yields `(Event, Range<usize>)` where
//! the range is the byte span of that event in the original source string. We walk
//! depth-0 `Start`/`End` pairs (and standalone `Rule`/`Html` events) to extract
//! each top-level block's exact source range.
//!
//! Use [`splice`] to commit an edited block back into the full document source,
//! then call [`segment`] again on the result to get fresh block ranges.

use std::ops::Range;

use pulldown_cmark::{Event, Options, Parser};

/// A single top-level markdown block and its position in the document source.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    /// Raw markdown source for this block (does not include inter-block whitespace).
    pub src: String,
    /// Byte range of this block within the full document source.
    pub range: Range<usize>,
}

fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_SMART_PUNCTUATION
        | Options::ENABLE_MATH
}

/// Segment `src` into top-level blocks.
///
/// Returns an empty `Vec` for empty or whitespace-only input.
/// The inter-block whitespace (blank lines separating blocks) is **not** part
/// of any block's source; [`splice`] preserves it automatically.
pub fn segment(src: &str) -> Vec<Block> {
    let iter = Parser::new_ext(src, options()).into_offset_iter();
    let mut blocks: Vec<Block> = Vec::new();
    let mut depth: usize = 0;
    let mut block_start: usize = 0;

    for (event, range) in iter {
        match event {
            Event::Start(_) => {
                if depth == 0 {
                    block_start = range.start;
                }
                depth += 1;
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let trimmed = src[block_start..range.end].trim_end();
                    if !trimmed.is_empty() {
                        // Use trimmed length, not range.end — pulldown-cmark's End
                        // range includes the trailing newline, which would eat the
                        // inter-block separator on splice.
                        blocks.push(Block {
                            src: trimmed.to_string(),
                            range: block_start..(block_start + trimmed.len()),
                        });
                    }
                }
            }
            // Standalone (non-paired) top-level events.
            Event::Rule | Event::Html(_) | Event::DisplayMath(_) if depth == 0 => {
                let trimmed = src[range.start..range.end].trim_end();
                if !trimmed.is_empty() {
                    blocks.push(Block {
                        src: trimmed.to_string(),
                        range: range.start..(range.start + trimmed.len()),
                    });
                }
            }
            _ => {}
        }
    }

    blocks
}

/// Splice `new_block_src` into `src` at the position occupied by `block`,
/// producing a new document string.
///
/// Call [`segment`] on the result to get updated block ranges.
pub fn splice(src: &str, block: &Block, new_block_src: &str) -> String {
    format!(
        "{}{}{}",
        &src[..block.range.start],
        new_block_src,
        &src[block.range.end..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input() {
        assert!(segment("").is_empty());
        assert!(segment("   \n  ").is_empty());
    }

    #[test]
    fn single_paragraph() {
        let src = "Hello world";
        let blocks = segment(src);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].src, "Hello world");
    }

    #[test]
    fn two_paragraphs() {
        let src = "First.\n\nSecond.";
        let blocks = segment(src);
        assert_eq!(blocks.len(), 2, "blocks: {blocks:?}");
        assert_eq!(blocks[0].src, "First.");
        assert_eq!(blocks[1].src, "Second.");
    }

    #[test]
    fn heading_and_paragraph() {
        let src = "# Title\n\nBody text.";
        let blocks = segment(src);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].src, "# Title");
        assert_eq!(blocks[1].src, "Body text.");
    }

    #[test]
    fn code_block_not_split_on_blank_line() {
        let src = "```rust\nlet x = 1;\n\nlet y = 2;\n```\n\nAfter.";
        let blocks = segment(src);
        assert_eq!(blocks.len(), 2, "blocks: {blocks:?}");
        assert!(blocks[0].src.contains("let x"));
        assert!(blocks[0].src.contains("let y"));
        assert_eq!(blocks[1].src, "After.");
    }

    #[test]
    fn blockquote_is_one_block() {
        let src = "> quoted\n> continued\n\nNormal.";
        let blocks = segment(src);
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].src.starts_with('>'));
        assert_eq!(blocks[1].src, "Normal.");
    }

    #[test]
    fn splice_replaces_middle_block() {
        let src = "First.\n\nSecond.\n\nThird.";
        let blocks = segment(src);
        assert_eq!(blocks.len(), 3, "blocks: {blocks:?}");
        let new_src = splice(src, &blocks[1], "Edited.");
        let new_blocks = segment(&new_src);
        assert_eq!(new_blocks.len(), 3, "new_blocks: {new_blocks:?}");
        assert_eq!(new_blocks[0].src, "First.");
        assert_eq!(new_blocks[1].src, "Edited.");
        assert_eq!(new_blocks[2].src, "Third.");
    }

    #[test]
    fn splice_preserves_separators() {
        let src = "A.\n\nB.";
        let blocks = segment(src);
        let new_src = splice(src, &blocks[0], "New.");
        // The blank-line separator must survive so the two paragraphs remain separate.
        let new_blocks = segment(&new_src);
        assert_eq!(new_blocks.len(), 2, "new_src: {new_src:?}");
        assert_eq!(new_blocks[0].src, "New.");
        assert_eq!(new_blocks[1].src, "B.");
    }

    #[test]
    fn display_math_standalone() {
        let src = "$$\nx^2\n$$";
        let blocks = segment(src);
        assert_eq!(blocks.len(), 1, "blocks: {blocks:?}");
        assert!(
            blocks[0].src.contains("$$"),
            "block src: {:?}",
            blocks[0].src
        );
    }

    #[test]
    fn display_math_between_paragraphs() {
        let src = "Before.\n\n$$\nx^2\n$$\n\nAfter.";
        let blocks = segment(src);
        assert_eq!(blocks.len(), 3, "blocks: {blocks:?}");
        assert_eq!(blocks[0].src, "Before.");
        assert!(
            blocks[1].src.starts_with("$$"),
            "math block: {:?}",
            blocks[1].src
        );
        assert_eq!(blocks[2].src, "After.");
    }

    #[test]
    fn inline_math_stays_in_paragraph() {
        let src = "Some $x^2$ text.";
        let blocks = segment(src);
        // Inline math doesn't split the paragraph — stays one block.
        assert_eq!(blocks.len(), 1, "blocks: {blocks:?}");
        assert_eq!(blocks[0].src, "Some $x^2$ text.");
    }
}
