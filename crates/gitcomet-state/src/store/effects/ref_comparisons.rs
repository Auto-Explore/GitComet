use super::super::executor::TaskExecutor;
use super::super::repository_runtime::RepositoryRuntimes;
use super::super::worker_channel::StoreWorkerSender;
use super::util::send_or_log;
use crate::model::RepoLifetime;
use crate::msg::Msg;
use crate::ref_comparisons::{
    ComparisonMetrics, MAX_COMPARISON_PROJECTIONS, RefComparisonSubscription, RefComparisonsEffect,
    RefComparisonsMsg,
};
use gitcomet_core::domain::CommitId;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::ref_comparison::*;
use gitcomet_core::ref_comparison_memory::{ComparisonReservation, hash_table_bytes};
use gitcomet_core::services::{CancellationToken, GitRepository};
use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const MAX_SUBSCRIBERS: usize = 256;
const PUBLICATION_INTERVAL: Duration = Duration::from_millis(100);

struct Subscriber {
    request: RefComparisonSubscription,
    repo: Arc<dyn GitRepository>,
    tx: StoreWorkerSender,
    target: Option<RefComparisonTarget>,
    subjects: Arc<FxHashSet<CommitId>>,
    visible: FxHashSet<CommitId>,
    pending: FxHashSet<CommitId>,
    last_publication: Option<Instant>,
    deferred: bool,
    _reservation: Arc<ComparisonReservation>,
}

fn subject_priority(subscriber: &Subscriber, id: &CommitId) -> u8 {
    if subscriber.visible.contains(id) || subscriber.request.projections.active.as_ref() == Some(id)
    {
        return 0;
    }
    if subscriber.request.projections.worktree_heads.contains(id) {
        return 1;
    }
    2
}

impl Subscriber {
    fn new(
        request: RefComparisonSubscription,
        repo: Arc<dyn GitRepository>,
        tx: StoreWorkerSender,
        previous: Option<Self>,
        reservation: Arc<ComparisonReservation>,
    ) -> Self {
        let subjects = request.projections.members.clone();
        let visible = request.visible.iter().cloned().collect();
        Self {
            request,
            repo,
            tx,
            target: None,
            subjects,
            visible,
            pending: FxHashSet::default(),
            deferred: false,
            last_publication: previous.and_then(|subscriber| subscriber.last_publication),
            _reservation: reservation,
        }
    }
}

struct Cohort {
    target: RefComparisonTarget,
    results: FxHashMap<CommitId, RefComparison>,
    subscribers: FxHashMap<RepoLifetime, Subscriber>,
    reservation: Option<Arc<ComparisonReservation>>,
    bytes: usize,
    metrics: ComparisonMetrics,
}

#[derive(Default)]
struct RepositoryWork {
    targets: FxHashMap<RepoLifetime, Subscriber>,
    cohorts: Vec<Cohort>,
    running: bool,
    cancellation: CancellationToken,
}

impl RepositoryWork {
    fn empty(&self) -> bool {
        self.targets.is_empty()
            && self
                .cohorts
                .iter()
                .all(|cohort| cohort.subscribers.is_empty())
    }
    fn remove(&mut self, lifetime: RepoLifetime) -> Option<Subscriber> {
        let mut previous = self.targets.remove(&lifetime);
        for cohort in &mut self.cohorts {
            if let Some(subscriber) = cohort.subscribers.remove(&lifetime) {
                previous = Some(subscriber);
            }
        }
        previous
    }
}

#[derive(Default)]
struct Queue {
    repositories: FxHashMap<u64, RepositoryWork>,
    owners: FxHashMap<RepoLifetime, u64>,
    workers: usize,
}

struct Pool {
    queue: Mutex<Queue>,
    executor: TaskExecutor,
}
static POOL: OnceLock<Pool> = OnceLock::new();
static ANALYSIS_THREAD: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "{}-ref-analysis",
        gitcomet_core::identity::current().executable_name()
    )
});
fn pool() -> &'static Pool {
    POOL.get_or_init(|| Pool {
        queue: Mutex::default(),
        executor: TaskExecutor::named(ANALYSIS_THREAD.as_str(), 2),
    })
}

enum Step {
    Resolve {
        owner: RepoLifetime,
        generation: u64,
        reference: Option<String>,
    },
    Analyze {
        cohort: usize,
        target: RefComparisonTarget,
        subjects: Vec<CommitId>,
        depth: RefComparisonDepth,
        budget: RefComparisonBudget,
    },
}

