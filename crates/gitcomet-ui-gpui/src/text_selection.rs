use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation as _;

pub(crate) fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn previous_boundary(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true)
        .rev()
        .find_map(|(idx, _)| (idx < offset).then_some(idx))
        .unwrap_or(0)
}

fn skip_left_while(
    text: &str,
    mut offset: usize,
    mut predicate: impl FnMut(char) -> bool,
) -> usize {
    offset = offset.min(text.len());
    while offset > 0 {
        let Some((idx, ch)) = text[..offset].char_indices().next_back() else {
            return 0;
        };
        if !predicate(ch) {
            break;
        }
        offset = idx;
    }
    offset
}

fn skip_right_while(
    text: &str,
    mut offset: usize,
    mut predicate: impl FnMut(char) -> bool,
) -> usize {
    offset = offset.min(text.len());
    while offset < text.len() {
        let Some(ch) = text[offset..].chars().next() else {
            break;
        };
        if !predicate(ch) {
            break;
        }
        offset += ch.len_utf8();
    }
    offset
}

pub(crate) fn token_range_for_offset(text: &str, offset: usize) -> Range<usize> {
    if text.is_empty() {
        return 0..0;
    }

    let mut probe = offset.min(text.len());
    if probe == text.len() && probe > 0 {
        probe = previous_boundary(text, probe);
    }

    let Some(ch) = text[probe..].chars().next() else {
        return probe..probe;
    };

    if ch.is_whitespace() {
        let start = skip_left_while(text, probe, |ch| ch.is_whitespace());
        let end = skip_right_while(text, probe, |ch| ch.is_whitespace());
        return start..end;
    }

    if is_word_char(ch) {
        let start = skip_left_while(text, probe, is_word_char);
        let end = skip_right_while(text, probe, is_word_char);
        return start..end;
    }

    let start = skip_left_while(text, probe, |ch| !ch.is_whitespace() && !is_word_char(ch));
    let end = skip_right_while(text, probe, |ch| !ch.is_whitespace() && !is_word_char(ch));
    start..end
}

fn is_ascii_hex(byte: u8) -> bool {
    byte.is_ascii_hexdigit()
}

/// What a linkified span of a commit message points at.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MessageLinkKind {
    Url,
    CommitSha,
}

/// A linkable span of a commit message, as a UTF-8 byte range into it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MessageLinkRange {
    pub range: Range<usize>,
    pub kind: MessageLinkKind,
}

/// Every span of a commit message worth turning into a link, in order and
/// without overlaps.
///
/// URLs are found first and claim their whole span, so the hex-looking pieces of
/// a URL's path — a Gerrit change number, a buildbucket id — never masquerade as
/// abbreviated commit ids.
pub(crate) fn commit_message_link_ranges(text: &str) -> Vec<MessageLinkRange> {
    let urls = web_url_ranges(text);
    let shas = commit_sha_ranges_outside(text, &urls);

    let mut links = Vec::with_capacity(urls.len() + shas.len());
    links.extend(urls.into_iter().map(|range| MessageLinkRange {
        range,
        kind: MessageLinkKind::Url,
    }));
    links.extend(shas.into_iter().map(|range| MessageLinkRange {
        range,
        kind: MessageLinkKind::CommitSha,
    }));
    links.sort_by_key(|link| link.range.start);
    links
}

/// The commit-id half of [`commit_message_link_ranges`], on its own.
///
/// Only the tests need the two kinds apart; the UI always wants both.
#[cfg(test)]
fn commit_sha_ranges(text: &str) -> Vec<Range<usize>> {
    commit_sha_ranges_outside(text, &web_url_ranges(text))
}

/// Bytes that may follow the first character of a URL scheme (RFC 3986).
fn is_url_scheme_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.')
}

/// Bytes that end a URL when it is written inside running prose.
///
/// Deliberately permissive about the URL's own grammar — anything that is not
/// whitespace, a control character, or one of the delimiters RFC 3986 excludes
/// stays part of the candidate, and [`url::Url::parse`] has the final say.
fn ends_url(byte: u8) -> bool {
    byte.is_ascii_whitespace()
        || byte < 0x20
        || byte == 0x7f
        || matches!(
            byte,
            b'<' | b'>' | b'"' | b'{' | b'}' | b'|' | b'\\' | b'^' | b'`'
        )
}

