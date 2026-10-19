use std::path::Path;
use std::time::{Duration, Instant};

use super::*;

fn input<'a>(path: &'a str, old: &'a str, new: &'a str) -> Input<'a> {
    Input {
        path: Path::new(path),
        old,
        new,
    }
}

fn nested_index(depth: usize, leaf: &str) -> String {
    format!("x = {}{leaf}{};\n", "a[".repeat(depth), "]".repeat(depth))
}

#[test]
fn a_javascript_change_marks_the_changed_tokens() {
    let outcome = compare(
        input("app.js", "call(a, b);\n", "call(a, c);\n"),
        &CancellationToken::new(),
    );
    let Ok(StructuralDiffOutcome::Compared(diff)) = outcome else {
        panic!("expected a structural comparison, got {outcome:?}");
    };
    assert_eq!(diff.language, "JavaScript");
    assert_eq!(diff.aligned_lines, vec![(Some(0), Some(0))]);
    assert_eq!(diff.old_changes[&0], vec![8..9]);
    assert_eq!(diff.new_changes[&0], vec![8..9]);
}

#[test]
fn an_expired_deadline_is_a_timeout_fallback() {
    let old = nested_index(3, "1");
    let new = nested_index(3, "2");
    let outcome = compare_until(
        input("app.js", &old, &new),
        &CancellationToken::new(),
        Instant::now(),
        0,
        |input, should_stop| api::compare(input, NESTING_LIMIT, should_stop),
    );
    assert_eq!(
        outcome,
        Ok(StructuralDiffOutcome::Fallback(DiffEngineFallback::Timeout))
    );
}

#[test]
fn a_contained_panic_is_a_failed_fallback() {
    let outcome = compare_until(
        input("app.js", "a\n", "b\n"),
        &CancellationToken::new(),
        Instant::now() + DEADLINE,
        0,
        |_, _| panic!("difftastic test panic"),
    );
    assert_eq!(
        outcome,
        Ok(StructuralDiffOutcome::Fallback(DiffEngineFallback::Failed))
    );
}

#[test]
fn cancelling_mid_comparison_returns_promptly() {
    // A list edited throughout searches ~3M vertices to the graph limit,
    // polling the stop check every 1,024; cancel a third of the way in.
    let list = |edited: bool| {
        let items: Vec<_> = (0..2000)
            .map(|i| if edited && i % 3 == 0 { i + 1 } else { i }.to_string())
            .collect();
        format!("[{}]\n", items.join(", "))
    };
    let (old, new) = (list(false), list(true));
    let cancellation = CancellationToken::new();
    let polls = std::sync::atomic::AtomicUsize::new(0);
    // A comparison that ends before the cancel fails the test, not hangs it.
    let returned = std::sync::atomic::AtomicBool::new(false);
    let (outcome, cancelled_at) = std::thread::scope(|scope| {
        let canceller = scope.spawn(|| {
            while polls.load(Ordering::Relaxed) < 1000 && !returned.load(Ordering::Relaxed) {
                std::thread::yield_now();
            }
            cancellation.cancel();
            Instant::now()
        });
        let outcome = compare_until(
            input("data.json", &old, &new),
            &cancellation,
            Instant::now() + DEADLINE,
            0,
            |input, should_stop| {
                let counted = || {
                    polls.fetch_add(1, Ordering::Relaxed);
                    should_stop()
                };
                api::compare(input, NESTING_LIMIT, &counted)
            },
        );
        returned.store(true, Ordering::Relaxed);
        (outcome, canceller.join().unwrap())
    });
    assert_eq!(outcome, Err(Cancelled));
    assert!(
        cancelled_at.elapsed() < Duration::from_secs(1),
        "returned {:?} after the cancel",
        cancelled_at.elapsed()
    );
}

#[test]
fn nesting_at_the_limit_compares_on_the_worker_stack() {
    let outcome = compare(
        input("app.js", &nested_index(997, "1"), &nested_index(997, "2")),
        &CancellationToken::new(),
    );
    assert!(
        matches!(outcome, Ok(StructuralDiffOutcome::Compared(_))),
        "{outcome:?}"
    );
    let outcome = compare(
        input("app.js", &nested_index(998, "1"), &nested_index(998, "2")),
        &CancellationToken::new(),
    );
    assert_eq!(
        outcome,
        Ok(StructuralDiffOutcome::Fallback(
            DiffEngineFallback::NestingLimit
        ))
    );
}

#[test]
fn the_deepest_source_under_the_byte_limit_falls_back_without_recursing() {
    let depth = (BYTE_LIMIT - 2) / 2;
    let side = |leaf: &str| format!("{}{leaf}{}", "[".repeat(depth - 1), "]".repeat(depth - 1));
    let (old, new) = (side("1"), side("2"));
    assert!(old.len() <= BYTE_LIMIT);
    assert_eq!(
        compare(input("data.json", &old, &new), &CancellationToken::new()),
        Ok(StructuralDiffOutcome::Fallback(
            DiffEngineFallback::NestingLimit
        ))
    );
}

#[test]
fn an_embedded_tree_at_the_limit_compares_on_the_worker_stack() {
    let page = |depth: usize, leaf: &str| {
        let script = nested_index(depth, leaf);
        format!("<script>{}</script>\n", script.trim_end())
    };
    let changed_ranges = |depth: usize| match compare(
        input("page.html", &page(depth, "1"), &page(depth, "2")),
        &CancellationToken::new(),
    ) {
        Ok(StructuralDiffOutcome::Compared(diff)) => diff.new_changes[&0].clone(),
        other => panic!("expected a structural comparison, got {other:?}"),
    };
    let leaf = |depth: usize| "<script>x = ".len() + 2 * depth;
    // The deepest script difftastic keeps as JavaScript: only the digit.
    assert_eq!(changed_ranges(994), vec![leaf(994)..leaf(994) + 1]);
    // One level deeper the script is diffed as HTML text, marked whole.
    assert_eq!(
        changed_ranges(996),
        vec![8..page(996, "2").len() - "</script>\n".len()]
    );
}

/// A file difftastic has no grammar for still queued for the one comparison
/// slot, so it waited out another pane's comparison (up to the deadline)
/// before falling back.
#[test]
fn an_unsupported_language_falls_back_without_waiting_for_the_slot() {
    let cancellation = CancellationToken::new();
    let held = Worker::acquire(&cancellation).unwrap();
    let (sent, outcome) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let _ = sent.send(compare(input("notes.txt", "a\n", "b\n"), &cancellation));
        });
        let outcome = outcome.recv_timeout(Duration::from_secs(2));
        drop(held);
        assert_eq!(
            outcome,
            Ok(Ok(StructuralDiffOutcome::Fallback(
                DiffEngineFallback::UnsupportedLanguage
            )))
        );
    });
}

/// The support check before queueing reads the content as difftastic does,
/// so a script named without an extension still compares by its shebang.
#[test]
fn a_script_without_an_extension_compares_by_its_shebang() {
    let outcome = compare(
        input(
            "build",
            "#!/usr/bin/env python\nx = 1\n",
            "#!/usr/bin/env python\nx = 2\n",
        ),
        &CancellationToken::new(),
    );
    let Ok(StructuralDiffOutcome::Compared(diff)) = outcome else {
        panic!("expected a structural comparison, got {outcome:?}");
    };
    assert_eq!(diff.language, "Python");
}
