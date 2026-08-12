//! Rope tests.
//!
//! The shape of the suite is borrowed from Zed's `crates/rope` tests: a set of
//! targeted cases for the awkward boundaries, plus a differential fuzz that
//! drives random `replace` operations against a `String` oracle and re-checks
//! every derived quantity after each one. The fuzz is the load-bearing part —
//! the summary monoid and the chunk-straddling arithmetic are exactly the kind
//! of code where hand-written cases pass and the tenth random edit does not.

use super::*;

/// Deterministic xorshift64*, so a failing seed is reproducible and no
/// dependency on `rand` is needed for a test-only generator.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(2685821657736338717).max(1))
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(2685821657736338717)
    }

    fn below(&mut self, limit: usize) -> usize {
        if limit == 0 {
            0
        } else {
            (self.next_u64() % limit as u64) as usize
        }
    }

    fn up_to(&mut self, limit: usize) -> usize {
        self.below(limit + 1)
    }

    fn bool(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }
}

/// A character mix that keeps every UTF-8 width in play, plus plenty of
/// newlines so the line arithmetic is exercised rather than incidental.
fn random_char(rng: &mut Rng) -> char {
    match rng.below(10) {
        0..=3 => (b'a' + rng.below(26) as u8) as char,
        4..=5 => '\n',
        6 => ' ',
        7 => 'é',         // 2 bytes, 1 UTF-16 unit
        8 => '√',         // 3 bytes, 1 UTF-16 unit
        _ => '\u{1F600}', // 4 bytes, 2 UTF-16 units (surrogate pair)
    }
}

fn random_text(rng: &mut Rng, len: usize) -> String {
    (0..len).map(|_| random_char(rng)).collect()
}

fn floor_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

// ── Oracles ────────────────────────────────────────────────────────────────

fn oracle_offset_to_point(text: &str, offset: usize) -> Point {
    let head = &text[..offset];
    let row = head.matches('\n').count() as u32;
    let column = match head.rfind('\n') {
        Some(newline) => (head.len() - newline - 1) as u32,
        None => head.len() as u32,
    };
    Point::new(row, column)
}

fn oracle_line_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for (index, _) in text.match_indices('\n') {
        ranges.push(start..index);
        start = index + 1;
    }
    ranges.push(start..text.len());
    ranges
}

fn oracle_longest_row(text: &str) -> (u32, u32) {
    let mut best_row = 0u32;
    let mut best_len = 0u32;
    for (row, range) in oracle_line_ranges(text).into_iter().enumerate() {
        let len = range.len() as u32;
        if len > best_len {
            best_row = row as u32;
            best_len = len;
        }
    }
    (best_row, best_len)
}

/// Assert every derived quantity against the oracle string.
fn assert_matches_oracle(rope: &Rope, expected: &str) {
    assert_eq!(rope.to_string(), expected, "text");
    assert!(*rope == *expected, "PartialEq<str>");
    assert_eq!(rope.len(), expected.len(), "len");
    assert_eq!(
        rope.len_utf16(),
        expected.chars().map(char::len_utf16).sum::<usize>(),
        "len_utf16"
    );

    let line_ranges = oracle_line_ranges(expected);
    assert_eq!(rope.line_count() as usize, line_ranges.len(), "line_count");
    assert_eq!(
        rope.max_point(),
        oracle_offset_to_point(expected, expected.len()),
        "max_point"
    );

    let (longest_row, longest_row_len) = oracle_longest_row(expected);
    assert_eq!(rope.longest_row_len(), longest_row_len, "longest_row_len");
    // The row *index* is only pinned down when it is unique; ties are resolved
    // differently by a left-to-right scan and by the summary merge, and either
    // answer is correct. Assert the length the reported row actually has.
    assert_eq!(
        rope.line_len(rope.longest_row()),
        longest_row_len,
        "longest_row {} should be a row of the maximal length (oracle said row {})",
        rope.longest_row(),
        longest_row,
    );

    for (row, range) in line_ranges.iter().enumerate() {
        let row = row as u32;
        assert_eq!(rope.line_len(row), range.len() as u32, "line_len({row})");
        assert_eq!(rope.line_range(row), *range, "line_range({row})");
        assert_eq!(
            rope.line_text(row),
            expected[range.clone()],
            "line_text({row})"
        );
    }
}

// ── Targeted cases ─────────────────────────────────────────────────────────

#[test]
fn empty_rope_is_one_empty_row() {
    let rope = Rope::new();
    assert_eq!(rope.len(), 0);
    assert!(rope.is_empty());
    assert_eq!(rope.line_count(), 1);
    assert_eq!(rope.max_point(), Point::ZERO);
    assert_eq!(rope.line_range(0), 0..0);
    assert_eq!(rope.to_string(), "");
    assert_matches_oracle(&rope, "");
}

