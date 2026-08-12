use regex::{Regex, RegexBuilder};

const USER_REGEX_SIZE_LIMIT: usize = 1_048_576; // 1 MiB
const USER_REGEX_DFA_SIZE_LIMIT: usize = 1_048_576; // 1 MiB

// ---------------------------------------------------------------------------
// History-aware auto-resolve (kdiff3-inspired)
// ---------------------------------------------------------------------------

/// Options for history-aware auto-resolve mode.
///
/// This mode detects structured history/changelog sections within conflict
/// blocks and merges their entries by deduplication and optional sorting.
/// Inspired by kdiff3's "history merge" feature for `$Log$` sections.
///
/// Disabled by default; opt-in via settings.
#[derive(Clone, Debug, Default)]
pub struct HistoryAutosolveOptions {
    /// Regex pattern that marks the start of a history section within a file.
    /// For example, `r".*\$Log.*\$.*"` for RCS/CVS-style history, or
    /// `r"^## Changelog"` for markdown changelogs.
    pub section_start: String,
    /// Regex pattern that marks the beginning of each individual history entry.
    /// For example, `r"^## \[.*\]"` for keepachangelog-style entries, or
    /// `r"^\s*\*\s+"` for bullet-list entries.
    pub entry_start: String,
    /// If true, sort entries using the sort key extracted from `entry_start`
    /// capture groups. If false, preserve order from both sides (ours first,
    /// then theirs additions).
    pub sort_entries: bool,
    /// Maximum number of entries to keep. `None` means keep all.
    pub max_entries: Option<usize>,
}

impl HistoryAutosolveOptions {
    /// Preset for keepachangelog-style markdown changelogs.
    /// Section starts with `## Changelog` or `## [Unreleased]`, entries start
    /// with version headers like `## [1.2.3]`.
    pub fn keepachangelog() -> Self {
        Self {
            section_start: r"^##\s+\[".to_string(),
            entry_start: r"^##\s+\[".to_string(),
            sort_entries: false,
            max_entries: None,
        }
    }

    /// Preset for bullet-list changelogs (`* Added foo`, `- Fixed bar`).
    pub fn bullet_list() -> Self {
        Self {
            section_start: r"(?i)^#+\s*(changelog|changes|history|release\s*notes)".to_string(),
            entry_start: r"^[-*]\s+".to_string(),
            sort_entries: false,
            max_entries: None,
        }
    }

    /// Returns true if this configuration has the minimum required patterns.
    pub fn is_valid(&self) -> bool {
        !self.section_start.is_empty() && !self.entry_start.is_empty()
    }
}

/// A parsed history entry within a history section.
#[derive(Clone, Debug)]
struct HistoryEntry {
    /// The full text of this entry (including the entry-start line and any
    /// continuation lines until the next entry or end of section).
    text: String,
    /// Normalized key for deduplication (trimmed, whitespace-collapsed).
    dedup_key: String,
}

/// Attempt to auto-resolve a conflict region by merging history/changelog entries.
///
/// Returns `Some(merged_text)` if the conflict looks like a history section
/// conflict and can be merged via entry deduplication. Returns `None` if:
/// - Options are invalid or patterns don't compile
/// - Neither side's content matches the section start pattern
/// - The conflict doesn't look like a history section
pub fn history_merge_region(
    base: Option<&str>,
    ours: &str,
    theirs: &str,
    options: &HistoryAutosolveOptions,
) -> Option<String> {
    if !options.is_valid() {
        return None;
    }

    let section_re = RegexBuilder::new(&options.section_start)
        .size_limit(USER_REGEX_SIZE_LIMIT)
        .dfa_size_limit(USER_REGEX_DFA_SIZE_LIMIT)
        .build()
        .ok()?;
    let entry_re = RegexBuilder::new(&options.entry_start)
        .size_limit(USER_REGEX_SIZE_LIMIT)
        .dfa_size_limit(USER_REGEX_DFA_SIZE_LIMIT)
        .build()
        .ok()?;

    // At least one side must contain a history section marker.
    let ours_has_section = ours.lines().any(|l| section_re.is_match(l));
    let theirs_has_section = theirs.lines().any(|l| section_re.is_match(l));
    if !ours_has_section && !theirs_has_section {
        return None;
    }

    let ours_entries = parse_history_entries(ours, &section_re, &entry_re);
    let theirs_entries = parse_history_entries(theirs, &section_re, &entry_re);

    // Need at least some entries on at least one side.
    if ours_entries.is_empty() && theirs_entries.is_empty() {
        return None;
    }

    // Build merged entry list by deduplication.
    let base_entries = base.map(|b| parse_history_entries(b, &section_re, &entry_re));

    let merged = merge_history_entries(
        base_entries.as_deref(),
        &ours_entries,
        &theirs_entries,
        options.sort_entries,
        options.max_entries,
    );

    // Reconstruct: use the "ours" prefix (text before the first entry), merged
    // entries, then the "ours" suffix (text after the last entry).
    let prefix = history_section_prefix(ours, &section_re, &entry_re);
    let suffix = history_section_suffix(ours, &entry_re);

    let mut result = String::new();
    result.push_str(&prefix);
    for entry in &merged {
        result.push_str(&entry.text);
    }
    result.push_str(&suffix);

    Some(result)
}

