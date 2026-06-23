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

pub(crate) fn commit_sha_ranges(text: &str) -> Vec<Range<usize>> {
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
        if (MIN_SHA_LEN..=MAX_SHA_LEN).contains(&len) {
            ranges.push(start..cursor);
        }
    }

    ranges
}

#[cfg(test)]
mod tests {
    use super::{commit_sha_ranges, token_range_for_offset};

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
}
