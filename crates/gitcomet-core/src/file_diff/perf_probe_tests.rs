use super::side_by_side_rows;

/// Mirrors the `conflict_two_way_diff_build` benchmark fixture: 50k lines
/// with 1024 (high) or 12 (low) conflict blocks spread through context.
fn two_way_texts(total_lines: usize, conflict_blocks: usize) -> (String, String) {
    let context_lines = total_lines - conflict_blocks;
    let slots = conflict_blocks + 1;
    let per_slot = context_lines / slots;
    let remainder = context_lines % slots;
    let mut ours = String::with_capacity(total_lines * 64);
    let mut theirs = String::with_capacity(total_lines * 64);
    for slot_ix in 0..slots {
        let slot_lines = per_slot + usize::from(slot_ix < remainder);
        for line_ix in 0..slot_lines {
            let seed = slot_ix * 1_000 + line_ix;
            let line = match seed % 5 {
                0 => format!("fn ctx_{slot_ix}_{line_ix}() -> usize {{ {seed} }}"),
                1 => format!("let ctx_{slot_ix}_{line_ix} = \"context line {seed}\";"),
                2 => format!("if guard_{seed} {{ println!(\"{seed}\"); }}"),
                3 => format!("match opt_{seed} {{ Some(v) => v, None => 0 }}"),
                _ => format!("// context {seed} repeated words for highlight coverage"),
            };
            ours.push_str(&line);
            ours.push('\n');
            theirs.push_str(&line);
            theirs.push('\n');
        }
        if slot_ix < conflict_blocks {
            match slot_ix % 6 {
                0 => {
                    ours.push_str(&format!(
                        "let shared_{slot_ix} = compute_local({slot_ix});\nlet shared_{slot_ix}_tail = {slot_ix} + 1;\n"
                    ));
                    theirs.push_str(&format!(
                        "let shared_{slot_ix} = compute_remote({slot_ix});\n"
                    ));
                }
                1 => {
                    ours.push_str(&format!(
                        "let shared_{slot_ix} = compute_local({slot_ix});\n"
                    ));
                    theirs.push_str(&format!(
                        "let shared_{slot_ix} = compute_remote({slot_ix});\nlet shared_{slot_ix}_tail = {slot_ix} + 2;\n"
                    ));
                }
                _ => {
                    ours.push_str(&format!(
                        "let shared_{slot_ix} = compute_local({slot_ix});\n"
                    ));
                    theirs.push_str(&format!(
                        "let shared_{slot_ix} = compute_remote({slot_ix});\n"
                    ));
                }
            }
        }
    }
    (ours, theirs)
}

#[test]
#[ignore = "timing probe"]
fn timing_side_by_side_rows_two_way() {
    for (label, blocks) in [("low_density", 12usize), ("high_density", 1_024usize)] {
        let (ours, theirs) = two_way_texts(50_000, blocks);
        let mut best = std::time::Duration::MAX;
        let mut rows = 0;
        for _ in 0..7 {
            let started = std::time::Instant::now();
            rows = side_by_side_rows(&ours, &theirs).len();
            best = best.min(started.elapsed());
        }
        println!("timing side_by_side_rows {label}: best of 7 {best:?} ({rows} rows)");
    }
}
