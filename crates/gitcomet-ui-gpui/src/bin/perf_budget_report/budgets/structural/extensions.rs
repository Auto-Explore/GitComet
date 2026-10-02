use super::*;
pub(crate) const STRUCTURAL_BUDGETS: &[StructuralBudgetSpec] = &[
    StructuralBudgetSpec {
        bench: "settings_window_render/active_page_only",
        metric: "pages_per_render",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "shell/empty_registry",
        metric: "extension_dispatch_calls",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "file_list/render_window_60_of_100k",
        metric: "rows_rendered",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 60.0,
    },
    StructuralBudgetSpec {
        bench: "file_list/decor_update_no_replan",
        metric: "replans",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "file_list/regroup_100k",
        metric: "replans",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/overlay_lane_window/200",
        metric: "rows_rendered",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/insets_window/200",
        metric: "rows_rendered",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
];