struct Task {
    scope: u64,
    repo: Arc<dyn GitRepository>,
    cancellation: CancellationToken,
    step: Step,
}
enum Completion {
    Target(gitcomet_core::services::Result<RefComparisonTarget>),
    Batch(gitcomet_core::services::Result<RefComparisonBatch>),
    Panicked,
}

impl Cohort {
    fn reserve_subjects(&mut self, subscriber: &Subscriber) -> bool {
        let additional = subscriber
            .subjects
            .iter()
            .filter(|id| !self.results.contains_key(*id))
            .count();
        let total = self.results.len().saturating_add(additional);
        if total > MAX_COMPARISON_PROJECTIONS {
            return false;
        }
        if self.results.capacity() < total {
            let bytes = hash_table_bytes::<CommitId, RefComparison>(total);
            let Some(reservation) = self
                .reservation
                .clone()
                .or_else(|| subscriber.request.memory.reserve(0))
            else {
                return false;
            };
            if !reservation.resize(self.bytes.saturating_add(bytes)) {
                return false;
            }
            self.results.reserve(total - self.results.len());
            reservation.resize(bytes);
            self.reservation = Some(reservation);
            self.bytes = bytes;
        }
        for id in subscriber.subjects.iter() {
            self.results
                .entry(id.clone())
                .or_insert_with(|| RefComparison {
                    subject: id.clone(),
                    counts: RefComparisonCounts::Pending,
                    integration: RefIntegration::Pending,
                });
        }
        true
    }

    fn next(&self, index: usize) -> Option<(u8, Step)> {
        for (stage, initial_depth, limit) in [
            (1, RefComparisonDepth::Counts, MAX_COMPARISON_SUBJECTS),
            (4, RefComparisonDepth::Evidence, 1),
        ] {
            let mut best = None;
            let mut subjects = Vec::with_capacity(limit);
            let mut seen = FxHashSet::with_capacity_and_hasher(limit, Default::default());
            for subscriber in self.subscribers.values() {
                if subscriber.deferred {
                    continue;
                }
                for id in &subscriber.request.projections.subjects {
                    let result = self.results.get(id);
                    let needs_counts =
                        result.is_none_or(|result| result.counts == RefComparisonCounts::Pending);
                    let needs_evidence = result.is_some_and(|result| {
                        result.integration == RefIntegration::Pending
                            && matches!(result.counts, RefComparisonCounts::Exact { .. })
                    });
                    if (initial_depth == RefComparisonDepth::Counts && !needs_counts)
                        || (initial_depth == RefComparisonDepth::Evidence && !needs_evidence)
                    {
                        continue;
                    }
                    let priority = subject_priority(subscriber, id);
                    let deep = self.subscribers.values().any(|owner| {
                        !owner.deferred && owner.request.deep && owner.subjects.contains(id)
                    });
                    let key = (priority, deep);
                    if best.is_none_or(|best| key < best) {
                        best = Some(key);
                        subjects.clear();
                        seen.clear();
                    }
                    if best == Some(key) && subjects.len() < limit && seen.insert(id.clone()) {
                        subjects.push(id.clone());
                    }
                }
            }
            if let Some((priority, deep)) = best {
                let depth = if deep && initial_depth == RefComparisonDepth::Evidence {
                    RefComparisonDepth::Deep
                } else {
                    initial_depth
                };
                return Some((
                    stage + priority,
                    Step::Analyze {
                        cohort: index,
                        target: self.target.clone(),
                        subjects,
                        depth,
                        budget: if deep {
                            RefComparisonBudget::deep()
                        } else {
                            RefComparisonBudget::automatic()
                        },
                    },
                ));
            }
        }
        None
    }
}

fn next(work: &RepositoryWork) -> Option<(u8, Step, Arc<dyn GitRepository>)> {
    if let Some((owner, subscriber)) = work.targets.iter().next() {
        return Some((
            0,
            Step::Resolve {
                owner: *owner,
                generation: subscriber.request.generation,
                reference: subscriber.request.reference.clone(),
            },
            subscriber.repo.clone(),
        ));
    }
    work.cohorts
        .iter()
        .enumerate()
        .filter_map(|(index, cohort)| {
            let (priority, step) = cohort.next(index)?;
            Some((
                priority,
                step,
                cohort.subscribers.values().next()?.repo.clone(),
            ))
        })
        .min_by_key(|(priority, ..)| *priority)
}

