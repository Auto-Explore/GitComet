/// Lengths of the longest common prefix and, of what remains, the longest
/// common suffix of `a` and `b`; together they never exceed either length.
#[inline]
pub fn common_affix_len<T: PartialEq>(a: &[T], b: &[T]) -> (usize, usize) {
    let shared = a.len().min(b.len());
    let mut prefix = 0;
    while prefix < shared && a[prefix] == b[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < shared - prefix && a[a.len() - 1 - suffix] == b[b.len() - 1 - suffix] {
        suffix += 1;
    }
    (prefix, suffix)
}

/// Mask credentials embedded in a URL's userinfo for display.
///
/// `scheme://user:secret@host/…` becomes `scheme://user:***@host/…`. For
/// http(s), where a token is commonly the whole userinfo
/// (`https://ghp_…@github.com`), the userinfo is masked entirely. Other
/// schemes keep a bare username (`ssh://git@host`), and inputs without an
/// authority — scp-like `git@host:path`, local paths — are returned as-is.
/// Only display strings go through here; the URL git receives is untouched.
pub fn redact_url_userinfo(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let Some(at) = rest[..authority_end].rfind('@') else {
        return url.to_string();
    };
    let userinfo = &rest[..at];
    let masked = match userinfo.split_once(':') {
        Some((user, _)) => format!("{user}:***"),
        None if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") => {
            "***".to_string()
        }
        None => return url.to_string(),
    };
    format!("{scheme}://{masked}{}", &rest[at..])
}

/// Wraps text in POSIX single quotes (`'` becomes `'"'"'`), for shell fragments
/// such as `GIT_EDITOR` that git hands to `sh` on every platform.
pub fn shell_single_quote(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
}

/// Heuristic used by [`detect_line_ending_from_texts`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineEndingDetectionMode {
    /// Prefer CRLF if any side contains it, then CR, otherwise LF.
    ///
    /// This mirrors the focused-merge marker renderer behavior.
    Presence,
    /// Pick the dominant ending between CRLF and LF by counting occurrences.
    ///
    /// Ties default to LF, matching existing autosolve/subchunk behavior.
    DominantCrlfVsLf,
}

/// Detect a line ending style from multiple text inputs.
///
/// The caller selects the detection mode to preserve context-specific legacy
/// behavior while sharing one implementation.
pub fn detect_line_ending_from_texts<'a, I>(texts: I, mode: LineEndingDetectionMode) -> &'static str
where
    I: IntoIterator<Item = &'a str>,
{
    match mode {
        LineEndingDetectionMode::Presence => detect_by_presence(texts),
        LineEndingDetectionMode::DominantCrlfVsLf => detect_by_dominant_counts(texts),
    }
}

fn detect_by_presence<'a, I>(texts: I) -> &'static str
where
    I: IntoIterator<Item = &'a str>,
{
    let mut saw_cr = false;

    for text in texts {
        if text.contains("\r\n") {
            return "\r\n";
        }
        if text.contains('\r') {
            saw_cr = true;
        }
    }

    if saw_cr { "\r" } else { "\n" }
}

/// Byte offset of each line's start: 0, then one past every `\n`, so text
/// ending in a newline has an empty last line and empty text has one. Callers
/// with another convention (no lines for empty text, or tree-sitter's, with
/// no start after a trailing newline) adjust the result.
pub fn line_starts(text: &str) -> Vec<usize> {
    let bytes = text.as_bytes();
    let mut starts = Vec::with_capacity(bytes.len() / 64 + 1);
    starts.push(0);
    starts.extend(memchr::memchr_iter(b'\n', bytes).map(|ix| ix + 1));
    starts
}

/// Lines described by [`line_starts`] for text of `source_len` bytes: none
/// for empty text, else one per start (a trailing newline keeps its final
/// empty line).
pub fn line_count_from_starts(source_len: usize, line_starts: &[usize]) -> usize {
    if source_len == 0 {
        0
    } else {
        line_starts.len().max(1)
    }
}

/// Byte range of line `line_ix` without its `\n`, from [`line_starts`];
/// the empty line after a trailing newline is `len..len`.
pub fn line_byte_range(
    line_starts: &[usize],
    source_len: usize,
    line_ix: usize,
) -> Option<std::ops::Range<usize>> {
    if line_ix >= line_count_from_starts(source_len, line_starts) {
        return None;
    }
    let start = line_starts
        .get(line_ix)
        .copied()
        .unwrap_or(source_len)
        .min(source_len);
    let end = line_starts
        .get(line_ix.saturating_add(1))
        .copied()
        .map(|next| next.saturating_sub(1))
        .unwrap_or(source_len)
        .min(source_len)
        .max(start);
    Some(start..end)
}

/// `(CRLF count, total LF count)` of `text` in one scan; every LF is found
/// with `memchr` and the byte before it decides whether it was a CRLF.
pub fn count_line_feeds(text: &str) -> (usize, usize) {
    let bytes = text.as_bytes();
    let mut crlf = 0usize;
    let mut lf = 0usize;
    for position in memchr::memchr_iter(b'\n', bytes) {
        lf += 1;
        if position > 0 && bytes[position - 1] == b'\r' {
            crlf += 1;
        }
    }
    (crlf, lf)
}

fn detect_by_dominant_counts<'a, I>(texts: I) -> &'static str
where
    I: IntoIterator<Item = &'a str>,
{
    let mut crlf_count = 0usize;
    let mut lf_only_count = 0usize;
    for text in texts {
        let (crlf, lf) = count_line_feeds(text);
        crlf_count += crlf;
        lf_only_count += lf.saturating_sub(crlf);
    }
    if crlf_count > lf_only_count {
        "\r\n"
    } else {
        "\n"
    }
}

/// Whether `text` mixes more than one line-terminator style.
///
/// Renderers that rebuild a document line by line emit a single detected
/// ending for every line, so a mixed document cannot survive that round trip.
/// Callers use this to keep the original bytes instead of normalizing them.
pub fn text_has_mixed_line_endings(text: &str) -> bool {
    let (crlf, lf) = count_line_feeds(text);
    let bare_lf = lf.saturating_sub(crlf);
    let bare_cr = memchr::memchr_iter(b'\r', text.as_bytes())
        .count()
        .saturating_sub(crlf);
    [crlf, bare_lf, bare_cr]
        .into_iter()
        .filter(|count| *count > 0)
        .count()
        > 1
}

/// Short size label for UI: `512 B`, `1.5 KB`, `12.3 MB`, `1.2 GB` (decimal
/// units, one fraction digit above bytes, no trailing `.0`).
pub fn human_readable_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        return format!("{bytes} B");
    }
    let mut rounded = (value * 10.0).round() / 10.0;
    if rounded >= 1000.0 && unit + 1 < UNITS.len() {
        rounded /= 1000.0;
        unit += 1;
    }
    if rounded.fract() == 0.0 {
        format!("{rounded:.0} {}", UNITS[unit])
    } else {
        format!("{rounded:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod human_readable_bytes_tests;
#[cfg(test)]
mod tests;
