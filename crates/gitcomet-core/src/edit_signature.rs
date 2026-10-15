//! A file's edit as text, reduced to a value two files can compare. Files that
//! share one were changed the same way: the same lines removed and the same
//! lines added, in the same order. A changed-file list sorted by edit puts
//! them next to each other, so a repeated change reads as one.
//!
//! Exact text only, after trimming each line: a rename that differs by one
//! character is a different edit. Indentation and line endings do not count,
//! so the same change made at two nesting depths, or on a CRLF checkout, still
//! matches.

use std::hash::Hasher as _;

/// What a file's change does, as text. Equal for two files whose removed and
/// added lines are the same, in the same order. Only meaningful within one
/// process: it is not stable across builds and is never stored.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EditSignature(u64);

/// Builds an [`EditSignature`] from a diff's changed lines, fed in diff order
/// (each hunk's removed lines, then its added lines).
#[derive(Default)]
pub struct EditSignatureBuilder {
    hasher: rustc_hash::FxHasher,
    any: bool,
}

impl EditSignatureBuilder {
    pub fn removed(&mut self, line: &[u8]) {
        self.line(b'-', line);
    }

    pub fn added(&mut self, line: &[u8]) {
        self.line(b'+', line);
    }

    /// Every line of `text` as added, for a side that did not exist before;
    /// returns how many lines that is.
    pub fn added_lines(&mut self, text: &[u8]) -> u32 {
        lines(text).fold(0, |count, line| {
            self.added(line);
            count.saturating_add(1)
        })
    }

    /// Every line of `text` as removed, for a side that no longer exists;
    /// returns how many lines that is.
    pub fn removed_lines(&mut self, text: &[u8]) -> u32 {
        lines(text).fold(0, |count, line| {
            self.removed(line);
            count.saturating_add(1)
        })
    }

    /// `None` when no line changed.
    pub fn finish(self) -> Option<EditSignature> {
        self.any.then(|| EditSignature(self.hasher.finish()))
    }

    fn line(&mut self, sign: u8, line: &[u8]) {
        self.any = true;
        // The sign opens each line and a newline closes it. A trimmed line
        // holds no newline, so no two line sequences hash the same bytes.
        self.hasher.write_u8(sign);
        self.hasher.write(line.trim_ascii());
        self.hasher.write_u8(b'\n');
    }
}

/// `text` split the way a line diff tokenizes it: on `\n`, with a last line
/// that has no newline still a line.
fn lines(text: &[u8]) -> impl Iterator<Item = &[u8]> {
    (!text.is_empty())
        .then(|| {
            text.strip_suffix(b"\n")
                .unwrap_or(text)
                .split(|byte| *byte == b'\n')
        })
        .into_iter()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signature(removed: &[&str], added: &[&str]) -> Option<EditSignature> {
        let mut builder = EditSignatureBuilder::default();
        for line in removed {
            builder.removed(line.as_bytes());
        }
        for line in added {
            builder.added(line.as_bytes());
        }
        builder.finish()
    }

    #[test]
    fn the_same_lines_changed_the_same_way_share_a_signature() {
        let one = signature(&["use a;"], &["use b;"]);
        assert!(one.is_some());
        assert_eq!(one, signature(&["    use a;\r\n"], &["\tuse b;\n"]));
    }

    #[test]
    fn direction_order_and_text_all_count() {
        let edit = signature(&["a"], &["b"]);
        assert_ne!(edit, signature(&["b"], &["a"]));
        assert_ne!(edit, signature(&[], &["a", "b"]));
        assert_ne!(signature(&[], &["a", "b"]), signature(&[], &["b", "a"]),);
        assert_ne!(edit, signature(&["a"], &["c"]));
    }

    #[test]
    fn nothing_changed_has_no_signature() {
        assert_eq!(EditSignatureBuilder::default().finish(), None);
        let mut empty = EditSignatureBuilder::default();
        assert_eq!(empty.added_lines(b""), 0);
        assert_eq!(empty.finish(), None);
    }

    #[test]
    fn a_whole_file_reads_as_its_lines() {
        let mut whole = EditSignatureBuilder::default();
        assert_eq!(whole.added_lines(b"a\nb\n"), 2);
        assert_eq!(whole.finish(), signature(&[], &["a", "b"]));

        let mut unterminated = EditSignatureBuilder::default();
        assert_eq!(unterminated.added_lines(b"a\nb"), 2);
        assert_eq!(unterminated.finish(), signature(&[], &["a", "b"]));

        let mut blank_kept = EditSignatureBuilder::default();
        assert_eq!(blank_kept.removed_lines(b"a\n\nb\n"), 3);
        assert_eq!(blank_kept.finish(), signature(&["a", "", "b"], &[]));
    }

    /// A file holding one empty line is a one-line file, as its count says.
    #[test]
    fn a_lone_newline_is_one_blank_line() {
        let mut blank = EditSignatureBuilder::default();
        assert_eq!(blank.added_lines(b"\n"), 1);
        assert_eq!(blank.finish(), signature(&[], &[""]));
    }
}
