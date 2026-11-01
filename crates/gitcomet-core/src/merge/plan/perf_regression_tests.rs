use super::*;
use crate::merge::{MergeOptions, render_merge_plan};

fn three_way_fixture(lines: usize) -> (String, String, String) {
    let mut base = String::with_capacity(lines * 24);
    let mut local = String::with_capacity(lines * 24);
    let mut remote = String::with_capacity(lines * 24);
    for ix in 0..lines {
        let line = format!("    fn item_{ix}() -> u32 {{ {ix} }}\n");
        base.push_str(&line);
        match ix % 350 {
            // Both sides change the same line differently: a conflict.
            0 => {
                local.push_str(&format!("    fn item_{ix}() -> u32 {{ {ix} + 1 }}\n"));
                remote.push_str(&format!("    fn item_{ix}() -> u32 {{ {ix} + 2 }}\n"));
            }
            // Local-only edit, plus an insertion.
            50 | 100 => {
                local.push_str(&format!("    fn item_{ix}() -> u32 {{ {ix} * 2 }}\n"));
                local.push_str("    // local note\n");
                remote.push_str(&line);
            }
            // Remote-only edit with whitespace-only change elsewhere.
            70 | 140 => {
                local.push_str(&line);
                remote.push_str(&format!("    fn item_{ix}() -> u32 {{ {ix} * 3 }}\n"));
            }
            7 => {
                local.push_str(&line);
                remote.push_str(&format!("  fn item_{ix}()  ->  u32 {{ {ix} }}\n"));
            }
            _ => {
                local.push_str(&line);
                remote.push_str(&line);
            }
        }
    }
    (base, local, remote)
}

#[test]
fn equal_ignoring_whitespace_matches_the_normalized_comparison() {
    let cases = [
        ("", ""),
        ("a", "a"),
        ("a b", "ab"),
        (" a\tb ", "a b"),
        ("ab", "a c"),
        ("a", ""),
        ("", " "),
        ("é ü", "éü"),
        ("x  y  z", "xyz"),
        ("xyz", "xy"),
    ];
    for (left, right) in cases {
        assert_eq!(
            equal_ignoring_whitespace(left, right),
            normalized_without_whitespace(left) == normalized_without_whitespace(right),
            "{left:?} vs {right:?}"
        );
    }
}

#[test]
fn line_count_matches_split_lines() {
    for text in [
        "",
        "a",
        "a\n",
        "a\nb",
        "a\nb\n",
        "\n",
        "\n\n",
        "a\r\nb\r\n",
        "a\r\nb",
    ] {
        assert_eq!(line_count(text), split_lines(text).len(), "{text:?}");
    }
}

#[test]
fn block_ancestor_ranges_match_per_block_lookup() {
    let (base, local, remote) = three_way_fixture(2_000);
    let plan = build_merge_plan(&base, &local, &remote, &MergeOptions::default());
    let all = plan.block_ancestor_ranges();
    assert_eq!(all.len(), plan.blocks.len());
    for (block, range) in plan.blocks.iter().zip(&all) {
        assert_eq!(range, &plan.block_ancestor_range(block));
    }
    let two_way =
        build_merge_plan_with_optional_base(None, &local, &remote, &MergeOptions::default());
    assert!(two_way.block_ancestor_ranges().iter().all(Option::is_none));
}

#[test]
fn base_anchor_subset_walk_matches_set_comparison() {
    let (base, local, remote) = three_way_fixture(1_000);
    let plan = build_merge_plan(&base, &local, &remote, &MergeOptions::default());
    let mut fewer = plan.rows.clone();
    // Drop every third anchor row from the "candidate": the baseline is
    // then not a subset of it, while the reverse still holds.
    let mut seen = 0usize;
    fewer.retain(|row| {
        if row.equal_ab && row.equal_ac && row.a.is_some() {
            seen += 1;
            !seen.is_multiple_of(3)
        } else {
            true
        }
    });
    for contributor in [MergeSource::B, MergeSource::C] {
        let set_subset = |left: &[AlignedRow], right: &[AlignedRow]| {
            let right: BTreeSet<(usize, usize)> =
                exact_base_anchor_pairs(right, contributor).collect();
            exact_base_anchor_pairs(left, contributor).all(|pair| right.contains(&pair))
        };
        assert_eq!(
            base_anchor_pairs_subset(&plan.rows, &plan.rows, contributor),
            set_subset(&plan.rows, &plan.rows)
        );
        assert_eq!(
            base_anchor_pairs_subset(&plan.rows, &fewer, contributor),
            set_subset(&plan.rows, &fewer)
        );
        assert_eq!(
            base_anchor_pairs_subset(&fewer, &plan.rows, contributor),
            set_subset(&fewer, &plan.rows)
        );
        assert!(base_anchor_pairs_subset(&fewer, &plan.rows, contributor));
        assert!(!base_anchor_pairs_subset(&plan.rows, &fewer, contributor));
    }
}

#[test]
fn count_line_feeds_matches_str_matches() {
    for text in ["", "a\n", "a\r\nb\n", "\r\n\r\n", "a\rb\n", "no newline"] {
        assert_eq!(
            crate::text_utils::count_line_feeds(text),
            (text.matches("\r\n").count(), text.matches('\n').count()),
            "{text:?}"
        );
    }
}

// Timing probe: `cargo test -p gitcomet-core --lib -- --ignored --nocapture timing_`
#[test]
#[ignore = "timing probe"]
fn timing_build_and_render_three_way_plan() {
    let (base, local, remote) = three_way_fixture(10_000);
    let options = MergeOptions::default();
    for round in 1..=3 {
        let started = std::time::Instant::now();
        let plan = build_merge_plan(&base, &local, &remote, &options);
        let built = started.elapsed();
        let started = std::time::Instant::now();
        let result = render_merge_plan(&plan, &options);
        let rendered = started.elapsed();
        println!(
            "timing merge_plan round {round}: build {built:?} render {rendered:?} ({} rows, {} blocks, {} conflicts)",
            plan.rows.len(),
            plan.blocks.len(),
            result.conflict_count
        );
    }
}
