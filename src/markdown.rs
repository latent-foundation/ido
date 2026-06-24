//! Markdown → HTML rendering for the reading view.
//!
//! Rendering happens here, in the frontend (WASM), so the preview updates
//! instantly without a backend round-trip. Output is dropped into a
//! `.markdown-body` element via `inner_html` and styled by `style/app.css`.
//!
//! ## Extending this (the future wiki/task surfaces)
//!
//! [`render`] parses to a stream of [`Event`]s and maps over them before
//! emitting HTML. That `Event` map is the seam for everything ido will grow:
//! - `[[wikilinks]]` → rewrite `Event::Text` runs into internal links
//! - `#tags` → wrap matches in styled spans
//! - task checkboxes, embeds, callouts, a backlinks index, …
//!
//! Add a transform by extending [`transform`]; keep each concern its own small
//! function so the pipeline stays readable.

use pulldown_cmark::{html, CowStr, Event, Options, Parser};

/// CommonMark plus the GitHub-flavoured extensions notes tend to use.
fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_SMART_PUNCTUATION
        | Options::ENABLE_MATH
}

/// Render markdown `src` to an HTML string for the reading view.
pub fn render(src: &str) -> String {
    let events = Parser::new_ext(src, options()).map(transform);
    let mut html_out = String::new();
    html::push_html(&mut html_out, events);
    html_out
}

/// Per-event transform — the single place to add wiki/task/tag behaviour later.
///
/// Math (`$…$` / `$$…$$`) is converted to MathML via `latex2mathml` and emitted
/// as trusted inline/block HTML *before* the HTML-sanitisation arms run — the
/// two concerns are handled in order in a single match so no event is transformed
/// twice.
///
/// Raw HTML in notes is shown as literal text rather than being executed: a stray
/// `<img onerror=…>` could reach the Tauri IPC bridge, so we treat
/// user-authored HTML as content, not markup.
fn transform(event: Event) -> Event {
    match event {
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
        // Sanitise user-authored raw HTML (never execute it inside the webview).
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        other => other,
    }
}
