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
    /// A block element whose tags are hidden: `<p>`, `<div>`, `<center>`,
    /// `<h1>`–`<h6>`, with its `align`.
    OpenContainer(HtmlContainerKind, MarkdownTextAlign),
    CloseContainer(HtmlContainerKind),
    /// `<hr>`: a thematic break.
    Rule,
    AppendLiteral,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HtmlContainerKind {
    Paragraph,
    Division,
    Center,
    Heading(u8),
}

/// The elements read as containers, by tag name.
const HTML_CONTAINERS: [(&str, HtmlContainerKind); 9] = [
    ("p", HtmlContainerKind::Paragraph),
    ("div", HtmlContainerKind::Division),
    ("center", HtmlContainerKind::Center),
    ("h1", HtmlContainerKind::Heading(1)),
    ("h2", HtmlContainerKind::Heading(2)),
    ("h3", HtmlContainerKind::Heading(3)),
    ("h4", HtmlContainerKind::Heading(4)),
    ("h5", HtmlContainerKind::Heading(5)),
    ("h6", HtmlContainerKind::Heading(6)),
];

impl HtmlContainerKind {
    /// The container a tag name, in any case, names.
    fn from_tag_name(name: &str) -> Option<Self> {
        HTML_CONTAINERS
            .iter()
            .find(|(tag, _)| name.eq_ignore_ascii_case(tag))
            .map(|(_, kind)| *kind)
    }

    /// Whether a closing tag of `self` ends `open`; any `</hN>` ends a heading.
    pub(crate) fn closes(self, open: Self) -> bool {
        match (self, open) {
            (Self::Heading(_), Self::Heading(_)) => true,
            _ => self == open,
        }
    }

    /// `<p>` and `<hN>` hold only text, so they end with their HTML block;
    /// `<div>` and `<center>` can wrap markdown written between blank lines.
    pub(crate) fn ends_with_its_block(self) -> bool {
        matches!(self, Self::Paragraph | Self::Heading(_))
    }
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

/// What a whole fragment means, for HTML read without splitting it into
/// tags: inline HTML, and blocks holding a tag the preview does not know.
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
    if is_single_html_tag(trimmed) {
        return classify_html_tag(trimmed);
    }
    if is_html_open_tag(lower.as_str(), "a") {
        return match extract_html_attribute(trimmed, "href") {
            Some(href) => HtmlHandling::StartLink(offered_link_destination(&href)),
            None => HtmlHandling::Ignore,
        };
    }
    if is_html_close_tag(lower.as_str(), "a") {
        return HtmlHandling::EndLink;
    }
    if lower.starts_with("<picture") || lower.starts_with("<source") {
        return HtmlHandling::Ignore;
    }
    if is_html_open_tag(lower.as_str(), "details") || is_html_close_tag(lower.as_str(), "details") {
        return HtmlHandling::Ignore;
    }

    HtmlHandling::AppendLiteral
}