/// Retry a narrowed merge block using the nearest section header from its
/// already-resolved prefix. This keeps KDiff3-sized append conflicts eligible
/// for history merging without pulling earlier history entries into the
/// block's replacement text.
pub(super) fn history_merge_region_with_context(
    context_before: &str,
    base: Option<&str>,
    ours: &str,
    theirs: &str,
    options: &HistoryAutosolveOptions,
) -> Option<String> {
    let section_re = RegexBuilder::new(&options.section_start)
        .size_limit(USER_REGEX_SIZE_LIMIT)
        .dfa_size_limit(USER_REGEX_DFA_SIZE_LIMIT)
        .build()
        .ok()?;
    let section = context_before
        .lines()
        .rev()
        .find(|line| section_re.is_match(line))?;
    let header = format!("{section}\n");
    let contextual_base = base.map(|text| format!("{header}{text}"));
    let contextual_ours = format!("{header}{ours}");
    let contextual_theirs = format!("{header}{theirs}");
    history_merge_region(
        contextual_base.as_deref(),
        &contextual_ours,
        &contextual_theirs,
        options,
    )?
    .strip_prefix(&header)
    .map(ToOwned::to_owned)
}

/// Parse text into history entries. Returns entries found after the section
/// start marker (or from the beginning if the entire text is a history block).
///
/// Trailing non-entry content (detected by a blank-line break followed by
/// non-entry lines) is excluded from the last entry so it can be preserved
/// separately by `history_section_suffix`.
fn parse_history_entries(text: &str, section_re: &Regex, entry_re: &Regex) -> Vec<HistoryEntry> {
    let mut entries = Vec::new();
    let lines: Vec<&str> = text.lines().collect();

    // Find where the history section starts.
    let section_start = lines
        .iter()
        .position(|l| section_re.is_match(l))
        .unwrap_or(0);

    // Determine if the section start line is itself an entry start.
    let scan_start = if entry_re.is_match(lines.get(section_start).unwrap_or(&"")) {
        section_start
    } else {
        // Skip the section header line, look for first entry after it.
        section_start + 1
    };

    // Find the last entry-start line and determine where trailing content
    // begins so we don't include it in the last entry's text.
    let last_entry_start = lines[scan_start..]
        .iter()
        .rposition(|l| entry_re.is_match(l))
        .map(|rel| rel + scan_start);
    let scan_end = last_entry_start
        .and_then(|last| find_trailing_content_start(&lines, last, entry_re))
        .unwrap_or(lines.len());

    let mut current_entry_text = String::new();

    for &line in &lines[scan_start..scan_end] {
        if entry_re.is_match(line) && !current_entry_text.is_empty() {
            // Finish previous entry.
            entries.push(make_history_entry(std::mem::take(&mut current_entry_text)));
        }
        current_entry_text.push_str(line);
        current_entry_text.push('\n');
    }

    // Don't forget the last entry.
    if !current_entry_text.is_empty() {
        entries.push(make_history_entry(current_entry_text));
    }

    entries
}

fn make_history_entry(text: String) -> HistoryEntry {
    // Normalize for dedup: trim, collapse whitespace.
    let dedup_key = text.split_whitespace().collect::<Vec<_>>().join(" ");
    HistoryEntry { text, dedup_key }
}

