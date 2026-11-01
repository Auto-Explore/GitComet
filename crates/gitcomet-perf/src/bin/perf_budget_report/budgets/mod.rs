use super::*;
use std::sync::LazyLock;

mod structural;
mod timing;
mod types;

pub(crate) use types::*;

fn collect_specs<T: Copy>(groups: &[&[T]]) -> Vec<T> {
    let total = groups.iter().map(|group| group.len()).sum::<usize>();
    let mut out = Vec::with_capacity(total);
    for group in groups {
        out.extend_from_slice(group);
    }
    out
}

pub(crate) static PERF_BUDGETS: LazyLock<Vec<PerfBudgetSpec>> =
    LazyLock::new(|| collect_specs(timing::PERF_BUDGET_GROUPS));

pub(crate) static STRUCTURAL_BUDGETS: LazyLock<Vec<StructuralBudgetSpec>> =
    LazyLock::new(|| collect_specs(structural::STRUCTURAL_BUDGET_GROUPS));