#[test]
fn trailing_newline_creates_a_final_empty_row() {
    let rope = Rope::from_str("a\nb\n");
    assert_eq!(rope.line_count(), 3);
    assert_eq!(rope.max_point(), Point::new(2, 0));
    assert_eq!(rope.line_len(2), 0);
    assert_matches_oracle(&rope, "a\nb\n");
}

#[test]
fn summary_merges_longest_row_across_chunk_boundaries() {
    // The row that spans the seam is longer than any row wholly inside a
    // chunk, so this only passes if `add_summary` joins last_line + first_line.
    let long = "x".repeat(MAX_CHUNK_BYTES * 3);
    let text = format!("short\n{long}\nshort\n");
    let rope = Rope::from_str(&text);

    assert!(
        rope.summary().len > MAX_CHUNK_BYTES,
        "fixture must span several chunks"
    );
    assert_eq!(rope.longest_row(), 1);
    assert_eq!(rope.longest_row_len(), long.len() as u32);
    assert_matches_oracle(&rope, &text);
}

#[test]
fn summary_is_independent_of_how_the_text_was_pushed() {
    // Same bytes, different chunk boundaries: every summary field must agree,
    // or the monoid is not associative.
    let text = "alpha\nbeta gamma\n\ndelta\n".repeat(97);

    let bulk = Rope::from_str(&text);

    let mut byte_at_a_time = Rope::new();
    for ch in text.chars() {
        byte_at_a_time.push(ch.encode_utf8(&mut [0u8; 4]));
    }

    let mut chunked = Rope::new();
    let mut rest = text.as_str();
    while !rest.is_empty() {
        let take = floor_boundary(rest, 7.min(rest.len()));
        let take = take.max(1);
        chunked.push(&rest[..take]);
        rest = &rest[take..];
    }

    assert_eq!(bulk.summary(), byte_at_a_time.summary());
    assert_eq!(bulk.summary(), chunked.summary());
    assert_matches_oracle(&byte_at_a_time, &text);
    assert_matches_oracle(&chunked, &text);
}

#[test]
fn all_four_byte_chars() {
    let text = "🙂".repeat(MAX_CHUNK_BYTES);
    let rope = Rope::from_str(&text);
    assert_eq!(rope.len(), text.len());
    assert_eq!(rope.len_utf16(), text.chars().count() * 2);
    for (index, _) in text.char_indices() {
        assert!(rope.is_char_boundary(index), "boundary at {index}");
        assert_eq!(rope.offset_to_utf16(index), index / 4 * 2);
        assert_eq!(rope.offset_from_utf16(index / 4 * 2), index);
    }
    assert_matches_oracle(&rope, &text);
}

#[test]
fn clip_offset_moves_off_multibyte_interiors() {
    let text = "aé√🙂b";
    let rope = Rope::from_str(text);
    for offset in 0..=text.len() {
        let left = rope.clip_offset(offset, Bias::Left);
        let right = rope.clip_offset(offset, Bias::Right);
        assert!(text.is_char_boundary(left), "left {offset} -> {left}");
        assert!(text.is_char_boundary(right), "right {offset} -> {right}");
        assert!(left <= offset && offset <= right);
        if text.is_char_boundary(offset) {
            assert_eq!(left, offset);
            assert_eq!(right, offset);
        }
    }
}

#[test]
fn clip_point_clamps_row_and_column() {
    let rope = Rope::from_str("ab\ncdef\n");
    assert_eq!(rope.clip_point(Point::new(0, 99)), Point::new(0, 2));
    assert_eq!(rope.clip_point(Point::new(1, 99)), Point::new(1, 4));
    assert_eq!(rope.clip_point(Point::new(99, 0)), Point::new(2, 0));
    assert_eq!(rope.clip_point(Point::new(1, 2)), Point::new(1, 2));
}

#[test]
fn chunks_in_range_covers_exactly_the_requested_bytes() {
    let text = "0123456789\n".repeat(200);
    let rope = Rope::from_str(&text);
    assert!(
        rope.len() > MAX_CHUNK_BYTES * 2,
        "fixture must be multi-chunk"
    );

    for (start, end) in [
        (0, 0),
        (0, text.len()),
        (5, 5),
        (0, 1),
        (text.len() - 1, text.len()),
        (MAX_CHUNK_BYTES, MAX_CHUNK_BYTES),
        (MAX_CHUNK_BYTES - 1, MAX_CHUNK_BYTES + 1),
        (7, text.len() - 7),
    ] {
        let actual: String = rope.chunks_in_range(start..end).collect();
        assert_eq!(actual, text[start..end], "chunks_in_range({start}..{end})");
        assert_eq!(rope.text_for_range(start..end), text[start..end]);
    }
}

