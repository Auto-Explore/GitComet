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

#[cfg(test)]
mod tests {
    use super::token_range_for_offset;

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
}
