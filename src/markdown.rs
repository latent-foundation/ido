//! Markdown → HTML rendering for the reading view.
//!
//! Rendering happens here, in the frontend (WASM), so the preview updates
//! instantly without a backend round-trip. Output is dropped into a
//! `.markdown-body` element via `inner_html` and styled by `style/app.css`.
//!
//! ## Wikilinks and the transform seam
//!
//! `[[slug]]` / `[[slug|label]]` are expanded to internal links before parsing
//! (see [`expand_wikilinks`]). [`render`] then parses to a stream of [`Event`]s
//! and maps [`transform`] over them — the seam for everything else ido grows:
//! - `#tags` → wrap matches in styled spans
//! - task checkboxes, embeds, callouts, a backlinks index, …
//!
//! Add a concern by extending [`transform`] (or, like wikilinks, preprocessing);
//! keep each its own small function so the pipeline stays readable.

use pulldown_cmark::{html, CowStr, Event, Options, Parser, Tag};

/// CommonMark plus the GitHub-flavoured extensions notes tend to use.
fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_SMART_PUNCTUATION
        | Options::ENABLE_MATH
}

/// The internal URL scheme a `[[wikilink]]` expands to. The reading view
/// intercepts clicks on these (see `components::mainpane`) and opens the page
/// rather than letting the webview navigate; CSS styles `a[href^=…]` distinctly.
pub const WIKI_HREF: &str = "ido:wiki/";

