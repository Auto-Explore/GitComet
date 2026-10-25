use super::super::executor::TaskExecutor;
use super::super::repository_runtime::RepositoryRuntimes;
use super::super::worker_channel::StoreWorkerSender;
use super::util::send_or_log;
use crate::model::RepoLifetime;
use crate::msg::Msg;
use crate::ref_comparisons::{RefComparisonsEffect, RefComparisonsMsg};
use gitcomet_core::domain::CommitId;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::ref_comparison::*;
use gitcomet_core::services::{CancellationToken, GitRepository};
use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const MAX_SUBSCRIBERS: usize = 256;
const PUBLICATION_INTERVAL: Duration = Duration::from_millis(100);

struct Subscriber {
    request: RefComparisonsEffect,
    repo: Arc<dyn GitRepository>,
    tx: StoreWorkerSender,
    target: Option<RefComparisonTarget>,
    subjects: Arc<FxHashSet<CommitId>>,
    visible: FxHashSet<CommitId>,
    pending: FxHashSet<CommitId>,
    last_publication: Option<Instant>,
}

struct Cohort {
    target: RefComparisonTarget,
    results: FxHashMap<CommitId, RefComparison>,
    subscribers: FxHashMap<RepoLifetime, Subscriber>,
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
fn pool() -> &'static Pool {
    POOL.get_or_init(|| Pool {
        queue: Mutex::default(),
        executor: TaskExecutor::named("gitcomet-ref-analysis", 2),
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
    fn next(&self, index: usize) -> Option<(u8, Step)> {
        let mut counts = FxHashSet::default();
        let mut evidence = FxHashSet::default();
        let mut visible = FxHashSet::default();
        let mut deep = false;
        for subscriber in self.subscribers.values() {
            deep |= subscriber.request.deep;
            visible.extend(subscriber.visible.iter().cloned());
            for id in subscriber.subjects.iter() {
                match self.results.get(id) {
                    None => {
                        counts.insert(id.clone());
                    }
                    Some(result) if result.integration == RefIntegration::Pending => {
                        evidence.insert(id.clone());
                    }
                    _ => {}
                }
            }
        }
        let budget = if deep {
            RefComparisonBudget::deep()
        } else {
            RefComparisonBudget::automatic()
        };
        for (set, stage, depth, limit) in [
            (
                &counts,
                1,
                RefComparisonDepth::Counts,
                MAX_COMPARISON_SUBJECTS,
            ),
            (
                &evidence,
                3,
                if deep {
                    RefComparisonDepth::Deep
                } else {
                    RefComparisonDepth::Evidence
                },
                1,
            ),
        ] {
            if set.is_empty() {
                continue;
            }
            let has_visible = set.iter().any(|id| visible.contains(id));
            let subjects = set
                .iter()
                .filter(|id| !has_visible || visible.contains(*id))
                .take(limit)
                .cloned()
                .collect();
            return Some((
                stage + u8::from(!has_visible),
                Step::Analyze {
                    cohort: index,
                    target: self.target.clone(),
                    subjects,
                    depth,
                    budget,
                },
            ));
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
            repo_id: subscriber.request.repo_id,
            lifetime: subscriber.request.lifetime,
            generation: subscriber.request.generation,
            result,
        }),
    );
}

fn complete(queue: &mut Queue, task: &Task, completion: Completion) {
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
            target_reply(&subscriber, result.clone());
            let Ok(target) = result else {
                queue.owners.remove(owner);
                return;
            };
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
                    });
                    work.cohorts.len() - 1
                });
            let cohort = &mut work.cohorts[index];
            if subscriber.request.deep {
                cohort
                    .results
                    .retain(|_, result| matches!(result.integration, RefIntegration::Confirmed(_)));
            }
            subscriber.pending = subscriber
                .subjects
                .iter()
                .filter(|id| cohort.results.contains_key(*id))
                .cloned()
                .collect();
            subscriber.target = Some(target);
            cohort.subscribers.insert(*owner, subscriber);
        }
        (Step::Analyze { cohort, .. }, Completion::Batch(Ok(batch))) => {
            let Some(cohort) = work.cohorts.get_mut(*cohort) else {
                return;
            };
            if cohort.target.oid != batch.target.oid
                || cohort.target.context != batch.target.context
            {
                return;
            }
            for result in batch.results {
                for subscriber in cohort.subscribers.values_mut() {
                    if subscriber.subjects.contains(&result.subject) {
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
                        for subject in subjects {
                            cohort.results.insert(
                                subject.clone(),
                                RefComparison {
                                    subject: subject.clone(),
                                    counts: cohort.results.get(subject).map_or(
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
                let ids: Vec<_> = subscriber
                    .pending
                    .iter()
                    .take(MAX_COMPARISON_SUBJECTS)
                    .cloned()
                    .collect();
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
    request: RefComparisonsEffect,
) {
    let pool = pool();
    let mut queue = pool.queue.lock().unwrap_or_else(|error| error.into_inner());
    if request.priority_only {
        if let Some(scope) = queue.owners.get(&request.lifetime).copied()
            && let Some(work) = queue.repositories.get_mut(&scope)
        {
            let subscriber = work.targets.get_mut(&request.lifetime).or_else(|| {
                work.cohorts
                    .iter_mut()
                    .find_map(|cohort| cohort.subscribers.get_mut(&request.lifetime))
            });
            if let Some(subscriber) = subscriber {
                subscriber.visible = request.subjects.iter().cloned().collect();
            }
        }
        start_workers(pool, &mut queue);
        return;
    }
    let Some(repo) = repos.repository(&request.repo_id).cloned() else {
        return;
    };
    let scope = repo.ref_comparison_scope();
    if scope.is_none()
        || (queue.owners.len() >= MAX_SUBSCRIBERS && !queue.owners.contains_key(&request.lifetime))
    {
        send_or_log(
            &tx,
            Msg::RefComparisons(RefComparisonsMsg::Target {
                repo_id: request.repo_id,
                lifetime: request.lifetime,
                generation: request.generation,
                result: Err(Error::new(ErrorKind::Unsupported(
                    "Comparison analysis is unavailable or its shared resource limit has been reached. Run a deeper check to retry.",
                ))),
            }),
        );
        return;
    }
    let Some(scope) = scope else {
        return;
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
    let subjects = Arc::new(request.subjects.iter().cloned().collect());
    let visible = previous
        .as_ref()
        .map(|subscriber| subscriber.visible.clone())
        .unwrap_or_else(|| {
            request
                .subjects
                .iter()
                .take(MAX_COMPARISON_SUBJECTS)
                .cloned()
                .collect()
        });
    work.targets.insert(
        request.lifetime,
        Subscriber {
            request,
            repo,
            tx,
            target: None,
            subjects,
            visible,
            pending: FxHashSet::default(),
            last_publication: previous.and_then(|subscriber| subscriber.last_publication),
        },
    );
    start_workers(pool, &mut queue);
}

pub(in crate::store) fn release(lifetime: RepoLifetime) {
    let Some(pool) = POOL.get() else {
        return;
    };
    let mut queue = pool.queue.lock().unwrap_or_else(|error| error.into_inner());
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