fn start_workers(pool: &'static Pool, queue: &mut Queue) {
    let available = queue
        .repositories
        .values()
        .filter(|work| {
            !work.running
                && (next(work).is_some()
                    || work.cohorts.iter().any(|cohort| {
                        cohort
                            .subscribers
                            .values()
                            .any(|subscriber| !subscriber.pending.is_empty())
                    }))
        })
        .count();
    for _ in 0..available.min(2usize.saturating_sub(queue.workers)) {
        queue.workers += 1;
        pool.executor.spawn(move || run(pool));
    }
}

fn take_task(queue: &mut Queue) -> Option<Task> {
    let (scope, (_, step, repo)) = queue
        .repositories
        .iter()
        .filter(|(_, work)| !work.running)
        .filter_map(|(scope, work)| next(work).map(|step| (*scope, step)))
        .min_by_key(|(_, (priority, ..))| *priority)?;
    let work = queue.repositories.get_mut(&scope)?;
    work.running = true;
    work.cancellation = CancellationToken::new();
    Some(Task {
        scope,
        repo,
        cancellation: work.cancellation.clone(),
        step,
    })
}

fn target_reply(
    subscriber: &Subscriber,
    result: gitcomet_core::services::Result<RefComparisonTarget>,
) {
    send_or_log(
        &subscriber.tx,
        Msg::RefComparisons(RefComparisonsMsg::Target {
            reservation: Some(subscriber._reservation.clone()),
            repo_id: subscriber.request.repo_id,
            lifetime: subscriber.request.lifetime,
            generation: subscriber.request.generation,
            result,
        }),
    );
}

