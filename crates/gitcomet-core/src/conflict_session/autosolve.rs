use super::marker_parse::{ParsedConflictSegment, parse_conflict_marker_segments};
use super::subchunk::{Subchunk, split_conflict_into_subchunks};
use super::{
    AutosolvePickSide, AutosolveRule, ConflictRegion, ConflictRegionResolution,
    RegexAutosolveOptions,
};
use regex::{Regex, RegexBuilder};

const USER_REGEX_SIZE_LIMIT: usize = 1_048_576; // 1 MiB
const USER_REGEX_DFA_SIZE_LIMIT: usize = 1_048_576; // 1 MiB

#[derive(Clone)]
pub(super) struct CompiledRegexAutosolvePattern {
    regex: Regex,
    replacement: String,
}

// ── Standalone autosolve for merged text ────────────────────────────

/// Try to resolve a single conflict block using safe heuristics.
///
/// Applies:
/// 1. Identical sides (ours == theirs).
/// 2. Single-side change when base is available.
/// 3. Whitespace-only difference.
/// 4. Subchunk splitting (line-level re-merge) when base is available.
///
/// Returns `Some(resolved_text)` if the block can be auto-resolved.
fn try_resolve_single_block(base: Option<&str>, ours: &str, theirs: &str) -> Option<String> {
    // Rule 1: identical sides.
    if ours == theirs {
        return Some(ours.to_string());
    }

    if let Some(base) = base {
        // Rule 2: only theirs changed.
        if ours == base && theirs != base {
            return Some(theirs.to_string());
        }
        // Rule 3: only ours changed.
        if theirs == base && ours != base {
            return Some(ours.to_string());
        }
    }

    // Rule 4: whitespace-only difference.
    if is_whitespace_only_diff(ours, theirs) {
        return Some(ours.to_string());
    }

    // Rule 5: subchunk splitting (requires base).
    if let Some(base) = base {
        let region = ConflictRegion {
            base: Some(base.into()),
            ours: ours.into(),
            theirs: theirs.into(),
            resolution: ConflictRegionResolution::Unresolved,
        };
        if let Some(subchunks) = split_conflict_into_subchunks(base, &region.ours, &region.theirs)
            .filter(|sc| sc.iter().all(|c| matches!(c, Subchunk::Resolved(_))))
        {
            let merged: String = subchunks
                .iter()
                .map(|c| match c {
                    Subchunk::Resolved(text) => text.as_str(),
                    _ => unreachable!(),
                })
                .collect();
            return Some(merged);
        }
    }

    None
}

/// Attempt to auto-resolve all conflict blocks in merged text.
///
/// Parses the merged text (with conflict markers) into alternating context and
/// conflict spans, then applies safe heuristic passes on each conflict block:
///
/// 1. **Identical sides** — ours == theirs.
/// 2. **Single-side change** — one side matches base (requires diff3/zdiff3 markers).
/// 3. **Whitespace-only** — sides differ only in whitespace.
/// 4. **Subchunk splitting** — line-level re-merge within the block (requires base).
///
/// Returns `Some(clean_text)` if ALL conflicts were resolved, `None` otherwise.
///
/// This is designed for use in headless mergetool `--auto` mode, matching
/// KDiff3's auto-resolve behavior: attempt to resolve all conflicts automatically,
/// write clean output and exit 0 if successful, otherwise leave markers and exit 1.
pub fn try_autosolve_merged_text(text: &str) -> Option<String> {
    let segments = parse_conflict_marker_segments(text);

    let has_conflicts = segments
        .iter()
        .any(|s| matches!(s, ParsedConflictSegment::Conflict(_)));
    if !has_conflicts {
        // No conflicts to resolve — return the text as-is.
        return Some(text.to_string());
    }

    let mut output = String::with_capacity(text.len());

    for segment in segments {
        match segment {
            ParsedConflictSegment::Text(text) => output.push_str(&text),
            ParsedConflictSegment::Conflict(block) => {
                if let Some(resolved) =
                    try_resolve_single_block(block.base.as_deref(), &block.ours, &block.theirs)
                {
                    output.push_str(&resolved);
                } else {
                    return None;
                }
            }
        }
    }

    Some(output)
}

/// Normalize a string by collapsing all whitespace runs into a single space.
fn normalize_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Returns `true` if `a` and `b` differ only in whitespace.
pub fn is_whitespace_only_diff(a: &str, b: &str) -> bool {
    a != b && normalize_whitespace(a) == normalize_whitespace(b)
}