/// What one tag means to the preview; `AppendLiteral` for a tag it does not
/// know.
pub(crate) fn classify_html_tag(tag: &str) -> HtmlHandling {
    let lower = tag.to_ascii_lowercase();
    // Comments, `<!DOCTYPE>`, processing instructions.
    if lower.starts_with("<!") || lower.starts_with("<?") {
        return HtmlHandling::Ignore;
    }
    let Some((name, closing)) = html_tag_name(&lower) else {
        return HtmlHandling::AppendLiteral;
    };
    if let Some(kind) = HtmlContainerKind::from_tag_name(name) {
        return if closing {
            HtmlHandling::CloseContainer(kind)
        } else if kind == HtmlContainerKind::Center {
            HtmlHandling::OpenContainer(kind, MarkdownTextAlign::Center)
        } else {
            HtmlHandling::OpenContainer(kind, html_align(tag))
        };
    }
    if let Some(style) = html_inline_style(name) {
        return if closing {
            HtmlHandling::EndInlineStyle(style)
        } else {
            HtmlHandling::StartInlineStyle(style)
        };
    }
    match (name, closing) {
        ("br", _) => HtmlHandling::HardBreak,
        ("hr", false) => HtmlHandling::Rule,
        ("hr", true) => HtmlHandling::Ignore,
        ("img", false) => {
            let images = extract_html_images(tag);
            if !images.is_empty() {
                HtmlHandling::Images(images)
            } else if let Some(alt) = extract_html_image_alt(tag) {
                HtmlHandling::AppendText(alt)
            } else {
                HtmlHandling::Ignore
            }
        }
        // A named anchor (`<a name>`/`<a id>`) is a jump target with nothing
        // to show; one with an `href` is a link like any other.
        ("a", false) => match extract_html_attribute(tag, "href") {
            Some(href) => HtmlHandling::StartLink(offered_link_destination(&href)),
            None => HtmlHandling::Ignore,
        },
        ("a", true) => HtmlHandling::EndLink,
        ("img" | "sub" | "sup" | "span" | "small" | "picture" | "source" | "details", _) => {
            HtmlHandling::Ignore
        }
        _ => HtmlHandling::AppendLiteral,
    }
}

/// The container tags in `html`, in order: where each lies, its kind, its
/// `align`, and whether it closes. Tags are read whole, so one inside a
/// comment or a quoted attribute value does not count; nor does anything in
/// a `<script>`, `<style>`, `<pre>` or `<textarea>` block, whose text is
/// not markup.
pub(crate) fn html_container_tags(
    html: &str,
) -> impl Iterator<Item = (Range<usize>, HtmlContainerKind, MarkdownTextAlign, bool)> + '_ {
    let raw_text = is_raw_text_html(html);
    html_tokens(html)
        .filter(move |_| !raw_text)
        .filter_map(move |token| {
            let HtmlToken::Tag(range) = token else {
                return None;
            };
            let tag = &html[range.clone()];
            if !is_html_container_tag(tag) {
                return None;
            }
            match classify_html_tag(tag) {
                HtmlHandling::OpenContainer(kind, align) => Some((range, kind, align, false)),
                HtmlHandling::CloseContainer(kind) => {
                    Some((range, kind, MarkdownTextAlign::None, true))
                }
                _ => None,
            }
        })
}

/// Whether an HTML block opens with `<script>`, `<style>`, `<pre>` or
/// `<textarea>`, whose content is text rather than markup.
pub(crate) fn is_raw_text_html(html: &str) -> bool {
    html_tokens(html)
        .find_map(|token| match token {
            HtmlToken::Tag(tag) => Some(tag),
            HtmlToken::Text(_) => None,
        })
        .and_then(|first| {
            html_tag_name(&html[first].to_ascii_lowercase())
                .map(|(name, _)| matches!(name, "script" | "style" | "pre" | "textarea"))
        })
        .unwrap_or(false)
}

/// Whether `tag` (or text starting with it) opens or closes a `<p>`, `<div>`,
/// `<center>` or `<hN>`, judged by its name without allocating.
fn is_html_container_tag(tag: &str) -> bool {
    let rest = tag.strip_prefix('<').unwrap_or(tag);
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    let end = rest
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '-')
        .unwrap_or(rest.len());
    HtmlContainerKind::from_tag_name(&rest[..end]).is_some()
}

/// The formatting a tag gives the text inside it.
fn html_inline_style(name: &str) -> Option<MarkdownInlineStyle> {
    Some(match name {
        "b" | "strong" => MarkdownInlineStyle::Bold,
        "i" | "em" => MarkdownInlineStyle::Italic,
        "code" | "kbd" | "samp" | "tt" => MarkdownInlineStyle::Code,
        "s" | "del" | "strike" => MarkdownInlineStyle::Strikethrough,
        "u" | "ins" => MarkdownInlineStyle::Underline,
        _ => return None,
    })
}