/// Slugify a wiki target the same way the backend does (`paths::slugify`):
/// lowercase, runs of non-alphanumerics collapse to `-`, ends trimmed. Kept in
/// sync by construction — both are tiny and pure (separate crates can't share).
pub fn slugify(title: &str) -> String {
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

/// Rewrite `[[slug]]` / `[[slug|label]]` into standard markdown links
/// (`[label](ido:wiki/slug)`) *before* parsing, so pulldown-cmark renders them
/// as ordinary anchors. Preprocessing (rather than a per-event transform) keeps
/// `[[…]]` from being split across text events; the trade-off is that `[[x]]`
/// inside code spans is also expanded, which is rare in practice.
fn expand_wikilinks(src: &str) -> String {
    if !src.contains("[[") {
        return src.to_string();
    }
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(i) = rest.find("[[") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        if let Some(j) = after.find("]]") {
            let inner = &after[..j];
            let (target, label) = match inner.split_once('|') {
                Some((t, l)) => (t.trim(), l.trim()),
                None => (inner.trim(), inner.trim()),
            };
            let slug = slugify(target);
            if !inner.contains('[') && !inner.contains('\n') && !slug.is_empty() {
                out.push('[');
                out.push_str(label);
                out.push_str("](");
                out.push_str(WIKI_HREF);
                out.push_str(&slug);
                out.push(')');
                rest = &after[j + 2..];
                continue;
            }
        }
        // Not a valid wikilink — keep the literal "[[" and continue past it.
        out.push_str("[[");
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Render markdown `src` to an HTML string for the reading view.
///
/// `resolve_asset` maps a well-relative image reference — `assets/foo.png`, as
/// embedded by `![](…)` when an image is pasted/dropped into the editor — to a
/// renderable URL (a `data:` URI in practice; see `State::resolve_asset`).
/// Returning `None` leaves the raw reference in place (the browser shows a
/// broken-image icon until a later re-render resolves it); absolute URLs
/// (`http(s)://`, `data:`) always pass through unchanged.
pub fn render(src: &str, resolve_asset: impl Fn(&str) -> Option<String>) -> String {
    let expanded = expand_wikilinks(src);
    // Numbers the task-list checkboxes in document order, so a click can be
    // traced back to the right marker (see `toggle_checkbox`).
    let checkboxes = std::cell::Cell::new(0usize);
    let events =
        Parser::new_ext(&expanded, options()).map(|ev| transform(ev, &resolve_asset, &checkboxes));
    let mut html_out = String::new();
    html::push_html(&mut html_out, events);
    html_out
}

/// Flip the `idx`-th task-list checkbox (`[ ]` ↔ `[x]`) in `src`, counting
/// markers in document order — the same numbering [`render`] stamps on the
/// rendered inputs. The marker is located through the parser's own source
/// offsets, so text that merely *looks* like a marker (say, inside a code
/// fence) is never touched. `None` when no such marker exists.
///
/// Wikilink expansion can't shift the numbering: it rewrites `[[…]]` spans
/// only, which are never task-list markers, so the expanded source [`render`]
/// parses and the raw source this edits agree on marker order.
pub fn toggle_checkbox(src: &str, idx: usize) -> Option<String> {
    let range = Parser::new_ext(src, options())
        .into_offset_iter()
        .filter_map(|(ev, range)| matches!(ev, Event::TaskListMarker(_)).then_some(range))
        .nth(idx)?;
    // The range spans the `[x]` marker; the state character sits after the `[`.
    let open = src[range.clone()].find('[')?;
    let state_pos = range.start + open + 1;
    let flipped = match src.get(state_pos..state_pos + 1)? {
        " " => "x",
        _ => " ",
    };
    let mut out = String::with_capacity(src.len());
    out.push_str(&src[..state_pos]);
    out.push_str(flipped);
    out.push_str(&src[state_pos + 1..]);
    Some(out)
}

/// Per-event transform — the single place to add wiki/task/tag behaviour later.
///
/// Math (`$…$` / `$$…$$`) is converted to MathML via `latex2mathml` and emitted
/// as trusted inline/block HTML *before* the HTML-sanitisation arms run — the
/// two concerns are handled in order in a single match so no event is transformed
/// twice. Task-list markers are emitted the same trusted way: as *enabled*
/// checkboxes numbered by `checkboxes` (the default writer's are disabled), so
/// a click can toggle the marker in the source (see [`toggle_checkbox`]).
///
/// Raw HTML in notes is shown as literal text rather than being executed: a stray
/// `<img onerror=…>` could reach the Tauri IPC bridge, so we treat
/// user-authored HTML as content, not markup.
fn transform<'a>(
    event: Event<'a>,
    resolve_asset: &impl Fn(&str) -> Option<String>,
    checkboxes: &std::cell::Cell<usize>,
) -> Event<'a> {
    match event {
        // Task-list markers: a real checkbox carrying its document-order index.
        Event::TaskListMarker(checked) => {
            let idx = checkboxes.get();
            checkboxes.set(idx + 1);
            let checked = if checked { " checked" } else { "" };
            Event::InlineHtml(CowStr::from(format!(
                r#"<input type="checkbox" class="ido-check" data-check-idx="{idx}"{checked}>"#
            )))
        }
        // Math: convert LaTeX → MathML and emit as trusted HTML before sanitisation.
        Event::InlineMath(latex) => {
            let src = latex.trim();
            let ml = latex2mathml::latex_to_mathml(src, latex2mathml::DisplayStyle::Inline)
                .unwrap_or_else(|_| format!("<code>${src}$</code>"));
            Event::InlineHtml(CowStr::from(ml))
        }
        Event::DisplayMath(latex) => {
            let src = latex.trim();
            let ml = latex2mathml::latex_to_mathml(src, latex2mathml::DisplayStyle::Block)
                .unwrap_or_else(|_| format!("<pre>$$\n{src}\n$$</pre>"));
            Event::Html(CowStr::from(ml))
        }
        // Resolve an inline image's `src` before the HTML writer renders it.
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => {
            let resolved = resolve_image_src(&dest_url, resolve_asset);
            Event::Start(Tag::Image {
                link_type,
                dest_url: CowStr::from(resolved),
                title,
                id,
            })
        }
        // Sanitise user-authored raw HTML (never execute it inside the webview).
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        other => other,
    }
}

