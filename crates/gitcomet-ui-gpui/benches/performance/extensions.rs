use super::common::*;
use gitcomet_ui_gpui::benchmarks::{
    EmptyExtensionFrameFixture, ExtensionFrameFixture, HistoryAnnotationFrameFixture,
    SettingsFrameFixture,
};

pub(crate) fn bench_extensions(c: &mut Criterion) {
    // These fixtures use TestAppContext's executor. Schedule CPU work on it too,
    // so settling a frame waits for its diff and syntax caches to finish.
    gitcomet_ui_kit::ui_runtime::with_override(
        gitcomet_ui_kit::ui_runtime::UiRuntime::deterministic(),
        || bench_extension_frames(c),
    );
}

fn bench_extension_frames(c: &mut Criterion) {
    let mut fixture = ExtensionFrameFixture::new(1_000, 100_000);
    for (name, operation) in [
        (
            "diff_view_open_first_window/200",
            ExtensionFrameFixture::open_first_window as fn(&mut ExtensionFrameFixture) -> usize,
        ),
        (
            "file_list/plan_build_tree_100k",
            ExtensionFrameFixture::file_plan,
        ),
    ] {
        bench_frame(c, name, |b| {
            b.iter(|| std::hint::black_box(operation(&mut fixture)))
        });
    }
    for (name, decor) in [
        ("file_list/render_window_60_of_100k", false),
        ("file_list/decor_update_no_replan", true),
    ] {
        bench_frame(c, name, |b| {
            b.iter(|| std::hint::black_box(fixture.file_window(decor)))
        });
        let plans_before = fixture.file_plan_builds();
        let rows = measure_sidecar_allocations(|| fixture.file_window(decor));
        let replans = fixture.file_plan_builds() - plans_before;
        emit_sidecar_metrics(
            name,
            json!({"rows_rendered":rows,"total_files":100000,"replans":replans})
                .as_object()
                .unwrap()
                .clone(),
        );
    }
    bench_frame(c, "file_list/regroup_100k", |b| {
        b.iter(|| std::hint::black_box(fixture.file_regroup()))
    });
    let plans_before = fixture.file_plan_builds();
    let rows = measure_sidecar_allocations(|| fixture.file_regroup());
    let replans = fixture.file_plan_builds() - plans_before;
    emit_sidecar_metrics(
        "file_list/regroup_100k",
        json!({"grouped_rows":rows,"total_files":100000,"replans":replans})
            .as_object()
            .unwrap()
            .clone(),
    );
    for (name, insets) in [
        ("diff_scroll/overlay_lane_window/200", false),
        ("diff_scroll/insets_window/200", true),
    ] {
        fixture.set_overlays(insets);
        bench_frame(c, name, |b| {
            b.iter(|| std::hint::black_box(fixture.scroll_window()))
        });
        let rows = measure_sidecar_allocations(|| fixture.scroll_window());
        emit_sidecar_metrics(
            name,
            json!({"rows_rendered":rows}).as_object().unwrap().clone(),
        );
    }
    bench_frame(c, "shell/view_switch", |b| b.iter(|| fixture.switch_view()));
    let mut empty = EmptyExtensionFrameFixture::default();
    bench_frame(c, "shell/empty_registry", |b| {
        b.iter(|| std::hint::black_box(empty.frame()))
    });
    let calls = measure_sidecar_allocations(|| empty.frame());
    emit_sidecar_metrics(
        "shell/empty_registry",
        json!({"extension_dispatch_calls":calls})
            .as_object()
            .unwrap()
            .clone(),
    );
    let mut settings = SettingsFrameFixture::default();
    bench_frame(c, "settings_window_render/active_page_only", |b| {
        b.iter(|| settings.frame())
    });
    let pages = measure_sidecar_allocations(|| settings.frame());
    emit_sidecar_metrics(
        "settings_window_render/active_page_only",
        json!({"pages_per_render":pages})
            .as_object()
            .unwrap()
            .clone(),
    );
    for (name, annotated) in [
        ("history_annotations/plain", false),
        ("history_annotations/marked", true),
    ] {
        let mut history = HistoryAnnotationFrameFixture::new(annotated);
        bench_frame(c, name, |b| b.iter(|| history.frame()));
    }
}

// Keep Criterion's group/function/parameter directories aligned with the
// budget report. A slash in a single bench_function name is escaped to `_`.
fn bench_frame(
    c: &mut Criterion,
    name: &str,
    mut measure: impl FnMut(&mut criterion::Bencher<'_>),
) {
    let (group, case) = name.split_once('/').expect("a grouped benchmark");
    let id = match case.split_once('/') {
        Some((function, parameter)) => BenchmarkId::new(function, parameter),
        None => BenchmarkId::from_parameter(case),
    };
    c.benchmark_group(group).bench_function(id, |b| measure(b));
}