fn web_url_ranges(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut ranges: Vec<Range<usize>> = Vec::new();
    let mut cursor = 0usize;

    while cursor < bytes.len() {
        let start = cursor;
        cursor += 1;

        if !bytes[start].is_ascii_alphabetic() {
            continue;
        }
        // A scheme has to start at the head of its run, otherwise the `ttps` of
        // `nothttps://x` would be read as a scheme of its own.
        if start > 0 && (is_url_scheme_byte(bytes[start - 1]) || bytes[start - 1] == b'_') {
            continue;
        }

        let mut scheme_end = start;
        while scheme_end < bytes.len() && is_url_scheme_byte(bytes[scheme_end]) {
            scheme_end += 1;
        }
        if bytes.get(scheme_end) != Some(&b':') {
            cursor = scheme_end.max(cursor);
            continue;
        }

        let mut end = scheme_end + 1;
        while end < bytes.len() && !ends_url(bytes[end]) {
            end += 1;
        }
        end = trim_url_tail(bytes, start, scheme_end + 1, end);

        let candidate = &text[start..end];
        if !crate::view::platform_open::is_supported_link_url(candidate)
            || url::Url::parse(candidate).is_err()
        {
            cursor = scheme_end.max(cursor);
            continue;
        }

        ranges.push(start..end);
        cursor = end;
    }

    ranges
}

/// Give back the punctuation a URL picked up from the sentence around it.
///
/// Closing brackets only come off when they are unmatched inside the candidate,
/// so `https://en.wikipedia.org/wiki/Git_(software)` keeps its parenthesis while
/// `(see https://example.com)` does not.
fn trim_url_tail(bytes: &[u8], start: usize, floor: usize, mut end: usize) -> usize {
    while end > floor {
        let last = bytes[end - 1];
        let brackets = match last {
            b')' => Some((b'(', b')')),
            b']' => Some((b'[', b']')),
            b'}' => Some((b'{', b'}')),
            _ => None,
        };

        let trim = match brackets {
            Some((open, close)) => {
                let span = &bytes[start..end];
                let opened = span.iter().filter(|byte| **byte == open).count();
                let closed = span.iter().filter(|byte| **byte == close).count();
                closed > opened
            }
            None => matches!(
                last,
                b'.' | b',' | b';' | b':' | b'!' | b'?' | b'\'' | b'"' | b'*' | b'_' | b'~'
            ),
        };

        if !trim {
            break;
        }
        end -= 1;
    }
    end
}

fn commit_sha_ranges_outside(text: &str, excluded: &[Range<usize>]) -> Vec<Range<usize>> {
    const MIN_SHA_LEN: usize = 7;
    const MAX_SHA_LEN: usize = 40;

    let bytes = text.as_bytes();
    let mut ranges = Vec::new();
    let mut cursor = 0;

    while cursor < bytes.len() {
        if !is_ascii_hex(bytes[cursor]) {
            cursor += 1;
            continue;
        }

        let start = cursor;
        while cursor < bytes.len() && is_ascii_hex(bytes[cursor]) {
            cursor += 1;
        }

        let len = cursor - start;
        if (MIN_SHA_LEN..=MAX_SHA_LEN).contains(&len)
            && is_whole_word(bytes, start, cursor)
            && has_hex_letter(&bytes[start..cursor])
            && !overlaps_any(start..cursor, excluded)
        {
            ranges.push(start..cursor);
        }
    }

    ranges
}

/// Whether a hex run is a token in its own right rather than a slice of a longer
/// identifier.
///
/// The run already ends at the first non-hex byte, so this is what separates a
/// real abbreviation from the tail of `Change-Id: I7a5d4808…` — Gerrit's `I`
/// prefix is a letter, so the 40 hex characters after it are not a word — and
/// from the hashes buried in a build artifact's name.
fn is_whole_word(bytes: &[u8], start: usize, end: usize) -> bool {
    let before = start
        .checked_sub(1)
        .map(|ix| (bytes[ix], start.checked_sub(2).map(|ix| bytes[ix])));
    let after = bytes
        .get(end)
        .map(|byte| (*byte, bytes.get(end + 1).copied()));

    [before, after].into_iter().all(|neighbours| {
        let Some((adjacent, beyond)) = neighbours else {
            // Nothing on this side at all: the run reaches the end of the text.
            return true;
        };
        if is_identifier_byte(adjacent) {
            return false;
        }
        if is_compound_separator(adjacent) {
            return beyond.is_none_or(|byte| !is_identifier_byte(byte));
        }
        true
    })
}

/// A byte a commit id cannot touch: the run would be a slice of a longer
/// identifier rather than a token of its own.
fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Whether a byte joins the parts of a compound name as readily as it
/// punctuates a sentence.
///
/// `-` and `.` do both, so neither is a boundary on its own: they only end a
/// commit id when nothing alphanumeric sits on their far side. That keeps
/// `reverts <sha>.` and `<sha>..<sha>` linkable while the hashes inside
/// `chrome-mac-7922-<sha>-<sha>.profdata` stay part of the filename.
fn is_compound_separator(byte: u8) -> bool {
    matches!(byte, b'-' | b'.')
}

