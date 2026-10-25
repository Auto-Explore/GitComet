use super::GixRepo;
use super::ref_comparison_process::{Analysis, AnalysisResult};
use gitcomet_core::domain::CommitId;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::ref_comparison::*;
use gitcomet_core::services::{CancellationToken, Result};
use rustc_hash::{FxHashMap, FxHashSet};
use std::io::{BufRead, Read};
use std::time::Instant;

impl GixRepo {
    pub(super) fn ref_comparison_target_impl(
        &self,
        reference: Option<&str>,
        cancellation: &CancellationToken,
    ) -> Result<RefComparisonTarget> {
        cancellation.check_cancelled()?;
        let analysis = Analysis {
            workdir: &self.spec.workdir,
            deadline: Instant::now() + RefComparisonBudget::automatic().elapsed,
            cancellation,
            memory: RefComparisonBudget::automatic().memory_bytes,
            empty_graph: None,
        };
        let reference = match reference {
            Some(reference) if !reference.is_empty() && !reference.starts_with('-') => {
                reference.to_string()
            }
            Some(_) => {
                return Err(Error::new(ErrorKind::Backend(
                    "Choose a valid comparison ref.".into(),
                )));
            }
            None => {
                let bytes = analysis.capture(&["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"], &[])
                    .map_err(|_| Error::new(ErrorKind::Backend("The locally recorded origin/HEAD is unavailable. Choose an explicit comparison ref, or use Fetch to update local remote refs.".into())))?;
                String::from_utf8(bytes)
                    .map_err(|_| {
                        Error::new(ErrorKind::Backend(
                            "The default comparison ref is not valid UTF-8.".into(),
                        ))
                    })?
                    .trim()
                    .to_string()
            }
        };
        let spec = format!("{reference}^{{commit}}");
        let bytes = analysis
            .capture(&["rev-parse", "--verify", "--end-of-options", &spec], &[])
            .map_err(|_| {
                Error::new(ErrorKind::Backend(format!(
                    "Comparison ref {reference} does not resolve to a locally available commit."
                )))
            })?;
        let oid = parse_oid(bytes.trim_ascii()).map_err(|_| {
            Error::new(ErrorKind::Backend(
                "The comparison target did not resolve to an object ID.".into(),
            ))
        })?;
        Ok(RefComparisonTarget {
            reference: reference
                .strip_prefix("refs/remotes/")
                .or_else(|| reference.strip_prefix("refs/heads/"))
                .unwrap_or(&reference)
                .to_string(),
            oid: CommitId(oid.to_string().into()),
            context: self.shared_store().id,
        })
    }

