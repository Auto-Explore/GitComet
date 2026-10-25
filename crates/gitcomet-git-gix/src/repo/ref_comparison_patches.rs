use super::ref_comparison_process::{Analysis, AnalysisResult};
use gitcomet_core::domain::CommitId;
use gitcomet_core::ref_comparison::{COMPARISON_REPOSITORY_BYTES, RefComparisonReason};
use gitcomet_core::ref_comparison_memory::{ComparisonMemory, ComparisonReservation};
use rustc_hash::FxHashMap;
use std::sync::Arc;
use std::time::Duration;

const HISTORY_PATCH_BATCH: usize = 64;
const HISTORY_COMMIT_BYTES: usize = 384;

pub(super) struct TargetPatchIndex {
    target: CommitId,
    commits: Vec<(gix::ObjectId, bool)>,
    records: Vec<(gix::ObjectId, Option<(gix::ObjectId, gix::ObjectId)>)>,
    available: FxHashMap<gix::ObjectId, usize>,
    processed: usize,
    exhausted: bool,
    requested: usize,
    reservation: Arc<ComparisonReservation>,
    deferred: Option<(RefComparisonReason, usize, Duration)>,
}

impl TargetPatchIndex {
    pub fn new(target: &CommitId, memory: &Arc<ComparisonMemory>) -> AnalysisResult<Self> {
        Ok(Self {
            target: target.clone(),
            commits: Vec::new(),
            records: Vec::new(),
            available: FxHashMap::default(),
            processed: 0,
            exhausted: false,
            requested: 0,
            reservation: memory.reserve(0).ok_or(RefComparisonReason::Budget)?,
            deferred: None,
        })
    }

    pub fn target(&self) -> &CommitId {
        &self.target
    }
    pub fn contains(&self, patch: &gix::ObjectId) -> bool {
        self.available.get(patch).is_some_and(|count| *count > 0)
    }
    pub fn includes(&self, required: &FxHashMap<gix::ObjectId, usize>) -> bool {
        required.iter().all(|(patch, count)| {
            self.available
                .get(patch)
                .is_some_and(|found| found >= count)
        })
    }
    pub fn complete(&self, limit: usize) -> bool {
        self.exhausted && self.processed >= self.commits.len() || self.processed >= limit
    }
    pub fn exhausted(&self) -> bool {
        self.exhausted && self.processed >= self.commits.len()
    }

    pub fn advance(
        &mut self,
        analysis: &Analysis<'_>,
        limit: usize,
        elapsed: Duration,
    ) -> AnalysisResult<()> {
        if let Some((reason, previous_limit, previous_elapsed)) = self.deferred
            && limit <= previous_limit
            && elapsed <= previous_elapsed
        {
            return Err(reason);
        }
        let result = self.advance_inner(analysis, limit);
        if let Err(reason @ (RefComparisonReason::Budget | RefComparisonReason::Deadline)) = result
        {
            self.deferred = Some((reason, limit, elapsed));
        }
        if result.is_ok() {
            self.deferred = None;
        }
        result
    }