#[test]
fn chunks_never_yields_an_empty_slice() {
    let rope = Rope::from_str(&"ab\n".repeat(500));
    assert!(
        rope.chunks().all(|chunk| !chunk.is_empty()),
        "an empty chunk would make callers that track offsets loop forever"
    );
}

#[test]
fn out_of_bounds_ranges_are_clamped_rather_than_panicking() {
    let rope = Rope::from_str("abc");
    assert_eq!(rope.text_for_range(0..999), "abc");
    assert_eq!(rope.text_for_range(999..1000), "");
    // Reversed ranges collapse to empty instead of panicking. The lint fires on
    // exactly the literal this case exists to exercise.
    #[allow(clippy::reversed_empty_ranges)]
    {
        assert_eq!(rope.text_for_range(2..1), "");
    }
    assert_eq!(rope.offset_to_point(999), Point::new(0, 3));
    assert_eq!(rope.point_to_offset(Point::new(9, 9)), 3);
    assert_eq!(rope.offset_to_utf16(999), 3);
    assert_eq!(rope.offset_from_utf16(999), 3);
    assert!(!rope.is_char_boundary(4));
}

#[test]
fn replace_at_the_very_end_appends() {
    let mut rope = Rope::from_str("abc");
    rope.replace(3..3, "def");
    assert_matches_oracle(&rope, "abcdef");
}

#[test]
fn replace_spanning_the_whole_document() {
    let mut rope = Rope::from_str(&"old\n".repeat(400));
    rope.replace(0..rope.len(), "new");
    assert_matches_oracle(&rope, "new");
}

#[test]
fn append_joins_without_leaving_a_short_interior_chunk() {
    let mut left = Rope::from_str("abc");
    left.append(Rope::from_str(&"z".repeat(MAX_CHUNK_BYTES * 2)));
    let expected = format!("abc{}", "z".repeat(MAX_CHUNK_BYTES * 2));
    assert_matches_oracle(&left, &expected);

    let mut empty = Rope::new();
    empty.append(Rope::from_str("only"));
    assert_matches_oracle(&empty, "only");

    let mut lhs = Rope::from_str("only");
    lhs.append(Rope::new());
    assert_matches_oracle(&lhs, "only");
}

#[test]
fn point_and_offset_round_trip_across_chunk_boundaries() {
    let text = "line\n".repeat(500);
    let rope = Rope::from_str(&text);
    for offset in (0..=text.len()).step_by(7) {
        let point = rope.offset_to_point(offset);
        assert_eq!(point, oracle_offset_to_point(&text, offset), "at {offset}");
        assert_eq!(
            rope.point_to_offset(point),
            offset,
            "round trip at {offset}"
        );
    }
}

#[test]
fn rope_equality_compares_content_not_chunking() {
    let text = "equal\n".repeat(300);
    let bulk = Rope::from_str(&text);
    let mut piecemeal = Rope::new();
    for chunk in text.as_bytes().chunks(3) {
        piecemeal.push(std::str::from_utf8(chunk).expect("ascii fixture"));
    }
    assert_eq!(bulk, piecemeal);
    assert_ne!(bulk, Rope::from_str(&text[..text.len() - 1]));
}

// ── Differential fuzz ──────────────────────────────────────────────────────

