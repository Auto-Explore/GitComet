use super::ref_comparison_process::{Analysis, AnalysisResult};
use super::{GixRepo, ref_comparison_patches as patches};
use gitcomet_core::domain::{CommitId, FileMode};
use gitcomet_core::ref_comparison::{
    IntegrationEvidence, RefComparisonBudget, RefComparisonReason,
};
use rustc_hash::FxHashMap;
use std::io::Read;

const TREE_ENTRY_BATCH: usize = 128;

pub(super) fn analyze(
    repo: &GixRepo,
    analysis: &Analysis<'_>,
    target: &CommitId,
    subject: &CommitId,
    budget: &RefComparisonBudget,
) -> AnalysisResult<IntegrationEvidence> {
    let bases =
        analysis.capture_merge_base(&["merge-base", "--all", target.as_ref(), subject.as_ref()])?;
    let bases: Vec<_> = bases
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect();
    if bases.is_empty() {
        return Err(RefComparisonReason::NoMergeBase);
    }
    let base = if bases.len() == 1 {
        Some(std::str::from_utf8(bases[0]).map_err(|_| RefComparisonReason::IncompleteHistory)?)
    } else {
        None
    };
    if let Some(base) = base
        && let Some(evidence) = current_content(analysis, base, target.as_ref(), subject.as_ref())?
    {
        return Ok(evidence);
    }
    let aggregate = match base {
        Some(base) => patches::aggregate_patch(analysis, base, subject.as_ref())?,
        None => None,
    };
    let limit = budget.history_commits.min(10_000);
    let unique = patches::unique_commits(analysis, target, subject, limit);
    let mut required = FxHashMap::default();
    let mut unmatched = None;
    match unique {
        Ok(commits) => {
            let ids = patches::patch_ids(analysis, &commits, false)?;
            if ids.len() != commits.len() || commits.is_empty() {
                unmatched = Some(RefComparisonReason::NoMatch);
            }
            for id in ids.values() {
                *required.entry(*id).or_default() += 1;
            }
        }
        Err(reason) => unmatched = Some(reason),
    }
    let store = repo.shared_store();
    let mut retained = loop {
        analysis.check()?;
        match store.comparison_patches.try_lock() {
            Ok(index) => break index,
            Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(std::time::Duration::from_millis(1))
            }
        }
    };
    if retained
        .as_ref()
        .is_none_or(|index| index.target() != target)
    {
        *retained = Some(patches::TargetPatchIndex::new(
            target,
            &store.common.comparison_memory,
        )?);
    }
    let index = retained
        .as_mut()
        .ok_or(RefComparisonReason::IncompleteHistory)?;
    loop {
        analysis.check()?;
        if aggregate
            .as_ref()
            .is_some_and(|patch| index.contains(patch))
        {
            return Ok(IntegrationEvidence::SquashPatch);
        }
        if unmatched.is_none() && !required.is_empty() && index.includes(&required) {
            return Ok(IntegrationEvidence::IndividualCommits);
        }
        if index.complete(limit) {
            if !index.exhausted() {
                return Err(RefComparisonReason::Budget);
            }
            return Err(unmatched.unwrap_or(if base.is_some() {
                RefComparisonReason::NoMatch
            } else {
                RefComparisonReason::MultipleMergeBases
            }));
        }
        index.advance(
            analysis,
            limit,
            budget.elapsed.min(RefComparisonBudget::deep().elapsed),
        )?;
    }
}

fn current_content(
    analysis: &Analysis<'_>,
    base: &str,
    target: &str,
    subject: &str,
) -> AnalysisResult<Option<IntegrationEvidence>> {
    let desired = changed_paths(&raw_diff(analysis, base, subject)?, analysis.memory / 8)?;
    if desired.is_empty() {
        return Ok(Some(IntegrationEvidence::NoNetChanges));
    }
    let paths: Vec<_> = desired.keys().collect();
    for paths in paths.chunks(TREE_ENTRY_BATCH) {
        let mut command = analysis.command(&["ls-tree", "--full-tree", "-z", target]);
        command.env("GIT_LITERAL_PATHSPECS", "1").arg("--");
        for path in paths {
            command.arg(
                crate::util::path_buf_from_git_bytes(path, "comparison tree path")
                    .map_err(|_| RefComparisonReason::IncompleteHistory)?,
            );
        }
        let limit = analysis.memory / 8;
        let entries = analysis.run(command, &[], move |reader| {
            let mut bytes = Vec::new();
            reader
                .take(limit as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| RefComparisonReason::IncompleteHistory)?;
            if bytes.len() > limit {
                return Err(RefComparisonReason::Budget);
            }
            parse_tree_entries(&bytes)
        })?;
        for path in paths {
            let matches = match desired.get(*path) {
                Some(TreeEntry::Deleted) => !entries.contains_key(*path),
                Some(entry) => entries.get(*path) == Some(entry),
                None => return Err(RefComparisonReason::IncompleteHistory),
            };
            if !matches {
                return Ok(None);
            }
        }
    }
    Ok(Some(IntegrationEvidence::ChangesMatch))
}