    pub(super) fn analyze_ref_comparisons_impl(
        &self,
        request: &RefComparisonRequest,
        cancellation: &CancellationToken,
    ) -> Result<RefComparisonBatch> {
        cancellation.check_cancelled()?;
        let mut seen = FxHashSet::default();
        let subjects: Vec<_> = request
            .subjects
            .iter()
            .filter(|oid| seen.insert((*oid).clone()))
            .cloned()
            .collect();
        if subjects.len() > MAX_COMPARISON_SUBJECTS {
            return Err(Error::new(ErrorKind::Backend(
                "A comparison batch is limited to 128 distinct subjects.".into(),
            )));
        }
        let store = self.shared_store();
        let memory_bytes = request
            .budget
            .memory_bytes
            .min(COMPARISON_REPOSITORY_BYTES / 4);
        let Some(_active_memory) = store.common.comparison_memory.reserve(memory_bytes) else {
            return Ok(RefComparisonBatch {
                target: request.target.clone(),
                results: subjects
                    .into_iter()
                    .map(|subject| RefComparison {
                        subject,
                        counts: RefComparisonCounts::Unavailable(RefComparisonReason::Budget),
                        integration: RefIntegration::NotConfirmed(RefComparisonReason::Budget),
                    })
                    .collect(),
                graph_reads: 0,
                cache_hits: 0,
            });
        };
        let empty_graph = tempfile::NamedTempFile::new().map_err(Error::from)?;
        let analysis = Analysis {
            workdir: &self.spec.workdir,
            deadline: Instant::now()
                + request
                    .budget
                    .elapsed
                    .min(RefComparisonBudget::deep().elapsed),
            cancellation,
            memory: memory_bytes,
            empty_graph: Some(empty_graph.path()),
        };
        let graph = graph_available(&analysis);
        let context = self.shared_store().id;
        let mut cached = Vec::new();
        let mut uncached = Vec::new();
        {
            let cache = store
                .comparisons
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            for subject in &subjects {
                match cache.get(&request.target.oid, subject, request.depth) {
                    Some(result)
                        if request.budget.elapsed <= RefComparisonBudget::automatic().elapsed
                            || !matches!(
                                result.counts,
                                RefComparisonCounts::Unavailable(
                                    RefComparisonReason::Budget | RefComparisonReason::Deadline
                                )
                            ) =>
                    {
                        cached.push(result)
                    }
                    _ => uncached.push(subject.clone()),
                }
            }
        }
        if context != request.target.context || graph.is_err() {
            cached.clear();
            uncached = subjects.clone();
        }
        if uncached.is_empty() && context == request.target.context && graph.is_ok() {
            return Ok(RefComparisonBatch {
                target: request.target.clone(),
                cache_hits: cached.len(),
                results: cached,
                graph_reads: 0,
            });
        }
        let counts = if context != request.target.context {
            Err(RefComparisonReason::IncompleteHistory)
        } else {
            graph.and_then(|()| shared_counts(&analysis, &request.target.oid, &uncached))
        };
        let counts = counts.and_then(|counts| graph_available(&analysis).map(|()| counts));
        cancellation.check_cancelled()?;
        let (mut results, graph_reads): (Vec<_>, _) = match counts {
            Ok((counts, reads)) => (
                uncached
                    .into_iter()
                    .zip(counts)
                    .map(|(subject, (ahead, behind))| {
                        let integration = if subject == request.target.oid {
                            RefIntegration::Confirmed(IntegrationEvidence::UpToDate)
                        } else if ahead == 0 {
                            RefIntegration::Confirmed(IntegrationEvidence::Merged)
                        } else {
                            if request.depth == RefComparisonDepth::Counts {
                                RefIntegration::Pending
                            } else {
                                RefIntegration::NotConfirmed(RefComparisonReason::Unsupported)
                            }
                        };
                        RefComparison {
                            subject,
                            counts: RefComparisonCounts::Exact { ahead, behind },
                            integration,
                        }
                    })
                    .collect(),
                reads,
            ),
            Err(reason) => (
                uncached
                    .into_iter()
                    .map(|subject| RefComparison {
                        subject,
                        counts: RefComparisonCounts::Unavailable(reason),
                        integration: RefIntegration::NotConfirmed(reason),
                    })
                    .collect(),
                0,
            ),
        };
        {
            let mut cache = store
                .comparisons
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            for result in &results {
                cache.insert(
                    &request.target.oid,
                    result,
                    request.depth,
                    &store.common.comparison_memory,
                );
            }
        }
        let cache_hits = cached.len();
        results.append(&mut cached);
        results.sort_by_key(|result| subjects.iter().position(|id| id == &result.subject));
        Ok(RefComparisonBatch {
            target: request.target.clone(),
            results,
            graph_reads,
            cache_hits,
        })
    }
}

fn graph_available(analysis: &Analysis<'_>) -> AnalysisResult<()> {
    let shallow = analysis.capture_live_graph(&["rev-parse", "--is-shallow-repository"])?;
    if shallow.trim_ascii() != b"false" {
        return Err(RefComparisonReason::IncompleteHistory);
    }
    let replacements = analysis.capture_live_graph(&[
        "for-each-ref",
        "--count=1",
        "--format=%(objectname)",
        "refs/replace/",
    ])?;
    if !replacements.is_empty() {
        return Err(RefComparisonReason::Unsupported);
    }
    let graft_path = analysis.capture_live_graph(&["rev-parse", "--git-path", "info/grafts"])?;
    let path = crate::util::path_buf_from_git_bytes(graft_path.trim_ascii(), "grafts path")
        .map_err(|_| RefComparisonReason::IncompleteHistory)?;
    let path = if path.is_absolute() {
        path
    } else {
        analysis.workdir.join(path)
    };
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.len() > 0 => Err(RefComparisonReason::Unsupported),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(RefComparisonReason::IncompleteHistory),
    }
}

