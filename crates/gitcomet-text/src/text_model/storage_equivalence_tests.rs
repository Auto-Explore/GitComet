use super::*;
use crate::rope::Rope;

/// Deterministic xorshift64*, matching the rope's own test generator.
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

    fn up_to(&mut self, limit: usize) -> usize {
        if limit == usize::MAX {
            return 0;
        }
        (self.next_u64() % (limit as u64 + 1)) as usize
    }
}

fn random_text(rng: &mut Rng, len: usize) -> String {
    (0..len)
        .map(|_| match rng.up_to(9) {
            0..=3 => (b'a' + rng.up_to(25) as u8) as char,
            4..=5 => '\n',
            6 => ' ',
            7 => 'é',
            8 => '√',
            _ => '\u{1F600}',
        })
        .collect()
}

fn floor_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn assert_agree(model: &TextModelSnapshot, rope: &Rope, expected: &str, context: &str) {
    assert_eq!(model.len(), rope.len(), "{context}: len");
    assert_eq!(model.len(), expected.len(), "{context}: len vs oracle");
    assert_eq!(
        model.line_count(),
        rope.line_count() as usize,
        "{context}: line_count"
    );

    for row in 0..model.line_count() {
        let model_range = model.line_range(row);
        let rope_range = rope.line_range(row as u32);
        assert_eq!(model_range, rope_range, "{context}: line_range({row})");
        assert_eq!(
            model.line_text(row).as_ref(),
            rope.line_text(row as u32),
            "{context}: line_text({row})"
        );
        assert_eq!(
            model.line_text(row).as_ref(),
            &expected[model_range],
            "{context}: line_text({row}) vs oracle"
        );
    }
}

#[test]
fn piece_table_and_rope_agree_across_random_edits() {
    for seed in 0..24 {
        let mut rng = Rng::new(seed);
        let mut expected = String::new();
        let mut model = TextModel::new();
        let mut rope = Rope::new();

        for op in 0..30 {
            let end = floor_boundary(&expected, rng.up_to(expected.len()));
            let start = floor_boundary(&expected, rng.up_to(end));
            let inserted = {
                let len = rng.up_to(24);
                random_text(&mut rng, len)
            };

            model.replace_range(start..end, &inserted);
            rope.replace(start..end, &inserted);
            expected.replace_range(start..end, &inserted);

            let context = format!("seed {seed} op {op}");
            let snapshot = model.snapshot();
            assert_agree(&snapshot, &rope, &expected, &context);

            // Windowed reads and the UTF-16 conversions, at random probes.
            for _ in 0..4 {
                let probe = floor_boundary(&expected, rng.up_to(expected.len()));
                let oracle_utf16 = expected[..probe]
                    .chars()
                    .map(char::len_utf16)
                    .sum::<usize>();

                assert_eq!(
                    snapshot.offset_to_utf16(probe),
                    oracle_utf16,
                    "{context}: offset_to_utf16({probe})"
                );
                assert_eq!(
                    snapshot.offset_to_utf16(probe),
                    rope.offset_to_utf16(probe),
                    "{context}: offset_to_utf16({probe}) vs rope"
                );
                assert_eq!(
                    snapshot.offset_from_utf16(oracle_utf16),
                    probe,
                    "{context}: offset_from_utf16({oracle_utf16})"
                );
                assert_eq!(
                    snapshot.offset_from_utf16(oracle_utf16),
                    rope.offset_from_utf16(oracle_utf16),
                    "{context}: offset_from_utf16({oracle_utf16}) vs rope"
                );

                let end = floor_boundary(&expected, rng.up_to(expected.len()));
                let start = floor_boundary(&expected, rng.up_to(end));
                assert_eq!(
                    snapshot.slice(start..end).as_ref(),
                    &expected[start..end],
                    "{context}: slice({start}..{end})"
                );
                assert_eq!(
                    snapshot.slice(start..end).as_ref(),
                    rope.text_for_range(start..end),
                    "{context}: slice({start}..{end}) vs rope"
                );
            }
        }
    }
}

/// Reading one row must not depend on the document's size — the property
/// the renderer relies on, and the reason `line_text` exists at all.
#[test]
fn line_text_reads_a_single_row_of_a_large_document() {
    let text = "the quick brown fox\n".repeat(50_000);
    let model = TextModel::from_large_text(&text);
    let snapshot = model.snapshot();
    let rope = Rope::from_text(&text);

    assert_eq!(snapshot.line_count(), 50_001);
    for row in [0usize, 1, 25_000, 49_999, 50_000] {
        assert_eq!(
            snapshot.line_text(row).as_ref(),
            rope.line_text(row as u32),
            "row {row}"
        );
    }
    assert_eq!(snapshot.line_text(0).as_ref(), "the quick brown fox");
    assert_eq!(snapshot.line_text(50_000).as_ref(), "");
}

/// ASCII documents — nearly all of them — must answer UTF-16 conversions
/// without inspecting the text at all.
#[test]
fn ascii_documents_convert_utf16_offsets_by_identity() {
    let model = TextModel::from_large_text(&"plain ascii\n".repeat(1000));
    let snapshot = model.snapshot();
    for probe in [0usize, 1, 500, snapshot.len()] {
        assert_eq!(snapshot.offset_to_utf16(probe), probe);
        assert_eq!(snapshot.offset_from_utf16(probe), probe);
    }
}

#[test]
fn utf16_conversions_handle_surrogate_pairs() {
    let text = "a🙂b";
    let model = TextModel::from_large_text(text);
    let snapshot = model.snapshot();
    let rope = Rope::from_text(text);

    // 'a' = 1 unit, '🙂' = 2 units (surrogate pair), 'b' = 1 unit.
    assert_eq!(snapshot.offset_to_utf16(0), 0);
    assert_eq!(snapshot.offset_to_utf16(1), 1);
    assert_eq!(snapshot.offset_to_utf16(5), 3);
    assert_eq!(snapshot.offset_to_utf16(6), 4);
    assert_eq!(snapshot.offset_from_utf16(3), 5);

    for probe in 0..=4 {
        assert_eq!(
            snapshot.offset_from_utf16(probe),
            rope.offset_from_utf16(probe),
            "offset_from_utf16({probe})"
        );
    }
}