fn complete(queue: &mut Queue, task: &Task, completion: Completion) {
    let completion = match (&task.step, completion) {
        (
            Step::Analyze {
                target,
                subjects,
                depth,
                ..
            },
            Completion::Batch(Ok(batch)),
        ) => {
            let valid = batch.target.oid == target.oid
                && batch.target.context == target.context
                && batch.results.len() == subjects.len()
                && batch.results.iter().all(|result| {
                    result.counts != RefComparisonCounts::Pending
                        && ((*depth == RefComparisonDepth::Counts
                            && matches!(result.counts, RefComparisonCounts::Exact { .. }))
                            || result.integration != RefIntegration::Pending)
                })
                && subjects.iter().all(|subject| {
                    batch
                        .results
                        .iter()
                        .filter(|result| &result.subject == subject)
                        .count()
                        == 1
                });
            if valid {
                Completion::Batch(Ok(batch))
            } else {
                Completion::Batch(Err(Error::new(ErrorKind::Backend(
                    "Comparison backend returned an incomplete or mismatched batch.".into(),
                ))))
            }
        }
        (_, completion) => completion,
    };
    let Some(work) = queue.repositories.get_mut(&task.scope) else {
        return;
    };
    work.running = false;
    if let Step::Resolve {
        owner, generation, ..
    } = &task.step
        && work
            .targets
            .get(owner)
            .is_none_or(|subscriber| subscriber.request.generation != *generation)
    {
        return;
    }
    match (&task.step, completion) {
        (Step::Resolve { owner, .. }, Completion::Target(result)) => {
            let Some(mut subscriber) = work.targets.remove(owner) else {
                return;
            };
            let target = match result {
                Ok(target) => target,
                Err(error) => {
                    target_reply(&subscriber, Err(error));
                    queue.owners.remove(owner);
                    return;
                }
            };
            if !subscriber
                ._reservation
                .grow(target.reference.len().saturating_mul(2))
            {
                target_reply(&subscriber, Err(Error::new(ErrorKind::Backend("Comparison target reached its shared memory limit. Run a deeper check to retry.".into()))));
                queue.owners.remove(owner);
                return;
            }
            let index = work
                .cohorts
                .iter()
                .position(|cohort| {
                    cohort.target.oid == target.oid && cohort.target.context == target.context
                })
                .unwrap_or_else(|| {
                    work.cohorts.push(Cohort {
                        target: target.clone(),
                        results: FxHashMap::default(),
                        subscribers: FxHashMap::default(),
                        reservation: None,
                        bytes: 0,
                        metrics: ComparisonMetrics::default(),
                    });
                    work.cohorts.len() - 1
                });
            let cohort = &mut work.cohorts[index];
            cohort.results.retain(|id, _| {
                subscriber.subjects.contains(id)
                    || cohort
                        .subscribers
                        .values()
                        .any(|owner| owner.subjects.contains(id))
            });
            if !cohort.reserve_subjects(&subscriber) {
                target_reply(&subscriber, Err(Error::new(ErrorKind::Backend("Comparison analysis reached its shared memory or subject limit. Run a deeper check to retry.".into()))));
                queue.owners.remove(owner);
                return;
            }
            target_reply(&subscriber, Ok(target.clone()));
            cohort.results.retain(|subject, result| {
                if !subscriber.subjects.contains(subject) {
                    return true;
                }
                if matches!(
                    result.counts,
                    RefComparisonCounts::Unavailable(
                        RefComparisonReason::IncompleteHistory | RefComparisonReason::Unsupported
                    )
                ) || (subscriber.request.deep
                    && matches!(result.counts, RefComparisonCounts::Unavailable(_)))
                {
                    return false;
                }
                if matches!(
                    result.integration,
                    RefIntegration::NotConfirmed(
                        RefComparisonReason::IncompleteHistory | RefComparisonReason::Unsupported
                    )
                ) || (subscriber.request.deep
                    && matches!(result.integration, RefIntegration::NotConfirmed(_)))
                {
                    result.integration = RefIntegration::Pending;
                }
                true
            });
            subscriber.pending = subscriber
                .subjects
                .iter()
                .filter(|id| {
                    cohort
                        .results
                        .get(*id)
                        .is_some_and(|result| result.counts != RefComparisonCounts::Pending)
                })
                .cloned()
                .collect();
            cohort.metrics.cache_hits = cohort
                .metrics
                .cache_hits
                .saturating_add(subscriber.pending.len() as u64);
            if subscriber.subjects.contains(&target.oid) {
                cohort.results.insert(
                    target.oid.clone(),
                    RefComparison {
                        subject: target.oid.clone(),
                        counts: RefComparisonCounts::Exact {
                            ahead: 0,
                            behind: 0,
                        },
                        integration: RefIntegration::Confirmed(IntegrationEvidence::UpToDate),
                    },
                );
                subscriber.pending.insert(target.oid.clone());
            }
            subscriber.target = Some(target);
            cohort.subscribers.insert(*owner, subscriber);
        }
        (Step::Analyze { cohort, depth, .. }, Completion::Batch(Ok(batch))) => {
            let Some(cohort) = work.cohorts.get_mut(*cohort) else {
                return;
            };
            if cohort.target.oid != batch.target.oid
                || cohort.target.context != batch.target.context
            {
                return;
            }
            cohort.metrics.jobs = cohort.metrics.jobs.saturating_add(1);
            cohort.metrics.graph_reads =
                cohort.metrics.graph_reads.saturating_add(batch.graph_reads);
            cohort.metrics.cache_hits = cohort
                .metrics
                .cache_hits
                .saturating_add(batch.cache_hits as u64);
            for mut result in batch.results {
                if *depth != RefComparisonDepth::Counts
                    && matches!(
                        result.counts,
                        RefComparisonCounts::Unavailable(
                            RefComparisonReason::Budget | RefComparisonReason::Deadline
                        )
                    )
                    && let Some(previous) = cohort.results.get(&result.subject)
                    && matches!(previous.counts, RefComparisonCounts::Exact { .. })
                {
                    result.counts = previous.counts;
                }
                for subscriber in cohort.subscribers.values_mut() {
                    if !subscriber.deferred && subscriber.subjects.contains(&result.subject) {
                        subscriber.pending.insert(result.subject.clone());
                    }
                }
                cohort.results.insert(result.subject.clone(), result);
            }
        }
        (_, Completion::Panicked | Completion::Batch(Err(_))) => {
            let error = Error::new(ErrorKind::Backend(
                "Comparison analysis could not complete. Run a deeper check to retry.".into(),
            ));
            match &task.step {
                Step::Resolve { owner, .. } => {
                    if let Some(subscriber) = work.targets.remove(owner) {
                        target_reply(&subscriber, Err(error));
                        queue.owners.remove(owner);
                    }
                }
                Step::Analyze {
                    cohort, subjects, ..
                } => {
                    if let Some(cohort) = work.cohorts.get_mut(*cohort) {
                        cohort.metrics.jobs = cohort.metrics.jobs.saturating_add(1);
                        for subject in subjects {
                            cohort.results.insert(
                                subject.clone(),
                                RefComparison {
                                    subject: subject.clone(),
                                    counts: cohort
                                        .results
                                        .get(subject)
                                        .filter(|result| {
                                            matches!(
                                                result.counts,
                                                RefComparisonCounts::Exact { .. }
                                            )
                                        })
                                        .map_or(
                                            RefComparisonCounts::Unavailable(
                                                RefComparisonReason::IncompleteHistory,
                                            ),
                                            |result| result.counts,
                                        ),
                                    integration: RefIntegration::NotConfirmed(
                                        RefComparisonReason::IncompleteHistory,
                                    ),
                                },
                            );
                        }
                        for subscriber in cohort.subscribers.values_mut() {
                            if subscriber.deferred {
                                continue;
                            }
                            subscriber.pending.extend(
                                subjects
                                    .iter()
                                    .filter(|id| subscriber.subjects.contains(*id))
                                    .cloned(),
                            );
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

fn publish(queue: &mut Queue) -> Option<Duration> {
    let mut delay = None;
    for work in queue.repositories.values_mut() {
        for cohort in &mut work.cohorts {
            for subscriber in cohort.subscribers.values_mut() {
                let Some(target) = &subscriber.target else {
                    continue;
                };
                if subscriber.pending.is_empty() {
                    continue;
                }
                let remaining = subscriber.last_publication.map_or(Duration::ZERO, |last| {
                    PUBLICATION_INTERVAL.saturating_sub(last.elapsed())
                });
                if !remaining.is_zero() {
                    delay =
                        Some(delay.map_or(remaining, |current: Duration| current.min(remaining)));
                    continue;
                }
                let Some(reservation) = subscriber.request.memory.reserve(32 * 1024) else {
                    subscriber.deferred = true;
                    subscriber.pending.clear();
                    subscriber.pending.shrink_to_fit();
                    send_or_log(
                        &subscriber.tx,
                        Msg::RefComparisons(RefComparisonsMsg::Deferred {
                            repo_id: subscriber.request.repo_id,
                            lifetime: subscriber.request.lifetime,
                            generation: subscriber.request.generation,
                            reason: RefComparisonReason::Budget,
                            reservation: subscriber._reservation.clone(),
                        }),
                    );
                    continue;
                };
                let mut ids = Vec::with_capacity(MAX_COMPARISON_SUBJECTS);
                for priority in 0..3 {
                    for id in &subscriber.pending {
                        if ids.len() == MAX_COMPARISON_SUBJECTS {
                            break;
                        }
                        if subject_priority(subscriber, id) == priority {
                            ids.push(id.clone());
                        }
                    }
                }
                let results = ids
                    .iter()
                    .filter_map(|id| cohort.results.get(id).cloned())
                    .collect();
                for id in ids {
                    subscriber.pending.remove(&id);
                }
                send_or_log(
                    &subscriber.tx,
                    Msg::RefComparisons(RefComparisonsMsg::Publish {
                        repo_id: subscriber.request.repo_id,
                        lifetime: subscriber.request.lifetime,
                        generation: subscriber.request.generation,
                        reservation,
                        metrics: cohort.metrics,
                        batch: RefComparisonBatch {
                            target: target.clone(),
                            results,
                            graph_reads: 0,
                            cache_hits: 0,
                        },
                    }),
                );
                subscriber.last_publication = Some(Instant::now());
                if !subscriber.pending.is_empty() {
                    delay = Some(PUBLICATION_INTERVAL);
                }
            }
        }
    }
    delay
}

fn run(pool: &'static Pool) {
    loop {
        let (task, delay) = {
            let mut queue = pool.queue.lock().unwrap_or_else(|error| error.into_inner());
            let delay = publish(&mut queue);
            let task = take_task(&mut queue);
            if task.is_none() && delay.is_none() {
                queue.workers -= 1;
                return;
            }
            (task, delay)
        };
        let Some(task) = task else {
            std::thread::sleep(
                delay
                    .unwrap_or(PUBLICATION_INTERVAL)
                    .min(PUBLICATION_INTERVAL),
            );
            continue;
        };
        let completion =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match &task.step {
                Step::Resolve { reference, .. } => Completion::Target(
                    task.repo
                        .ref_comparison_target(reference.as_deref(), &task.cancellation),
                ),
                Step::Analyze {
                    target,
                    subjects,
                    depth,
                    budget,
                    ..
                } => Completion::Batch(task.repo.analyze_ref_comparisons(
                    &RefComparisonRequest {
                        target: target.clone(),
                        subjects: subjects.clone(),
                        depth: *depth,
                        budget: *budget,
                    },
                    &task.cancellation,
                )),
            }))
            .unwrap_or(Completion::Panicked);
        let mut queue = pool.queue.lock().unwrap_or_else(|error| error.into_inner());
        complete(&mut queue, &task, completion);
        for work in queue.repositories.values_mut().filter(|work| !work.running) {
            work.cohorts.retain(|cohort| !cohort.subscribers.is_empty());
        }
        queue
            .repositories
            .retain(|_, work| work.running || !work.empty());
        start_workers(pool, &mut queue);
    }
}

pub(super) fn schedule(
    repos: &RepositoryRuntimes,
    tx: StoreWorkerSender,
    effect: RefComparisonsEffect,
) {
    let pool = pool();
    let mut queue = pool.queue.lock().unwrap_or_else(|error| error.into_inner());
    let request = match effect {
        RefComparisonsEffect::Release { lifetime } => {
            drop(queue);
            release(lifetime);
            return;
        }
        RefComparisonsEffect::Compare(request) => request,
        RefComparisonsEffect::Prioritize { lifetime, subjects } => {
            if let Some(scope) = queue.owners.get(&lifetime).copied()
                && let Some(work) = queue.repositories.get_mut(&scope)
            {
                let subscriber = work.targets.get_mut(&lifetime).or_else(|| {
                    work.cohorts
                        .iter_mut()
                        .find_map(|cohort| cohort.subscribers.get_mut(&lifetime))
                });
                if let Some(subscriber) = subscriber {
                    subscriber.visible = subjects.iter().take(128).cloned().collect();
                }
            }
            start_workers(pool, &mut queue);
            return;
        }
    };
    let Some(repo) = repos.repository(&request.repo_id).cloned() else {
        return;
    };
    let scope = repo.ref_comparison_scope();
    if admit(&mut queue, request, repo, tx, scope) {
        start_workers(pool, &mut queue);
    }
}

fn admit(
    queue: &mut Queue,
    request: RefComparisonSubscription,
    repo: Arc<dyn GitRepository>,
    tx: StoreWorkerSender,
    scope: Option<u64>,
) -> bool {
    if scope.is_none()
        || (queue.owners.len() >= MAX_SUBSCRIBERS && !queue.owners.contains_key(&request.lifetime))
    {
        retire(queue, request.lifetime);
        send_or_log(
            &tx,
            Msg::RefComparisons(RefComparisonsMsg::Target {
                reservation: None,
                repo_id: request.repo_id,
                lifetime: request.lifetime,
                generation: request.generation,
                result: Err(Error::new(ErrorKind::Unsupported(
                    "Comparison analysis is unavailable or its shared resource limit has been reached. Run a deeper check to retry.",
                ))),
            }),
        );
        return false;
    }
    let Some(scope) = scope else {
        return false;
    };
    let bytes = hash_table_bytes::<CommitId, ()>(request.projections.subjects.len())
        .saturating_mul(2)
        .saturating_add(64 * 1024)
        .saturating_add(
            request
                .reference
                .as_ref()
                .map_or(0, |reference| reference.len().saturating_mul(6)),
        );
    let Some(reservation) = request.memory.reserve(bytes) else {
        retire(queue, request.lifetime);
        send_or_log(&tx, Msg::RefComparisons(RefComparisonsMsg::Target {
                reservation: None, repo_id: request.repo_id, lifetime: request.lifetime, generation: request.generation, result: Err(Error::new(ErrorKind::Backend("Comparison queue reached its shared memory limit. Run a deeper check to retry.".into()))) }));
        return false;
    };
    if let Some(old) = queue.owners.insert(request.lifetime, scope)
        && old != scope
        && let Some(work) = queue.repositories.get_mut(&old)
    {
        work.remove(request.lifetime);
        if work.empty() {
            work.cancellation.cancel();
        }
    }
    let work = queue.repositories.entry(scope).or_default();
    let previous = work.remove(request.lifetime);
    work.targets.insert(
        request.lifetime,
        Subscriber::new(request, repo, tx, previous, reservation),
    );
    true
}

pub(in crate::store) fn release(lifetime: RepoLifetime) {
    let Some(pool) = POOL.get() else {
        return;
    };
    let mut queue = pool.queue.lock().unwrap_or_else(|error| error.into_inner());
    retire(&mut queue, lifetime);
}

fn retire(queue: &mut Queue, lifetime: RepoLifetime) {
    let Some(scope) = queue.owners.remove(&lifetime) else {
        return;
    };
    if let Some(work) = queue.repositories.get_mut(&scope) {
        work.remove(lifetime);
        if work.empty() {
            work.cancellation.cancel();
        }
    }
    queue
        .repositories
        .retain(|_, work| work.running || !work.empty());
}

#[cfg(test)]
mod tests;