fn raw_diff(analysis: &Analysis<'_>, base: &str, tip: &str) -> AnalysisResult<Vec<u8>> {
    analysis.capture(
        &[
            "diff-tree",
            "--raw",
            "--no-commit-id",
            "--no-abbrev",
            "--no-renames",
            "--ignore-submodules=none",
            "-r",
            "-z",
            base,
            tip,
            "--",
        ],
        &[],
    )
}

#[derive(Eq, PartialEq)]
enum TreeEntry {
    Deleted,
    Directory(gix::ObjectId),
    Present { mode: [u8; 6], oid: gix::ObjectId },
}

fn parse_tree_entries(bytes: &[u8]) -> AnalysisResult<FxHashMap<Vec<u8>, TreeEntry>> {
    let mut entries = FxHashMap::default();
    for record in bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let tab = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or(RefComparisonReason::IncompleteHistory)?;
        let mut header = record[..tab].split(|byte| *byte == b' ');
        let mode = header
            .next()
            .ok_or(RefComparisonReason::IncompleteHistory)?;
        let kind = header
            .next()
            .ok_or(RefComparisonReason::IncompleteHistory)?;
        let oid = gix::ObjectId::from_hex(
            header
                .next()
                .ok_or(RefComparisonReason::IncompleteHistory)?,
        )
        .map_err(|_| RefComparisonReason::IncompleteHistory)?;
        if header.next().is_some() || oid.is_null() {
            return Err(RefComparisonReason::IncompleteHistory);
        }
        let entry = if mode == b"040000" && kind == b"tree" {
            TreeEntry::Directory(oid)
        } else {
            let parsed =
                FileMode::from_octal(mode).ok_or(RefComparisonReason::IncompleteHistory)?;
            if (parsed == FileMode::Gitlink && kind != b"commit")
                || (parsed != FileMode::Gitlink && kind != b"blob")
            {
                return Err(RefComparisonReason::IncompleteHistory);
            }
            TreeEntry::Present {
                mode: mode
                    .try_into()
                    .map_err(|_| RefComparisonReason::IncompleteHistory)?,
                oid,
            }
        };
        let path = &record[tab + 1..];
        if path.is_empty() || entries.insert(path.to_vec(), entry).is_some() {
            return Err(RefComparisonReason::IncompleteHistory);
        }
    }
    Ok(entries)
}

fn changed_paths(bytes: &[u8], memory: usize) -> AnalysisResult<FxHashMap<Vec<u8>, TreeEntry>> {
    let mut records = bytes.split(|byte| *byte == 0);
    let mut paths = FxHashMap::default();
    let mut path_bytes = 0usize;
    while let Some(header) = records.next() {
        if header.is_empty() {
            continue;
        }
        let mut columns = header.split(|byte| *byte == b' ');
        let _old_mode = columns
            .next()
            .ok_or(RefComparisonReason::IncompleteHistory)?;
        let mode = columns
            .next()
            .ok_or(RefComparisonReason::IncompleteHistory)?;
        let _old_oid = columns
            .next()
            .ok_or(RefComparisonReason::IncompleteHistory)?;
        let oid = columns
            .next()
            .ok_or(RefComparisonReason::IncompleteHistory)?;
        let oid =
            gix::ObjectId::from_hex(oid).map_err(|_| RefComparisonReason::IncompleteHistory)?;
        let status = columns
            .next()
            .ok_or(RefComparisonReason::IncompleteHistory)?;
        if !matches!(status, b"A" | b"D" | b"M" | b"T") || columns.next().is_some() {
            return Err(RefComparisonReason::IncompleteHistory);
        }
        let path = records
            .next()
            .filter(|path| !path.is_empty())
            .ok_or(RefComparisonReason::IncompleteHistory)?;
        path_bytes = path_bytes.saturating_add(path.len());
        if (paths.len() + 1) * 256 + path_bytes > memory {
            return Err(RefComparisonReason::Budget);
        }
        let entry = if mode == b"000000" && oid.is_null() {
            TreeEntry::Deleted
        } else {
            if FileMode::from_octal(mode).is_none() {
                return Err(RefComparisonReason::IncompleteHistory);
            }
            let mode = mode
                .try_into()
                .map_err(|_| RefComparisonReason::IncompleteHistory)?;
            if oid.is_null() {
                return Err(RefComparisonReason::IncompleteHistory);
            }
            TreeEntry::Present { mode, oid }
        };
        paths.insert(path.to_vec(), entry);
    }
    Ok(paths)
}