/// Pass 1 safe auto-resolve decision helper.
///
/// Returns which side to pick when one of the always-safe rules applies.
pub fn safe_auto_resolve_pick(
    base: Option<&str>,
    ours: &str,
    theirs: &str,
    whitespace_normalize: bool,
) -> Option<(AutosolveRule, AutosolvePickSide)> {
    // Rule 1: both sides identical.
    if ours == theirs {
        return Some((AutosolveRule::IdenticalSides, AutosolvePickSide::Ours));
    }

    // Rules 2 & 3 require a base.
    if let Some(base_raw) = base {
        // Rule 2: only theirs changed (ours == base).
        if ours == base_raw && theirs != base_raw {
            return Some((AutosolveRule::OnlyTheirsChanged, AutosolvePickSide::Theirs));
        }

        // Rule 3: only ours changed (theirs == base).
        if theirs == base_raw && ours != base_raw {
            return Some((AutosolveRule::OnlyOursChanged, AutosolvePickSide::Ours));
        }
    }

    // Rule 4 (optional): whitespace-only difference between sides.
    // This rule does not require a base.
    if whitespace_normalize && is_whitespace_only_diff(ours, theirs) {
        return Some((AutosolveRule::WhitespaceOnly, AutosolvePickSide::Ours));
    }

    None
}

/// Attempt to auto-resolve a single conflict region using Pass 1 safe rules.
///
/// When `whitespace_normalize` is true, an additional rule checks whether
/// the ours/theirs difference is whitespace-only, in which case "ours" is
/// picked (the design specifies this as an optional Pass 1 toggle).
///
/// Returns `Some((rule, resolved_content))` if a safe resolution is found.
pub(super) fn safe_auto_resolve(
    region: &ConflictRegion,
    whitespace_normalize: bool,
) -> Option<(AutosolveRule, String)> {
    let (rule, pick) = safe_auto_resolve_pick(
        region.base.as_deref(),
        &region.ours,
        &region.theirs,
        whitespace_normalize,
    )?;
    let content = match pick {
        AutosolvePickSide::Ours => region.ours.to_string(),
        AutosolvePickSide::Theirs => region.theirs.to_string(),
    };
    Some((rule, content))
}

pub(super) fn compile_regex_patterns(
    options: &RegexAutosolveOptions,
) -> Option<Vec<CompiledRegexAutosolvePattern>> {
    if options.is_empty() {
        return None;
    }
    let mut compiled = Vec::with_capacity(options.patterns.len());
    for pattern in &options.patterns {
        let regex = RegexBuilder::new(&pattern.pattern)
            .size_limit(USER_REGEX_SIZE_LIMIT)
            .dfa_size_limit(USER_REGEX_DFA_SIZE_LIMIT)
            .build()
            .ok()?;
        compiled.push(CompiledRegexAutosolvePattern {
            regex,
            replacement: pattern.replacement.clone(),
        });
    }
    Some(compiled)
}

fn normalize_with_patterns(text: &str, patterns: &[CompiledRegexAutosolvePattern]) -> String {
    let mut out = text.to_string();
    for rule in patterns {
        out = rule
            .regex
            .replace_all(&out, rule.replacement.as_str())
            .into_owned();
    }
    out
}

/// Pass 3 regex-assisted decision helper.
///
/// Returns which side to pick when regex-normalized comparison indicates a
/// conservative auto-resolution opportunity.
pub fn regex_assisted_auto_resolve_pick(
    base: Option<&str>,
    ours: &str,
    theirs: &str,
    options: &RegexAutosolveOptions,
) -> Option<(AutosolveRule, AutosolvePickSide)> {
    let compiled = compile_regex_patterns(options)?;
    regex_assisted_auto_resolve_pick_with_compiled(base, ours, theirs, &compiled)
}

pub(super) fn regex_assisted_auto_resolve_pick_with_compiled(
    base: Option<&str>,
    ours: &str,
    theirs: &str,
    compiled: &[CompiledRegexAutosolvePattern],
) -> Option<(AutosolveRule, AutosolvePickSide)> {
    // Skip cases already covered by Pass 1 safe rules.
    if ours == theirs {
        return None;
    }
    if let Some(base_raw) = base
        && ((ours == base_raw && theirs != base_raw) || (theirs == base_raw && ours != base_raw))
    {
        return None;
    }

    let norm_ours = normalize_with_patterns(ours, compiled);
    let norm_theirs = normalize_with_patterns(theirs, compiled);

    if norm_ours == norm_theirs {
        return Some((AutosolveRule::RegexEquivalentSides, AutosolvePickSide::Ours));
    }

    let base = base?;
    let norm_base = normalize_with_patterns(base, compiled);

    if norm_ours == norm_base && norm_theirs != norm_base {
        return Some((
            AutosolveRule::RegexOnlyTheirsChanged,
            AutosolvePickSide::Theirs,
        ));
    }
    if norm_theirs == norm_base && norm_ours != norm_base {
        return Some((AutosolveRule::RegexOnlyOursChanged, AutosolvePickSide::Ours));
    }

    None
}
