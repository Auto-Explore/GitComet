use super::super::*;

pub(crate) const PERF_BUDGETS: &[PerfBudgetSpec] = &[
    PerfBudgetSpec {
        label: "history_annotations/plain",
        estimate_path: "history_annotations/plain/new/estimates.json",
        threshold_ns: 16.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "history_annotations/marked",
        estimate_path: "history_annotations/marked/new/estimates.json",
        threshold_ns: 16.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "diff_view_open_first_window/200",
        estimate_path: "diff_view_open_first_window/200/new/estimates.json",
        threshold_ns: 15.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "diff_scroll/overlay_lane_window/200",
        estimate_path: "diff_scroll/overlay_lane_window/200/new/estimates.json",
        threshold_ns: 8.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "diff_scroll/insets_window/200",
        estimate_path: "diff_scroll/insets_window/200/new/estimates.json",
        threshold_ns: 8.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "file_list/plan_build_tree_100k",
        estimate_path: "file_list/plan_build_tree_100k/new/estimates.json",
        threshold_ns: 500.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "file_list/render_window_60_of_100k",
        estimate_path: "file_list/render_window_60_of_100k/new/estimates.json",
        threshold_ns: 8.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "file_list/decor_update_no_replan",
        estimate_path: "file_list/decor_update_no_replan/new/estimates.json",
        threshold_ns: 8.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "file_list/regroup_100k",
        estimate_path: "file_list/regroup_100k/new/estimates.json",
        threshold_ns: 8.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "settings_window_render/active_page_only",
        estimate_path: "settings_window_render/active_page_only/new/estimates.json",
        threshold_ns: 16.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "shell/view_switch",
        estimate_path: "shell/view_switch/new/estimates.json",
        threshold_ns: 16.0 * NANOS_PER_MILLISECOND,
    },
];