    fn advance_inner(&mut self, analysis: &Analysis<'_>, limit: usize) -> AnalysisResult<()> {
        analysis.check()?;
        if self.requested < limit {
            let bytes = (limit + 1).saturating_mul(HISTORY_COMMIT_BYTES);
            if bytes > COMPARISON_REPOSITORY_BYTES / 8 || !self.reservation.resize(bytes) {
                return Err(RefComparisonReason::Budget);
            }
            let max_count = format!("--max-count={}", limit + 1);
            let output = analysis.capture(
                &[
                    "rev-list",
                    "--topo-order",
                    "--parents",
                    &max_count,
                    self.target.as_ref(),
                    "--",
                ],
                &[],
            )?;
            self.commits = parse_history(&output)?;
            self.exhausted = self.commits.len() <= limit;
            self.requested = limit;
        }
        let end = (self.processed + HISTORY_PATCH_BATCH)
            .min(self.commits.len())
            .min(limit);
        let commits: Vec<_> = self.commits[self.processed..end]
            .iter()
            .filter(|(_, ordinary)| *ordinary)
            .map(|(id, _)| *id)
            .collect();
        let forward = patch_ids(analysis, &commits, false)?;
        let reverse = patch_ids(analysis, &commits, true)?;
        let records = commits
            .into_iter()
            .map(|commit| {
                let record = match (forward.get(&commit), reverse.get(&commit)) {
                    (Some(forward), Some(reverse)) => Some((*forward, *reverse)),
                    (None, None) => None,
                    _ => return Err(RefComparisonReason::IncompleteHistory),
                };
                Ok((commit, record))
            })
            .collect::<AnalysisResult<Vec<_>>>()?;
        self.records.extend(records);
        self.processed = end;
        self.available.clear();
        for (_, record) in self.records.iter().rev() {
            let Some((forward, reverse)) = record else {
                continue;
            };
            if let Some(inverse) = self.available.get_mut(reverse) {
                *inverse = inverse.saturating_sub(1);
            }
            *self.available.entry(*forward).or_default() += 1;
        }
        Ok(())
    }
}

fn parse_history(bytes: &[u8]) -> AnalysisResult<Vec<(gix::ObjectId, bool)>> {
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut columns = line.split(|byte| *byte == b' ');
            let id = gix::ObjectId::from_hex(
                columns
                    .next()
                    .ok_or(RefComparisonReason::IncompleteHistory)?,
            )
            .map_err(|_| RefComparisonReason::IncompleteHistory)?;
            let mut parents = 0;
            for parent in columns {
                gix::ObjectId::from_hex(parent)
                    .map_err(|_| RefComparisonReason::IncompleteHistory)?;
                parents += 1;
            }
            Ok((id, parents <= 1))
        })
        .collect()
}

fn diff_args<'a>() -> Vec<&'a str> {
    vec![
        "-c",
        "diff.algorithm=myers",
        "-c",
        "diff.indentHeuristic=false",
        "-c",
        "diff.submodule=short",
        "-c",
        "diff.ignoreSubmodules=none",
        "-c",
        "core.quotePath=true",
        "diff-tree",
        "-r",
        "--patch",
        "--binary",
        "--full-index",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--no-renames",
        "--no-indent-heuristic",
        "--diff-algorithm=myers",
        "--unified=3",
        "--src-prefix=a/",
        "--dst-prefix=b/",
    ]
}

pub(super) fn aggregate_patch(
    analysis: &Analysis<'_>,
    base: &str,
    subject: &str,
) -> AnalysisResult<Option<gix::ObjectId>> {
    let mut args = diff_args();
    args.extend(["--no-commit-id", base, subject, "--"]);
    let patch = analysis.capture(&args, &[])?;
    let ids = analysis.capture(&["patch-id", "--verbatim"], &patch)?;
    let ids = ids
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|column| !column.is_empty())
        .collect::<Vec<_>>();
    match ids.as_slice() {
        [] => Ok(None),
        [id, _] => gix::ObjectId::from_hex(id)
            .map(Some)
            .map_err(|_| RefComparisonReason::IncompleteHistory),
        _ => Err(RefComparisonReason::IncompleteHistory),
    }
}

