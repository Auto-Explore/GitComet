use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HtmlHandling {
    Ignore,
    HardBreak,
    DetailsSummary(String),
    StartInlineStyle(MarkdownInlineStyle),
    EndInlineStyle(MarkdownInlineStyle),
    /// `<a href>`: link style plus the destination, when the preview can open it.
    StartLink(Option<SharedString>),
    EndLink,
    AppendText(String),
    /// The `<img>` tags a fragment holds, each with the byte offset of its tag
    /// inside that fragment, the `alt` describing it if it cannot be drawn, and
    /// the `<a href>` it sits in within the same fragment.
    Images(Vec<HtmlImage>),
    AppendLiteral,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HtmlImage {
    pub(crate) tag_offset: usize,
    pub(crate) image: MarkdownImage,
    pub(crate) alt: String,
    pub(crate) link_url: Option<SharedString>,
}

pub(crate) fn markdown_alert_kind_from_blockquote_kind(
    kind: pulldown_cmark::BlockQuoteKind,
) -> Option<MarkdownAlertKind> {
    Some(match kind {
        pulldown_cmark::BlockQuoteKind::Note => MarkdownAlertKind::Note,
        pulldown_cmark::BlockQuoteKind::Tip => MarkdownAlertKind::Tip,
        pulldown_cmark::BlockQuoteKind::Important => MarkdownAlertKind::Important,
        pulldown_cmark::BlockQuoteKind::Warning => MarkdownAlertKind::Warning,
        pulldown_cmark::BlockQuoteKind::Caution => MarkdownAlertKind::Caution,
    })
}

pub(crate) fn markdown_parser_options() -> pulldown_cmark::Options {
    pulldown_cmark::Options::ENABLE_TABLES
        | pulldown_cmark::Options::ENABLE_STRIKETHROUGH
        | pulldown_cmark::Options::ENABLE_TASKLISTS
        | pulldown_cmark::Options::ENABLE_FOOTNOTES
        | pulldown_cmark::Options::ENABLE_GFM
}

pub(crate) fn classify_supported_html(html: &str) -> HtmlHandling {
    let trimmed = html.trim();
    if trimmed.is_empty() {
        return HtmlHandling::Ignore;
    }

    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("<!--") {
        return HtmlHandling::Ignore;
    }
    if let Some(summary_source) = extract_html_summary_content(trimmed) {
        return HtmlHandling::DetailsSummary(summary_source);
    }
    let images = extract_html_images(trimmed);
    if !images.is_empty() {
        return HtmlHandling::Images(images);
    }
    if let Some(alt_text) = extract_html_image_alt(trimmed) {
        return HtmlHandling::AppendText(alt_text);
    }
    if matches!(lower.as_str(), "<br>" | "<br/>" | "<br />") {
        return HtmlHandling::HardBreak;
    }
    if matches!(lower.as_str(), "<ins>") {
        return HtmlHandling::StartInlineStyle(MarkdownInlineStyle::Underline);
    }
    if matches!(lower.as_str(), "</ins>") {
        return HtmlHandling::EndInlineStyle(MarkdownInlineStyle::Underline);
    }
    if matches!(lower.as_str(), "<sub>" | "</sub>" | "<sup>" | "</sup>") {
        return HtmlHandling::Ignore;
    }
    if is_html_open_tag(lower.as_str(), "a") {
        // A named anchor (`<a name>`/`<a id>`) is a jump target with nothing
        // to show; one with an `href` is a link like any other.
        return match extract_html_attribute(trimmed, "href") {
            Some(href) => HtmlHandling::StartLink(offered_link_destination(&href)),
            None => HtmlHandling::Ignore,
        };
    }
    if is_html_close_tag(lower.as_str(), "a") {
        return HtmlHandling::EndLink;
    }
    if lower.starts_with("<picture")
        || lower == "</picture>"
        || lower.starts_with("<source")
        || lower == "</source>"
    {
        return HtmlHandling::Ignore;
    }
    if is_html_open_tag(lower.as_str(), "details") || is_html_close_tag(lower.as_str(), "details") {
        return HtmlHandling::Ignore;
    }

    HtmlHandling::AppendLiteral
}

pub(crate) fn is_html_open_tag(lower_html: &str, tag_name: &str) -> bool {
    if !lower_html.starts_with('<') || lower_html.starts_with("</") {
        return false;
    }

    let Some(rest) = lower_html.strip_prefix('<') else {
        return false;
    };
    let Some(rest) = rest.strip_prefix(tag_name) else {
        return false;
    };

    rest.is_empty()
        || rest.starts_with('>')
        || rest.starts_with('/')
        || rest.starts_with(char::is_whitespace)
}

pub(crate) fn is_html_close_tag(lower_html: &str, tag_name: &str) -> bool {
    let Some(rest) = lower_html.strip_prefix("</") else {
        return false;
    };
    let Some(rest) = rest.strip_prefix(tag_name) else {
        return false;
    };

    rest.is_empty() || rest.starts_with('>') || rest.starts_with(char::is_whitespace)
}

pub(crate) fn extract_html_summary_content(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open_ix = lower.find("<summary")?;
    let start_tag_end_rel = html[open_ix..].find('>')?;
    let content_start = open_ix + start_tag_end_rel + 1;
    let close_rel = lower[content_start..].find("</summary>")?;
    Some(html[content_start..content_start + close_rel].to_owned())
}

pub(crate) fn extract_html_image_alt(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let img_ix = lower.find("<img")?;
    extract_html_attribute(&html[img_ix..], "alt")
}

/// The image an `<img>` tag describes, for the tags markdown documents use in
/// place of `![alt](src)` — typically a logo sized with `width`.
/// One fragment often holds several — a row of badges is written as a single
/// block of HTML — so every tag is collected, and each is bounded to its own
/// `>` before its attributes are read so it cannot borrow the next tag's.
pub(crate) fn extract_html_images(html: &str) -> Vec<HtmlImage> {
    let lower = html.to_ascii_lowercase();
    let mut images = Vec::new();
    let mut search_start = 0usize;

    while let Some(offset) = lower[search_start..].find("<img") {
        let tag_start = search_start + offset;
        let tag_end = lower[tag_start..]
            .find('>')
            .map_or(html.len(), |end| tag_start + end + 1);
        search_start = tag_end;

        let tag = &html[tag_start..tag_end];
        let Some(source) = extract_html_attribute(tag, "src") else {
            continue;
        };
        if source.trim().is_empty() {
            continue;
        }
        images.push(HtmlImage {
            tag_offset: tag_start,
            image: MarkdownImage {
                source: source.into(),
                width_px: extract_html_pixel_attribute(tag, "width"),
                height_px: extract_html_pixel_attribute(tag, "height"),
            },
            alt: extract_html_attribute(tag, "alt").unwrap_or_default(),
            link_url: enclosing_html_link(html, &lower, tag_start),
        });
    }

    images
}

/// The destination of the `<a href>` still open at `at` within `html`, as
/// badges written `<a href="…"><img …></a>` in one fragment are.
fn enclosing_html_link(html: &str, lower: &str, at: usize) -> Option<SharedString> {
    let open = lower[..at].rfind("<a ")?;
    if lower[open..at].contains("</a>") {
        return None;
    }
    let tag_end = lower[open..]
        .find('>')
        .map_or(html.len(), |end| open + end + 1);
    offered_link_destination(&extract_html_attribute(&html[open..tag_end], "href")?)
}

/// A `width`/`height` attribute in CSS pixels.
///
/// Percentages and other units describe a size relative to something the
/// preview's fixed row grid does not have, so they are ignored and the image
/// falls back to the default block.
pub(crate) fn extract_html_pixel_attribute(html: &str, name: &str) -> Option<u32> {
    let value = extract_html_attribute(html, name)?;
    let value = value.trim();
    let digits = value.strip_suffix("px").unwrap_or(value).trim();
    digits.parse::<u32>().ok().filter(|px| *px > 0)
}

pub(crate) fn extract_html_attribute(html: &str, name: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let needle = format!("{name}=");
    let mut search_start = 0;

    while let Some(rel_ix) = lower[search_start..].find(&needle) {
        let attr_ix = search_start + rel_ix;
        if attr_ix > 0 {
            let prev = lower.as_bytes()[attr_ix - 1];
            if !prev.is_ascii_whitespace() && prev != b'<' {
                search_start = attr_ix + needle.len();
                continue;
            }
        }

        let value_start = attr_ix + needle.len();
        if value_start >= html.len() {
            return None;
        }

        let value = &html[value_start..];
        let mut chars = value.chars();
        let first = chars.next()?;
        if first == '"' || first == '\'' {
            let end_rel = value[1..].find(first)?;
            return Some(value[1..1 + end_rel].to_owned());
        }

        let end = value
            .find(|c: char| c.is_ascii_whitespace() || matches!(c, '>' | '/'))
            .unwrap_or(value.len());
        return Some(value[..end].to_owned());
    }

    None
}

/// Destination of the innermost link currently open, if it is a web URL.
pub(crate) fn current_link_url(link_stack: &[Option<SharedString>]) -> Option<SharedString> {
    link_stack.last().cloned().flatten()
}

/// Where a link destination points, when the preview can offer it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MarkdownLinkTarget {
    /// An `http(s)://` URL, opened in the browser.
    Web(SharedString),
    /// A path relative to the document (or to the repository root when it
    /// starts with `/`), kept verbatim: fragment and query are stripped when
    /// it is resolved against the tree.
    LocalFile(SharedString),
    /// `#fragment`: a heading in the same document, scrolled to on click.
    /// Holds the fragment without its `#`.
    Anchor(SharedString),
}

