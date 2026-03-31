use super::*;

mod foundation;
mod history_layout;
mod interactions;
mod text_rendering;

pub(crate) const PERF_BUDGET_GROUPS: &[&[PerfBudgetSpec]] = &[
    foundation::PERF_BUDGETS,
    history_layout::PERF_BUDGETS,
    interactions::PERF_BUDGETS,
    text_rendering::PERF_BUDGETS,
];