/// Whether a hex run contains something that is not also a decimal digit.
///
/// Bug numbers, build ids, Gerrit change numbers and timestamps are all long
/// enough to pass for abbreviated ids, and all of them are pure decimal. A real
/// commit id that happens to have no `a`-`f` in its first seven characters is
/// rare enough to be worth the trade.
fn has_hex_letter(run: &[u8]) -> bool {
    run.iter().any(|byte| byte.is_ascii_alphabetic())
}

fn overlaps_any(range: Range<usize>, excluded: &[Range<usize>]) -> bool {
    excluded
        .iter()
        .any(|other| other.start < range.end && range.start < other.end)
}

#[cfg(test)]
mod tests {
    use super::{
        MessageLinkKind, commit_message_link_ranges, commit_sha_ranges, token_range_for_offset,
        web_url_ranges,
    };

    /// The message that motivated the stricter rules: every hex-looking run in
    /// it is a build id, a Gerrit change id, or part of a URL — none is a commit.
    const CHROMIUM_LKGM_MESSAGE: &str = "\
Uploaded by https://ci.chromium.org/b/8674534147806418049

CrOS-LKGM: 16733.40.0
Merge-Approval-Bypass: Automated LKGM update
Cr-Original-Build-Id: 8674534147806418049
Change-Id: I7a5d480873e839444e4e188ffa87f9c635e2fb81
Reviewed-on: https://chromium-review.googlesource.com/c/chromium/src/+/8186904";

    fn spans(text: &str, kind: MessageLinkKind) -> Vec<&str> {
        commit_message_link_ranges(text)
            .into_iter()
            .filter(|link| link.kind == kind)
            .map(|link| &text[link.range])
            .collect()
    }

    #[test]
    fn token_range_selects_words_whitespace_and_symbols() {
        let text = "alpha  :: beta";
        assert_eq!(token_range_for_offset(text, 1), 0..5);
        assert_eq!(token_range_for_offset(text, 6), 5..7);
        assert_eq!(token_range_for_offset(text, 8), 7..9);
        assert_eq!(token_range_for_offset(text, 11), 10..14);
    }

    #[test]
    fn token_range_uses_previous_boundary_at_end_of_text() {
        let text = "alpha";
        assert_eq!(token_range_for_offset(text, text.len()), 0..5);
    }

    #[test]
    fn commit_sha_ranges_find_hex_runs_with_boundaries() {
        let text = "fix deadbee, parent 0123456789abcdef0123456789abcdef01234567.";
        assert_eq!(commit_sha_ranges(text), vec![4..11, 20..60]);
    }

    #[test]
    fn commit_sha_ranges_accept_uppercase_and_reject_short_or_long_runs() {
        let text = "abc123 89ABCDEF 0123456789abcdef0123456789abcdef012345678";
        assert_eq!(commit_sha_ranges(text), vec![7..15]);
    }

    #[test]
    fn commit_sha_ranges_keep_embedded_non_hex_boundaries() {
        let text = "(deadbee)/feedface not-a-sha";
        assert_eq!(commit_sha_ranges(text), vec![1..8, 10..18]);
    }

    #[test]
    fn commit_sha_ranges_reject_runs_glued_to_a_letter() {
        // Gerrit's `I` prefix makes the 40 hex characters after it a slice of a
        // longer token, not an abbreviation.
        let text = "Change-Id: I7a5d480873e839444e4e188ffa87f9c635e2fb81";
        assert_eq!(
            commit_sha_ranges(text),
            Vec::<std::ops::Range<usize>>::new()
        );
        // The same digits standing on their own are a commit id again.
        let text = "reverts 7a5d480873e839444e4e188ffa87f9c635e2fb81 cleanly";
        assert_eq!(commit_sha_ranges(text), vec![8..48]);
    }

    #[test]
    fn commit_sha_ranges_reject_hex_joined_into_a_filename() {
        // Every hash here is one dash-joined piece of a build artifact's name.
        let text = "Roll Chrome Mac PGO profile from \
chrome-mac-7922-1785736271-37240ae8aae5f01fc00cbf0b7ea19b73826e0dba-d9e99b2bafcc6df3c2a5bf803fcb5483d33dbdd0.profdata \
to \
chrome-mac-7922-1785755104-c2eee60da6765f60eca833b7c5c0d85ddcbc2940-551a1e94b700524e479bd2d64ccaf8cdb71d43a6.profdata";
        assert_eq!(
            commit_sha_ranges(text),
            Vec::<std::ops::Range<usize>>::new()
        );
    }

