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

/// Validation error for sync-point-constrained matching.
#[cfg(test)]
mod tests {
    #[test]
    fn line_starts_begin_after_every_newline() {
        use super::line_starts;
        assert_eq!(line_starts(""), vec![0]);
        assert_eq!(line_starts("a"), vec![0]);
        assert_eq!(line_starts("a\n"), vec![0, 2]);
        assert_eq!(line_starts("a\r\nbc\n\nd"), vec![0, 3, 6, 7]);
    }

    #[test]
    fn line_ranges_exclude_newlines_and_keep_the_trailing_empty_line() {
        use super::{line_byte_range, line_count_from_starts, line_starts};
        let text = "ab\n\ncd\n";
        let starts = line_starts(text);
        assert_eq!(line_count_from_starts(text.len(), &starts), 4);
        let ranges: Vec<_> = (0..5)
            .map(|ix| line_byte_range(&starts, text.len(), ix))
            .collect();
        assert_eq!(
            ranges,
            vec![Some(0..2), Some(3..3), Some(4..6), Some(7..7), None]
        );
        assert_eq!(line_count_from_starts(0, &line_starts("")), 0);
        assert_eq!(line_byte_range(&line_starts(""), 0, 0), None);
    }

    #[test]
    fn redact_url_userinfo_masks_secrets_and_keeps_the_rest() {
        use super::redact_url_userinfo;
        assert_eq!(
            redact_url_userinfo("https://user:s3cret@example.com/org/repo.git"),
            "https://user:***@example.com/org/repo.git"
        );
        assert_eq!(
            redact_url_userinfo("https://ghp_token@github.com/org/repo.git"),
            "https://***@github.com/org/repo.git"
        );
        assert_eq!(
            redact_url_userinfo("HTTP://user@example.com:8080/repo?x=a@b#f"),
            "HTTP://***@example.com:8080/repo?x=a@b#f"
        );
        assert_eq!(
            redact_url_userinfo("ssh://git@example.com/org/repo.git"),
            "ssh://git@example.com/org/repo.git"
        );
        assert_eq!(
            redact_url_userinfo("ssh://git:pass@example.com/org/repo.git"),
            "ssh://git:***@example.com/org/repo.git"
        );
        for untouched in [
            "git@github.com:org/repo.git",
            "/tmp/repo.git",
            "https://example.com/repo?token=a@b",
            "",
        ] {
            assert_eq!(redact_url_userinfo(untouched), untouched);
        }
    }

    use super::{LineEndingDetectionMode, detect_line_ending_from_texts};

    #[test]
    fn detect_line_ending_presence_prefers_crlf_when_present() {
        assert_eq!(
            detect_line_ending_from_texts(
                ["ours\n", "theirs\r\n", "base\n"],
                LineEndingDetectionMode::Presence,
            ),
            "\r\n"
        );
    }

    #[test]
    fn detect_line_ending_presence_prefers_cr_when_no_crlf() {
        assert_eq!(
            detect_line_ending_from_texts(
                ["ours\rmore", "theirs\n", "base"],
                LineEndingDetectionMode::Presence,
            ),
            "\r"
        );
    }

    #[test]
    fn detect_line_ending_presence_defaults_to_lf() {
        assert_eq!(
            detect_line_ending_from_texts(["no separators"], LineEndingDetectionMode::Presence),
            "\n"
        );
        assert_eq!(
            detect_line_ending_from_texts([], LineEndingDetectionMode::Presence),
            "\n"
        );
    }

    #[test]
    fn detect_line_ending_dominant_prefers_majority() {
        assert_eq!(
            detect_line_ending_from_texts(
                ["a\r\nb\r\nc\n"],
                LineEndingDetectionMode::DominantCrlfVsLf,
            ),
            "\r\n"
        );
        assert_eq!(
            detect_line_ending_from_texts(
                ["a\r\nb\nc\n"],
                LineEndingDetectionMode::DominantCrlfVsLf,
            ),
            "\n"
        );
    }

    #[test]
    fn detect_line_ending_dominant_tie_defaults_to_lf() {
        assert_eq!(
            detect_line_ending_from_texts(["a\r\nb\n"], LineEndingDetectionMode::DominantCrlfVsLf,),
            "\n"
        );
        assert_eq!(
            detect_line_ending_from_texts([], LineEndingDetectionMode::DominantCrlfVsLf),
            "\n"
        );
    }
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
mod human_readable_bytes_tests {
    use super::human_readable_bytes;

    #[test]
    fn formats_decimal_units_with_one_fraction_digit() {
        assert_eq!(human_readable_bytes(0), "0 B");
        assert_eq!(human_readable_bytes(999), "999 B");
        assert_eq!(human_readable_bytes(1_000), "1 KB");
        assert_eq!(human_readable_bytes(1_536), "1.5 KB");
        assert_eq!(human_readable_bytes(999_949), "999.9 KB");
        assert_eq!(human_readable_bytes(999_950), "1 MB");
        assert_eq!(human_readable_bytes(999_950_000), "1 GB");
        assert_eq!(human_readable_bytes(12_345_678), "12.3 MB");
        assert_eq!(human_readable_bytes(1_200_000_000), "1.2 GB");
        assert_eq!(human_readable_bytes(5_000_000_000_000_000), "5000 TB");
    }
}