/// A block element's `align` attribute.
pub(crate) fn html_align(tag: &str) -> MarkdownTextAlign {
    let value = extract_html_attribute(tag, "align").unwrap_or_default();
    match value.trim().to_ascii_lowercase().as_str() {
        "center" => MarkdownTextAlign::Center,
        "right" => MarkdownTextAlign::Right,
        "left" => MarkdownTextAlign::Left,
        _ => MarkdownTextAlign::None,
    }
}

/// A lowercased tag's name and whether it closes.
fn html_tag_name(lower_tag: &str) -> Option<(&str, bool)> {
    let rest = lower_tag.strip_prefix('<')?;
    let (rest, closing) = match rest.strip_prefix('/') {
        Some(rest) => (rest, true),
        None => (rest, false),
    };
    let end = rest
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '-')
        .unwrap_or(rest.len());
    (end > 0).then(|| (&rest[..end], closing))
}

/// One piece of an HTML fragment, as a byte range into it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HtmlToken {
    Tag(Range<usize>),
    Text(Range<usize>),
}

/// Split `html` into tags (comments included) and the text between them,
/// lazily, so a reader can stop at the first tag it cannot use.
pub(crate) fn html_tokens(html: &str) -> impl Iterator<Item = HtmlToken> + '_ {
    let bytes = html.as_bytes();
    let mut at = 0;
    // A tag found after a run of text, handed out on the next call.
    let mut next_tag = None;
    std::iter::from_fn(move || {
        if let Some(tag) = next_tag.take() {
            return Some(HtmlToken::Tag(tag));
        }
        let text_start = at;
        let mut search = at;
        while let Some(found) = memchr::memchr(b'<', &bytes[search..]) {
            let ix = search + found;
            if !starts_html_tag(bytes, ix) {
                search = ix + 1;
                continue;
            }
            let tag = ix..html_tag_end(html, ix);
            at = tag.end;
            if text_start == ix {
                return Some(HtmlToken::Tag(tag));
            }
            next_tag = Some(tag);
            return Some(HtmlToken::Text(text_start..ix));
        }
        at = bytes.len();
        (text_start < at).then_some(HtmlToken::Text(text_start..at))
    })
}

/// `<` that opens a tag, by the rule `strip_generic_html_tags` uses.
fn starts_html_tag(bytes: &[u8], ix: usize) -> bool {
    bytes[ix] == b'<'
        && bytes
            .get(ix + 1)
            .is_some_and(|next| next.is_ascii_alphabetic() || matches!(next, b'/' | b'!' | b'?'))
}

/// Where the tag opening at `start` ends: past its `>`, skipping any inside a
/// quoted attribute value, or the end of `html` when it never closes.
fn html_tag_end(html: &str, start: usize) -> usize {
    if html[start..].starts_with("<!--") {
        // `<!-->` and `<!--->` are comments that end at once.
        for empty in ["<!-->", "<!--->"] {
            if html[start..].starts_with(empty) {
                return start + empty.len();
            }
        }
        return html[start + 4..]
            .find("-->")
            .map_or(html.len(), |end| start + 4 + end + 3);
    }
    let bytes = html.as_bytes();
    let mut quote = None;
    let mut after_equals = false;
    for (ix, &byte) in bytes.iter().enumerate().skip(start + 1) {
        match quote {
            Some(open) if byte == open => quote = None,
            Some(_) => {}
            None if byte == b'>' => return ix + 1,
            None if after_equals && matches!(byte, b'"' | b'\'') => quote = Some(byte),
            None => {}
        }
        if !byte.is_ascii_whitespace() {
            after_equals = byte == b'=';
        }
    }
    html.len()
}

fn is_single_html_tag(trimmed: &str) -> bool {
    starts_html_tag(trimmed.as_bytes(), 0) && html_tag_end(trimmed, 0) == trimmed.len()
}