pub(super) fn patch_ids(
    analysis: &Analysis<'_>,
    commits: &[gix::ObjectId],
    reverse: bool,
) -> AnalysisResult<FxHashMap<gix::ObjectId, gix::ObjectId>> {
    if commits.is_empty() {
        return Ok(FxHashMap::default());
    }
    let mut input = String::new();
    for commit in commits {
        input.push_str(&format!("{commit}\n"));
    }
    let mut args = diff_args();
    args.extend(["--root", "--stdin"]);
    if reverse {
        args.extend(["-R", "--src-prefix=b/", "--dst-prefix=a/"]);
    }
    let patches = analysis.capture(&args, input.as_bytes())?;
    let output = analysis.capture(&["patch-id", "--verbatim"], &patches)?;
    let mut result = FxHashMap::default();
    for row in output
        .split(|byte| *byte == b'\n')
        .filter(|row| !row.is_empty())
    {
        let columns: Vec<_> = row
            .split(|byte| byte.is_ascii_whitespace())
            .filter(|column| !column.is_empty())
            .collect();
        if columns.len() != 2 {
            return Err(RefComparisonReason::IncompleteHistory);
        }
        let patch = gix::ObjectId::from_hex(columns[0])
            .map_err(|_| RefComparisonReason::IncompleteHistory)?;
        let commit = gix::ObjectId::from_hex(columns[1])
            .map_err(|_| RefComparisonReason::IncompleteHistory)?;
        if !commits.contains(&commit) || result.insert(commit, patch).is_some() {
            return Err(RefComparisonReason::IncompleteHistory);
        }
    }
    Ok(result)
}

pub(super) fn unique_commits(
    analysis: &Analysis<'_>,
    target: &CommitId,
    subject: &CommitId,
    limit: usize,
) -> AnalysisResult<Vec<gix::ObjectId>> {
    let excluded = format!("^{target}");
    let count = format!("--max-count={}", limit + 1);
    let bytes = analysis.capture(
        &[
            "rev-list",
            "--parents",
            &count,
            subject.as_ref(),
            &excluded,
            "--",
        ],
        &[],
    )?;
    let commits = parse_history(&bytes)?;
    if commits.len() > limit {
        return Err(RefComparisonReason::Budget);
    }
    if commits.iter().any(|(_, ordinary)| !ordinary) {
        return Err(RefComparisonReason::UniqueMerge);
    }
    Ok(commits.into_iter().map(|(id, _)| id).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::services::CancellationToken;
    use std::time::Instant;

    #[test]
    fn a_shared_deferred_batch_is_not_retried_until_coverage_or_elapsed_budget_increases() {
        let tmp = tempfile::tempdir().unwrap();
        crate::test_support::init_test_repo(tmp.path());
        for index in 0..4 {
            crate::test_support::git_success(
                tmp.path(),
                &["commit", "--allow-empty", "-m", &format!("empty {index}")],
            );
        }
        let output = crate::test_support::git_output(tmp.path(), &["rev-parse", "HEAD"]);
        let target = CommitId(String::from_utf8(output.stdout).unwrap().trim().into());
        let cancellation = CancellationToken::new();
        let memory = Arc::new(ComparisonMemory::default());
        let mut index = TargetPatchIndex::new(&target, &memory).unwrap();
        let analysis = Analysis {
            workdir: tmp.path(),
            deadline: Instant::now() + Duration::from_secs(5),
            cancellation: &cancellation,
            memory: 64,
            empty_graph: None,
        };
        assert_eq!(
            index.advance(&analysis, 1_000, Duration::from_secs(5)),
            Err(RefComparisonReason::Budget)
        );
        let missing = tmp.path().join("missing");
        let blocked = Analysis {
            workdir: &missing,
            deadline: Instant::now() + Duration::from_secs(5),
            cancellation: &cancellation,
            memory: 8 * 1024 * 1024,
            empty_graph: None,
        };
        assert_eq!(
            index.advance(&blocked, 1_000, Duration::from_secs(5)),
            Err(RefComparisonReason::Budget)
        );
        let deeper = Analysis {
            workdir: tmp.path(),
            deadline: Instant::now() + Duration::from_secs(30),
            cancellation: &cancellation,
            memory: 8 * 1024 * 1024,
            empty_graph: None,
        };
        index
            .advance(&deeper, 10_000, Duration::from_secs(30))
            .unwrap();
        assert!(index.exhausted());
    }
}