/// Classify a link destination as something the preview can act on.
///
/// Protocol-relative URLs, a bare `#`, and flat schemes such as
/// `mailto:`/`javascript:`/`data:` have no meaning here, so they render as
/// links but are never offered. A Windows drive (`C:`) reads as a flat
/// scheme, which is right: an absolute OS path is not a repository file.
pub(crate) fn classify_markdown_link_destination(dest_url: &str) -> Option<MarkdownLinkTarget> {
    let trimmed = dest_url.trim();
    if let Some(fragment) = trimmed.strip_prefix('#') {
        return (!fragment.is_empty())
            .then(|| MarkdownLinkTarget::Anchor(SharedString::from(fragment.to_owned())));
    }
    if trimmed.is_empty() || trimmed.starts_with("//") {
        return None;
    }
    if let Some(scheme_end) = trimmed.find("://") {
        let scheme = &trimmed[..scheme_end];
        return (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
            .then(|| MarkdownLinkTarget::Web(SharedString::from(trimmed.to_owned())));
    }
    if has_flat_scheme(trimmed) {
        return None;
    }
    Some(MarkdownLinkTarget::LocalFile(SharedString::from(
        trimmed.to_owned(),
    )))
}

/// `scheme:` per RFC 3986: a letter, then letters, digits, `+`, `-`, `.`.
fn has_flat_scheme(dest: &str) -> bool {
    let Some(colon) = dest.find(':') else {
        return false;
    };
    let scheme = &dest[..colon];
    let mut chars = scheme.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// The destination a link span carries, as written: a web URL, a local file
/// path, or a `#fragment`; `None` for targets the preview cannot open.
pub(crate) fn offered_link_destination(dest_url: &str) -> Option<SharedString> {
    classify_markdown_link_destination(dest_url)
        .is_some()
        .then(|| SharedString::from(dest_url.trim().to_owned()))
}

pub(crate) fn pop_matching_inline_style(
    stack: &mut Vec<MarkdownInlineStyle>,
    style: MarkdownInlineStyle,
) {
    if let Some(ix) = stack.iter().rposition(|s| *s == style) {
        stack.remove(ix);
    }
}

pub(crate) fn strip_generic_html_tags(fragment: &str) -> String {
    let mut stripped = String::with_capacity(fragment.len());
    let mut chars = fragment.chars().peekable();
    let mut in_tag = false;

    while let Some(ch) = chars.next() {
        if in_tag {
            if ch == '>' {
                in_tag = false;
            }
            continue;
        }

        if ch == '<'
            && chars
                .peek()
                .is_some_and(|next| next.is_ascii_alphabetic() || matches!(next, '/' | '!' | '?'))
        {
            in_tag = true;
            continue;
        }

        stripped.push(ch);
    }

    stripped
}