/// `text` with its character references replaced. Unknown names stay as
/// written.
pub(crate) fn decode_html_entities(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains('&') {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut decoded = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        decoded.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        // Names are short; looking further would make text full of bare `&`
        // quadratic.
        match after
            .as_bytes()
            .iter()
            .take(11)
            .position(|byte| *byte == b';')
            .and_then(|semi| Some((semi, html_entity(&after[..semi])?)))
        {
            Some((semi, ch)) => {
                decoded.push(ch);
                rest = &after[semi + 1..];
            }
            None => {
                decoded.push('&');
                rest = after;
            }
        }
    }
    decoded.push_str(rest);
    std::borrow::Cow::Owned(decoded)
}

/// The character windows-1252 puts at `byte` in 0x80–0x9F, where Unicode
/// has control characters.
fn windows_1252_char(byte: u32) -> Option<u32> {
    const TABLE: [u32; 32] = [
        0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160,
        0x2039, 0x0152, 0x8D, 0x017D, 0x8F, 0x90, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013,
        0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
    ];
    (0x80..=0x9F)
        .contains(&byte)
        .then(|| TABLE[(byte - 0x80) as usize])
}

fn html_entity(name: &str) -> Option<char> {
    if let Some(number) = name.strip_prefix('#') {
        let (digits, radix) = match number.strip_prefix(['x', 'X']) {
            Some(hex) => (hex, 16),
            None => (number, 10),
        };
        if digits.is_empty() || !digits.chars().all(|ch| ch.is_digit(radix)) {
            return None;
        }
        let value = u32::from_str_radix(digits, radix).ok()?;
        // HTML reads 0x80–0x9F as windows-1252, as old documents meant them.
        let value = windows_1252_char(value).unwrap_or(value);
        return char::from_u32(value).filter(|ch| *ch != '\0');
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "ensp" => '\u{2002}',
        "emsp" => '\u{2003}',
        "thinsp" => '\u{2009}',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "middot" => '·',
        "bull" => '•',
        "ndash" => '–',
        "mdash" => '—',
        "hellip" => '…',
        "laquo" => '«',
        "raquo" => '»',
        "larr" => '←',
        "rarr" => '→',
        _ => return None,
    })
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
    let whitespace = |ch: char| ch.is_ascii_whitespace();
    let mut rest = html.trim_start_matches(whitespace).strip_prefix('<')?;
    let tag_end = rest.find(|ch: char| whitespace(ch) || matches!(ch, '>' | '/'))?;
    rest = &rest[tag_end..];
    loop {
        rest = rest.trim_start_matches(whitespace);
        if rest.is_empty() || rest.starts_with('>') || rest.starts_with('/') {
            return None;
        }
        let attr_end = rest
            .find(|ch: char| whitespace(ch) || matches!(ch, '=' | '>' | '/'))
            .unwrap_or(rest.len());
        if attr_end == 0 {
            return None;
        }
        let attribute = &rest[..attr_end];
        rest = rest[attr_end..].trim_start_matches(whitespace);
        let Some(after_equals) = rest.strip_prefix('=') else {
            if attribute.eq_ignore_ascii_case(name) {
                return Some(String::new());
            }
            continue;
        };
        rest = after_equals.trim_start_matches(whitespace);
        // Consume whole values, including quoted text that resembles another
        // attribute, before looking for the next attribute name.
        let value = if let Some(quote) = rest.chars().next().filter(|ch| matches!(ch, '"' | '\'')) {
            rest = &rest[1..];
            let end = rest.find(quote)?;
            let value = &rest[..end];
            rest = &rest[end + 1..];
            value
        } else {
            let end = rest
                .find(|ch: char| whitespace(ch) || ch == '>')
                .unwrap_or(rest.len());
            let value = &rest[..end];
            rest = &rest[end..];
            value
        };
        if attribute.eq_ignore_ascii_case(name) {
            return Some(value.to_owned());
        }
    }
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