/// Merge history entries from ours and theirs, deduplicating against base.
///
/// Strategy:
/// 1. Start with all entries from "ours" (preserving order).
/// 2. Add entries from "theirs" that aren't already present (by dedup key).
/// 3. If base is available, entries deleted by one side and present in the
///    other are kept (conservative — don't lose entries).
/// 4. Optionally sort and/or truncate.
fn merge_history_entries(
    base_entries: Option<&[HistoryEntry]>,
    ours_entries: &[HistoryEntry],
    theirs_entries: &[HistoryEntry],
    sort: bool,
    max_entries: Option<usize>,
) -> Vec<HistoryEntry> {
    use rustc_hash::FxHashSet as HashSet;

    let mut seen_keys: HashSet<String> = HashSet::default();
    let mut merged: Vec<HistoryEntry> = Vec::new();

    // Add all "ours" entries.
    for entry in ours_entries {
        if seen_keys.insert(entry.dedup_key.clone()) {
            merged.push(entry.clone());
        }
    }

    // Determine where to insert "theirs" new entries.
    // Find entries in base that are also in ours — theirs-only entries
    // should be inserted at the position they would naturally appear.
    let base_keys: HashSet<String> = base_entries
        .map(|entries| entries.iter().map(|e| e.dedup_key.clone()).collect())
        .unwrap_or_default();

    // Add entries from "theirs" that we haven't seen yet.
    // Insert new theirs entries at the beginning (they're typically newer).
    let mut theirs_new: Vec<HistoryEntry> = Vec::new();
    for entry in theirs_entries {
        if seen_keys.insert(entry.dedup_key.clone()) {
            // This entry is unique to theirs.
            if !base_keys.contains(&entry.dedup_key) {
                // Truly new entry (not in base either) — insert near top.
                theirs_new.push(entry.clone());
            } else {
                // Was in base, deleted by ours — keep it conservatively.
                merged.push(entry.clone());
            }
        }
    }

    // Insert theirs-new entries after any existing ours-new entries
    // (entries not in base) to interleave chronologically.
    if !theirs_new.is_empty() {
        // Find the first entry that was also in base (i.e., not new from ours).
        let insert_pos = merged
            .iter()
            .position(|e| base_keys.contains(&e.dedup_key))
            .unwrap_or(merged.len());
        for (i, entry) in theirs_new.into_iter().enumerate() {
            merged.insert(insert_pos + i, entry);
        }
    }

    if sort {
        merged.sort_by(|a, b| a.dedup_key.cmp(&b.dedup_key));
    }

    if let Some(max) = max_entries {
        merged.truncate(max);
    }

    merged
}

/// Extract the text before the first history entry (section header, etc.).
fn history_section_prefix(text: &str, section_re: &Regex, entry_re: &Regex) -> String {
    let mut prefix = String::new();
    for line in text.lines() {
        if entry_re.is_match(line) {
            // If the section start is also the entry start (e.g., keepachangelog),
            // the prefix is everything before this line.
            break;
        }
        prefix.push_str(line);
        prefix.push('\n');
        if section_re.is_match(line) {
            // Include the section header line, then stop after it.
            // The next entry_re match will be the first entry.
            // But we need to also include any lines between header and first entry.
            continue;
        }
    }
    prefix
}

/// Extract text after the last history entry (trailing content).
///
/// Uses a blank-line heuristic: after the last `entry_re` match, the first
/// blank line followed by a non-blank, non-entry-start line marks the
/// boundary between entry content and trailing content. Trailing blank
/// lines at end-of-text are also captured so file formatting is preserved.
pub(super) fn history_section_suffix(text: &str, entry_re: &Regex) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let last_entry_start = lines.iter().rposition(|l| entry_re.is_match(l));
    let Some(last_start) = last_entry_start else {
        return String::new();
    };

    if let Some(suffix_start) = find_trailing_content_start(&lines, last_start, entry_re) {
        let mut suffix = String::new();
        for &line in &lines[suffix_start..] {
            suffix.push_str(line);
            suffix.push('\n');
        }
        suffix
    } else {
        String::new()
    }
}

/// Find the line index where trailing non-entry content begins after the
/// last history entry. Returns `None` if entries extend to end of text
/// without a blank-line-separated non-entry section.
///
/// Heuristic: scan forward from the last `entry_re` match. When we hit
/// one or more blank lines followed by a non-blank line that doesn't match
/// `entry_re`, everything from the first blank line onward is trailing
/// content. Trailing blank lines at end-of-text are also treated as
/// trailing content to preserve file formatting.
fn find_trailing_content_start(
    lines: &[&str],
    last_entry_start: usize,
    entry_re: &Regex,
) -> Option<usize> {
    let mut i = last_entry_start + 1;

    while i < lines.len() {
        if lines[i].trim().is_empty() {
            let blank_start = i;
            // Skip past consecutive blank lines.
            while i < lines.len() && lines[i].trim().is_empty() {
                i += 1;
            }
            if i >= lines.len() {
                // Only blank lines remain — treat as trailing content.
                return Some(blank_start);
            }
            if !entry_re.is_match(lines[i]) {
                // Non-blank, non-entry line after blank gap → trailing content.
                return Some(blank_start);
            }
            // Blank line between entries — continue scanning.
        }
        i += 1;
    }

    None
}
