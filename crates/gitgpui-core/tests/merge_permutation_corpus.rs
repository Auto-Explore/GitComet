//! KDiff3-style generated permutation corpus for merge regression testing.
//!
//! This ports the option table used by KDiff3's
//! `generate_testdata_from_permutations.py` into an in-process Rust generator.
//! The test validates that `merge_file` handles broad combinations of
//! unchanged / modified / deleted / added lines without violating basic
//! invariants.

use gitgpui_core::merge::{merge_file, MergeOptions};
use std::collections::HashSet;

const DEFAULT_LINES: [&str; 5] = ["aaa\n", "bbb\n", "ccc\n", "ddd\n", "eee\n"];

/// Option table ported from KDiff3's permutation generator.
///
/// Tuple fields are `(base, contrib1, contrib2)`:
/// - `Some(1)` => original line
/// - `Some(2)` => modified line with `xxx` prefix
/// - `Some(3)` => modified line with `yyy` prefix (contrib2 only)
/// - `None`    => line absent
const OPTIONS: [(Option<u8>, Option<u8>, Option<u8>); 11] = [
    (Some(1), Some(1), Some(1)),
    (Some(1), Some(1), Some(2)),
    (Some(1), Some(1), None),
    (Some(1), Some(2), Some(1)),
    (Some(1), Some(2), Some(2)),
    (Some(1), Some(2), Some(3)),
    (Some(1), Some(2), None),
    (None, Some(1), Some(1)),
    (None, Some(1), Some(2)),
    (None, Some(1), None),
    (None, None, Some(1)),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CorpusKind {
    Sampled { options_per_line: usize, seed: u64 },
    Exhaustive,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PermutationCase {
    id: String,
    base: String,
    contrib1: String,
    contrib2: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct CorpusSummary {
    total: usize,
    clean: usize,
    conflicts: usize,
}

#[test]
fn kdiff3_permutation_corpus_sampled_r3_seed0() {
    let summary = run_corpus(CorpusKind::Sampled {
        options_per_line: 3,
        seed: 0,
    });

    assert_eq!(summary.total, 243, "expected 3^5 sampled cases");
    assert!(
        summary.conflicts > 0,
        "expected at least one conflicted merge in sampled corpus"
    );
}

/// Exhaustive run of all 11^5 permutations (161_051 cases).
///
/// This is intentionally ignored to keep default test runs fast:
/// `cargo test -p gitgpui-core --test merge_permutation_corpus -- --ignored`
#[test]
#[ignore]
fn kdiff3_permutation_corpus_exhaustive_11_pow_5() {
    let summary = run_corpus(CorpusKind::Exhaustive);
    assert_eq!(summary.total, 161_051);
    assert!(summary.clean > 0);
    assert!(summary.conflicts > 0);
}

fn run_corpus(kind: CorpusKind) -> CorpusSummary {
    let cases = generate_cases(kind);
    let mut summary = CorpusSummary::default();

    for case in &cases {
        let result = merge_file(
            &case.base,
            &case.contrib1,
            &case.contrib2,
            &MergeOptions::default(),
        );

        validate_marker_wellformedness(&result.output, &case.id);
        validate_content_integrity(
            &case.base,
            &case.contrib1,
            &case.contrib2,
            &result.output,
            &case.id,
        );
        validate_context_preservation(
            &case.base,
            &case.contrib1,
            &case.contrib2,
            &result.output,
            &case.id,
        );

        let marker_conflicts = count_open_markers(&result.output);
        assert_eq!(
            marker_conflicts, result.conflict_count,
            "[{}] marker conflict count does not match reported conflict_count",
            case.id
        );

        summary.total += 1;
        if result.conflict_count == 0 {
            summary.clean += 1;
        } else {
            summary.conflicts += 1;
        }
    }

    summary
}

fn generate_cases(kind: CorpusKind) -> Vec<PermutationCase> {
    let index_vectors = match kind {
        CorpusKind::Sampled {
            options_per_line,
            seed,
        } => generate_sampled_indices(options_per_line, seed),
        CorpusKind::Exhaustive => generate_exhaustive_indices(),
    };

    index_vectors
        .into_iter()
        .map(|indices| build_case(&indices))
        .collect()
}

fn generate_exhaustive_indices() -> Vec<[usize; 5]> {
    let mut out = Vec::with_capacity(11usize.pow(5));
    let mut current = [0usize; 5];
    generate_indices_recursive_exhaustive(0, &mut current, &mut out);
    out
}

fn generate_indices_recursive_exhaustive(
    depth: usize,
    current: &mut [usize; 5],
    out: &mut Vec<[usize; 5]>,
) {
    if depth == 5 {
        out.push(*current);
        return;
    }

    for option_ix in 0..OPTIONS.len() {
        current[depth] = option_ix;
        generate_indices_recursive_exhaustive(depth + 1, current, out);
    }
}

fn generate_sampled_indices(options_per_line: usize, seed: u64) -> Vec<[usize; 5]> {
    assert!(
        options_per_line > 0 && options_per_line <= OPTIONS.len(),
        "options_per_line must be in 1..={}",
        OPTIONS.len()
    );

    let mut out = Vec::with_capacity(options_per_line.pow(5u32));
    let mut current = [0usize; 5];
    let mut rng = LcgRng::new(seed);
    generate_indices_recursive_sampled(options_per_line, 0, &mut current, &mut rng, &mut out);
    out
}

fn generate_indices_recursive_sampled(
    options_per_line: usize,
    depth: usize,
    current: &mut [usize; 5],
    rng: &mut LcgRng,
    out: &mut Vec<[usize; 5]>,
) {
    if depth == 5 {
        out.push(*current);
        return;
    }

    let option_indices = sample_without_replacement(OPTIONS.len(), options_per_line, rng);
    for option_ix in option_indices {
        current[depth] = option_ix;
        generate_indices_recursive_sampled(options_per_line, depth + 1, current, rng, out);
    }
}

fn sample_without_replacement(total: usize, count: usize, rng: &mut LcgRng) -> Vec<usize> {
    let mut pool: Vec<usize> = (0..total).collect();
    for i in 0..count {
        let remaining = total - i;
        let offset = (rng.next_u64() as usize) % remaining;
        pool.swap(i, i + offset);
    }
    pool.into_iter().take(count).collect()
}

fn build_case(indices: &[usize; 5]) -> PermutationCase {
    let mut base = String::new();
    let mut contrib1 = String::new();
    let mut contrib2 = String::new();

    for (line_ix, option_ix) in indices.iter().copied().enumerate() {
        let option = OPTIONS[option_ix];
        let default_line = DEFAULT_LINES[line_ix];

        if option.0.is_some() {
            base.push_str(default_line);
        }

        match option.1 {
            Some(1) => contrib1.push_str(default_line),
            Some(2) => contrib1.push_str(&format!("xxx{default_line}")),
            Some(other) => panic!("unsupported contrib1 option value: {other}"),
            None => {}
        }

        match option.2 {
            Some(1) => contrib2.push_str(default_line),
            Some(2) => contrib2.push_str(&format!("xxx{default_line}")),
            Some(3) => contrib2.push_str(&format!("yyy{default_line}")),
            Some(other) => panic!("unsupported contrib2 option value: {other}"),
            None => {}
        }
    }

    let id = indices
        .iter()
        .map(|ix| format!("{ix:x}"))
        .collect::<Vec<_>>()
        .join("");

    PermutationCase {
        id: format!("perm_{id}"),
        base,
        contrib1,
        contrib2,
    }
}

fn validate_marker_wellformedness(output: &str, case_id: &str) {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum State {
        Outside,
        InOurs,
        InBase,
        InTheirs,
    }

    let mut state = State::Outside;
    for (line_ix, line) in output.lines().enumerate() {
        let line_no = line_ix + 1;
        if is_open_marker(line) {
            assert_eq!(
                state,
                State::Outside,
                "[{case_id}] line {line_no}: nested <<<<<<< marker"
            );
            state = State::InOurs;
        } else if is_base_marker(line) {
            assert_eq!(
                state,
                State::InOurs,
                "[{case_id}] line {line_no}: unexpected ||||||| marker"
            );
            state = State::InBase;
        } else if is_separator_marker(line) {
            assert!(
                state == State::InOurs || state == State::InBase,
                "[{case_id}] line {line_no}: unexpected ======= marker"
            );
            state = State::InTheirs;
        } else if is_close_marker(line) {
            assert_eq!(
                state,
                State::InTheirs,
                "[{case_id}] line {line_no}: unexpected >>>>>>> marker"
            );
            state = State::Outside;
        }
    }

    assert_eq!(state, State::Outside, "[{case_id}] unclosed marker block");
}

fn validate_content_integrity(
    base: &str,
    contrib1: &str,
    contrib2: &str,
    output: &str,
    case_id: &str,
) {
    let base_lines: HashSet<&str> = base.lines().collect();
    let contrib1_lines: HashSet<&str> = contrib1.lines().collect();
    let contrib2_lines: HashSet<&str> = contrib2.lines().collect();

    for (line_ix, line) in output.lines().enumerate() {
        if is_open_marker(line)
            || is_base_marker(line)
            || is_separator_marker(line)
            || is_close_marker(line)
        {
            continue;
        }

        assert!(
            base_lines.contains(line)
                || contrib1_lines.contains(line)
                || contrib2_lines.contains(line),
            "[{case_id}] line {} is not traceable to base/local/remote content: {:?}",
            line_ix + 1,
            line
        );
    }
}

fn validate_context_preservation(
    base: &str,
    contrib1: &str,
    contrib2: &str,
    output: &str,
    case_id: &str,
) {
    let contrib1_lines: HashSet<&str> = contrib1.lines().collect();
    let contrib2_lines: HashSet<&str> = contrib2.lines().collect();
    let output_lines: HashSet<&str> = output.lines().collect();

    for line in base.lines() {
        if contrib1_lines.contains(line) && contrib2_lines.contains(line) {
            assert!(
                output_lines.contains(line),
                "[{case_id}] line common to all inputs missing from output: {:?}",
                line
            );
        }
    }
}

fn count_open_markers(output: &str) -> usize {
    output.lines().filter(|line| is_open_marker(line)).count()
}

fn is_open_marker(line: &str) -> bool {
    line.starts_with("<<<<<<<")
}

fn is_base_marker(line: &str) -> bool {
    line.starts_with("|||||||")
}

fn is_separator_marker(line: &str) -> bool {
    line.starts_with("=======")
}

fn is_close_marker(line: &str) -> bool {
    line.starts_with(">>>>>>>")
}

#[derive(Clone, Copy, Debug)]
struct LcgRng {
    state: u64,
}

impl LcgRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        // Numerical Recipes LCG constants.
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1);
        self.state
    }
}
