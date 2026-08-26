use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HtmlHandling {
    Ignore,
    HardBreak,
    DetailsSummary(String),
    StartInlineStyle(MarkdownInlineStyle),
    EndInlineStyle(MarkdownInlineStyle),
    AppendText(String),
    /// The `<img>` tags a fragment holds, each with the byte offset of its tag
    /// inside that fragment and the `alt` describing it if it cannot be drawn.
    Images(Vec<(usize, MarkdownImage, String)>),
    AppendLiteral,
}

pub(crate) fn current_row_kind(
    list_item_stack: &[MarkdownPreviewRowKind],
    blockquote_level: u8,
) -> MarkdownPreviewRowKind {
    if let Some(kind) = list_item_stack.last().copied() {
        kind
    } else if blockquote_level > 0 {
        MarkdownPreviewRowKind::BlockquoteLine
    } else {
        MarkdownPreviewRowKind::Paragraph
    }
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

pub(crate) fn html_event_should_append(
    in_paragraph: bool,
    in_heading: bool,
    in_list: bool,
    blockquote_level: u8,
    in_code_block: bool,
    in_table_row: bool,
) -> bool {
    in_paragraph || in_heading || in_list || blockquote_level > 0 || in_code_block || in_table_row
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
    if lower.starts_with("<a ") && (lower.contains(" name=") || lower.contains(" id=")) {
        return HtmlHandling::Ignore;
    }
    if lower.starts_with("<a ") && lower.contains(" href=") {
        return HtmlHandling::StartInlineStyle(MarkdownInlineStyle::Link);
    }
    if lower == "</a>" {
        return HtmlHandling::EndInlineStyle(MarkdownInlineStyle::Link);
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
pub(crate) fn extract_html_images(html: &str) -> Vec<(usize, MarkdownImage, String)> {
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
        images.push((
            tag_start,
            MarkdownImage {
                source: source.into(),
                width_px: extract_html_pixel_attribute(tag, "width"),
                height_px: extract_html_pixel_attribute(tag, "height"),
            },
            extract_html_attribute(tag, "alt").unwrap_or_default(),
        ));
    }

    images
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

/// Keep only destinations that open in a browser.
///
/// Relative links, in-document anchors, and `mailto:`/`javascript:` targets
/// have no meaning for a preview of a file at some commit, so they render as
/// links but are not offered as something to open.
pub(crate) fn web_link_url(dest_url: &str) -> Option<SharedString> {
    let trimmed = dest_url.trim();
    let scheme_end = trimmed.find("://")?;
    let scheme = &trimmed[..scheme_end];
    (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        .then(|| SharedString::from(trimmed.to_owned()))
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
