use super::*;
use crate::perf_alloc::measure_allocations;

// The allocator is process-global. Run this guard alone so other tests cannot
// hide a regression in noise or cause a false positive.
fn in_allocation_test_process(full_name: &str, guard: impl FnOnce()) {
    const CHILD: &str = "GITCOMET_DISPLAY_ALLOCATION_TEST_CHILD";
    let (_, test_name) = full_name.split_once("::").expect("qualified test name");
    let completed = format!("allocation guard completed: {test_name}");
    if let Some(child_name) = std::env::var_os(CHILD) {
        assert_eq!(child_name, std::ffi::OsStr::new(test_name));
        guard();
        eprintln!("{completed}");
        return;
    }

    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test_name, "--nocapture", "--test-threads=1"])
        .env(CHILD, test_name)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stderr.lines().filter(|line| *line == completed).count() == 1,
        "allocation guard {test_name} did not complete: {}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status,
    );
}

fn prepare_rows(rows: &[(SharedString, CachedDiffStyledText)]) {
    for (source, styled) in rows {
        std::hint::black_box(display_plain_text(source.clone()));
        std::hint::black_box(display_diff_text(None, source.clone()));
        std::hint::black_box(display_diff_text(Some(styled), source.clone()));
        // One middle slice straddles the interior CR in the third fixture;
        // the last slice includes the hidden terminator where present.
        std::hint::black_box(display_styled_slice(styled, 20_352..20_608, false));
        std::hint::black_box(display_styled_slice(
            styled,
            styled.text.len() - 256..styled.text.len(),
            true,
        ));
    }
}

#[test]
fn warmed_display_rows_and_wrap_slices_allocate_nothing() {
    in_allocation_test_process(
        concat!(
            module_path!(),
            "::warmed_display_rows_and_wrap_slices_allocate_nothing"
        ),
        || {
            let half = "x".repeat(20 * 1024);
            let rows = [
                format!("{half}{half}"),
                format!("{half}{half}\r"),
                format!("{half}\r{half}\r"),
            ]
            .into_iter()
            .map(|text| {
                let source: SharedString = text.into();
                let styled =
                    build_cached_diff_styled_text_from_relative_highlights(4, &source, &[]);
                (source, styled)
            })
            .collect::<Vec<_>>();

            prepare_rows(&rows);
            // Initialize the allocation measurement's thread-local channels.
            let _ = measure_allocations(|| {});
            let (_, allocations) = measure_allocations(|| {
                for _ in 0..100 {
                    prepare_rows(&rows);
                }
            });
            assert_eq!(allocations.alloc_ops, 0, "{allocations:?}");
            assert_eq!(allocations.realloc_ops, 0, "{allocations:?}");
            assert_eq!(allocations.alloc_bytes, 0, "{allocations:?}");
        },
    );
}