fn fuzz_seed(seed: u64, operations: usize) {
    let mut rng = Rng::new(seed);
    let mut expected = String::new();
    let mut actual = Rope::new();

    for op in 0..operations {
        let end = floor_boundary(&expected, rng.up_to(expected.len()));
        let start = floor_boundary(&expected, rng.up_to(end));
        let new_text = {
            let len = rng.up_to(24);
            random_text(&mut rng, len)
        };

        actual.replace(start..end, &new_text);
        expected.replace_range(start..end, &new_text);

        assert_matches_oracle(&actual, &expected);

        // Random reads over the mutated document.
        for _ in 0..4 {
            let end = floor_boundary(&expected, rng.up_to(expected.len()));
            let start = floor_boundary(&expected, rng.up_to(end));

            let read: String = actual.chunks_in_range(start..end).collect();
            assert_eq!(
                read,
                expected[start..end],
                "seed {seed} op {op}: chunks_in_range({start}..{end})"
            );

            let offset = floor_boundary(&expected, rng.up_to(expected.len()));
            assert_eq!(
                actual.offset_to_point(offset),
                oracle_offset_to_point(&expected, offset),
                "seed {seed} op {op}: offset_to_point({offset})"
            );
            assert_eq!(
                actual.point_to_offset(actual.offset_to_point(offset)),
                offset,
                "seed {seed} op {op}: point round trip at {offset}"
            );

            let utf16 = expected[..offset]
                .chars()
                .map(char::len_utf16)
                .sum::<usize>();
            assert_eq!(
                actual.offset_to_utf16(offset),
                utf16,
                "seed {seed} op {op}: offset_to_utf16({offset})"
            );
            assert_eq!(
                actual.offset_from_utf16(utf16),
                offset,
                "seed {seed} op {op}: offset_from_utf16({utf16})"
            );

            // Every byte index must agree with `str::is_char_boundary`.
            let probe = rng.up_to(expected.len());
            assert_eq!(
                actual.is_char_boundary(probe),
                expected.is_char_boundary(probe),
                "seed {seed} op {op}: is_char_boundary({probe})"
            );
        }

        // Occasionally exercise the append path too, since it has its own
        // seam handling.
        if rng.bool() && expected.len() < 4096 {
            let tail = {
                let len = rng.up_to(48);
                random_text(&mut rng, len)
            };
            actual.append(Rope::from_str(&tail));
            expected.push_str(&tail);
            assert_matches_oracle(&actual, &expected);
        }
    }
}

#[test]
fn random_edits_match_a_string_oracle() {
    for seed in 0..48 {
        fuzz_seed(seed, 40);
    }
}

/// The scenario the whole design exists for: small edits landing in the middle
/// of a document that stays many chunks deep, so every edit is in a tree
/// interior rather than in a single leaf. Spans are bounded (rather than random
/// over the whole document) both to keep the fixture large and because that is
/// what typing inside a block actually looks like.
#[test]
fn small_edits_inside_a_large_document_match_a_string_oracle() {
    for seed in 100..116 {
        let mut rng = Rng::new(seed);
        let mut expected = random_text(&mut rng, MAX_CHUNK_BYTES * 6);
        let mut actual = Rope::from_str(&expected);
        assert_matches_oracle(&actual, &expected);

        for op in 0..24 {
            let start = floor_boundary(&expected, rng.up_to(expected.len()));
            let span = rng.up_to(96.min(expected.len() - start));
            let end = floor_boundary(&expected, start + span);
            let new_text = {
                let len = rng.up_to(64);
                random_text(&mut rng, len)
            };

            actual.replace(start..end, &new_text);
            expected.replace_range(start..end, &new_text);

            assert_matches_oracle(&actual, &expected);
            assert!(
                expected.len() > MAX_CHUNK_BYTES,
                "seed {seed} op {op}: fixture shrank out of the multi-chunk regime"
            );
        }
    }
}

/// A snapshot taken before an edit must keep observing the old text — the
/// property the whole copy-on-write design rests on.
#[test]
fn clones_are_snapshots_unaffected_by_later_edits() {
    let mut rope = Rope::from_str(&"before\n".repeat(300));
    let before = rope.to_string();
    let snapshot = rope.clone();

    rope.replace(0..6, "after");

    assert_matches_oracle(&snapshot, &before);
    assert_ne!(rope, snapshot);
    assert!(rope.to_string().starts_with("after\n"));
}

/// The claim the whole design rests on, asserted structurally rather than by
/// timing: a one-character edit in a large document must leave almost every
/// chunk allocation untouched and shared with the pre-edit snapshot.
///
/// A piece table that re-concatenates, or any implementation that copies the
/// document per edit, fails this outright — the shared count would be zero.
#[test]
fn a_small_edit_shares_all_but_a_few_chunks_with_the_previous_version() {
    let text = "fn value() -> usize { 42 }\n".repeat(4000);
    let mut rope = Rope::from_str(&text);
    let before = rope.clone();
    let chunk_count = before.chunk_identities().len();
    assert!(
        chunk_count > 100,
        "fixture should be many chunks deep, got {chunk_count}"
    );

    // Edit in the middle, the case a "streaming" claim has to survive.
    let offset = rope.clip_offset(rope.len() / 2, Bias::Left);
    rope.replace(offset..offset, "x");

    let old: std::collections::HashSet<_> = before.chunk_identities().into_iter().collect();
    let shared = rope
        .chunk_identities()
        .into_iter()
        .filter(|ptr| old.contains(ptr))
        .count();

    // Only the chunks straddling the edit may be rebuilt; everything else is
    // the same allocation the snapshot still points at.
    assert!(
        shared >= chunk_count - 4,
        "expected a mid-document edit to share nearly every chunk, but only \
         {shared} of {chunk_count} survived"
    );

    // And the snapshot must still read as the original text.
    assert_eq!(before.len(), text.len());
    assert_eq!(rope.len(), text.len() + 1);
}

