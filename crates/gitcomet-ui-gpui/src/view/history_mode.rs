use gitcomet_core::domain::HistoryMode;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HistoryModeUiSpec {
    pub mode: HistoryMode,
    pub label: &'static str,
    pub shortcut: &'static str,
    pub settings_description: &'static str,
    pub settings_row_id: &'static str,
}

const HISTORY_MODE_UI_SPECS: [HistoryModeUiSpec; 5] = [
    HistoryModeUiSpec {
        mode: HistoryMode::FullReachable,
        label: "Full reachable",
        shortcut: "F",
        settings_description: "Show every commit reachable from the current branch tip.",
        settings_row_id: "settings_window_git_log_default_mode_full_reachable",
    },
    HistoryModeUiSpec {
        mode: HistoryMode::FirstParent,
        label: "First-parent",
        shortcut: "P",
        settings_description: "Follow only the mainline path through merges.",
        settings_row_id: "settings_window_git_log_default_mode_first_parent",
    },
    HistoryModeUiSpec {
        mode: HistoryMode::NoMerges,
        label: "No merges",
        shortcut: "N",
        settings_description: "Hide merge commits and show only ordinary commits.",
        settings_row_id: "settings_window_git_log_default_mode_no_merges",
    },
    HistoryModeUiSpec {
        mode: HistoryMode::MergesOnly,
        label: "Merges only",
        shortcut: "M",
        settings_description: "Show only merge commits for high-level integration history.",
        settings_row_id: "settings_window_git_log_default_mode_merges_only",
    },
    HistoryModeUiSpec {
        mode: HistoryMode::AllBranches,
        label: "All branches",
        shortcut: "A",
        settings_description: "Show commits reachable from all refs, not just the current branch.",
        settings_row_id: "settings_window_git_log_default_mode_all_branches",
    },
];

pub(crate) const HISTORY_MODE_TOOLTIP_TEXT: &str =
    "History mode (Full reachable / First-parent / No merges / Merges only / All branches)";

pub(crate) fn history_mode_ui_specs() -> &'static [HistoryModeUiSpec] {
    &HISTORY_MODE_UI_SPECS
}

pub(crate) fn history_mode_label(mode: HistoryMode) -> &'static str {
    history_mode_ui_specs()
        .iter()
        .find(|spec| spec.mode == mode)
        .map(|spec| spec.label)
        .unwrap_or("Unknown")
}
