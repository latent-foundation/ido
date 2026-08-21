//! A minimal, hand-rolled `--- key: value ---` frontmatter parser (zero deps).
//!
//! Deliberately tiny: a leading `---` line opens the header, a `---` line closes
//! it, and each line between is a flat `key: value` pair. Everything after the
//! closing fence is the body. Values are scalar strings; the caller interprets
//! them (e.g. splitting `tags` on commas). Keys not understood by the UI are
//! preserved on a [`parse`] → mutate → [`merge`] round-trip.

use std::collections::BTreeMap;

/// Parse a document's leading frontmatter into `(fields, body)`. With no valid
/// `--- … ---` header, `fields` is empty and `body` is the whole input.
///
/// The body is sliced from the original source (byte-accurate, line endings
/// preserved), minus the blank line(s) between the closing fence and the body.
pub fn parse(src: &str) -> (BTreeMap<String, String>, String) {
    let empty = BTreeMap::new();
    let Some(after_open) = src
        .strip_prefix("---\n")
        .or_else(|| src.strip_prefix("---\r\n"))
    else {
        return (empty, src.to_string());
    };

    // Walk header lines (keeping byte offsets) until a `---` line closes it.
    let mut offset = src.len() - after_open.len();
    let mut header = String::new();
    let mut body_start = None;
    for line in after_open.split_inclusive('\n') {
        if line.trim_end_matches(['\n', '\r']) == "---" {
            body_start = Some(offset + line.len());
            break;
        }
        header.push_str(line);
        offset += line.len();
    }
    let Some(body_start) = body_start else {
        // No closing fence — not frontmatter after all.
        return (empty, src.to_string());
    };

    let mut fields = BTreeMap::new();
    for line in header.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim();
            if !k.is_empty() {
                fields.insert(k.to_string(), v.trim().to_string());
            }
        }
    }
    let body = src[body_start..]
        .trim_start_matches(['\n', '\r'])
        .to_string();
    (fields, body)
}

/// Re-emit `fields` + `body` as a document. With no fields, returns the body
/// unchanged (no empty `--- ---` block). Keys are emitted sorted (BTreeMap), so
/// output is deterministic.
pub fn merge(fields: &BTreeMap<String, String>, body: &str) -> String {
    if fields.is_empty() {
        return body.to_string();
    }
    let mut out = String::from("---\n");
    for (k, v) in fields {
        out.push_str(k);
        out.push_str(": ");
        out.push_str(v);
        out.push('\n');
    }
    out.push_str("---\n\n");
    out.push_str(body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_frontmatter_is_all_body() {
        let (f, b) = parse("# Title\n\nbody");
        assert!(f.is_empty());
        assert_eq!(b, "# Title\n\nbody");
    }

    #[test]
    fn parses_fields_and_body() {
        let (f, b) = parse("---\nstatus: doing\ntags: a, b\n---\n\n# Hi\n\ntext");
        assert_eq!(f.get("status").unwrap(), "doing");
        assert_eq!(f.get("tags").unwrap(), "a, b");
        assert_eq!(b, "# Hi\n\ntext");
    }

    #[test]
    fn unterminated_header_is_body() {
        // No closing fence → treat the whole thing as body.
        let (f, b) = parse("---\nstatus: doing\n\nbody");
        assert!(f.is_empty());
        assert_eq!(b, "---\nstatus: doing\n\nbody");
    }

    #[test]
    fn round_trip_preserves_unknown_keys() {
        let (mut f, b) = parse("---\nstatus: todo\ncustom: keep-me\n---\n\nbody");
        f.insert("status".into(), "done".into());
        let out = merge(&f, &b);
        let (f2, b2) = parse(&out);
        assert_eq!(f2.get("status").unwrap(), "done");
        assert_eq!(f2.get("custom").unwrap(), "keep-me");
        assert_eq!(b2, "body");
    }

    #[test]
    fn empty_fields_emit_no_header() {
        assert_eq!(merge(&BTreeMap::new(), "just body"), "just body");
    }
}