/// Reading a window must not depend on document size: the chunks yielded for a
/// small range are bounded by the range, not by the buffer.
#[test]
fn reading_a_window_touches_only_that_window() {
    let text = "0123456789\n".repeat(20_000);
    let rope = Rope::from_str(&text);

    let start = rope.clip_offset(text.len() / 2, Bias::Left);
    let end = start + 200;
    let chunks: Vec<_> = rope.chunks_in_range(start..end).collect();

    assert_eq!(chunks.concat(), text[start..end]);
    assert!(
        chunks.len() <= 200 / MAX_CHUNK_BYTES + 2,
        "a 200-byte read yielded {} chunks",
        chunks.len()
    );
}

/// Every offset-taking read must be total, including inside a character.
///
/// Offsets reach the rope from places that cannot promise boundary alignment —
/// an IME marked range, a caret carried across an edit, a row end whose last
/// character is not ASCII. Slicing a chunk as `&str` at such an offset panics,
/// so each of these clips first. Widening (start left, end right) is the repair:
/// a partial character is not representable, so the range grows to whole ones.
#[test]
fn reads_at_offsets_inside_a_character_clip_instead_of_panicking() {
    let rope = Rope::from_str("a\u{e9}\u{1f642}z");
    // Byte layout: a=0, é=1..3, 🙂=3..7, z=7. len=8.
    assert_eq!(rope.len(), 8);

    // Interior offsets widen to the enclosing character rather than panicking.
    assert_eq!(rope.text_for_range(2..3), "\u{e9}");
    assert_eq!(rope.text_for_range(4..6), "\u{1f642}");
    assert_eq!(rope.text_for_range(2..6), "\u{e9}\u{1f642}");

    // UTF-16 conversion clips left, so an interior offset answers as its
    // character's start rather than aborting.
    assert_eq!(rope.offset_to_utf16(2), rope.offset_to_utf16(1));
    assert_eq!(rope.offset_to_utf16(5), rope.offset_to_utf16(3));
    assert_eq!(rope.offset_to_utf16(8), rope.len_utf16());

    // Replacement covers whole characters: half of one cannot be replaced.
    let mut edited = rope.clone();
    edited.replace(4..6, "X");
    assert_eq!(edited.to_string(), "a\u{e9}Xz");
}

/// A mid-character range must not silently drop the character it lands in.
#[test]
fn a_replace_that_starts_inside_a_character_removes_the_whole_character() {
    let mut rope = Rope::from_str("\u{1f642}");
    rope.replace(1..3, "x");
    assert_eq!(
        rope.to_string(),
        "x",
        "a range touching part of a character consumes all of it"
    );
    assert!(rope.is_char_boundary(rope.len()));
}

/// Typing must not fragment the tree.
///
/// An edit rebuilds the straddled chunk plus the inserted text and re-splits the
/// result. Splitting greedily leaves a sliver behind on every keystroke — a
/// one-character insert into a full chunk yields `[512][1]` — and those slivers
/// accumulate for the life of the document, each costing an `Arc` allocation and
/// a `TextSummary` and deepening every descent.
#[test]
fn many_small_edits_do_not_fragment_the_tree() {
    let text = "fn f(v: usize) -> String { format!(\"x\", v) }\n".repeat(500);
    let mut rope = Rope::from_str(&text);
    let mut oracle = text.clone();

    let edits = 500usize;
    for ix in 0..edits {
        let at = rope.clip_offset((ix * 7) % rope.len(), gpui::sum_tree::Bias::Left);
        rope.replace(at..at, "x");
        oracle.insert(at, 'x');
    }

    assert_eq!(rope.to_string(), oracle, "edits must still be correct");

    let sizes = rope.chunk_sizes_for_test();
    // Growth is bounded by the bytes added, not by the number of edits.
    assert!(
        sizes.len() < 100,
        "{edits} one-character inserts left {} chunks; the tree is fragmenting \
         in proportion to keystrokes",
        sizes.len()
    );
    // A handful of small chunks is fine (document ends, appends); one per edit
    // is the failure this guards.
    let slivers = sizes.iter().filter(|len| **len < 64).count();
    assert!(
        slivers < 16,
        "{slivers} chunks under 64 bytes after {edits} edits: {sizes:?}"
    );
}