    #[test]
    fn commit_sha_ranges_keep_sentence_and_range_punctuation() {
        // A separator only blocks the run when something alphanumeric sits on
        // its far side, so a trailing full stop and a `..` range still link.
        let text = "This reverts commit 37240ae8aae5f01fc00cbf0b7ea19b73826e0dba.";
        assert_eq!(commit_sha_ranges(text), vec![20..60]);

        let text = "Roll deps deadbee..feedface (2 commits)";
        assert_eq!(commit_sha_ranges(text), vec![10..17, 19..27]);
    }

    #[test]
    fn commit_sha_ranges_reject_all_decimal_runs() {
        for text in [
            "Cr-Original-Build-Id: 8674534147806418049",
            "Bug: 389629573",
            "Cr-Commit-Position: refs/heads/main@{#1234567}",
        ] {
            assert_eq!(
                commit_sha_ranges(text),
                Vec::<std::ops::Range<usize>>::new(),
                "expected no commit ids in {text:?}"
            );
        }
    }

    #[test]
    fn commit_sha_ranges_ignore_hex_inside_urls() {
        let text = "see https://review.example.com/c/src/+/8186904abc for context";
        assert_eq!(
            commit_sha_ranges(text),
            Vec::<std::ops::Range<usize>>::new()
        );
    }

    #[test]
    fn chromium_lkgm_message_links_only_its_two_urls() {
        assert_eq!(
            spans(CHROMIUM_LKGM_MESSAGE, MessageLinkKind::CommitSha),
            Vec::<&str>::new()
        );
        assert_eq!(
            spans(CHROMIUM_LKGM_MESSAGE, MessageLinkKind::Url),
            vec![
                "https://ci.chromium.org/b/8674534147806418049",
                "https://chromium-review.googlesource.com/c/chromium/src/+/8186904",
            ]
        );
    }

    #[test]
    fn web_urls_are_found_for_any_hierarchical_scheme_and_mailto() {
        let text = "clone ssh://git@example.com/repo.git or git://example.com/repo.git, ask mailto:dev@example.com";
        assert_eq!(
            web_url_ranges(text)
                .into_iter()
                .map(|range| &text[range])
                .collect::<Vec<_>>(),
            vec![
                "ssh://git@example.com/repo.git",
                "git://example.com/repo.git",
                "mailto:dev@example.com",
            ]
        );
    }

    #[test]
    fn web_urls_drop_script_and_file_schemes() {
        let text = "javascript://evil.example.com/x data:text/html,x file:///etc/passwd";
        assert_eq!(web_url_ranges(text), Vec::<std::ops::Range<usize>>::new());
    }

    #[test]
    fn web_urls_give_back_sentence_punctuation_but_keep_balanced_brackets() {
        let text = "See https://en.wikipedia.org/wiki/Git_(software), or (https://example.com/a).";
        assert_eq!(
            web_url_ranges(text)
                .into_iter()
                .map(|range| &text[range])
                .collect::<Vec<_>>(),
            vec![
                "https://en.wikipedia.org/wiki/Git_(software)",
                "https://example.com/a",
            ]
        );
    }

    #[test]
    fn web_urls_take_the_whole_scheme_token() {
        // The scheme here is `nothttps`, not the `https` hiding at offset 3 —
        // a scheme only starts at the head of its run. Any well-formed scheme
        // is linkable, so the token as a whole is still a URL.
        let text = "nothttps://example.com";
        assert_eq!(web_url_ranges(text), vec![0..text.len()]);

        // Glued to a preceding word there is no scheme start at all.
        let text = "path/to_https://example.com";
        assert_eq!(web_url_ranges(text), Vec::<std::ops::Range<usize>>::new());
    }

    #[test]
    fn message_links_are_sorted_and_do_not_overlap() {
        let text = "fix deadbee, see https://example.com/c/8186904 and cafebabe1";
        let links = commit_message_link_ranges(text);
        assert_eq!(
            links
                .iter()
                .map(|link| (&text[link.range.clone()], link.kind))
                .collect::<Vec<_>>(),
            vec![
                ("deadbee", MessageLinkKind::CommitSha),
                ("https://example.com/c/8186904", MessageLinkKind::Url),
                ("cafebabe1", MessageLinkKind::CommitSha),
            ]
        );
        assert!(
            links
                .windows(2)
                .all(|pair| pair[0].range.end <= pair[1].range.start)
        );
    }
}
