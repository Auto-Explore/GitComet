//! Lines holding novel positions.

use line_numbers::LineNumber;

use crate::hash::DftHashSet;
use crate::parse::syntax::MatchedPos;

pub(crate) fn lines_with_novel(
    lhs_mps: &[MatchedPos],
    rhs_mps: &[MatchedPos],
) -> (DftHashSet<LineNumber>, DftHashSet<LineNumber>) {
    let lhs_lines_with_novel: DftHashSet<LineNumber> = lhs_mps
        .iter()
        .filter(|mp| mp.kind.is_novel())
        .map(|mp| mp.pos.line)
        .collect();
    let rhs_lines_with_novel: DftHashSet<LineNumber> = rhs_mps
        .iter()
        .filter(|mp| mp.kind.is_novel())
        .map(|mp| mp.pos.line)
        .collect();

    (lhs_lines_with_novel, rhs_lines_with_novel)
}