fn parse_oid(bytes: &[u8]) -> AnalysisResult<gix::ObjectId> {
    gix::ObjectId::from_hex(bytes).map_err(|_| RefComparisonReason::IncompleteHistory)
}

fn shared_counts(
    analysis: &Analysis<'_>,
    target: &CommitId,
    subjects: &[CommitId],
) -> AnalysisResult<(Vec<(u64, u64)>, u64)> {
    if subjects.is_empty() {
        return Ok((Vec::new(), 0));
    }
    let seeds: Vec<_> = std::iter::once(target)
        .chain(subjects)
        .map(|id| parse_oid(id.as_ref().as_bytes()))
        .collect::<AnalysisResult<_>>()?;
    let mut command = analysis.command(&["merge-base", "--all", "--octopus"]);
    command.args(seeds.iter().map(ToString::to_string));
    let common = analysis.run(command, &[], |mut reader| {
        let mut bytes = Vec::new();
        std::io::Read::by_ref(&mut reader)
            .take(64 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| RefComparisonReason::IncompleteHistory)?;
        if bytes.len() > 64 * 1024 {
            return Err(RefComparisonReason::Budget);
        }
        bytes
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .map(parse_oid)
            .collect::<AnalysisResult<FxHashSet<_>>>()
    })?;
    let mut input = String::new();
    for seed in &seeds {
        input.push_str(&format!("{seed}\n"));
    }
    for base in &common {
        input.push_str(&format!("^{base}\n"));
    }
    let memory = analysis.memory;
    let cancellation = analysis.cancellation;
    let deadline = analysis.deadline;
    analysis.run(
        analysis.command(&["rev-list", "--topo-order", "--parents", "--stdin"]),
        input.as_bytes(),
        move |mut reader| {
            let mut frontier: FxHashMap<gix::ObjectId, [u64; 3]> = FxHashMap::default();
            for (index, seed) in seeds.iter().enumerate() {
                frontier.entry(*seed).or_default()[index / 64] |= 1 << (index % 64);
            }
            let mut ahead = vec![0u64; seeds.len() - 1];
            let mut shared = vec![0u64; ahead.len()];
            let mut target_total = 0;
            let mut reads = 0;
            let mut line = Vec::new();
            loop {
                if cancellation.is_cancelled() {
                    return Err(RefComparisonReason::IncompleteHistory);
                }
                if Instant::now() >= deadline {
                    return Err(RefComparisonReason::Deadline);
                }
                line.clear();
                let read = std::io::Read::by_ref(&mut reader)
                    .take(64 * 1024 + 1)
                    .read_until(b'\n', &mut line)
                    .map_err(|_| RefComparisonReason::IncompleteHistory)?;
                if read == 0 {
                    break;
                }
                if read > 64 * 1024 {
                    return Err(RefComparisonReason::Budget);
                }
                let mut ids = line.trim_ascii().split(|b| *b == b' ');
                let id = parse_oid(ids.next().ok_or(RefComparisonReason::IncompleteHistory)?)?;
                let membership = frontier
                    .remove(&id)
                    .ok_or(RefComparisonReason::IncompleteHistory)?;
                let in_target = membership[0] & 1 != 0;
                target_total += u64::from(in_target);
                for (word_index, word) in membership.iter().enumerate() {
                    let mut word = *word;
                    if word_index == 0 {
                        word &= !1;
                    }
                    while word != 0 {
                        let index = word_index * 64 + word.trailing_zeros() as usize - 1;
                        if in_target {
                            shared[index] += 1;
                        } else {
                            ahead[index] += 1;
                        }
                        word &= word - 1;
                    }
                }
                for parent in ids {
                    let parent = parse_oid(parent)?;
                    if common.contains(&parent) {
                        continue;
                    }
                    let entry = frontier.entry(parent).or_default();
                    for index in 0..3 {
                        entry[index] |= membership[index];
                    }
                }
                reads += 1;
                if frontier.capacity().saturating_mul(112) > memory / 2 {
                    return Err(RefComparisonReason::Budget);
                }
            }
            Ok((
                ahead
                    .into_iter()
                    .zip(shared)
                    .map(|(ahead, shared)| (ahead, target_total - shared))
                    .collect(),
                reads,
            ))
        },
    )
}
