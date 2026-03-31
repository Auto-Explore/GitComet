use super::*;

mod diff_repo;
mod editing_ops;
mod history_status;
mod layout_navigation;
mod render_preview;
mod system_runtime;
mod text_model;

pub(crate) const STRUCTURAL_BUDGET_GROUPS: &[&[StructuralBudgetSpec]] = &[
    diff_repo::STRUCTURAL_BUDGETS,
    history_status::STRUCTURAL_BUDGETS,
    layout_navigation::STRUCTURAL_BUDGETS,
    editing_ops::STRUCTURAL_BUDGETS,
    system_runtime::STRUCTURAL_BUDGETS,
    render_preview::STRUCTURAL_BUDGETS,
    text_model::STRUCTURAL_BUDGETS,
];