/// Resolve an image's markdown destination for rendering: absolute URLs pass
/// through unchanged; anything else is a well-relative asset id (as saved by
/// the backend's `save_asset`) and is looked up via `resolve_asset`, falling
/// back to the raw path when it isn't resolved (yet).
fn resolve_image_src(dest_url: &str, resolve_asset: &impl Fn(&str) -> Option<String>) -> String {
    if dest_url.starts_with("http://")
        || dest_url.starts_with("https://")
        || dest_url.starts_with("data:")
    {
        return dest_url.to_string();
    }
    resolve_asset(dest_url).unwrap_or_else(|| dest_url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_matches_backend_rules() {
        assert_eq!(slugify("Auth System"), "auth-system");
        assert_eq!(slugify("  Hello, World!  "), "hello-world");
        assert_eq!(slugify("!!!"), "");
    }

    #[test]
    fn expand_simple_and_labelled_wikilinks() {
        assert_eq!(
            expand_wikilinks("see [[Auth System]] now"),
            "see [Auth System](ido:wiki/auth-system) now"
        );
        assert_eq!(
            expand_wikilinks("see [[auth-system|the hub]]"),
            "see [the hub](ido:wiki/auth-system)"
        );
    }

    #[test]
    fn unterminated_or_empty_wikilink_left_literal() {
        assert_eq!(expand_wikilinks("a [[ b"), "a [[ b");
        assert_eq!(expand_wikilinks("[[ !!! ]]"), "[[ !!! ]]");
        assert_eq!(expand_wikilinks("no links here"), "no links here");
    }

    /// A resolver that never resolves — used by tests that don't care about images.
    fn no_assets(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn render_emits_wiki_anchor() {
        let html = render("see [[Auth System]]", no_assets);
        assert!(
            html.contains(r#"href="ido:wiki/auth-system""#),
            "html: {html}"
        );
        assert!(html.contains(">Auth System</a>"), "html: {html}");
    }

    #[test]
    fn image_src_resolved_via_callback() {
        let html = render("![alt](assets/foo.png)", |id| {
            (id == "assets/foo.png").then(|| "data:image/png;base64,AAAA".to_string())
        });
        assert!(
            html.contains(r#"src="data:image/png;base64,AAAA""#),
            "html: {html}"
        );
    }

    #[test]
    fn unresolved_image_keeps_raw_path() {
        let html = render("![alt](assets/missing.png)", no_assets);
        assert!(html.contains(r#"src="assets/missing.png""#), "html: {html}");
    }

    #[test]
    fn external_image_url_passes_through() {
        let html = render("![alt](https://example.com/x.png)", no_assets);
        assert!(
            html.contains(r#"src="https://example.com/x.png""#),
            "html: {html}"
        );
    }

    #[test]
    fn checkboxes_render_enabled_and_numbered() {
        let html = render("- [ ] one\n- [x] two", no_assets);
        assert!(
            html.contains(r#"<input type="checkbox" class="ido-check" data-check-idx="0">"#),
            "html: {html}"
        );
        assert!(
            html.contains(
                r#"<input type="checkbox" class="ido-check" data-check-idx="1" checked>"#
            ),
            "html: {html}"
        );
        assert!(!html.contains("disabled"), "html: {html}");
    }

    #[test]
    fn toggle_checkbox_flips_the_nth_marker() {
        let src = "- [ ] one\n- [x] two";
        assert_eq!(toggle_checkbox(src, 0).unwrap(), "- [x] one\n- [x] two");
        assert_eq!(toggle_checkbox(src, 1).unwrap(), "- [ ] one\n- [ ] two");
        assert!(toggle_checkbox(src, 2).is_none());
    }

    #[test]
    fn toggle_checkbox_ignores_markers_inside_code() {
        // The fenced "- [ ]" is code, not a marker — index 0 is the real item.
        let src = "```\n- [ ] not a task\n```\n\n- [ ] real";
        assert_eq!(
            toggle_checkbox(src, 0).unwrap(),
            "```\n- [ ] not a task\n```\n\n- [x] real"
        );
        assert!(toggle_checkbox(src, 1).is_none());
    }

    #[test]
    fn toggle_checkbox_survives_wikilinks_upstream() {
        // A wikilink before the marker must not shift the numbering between the
        // rendered (expanded) source and the raw source being toggled.
        let src = "see [[Auth System]]\n\n- [ ] follow up";
        assert_eq!(
            toggle_checkbox(src, 0).unwrap(),
            "see [[Auth System]]\n\n- [x] follow up"
        );
    }
}
