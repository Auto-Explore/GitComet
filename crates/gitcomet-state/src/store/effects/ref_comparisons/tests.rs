use super::*;
use crate::model::RepoId;
use gitcomet_core::test_support::UnconfiguredRepository;
use std::sync::mpsc;

fn oid(index: usize) -> CommitId {
    CommitId(format!("{index:040x}").into())
}

fn subscriber(path: &str, subjects: &[CommitId]) -> (Subscriber, mpsc::Receiver<Msg>) {
    let (tx, rx) = mpsc::channel();
    let lifetime = RepoLifetime::allocate();
    let memory = Arc::new(gitcomet_core::ref_comparison_memory::ComparisonMemory::default());
    let branches: Vec<_> = subjects
        .iter()
        .enumerate()
        .map(|(ix, id)| gitcomet_core::domain::Branch {
            name: ix.to_string(),
            target: id.clone(),
            upstream: None,
            divergence: None,
        })
        .collect();
    let projections = crate::ref_comparison_projections::ComparisonProjections::capture(
        &memory,
        &branches,
        &[],
        &[],
        None,
        0,
    )
    .unwrap();
    let reservation = memory.reserve(64 * 1024).unwrap();
    let visible = crate::ref_comparisons::ComparisonPriorities::capture(
        &memory,
        subjects.iter().take(128).cloned().collect(),
    )
    .unwrap();
    (
        Subscriber {
            request: RefComparisonSubscription {
                repo_id: RepoId(lifetime.get()),
                lifetime,
                generation: 1,
                reference: Some("HEAD".into()),
                projections,
                memory,
                visible,
                deep: false,
            },
            repo: Arc::new(UnconfiguredRepository::new(path)),
            tx: StoreWorkerSender::for_test_msg_sender(tx),
            target: None,
            subjects: Arc::new(subjects.iter().cloned().collect()),
            visible: subjects.iter().take(128).cloned().collect(),
            pending: FxHashSet::default(),
            deferred: false,
            last_publication: None,
            _reservation: reservation,
        },
        rx,
    )
}

fn target(index: usize) -> RefComparisonTarget {
    RefComparisonTarget {
        reference: "HEAD".into(),
        oid: oid(index),
        context: 7,
    }
}

fn resolve_all(queue: &mut Queue, targets: &FxHashMap<RepoLifetime, RefComparisonTarget>) {
    while queue
        .repositories
        .values()
        .any(|work| !work.targets.is_empty())
    {
        let task = take_task(queue).unwrap();
        let Step::Resolve { owner, .. } = task.step else {
            panic!("resolve first");
        };
        complete(
            queue,
            &task,
            Completion::Target(Ok(targets[&owner].clone())),
        );
    }
}

#[test]
fn targets_resolve_through_each_subscribers_backend_before_coalescing_by_oid() {
    let (first, _) = subscriber("/first", &[oid(1)]);
    let (second, _) = subscriber("/second", &[oid(1)]);
    let targets = FxHashMap::from_iter([
        (first.request.lifetime, target(8)),
        (second.request.lifetime, target(9)),
    ]);
    let mut work = RepositoryWork::default();
    work.targets.insert(first.request.lifetime, first);
    work.targets.insert(second.request.lifetime, second);
    let mut queue = Queue::default();
    queue.repositories.insert(1, work);
    for _ in 0..2 {
        let task = take_task(&mut queue).unwrap();
        let Step::Resolve { owner, .. } = task.step else {
            panic!();
        };
        assert_eq!(
            task.repo.spec().workdir,
            queue.repositories[&1].targets[&owner].repo.spec().workdir
        );
        complete(
            &mut queue,
            &task,
            Completion::Target(Ok(targets[&owner].clone())),
        );
    }
    assert_eq!(queue.repositories[&1].cohorts.len(), 2);
}

#[test]
fn identical_tips_share_counting_and_a_surviving_subscribers_backend() {
    let (first, first_rx) = subscriber("/first", &[oid(1), oid(2)]);
    let (second, second_rx) = subscriber("/second", &[oid(1), oid(2)]);
    let first_owner = first.request.lifetime;
    let second_owner = second.request.lifetime;
    let mut work = RepositoryWork::default();
    work.targets.insert(first_owner, first);
    work.targets.insert(second_owner, second);
    let mut queue = Queue::default();
    queue.repositories.insert(1, work);
    resolve_all(
        &mut queue,
        &FxHashMap::from_iter([(first_owner, target(8)), (second_owner, target(8))]),
    );
    assert_eq!(queue.repositories[&1].cohorts.len(), 1);
    first_rx.try_recv().unwrap();
    second_rx.try_recv().unwrap();
    let task = take_task(&mut queue).unwrap();
    let Step::Analyze {
        subjects, target, ..
    } = &task.step
    else {
        panic!();
    };
    assert_eq!(subjects.len(), 2);
    complete(
        &mut queue,
        &task,
        Completion::Batch(Ok(RefComparisonBatch {
            target: target.clone(),
            results: subjects
                .iter()
                .map(|subject| RefComparison {
                    subject: subject.clone(),
                    counts: RefComparisonCounts::Exact {
                        ahead: 0,
                        behind: 1,
                    },
                    integration: RefIntegration::Confirmed(IntegrationEvidence::Merged),
                })
                .collect(),
            graph_reads: 2,
            cache_hits: 0,
        })),
    );
    publish(&mut queue);
    assert!(
        matches!(first_rx.try_recv().unwrap(), Msg::RefComparisons(RefComparisonsMsg::Publish { batch, .. }) if batch.results.len() == 2)
    );
    assert!(
        matches!(second_rx.try_recv().unwrap(), Msg::RefComparisons(RefComparisonsMsg::Publish { batch, .. }) if batch.results.len() == 2)
    );
    let work = queue.repositories.get_mut(&1).unwrap();
    work.remove(first_owner);
    assert!(!work.empty());
    assert!(!work.cancellation.is_cancelled());
    assert!(
        work.cohorts[0]
            .subscribers
            .values()
            .all(|subscriber| subscriber.repo.spec().workdir == std::path::Path::new("/second"))
    );
}

#[test]
fn visible_priority_changes_during_running_analysis_apply_to_the_next_batch() {
    let subjects: Vec<_> = (1..=500).map(oid).collect();
    let (first, _) = subscriber("/first", &subjects);
    let owner = first.request.lifetime;
    let mut work = RepositoryWork::default();
    work.targets.insert(owner, first);
    let mut queue = Queue::default();
    queue.repositories.insert(1, work);
    resolve_all(&mut queue, &FxHashMap::from_iter([(owner, target(999))]));
    let task = take_task(&mut queue).unwrap();
    queue.repositories.get_mut(&1).unwrap().cohorts[0]
        .subscribers
        .get_mut(&owner)
        .unwrap()
        .visible = FxHashSet::from_iter([oid(500)]);
    let Step::Analyze {
        subjects, target, ..
    } = &task.step
    else {
        panic!();
    };
    assert_eq!(subjects.len(), 128);
    complete(
        &mut queue,
        &task,
        Completion::Batch(Ok(RefComparisonBatch {
            target: target.clone(),
            results: subjects
                .iter()
                .map(|subject| RefComparison {
                    subject: subject.clone(),
                    counts: RefComparisonCounts::Exact {
                        ahead: 0,
                        behind: 1,
                    },
                    integration: RefIntegration::Confirmed(IntegrationEvidence::Merged),
                })
                .collect(),
            graph_reads: 128,
            cache_hits: 0,
        })),
    );
    let next = take_task(&mut queue).unwrap();
    let Step::Analyze {
        subjects, depth, ..
    } = next.step
    else {
        panic!();
    };
    assert_eq!(depth, RefComparisonDepth::Counts);
    assert_eq!(subjects, vec![oid(500)]);
}

#[test]
fn a_deeper_request_reuses_exact_counts_and_retries_integration() {
    let (mut subscriber, _rx) = subscriber("/first", &[oid(1)]);
    subscriber.request.deep = true;
    let owner = subscriber.request.lifetime;
    let mut work = RepositoryWork::default();
    work.targets.insert(owner, subscriber);
    work.cohorts.push(Cohort {
        reservation: None,
        bytes: 0,
        target: target(8),
        results: FxHashMap::from_iter([(
            oid(1),
            RefComparison {
                subject: oid(1),
                counts: RefComparisonCounts::Exact {
                    ahead: 1,
                    behind: 3,
                },
                integration: RefIntegration::NotConfirmed(RefComparisonReason::Budget),
            },
        )]),
        subscribers: FxHashMap::default(),
    });
    let mut queue = Queue::default();
    queue.repositories.insert(1, work);
    resolve_all(&mut queue, &FxHashMap::from_iter([(owner, target(8))]));
    let task = take_task(&mut queue).unwrap();
    assert!(
        matches!(task.step, Step::Analyze { depth: RefComparisonDepth::Deep, budget, .. } if budget.elapsed == Duration::from_secs(30))
    );
    assert!(matches!(
        queue.repositories[&1].cohorts[0].results[&oid(1)].counts,
        RefComparisonCounts::Exact {
            ahead: 1,
            behind: 3
        }
    ));
}

#[test]
fn deeper_coverage_applies_to_the_requested_subjects() {
    let (mut deeper, _deeper_rx) = subscriber("/deep", &[oid(1)]);
    deeper.request.deep = true;
    let (automatic, _automatic_rx) = subscriber("/automatic", &[oid(2)]);
    let cohort = Cohort {
        reservation: None,
        bytes: 0,
        target: target(8),
        results: FxHashMap::default(),
        subscribers: FxHashMap::from_iter([
            (deeper.request.lifetime, deeper),
            (automatic.request.lifetime, automatic),
        ]),
    };
    let (_, step) = cohort.next(0).unwrap();
    assert!(
        matches!(step, Step::Analyze { subjects, budget, .. } if subjects == vec![oid(2)] && budget.elapsed == Duration::from_secs(5))
    );
}

#[test]
fn active_and_visible_tips_precede_other_worktrees_and_hidden_branches() {
    let ids: Vec<_> = (1..=512).map(oid).collect();
    let (mut subscriber, _) = subscriber("/priorities", &ids);
    subscriber.request.visible = crate::ref_comparisons::ComparisonPriorities::capture(
        &subscriber.request.memory,
        vec![oid(500)],
    )
    .unwrap();
    let projections = Arc::get_mut(&mut subscriber.request.projections).unwrap();
    projections.active = Some(oid(450));
    projections.worktree_heads = FxHashSet::from_iter([oid(300), oid(400)]);
    let subscriber = Subscriber::new(
        subscriber.request,
        subscriber.repo,
        subscriber.tx,
        None,
        subscriber._reservation,
    );
    let mut cohort = Cohort {
        target: target(8),
        results: FxHashMap::default(),
        subscribers: FxHashMap::from_iter([(subscriber.request.lifetime, subscriber)]),
        reservation: None,
        bytes: 0,
    };
    let (_, Step::Analyze { subjects, .. }) = cohort.next(0).unwrap() else {
        panic!("count batch")
    };
    assert_eq!(
        subjects.iter().cloned().collect::<FxHashSet<_>>(),
        FxHashSet::from_iter([oid(450), oid(500)])
    );
    for subject in subjects {
        cohort.results.insert(
            subject.clone(),
            RefComparison {
                subject,
                counts: RefComparisonCounts::Exact {
                    ahead: 0,
                    behind: 1,
                },
                integration: RefIntegration::Confirmed(IntegrationEvidence::Merged),
            },
        );
    }
    let (
        _,
        Step::Analyze {
            subjects, depth, ..
        },
    ) = cohort.next(0).unwrap()
    else {
        panic!("worktree batch")
    };
    assert_eq!(depth, RefComparisonDepth::Counts);
    assert_eq!(
        subjects.iter().cloned().collect::<FxHashSet<_>>(),
        FxHashSet::from_iter([oid(300), oid(400)])
    );
}

#[test]
fn visible_publications_precede_a_backlog_of_hidden_completed_results() {
    let ids: Vec<_> = (1..=256).map(oid).collect();
    let (mut subscriber, rx) = subscriber("/publications", &ids);
    subscriber.target = Some(target(8));
    subscriber.pending.extend(ids.iter().cloned());
    let visible = subscriber.pending.iter().nth(200).unwrap().clone();
    subscriber.visible = FxHashSet::from_iter([visible.clone()]);
    let results = ids
        .into_iter()
        .map(|subject| {
            (
                subject.clone(),
                RefComparison {
                    subject,
                    counts: RefComparisonCounts::Exact {
                        ahead: 0,
                        behind: 1,
                    },
                    integration: RefIntegration::Confirmed(IntegrationEvidence::Merged),
                },
            )
        })
        .collect();
    let mut work = RepositoryWork::default();
    work.cohorts.push(Cohort {
        target: target(8),
        results,
        subscribers: FxHashMap::from_iter([(subscriber.request.lifetime, subscriber)]),
        reservation: None,
        bytes: 0,
    });
    let mut queue = Queue::default();
    queue.repositories.insert(1, work);
    publish(&mut queue);
    let Msg::RefComparisons(RefComparisonsMsg::Publish { batch, .. }) = rx.try_recv().unwrap()
    else {
        panic!("publication")
    };
    assert_eq!(batch.results.len(), 128);
    assert_eq!(batch.results[0].subject, visible);
    publish(&mut queue);
    assert!(
        rx.try_recv().is_err(),
        "later batches wait for the 100 ms interval"
    );
}

#[test]
fn incomplete_backend_batches_are_deferred_without_automatic_retries() {
    let (subscriber, _) = subscriber("/invalid-batch", &[oid(1)]);
    let owner = subscriber.request.lifetime;
    let memory = subscriber.request.memory.clone();
    let mut work = RepositoryWork::default();
    work.targets.insert(owner, subscriber);
    let mut queue = Queue::default();
    queue.repositories.insert(1, work);
    resolve_all(&mut queue, &FxHashMap::from_iter([(owner, target(8))]));
    let task = take_task(&mut queue).unwrap();
    complete(
        &mut queue,
        &task,
        Completion::Batch(Ok(RefComparisonBatch {
            target: target(8),
            results: Vec::new(),
            graph_reads: 0,
            cache_hits: 0,
        })),
    );
    assert!(take_task(&mut queue).is_none());
    assert_eq!(
        queue.repositories[&1].cohorts[0].results[&oid(1)].counts,
        RefComparisonCounts::Unavailable(RefComparisonReason::IncompleteHistory)
    );
    drop(task);
    assert!(memory.used_bytes() > 0);
    drop(queue);
    assert_eq!(memory.used_bytes(), 0);
}

#[test]
fn publication_memory_exhaustion_defers_without_a_worker_or_publication_retry() {
    let _guard = crate::ref_comparisons::MEMORY_TESTS.lock().unwrap();
    let (mut subscriber, rx) = subscriber("/publication-limit", &[oid(1)]);
    subscriber.target = Some(target(8));
    subscriber.pending.insert(oid(1));
    let memory = subscriber.request.memory.clone();
    let held = memory
        .reserve(COMPARISON_REPOSITORY_BYTES - memory.used_bytes())
        .unwrap();
    let mut work = RepositoryWork::default();
    work.cohorts.push(Cohort {
        target: target(8),
        results: FxHashMap::default(),
        subscribers: FxHashMap::from_iter([(subscriber.request.lifetime, subscriber)]),
        reservation: None,
        bytes: 0,
    });
    let mut queue = Queue::default();
    queue.repositories.insert(1, work);
    assert!(publish(&mut queue).is_none());
    assert!(matches!(
        rx.try_recv().unwrap(),
        Msg::RefComparisons(RefComparisonsMsg::Deferred {
            reason: RefComparisonReason::Budget,
            ..
        })
    ));
    drop(held);
    assert!(publish(&mut queue).is_none());
    assert!(take_task(&mut queue).is_none());
    assert!(rx.try_recv().is_err());
    drop(queue);
    assert_eq!(memory.used_bytes(), 0);
}

#[test]
fn refusal_retires_previous_ownership_without_cancelling_another_subscriber() {
    let _guard = crate::ref_comparisons::MEMORY_TESTS.lock().unwrap();
    let (first, rx) = subscriber("/first", &[oid(1)]);
    let (second, _) = subscriber("/second", &[oid(1)]);
    let mut request = first.request.clone();
    request.generation += 1;
    let memory = request.memory.clone();
    let repo = first.repo.clone();
    let tx = first.tx.clone();
    let owner = request.lifetime;
    let survivor = second.request.lifetime;
    let held = memory
        .reserve(COMPARISON_REPOSITORY_BYTES - memory.used_bytes())
        .unwrap();
    let mut work = RepositoryWork {
        running: true,
        ..Default::default()
    };
    let cancellation = work.cancellation.clone();
    work.targets.insert(owner, first);
    work.targets.insert(survivor, second);
    let mut queue = Queue::default();
    queue.repositories.insert(1, work);
    queue.owners.extend([(owner, 1), (survivor, 1)]);
    assert!(!admit(&mut queue, request, repo, tx, Some(1)));
    assert!(matches!(
        rx.try_recv().unwrap(),
        Msg::RefComparisons(RefComparisonsMsg::Target {
            result: Err(_),
            generation: 2,
            ..
        })
    ));
    assert!(!queue.owners.contains_key(&owner));
    assert!(!queue.repositories[&1].targets.contains_key(&owner));
    assert!(queue.repositories[&1].targets.contains_key(&survivor));
    assert!(!cancellation.is_cancelled());
    retire(&mut queue, survivor);
    assert!(cancellation.is_cancelled());
    drop(held);
    drop(queue);
    assert_eq!(memory.used_bytes(), 0);
}

#[test]
fn pending_successes_are_incomplete_terminal_results_for_both_analysis_phases() {
    for evidence in [false, true] {
        let (subscriber, _rx) = subscriber("/pending-success", &[oid(1)]);
        let owner = subscriber.request.lifetime;
        let mut work = RepositoryWork::default();
        work.targets.insert(owner, subscriber);
        let mut queue = Queue::default();
        queue.repositories.insert(1, work);
        resolve_all(&mut queue, &FxHashMap::from_iter([(owner, target(8))]));
        let counts = if evidence {
            RefComparisonCounts::Exact {
                ahead: 1,
                behind: 3,
            }
        } else {
            RefComparisonCounts::Pending
        };
        if evidence {
            queue.repositories.get_mut(&1).unwrap().cohorts[0]
                .results
                .get_mut(&oid(1))
                .unwrap()
                .counts = counts;
        }
        let task = take_task(&mut queue).unwrap();
        assert!(
            matches!(&task.step, Step::Analyze { depth, .. } if *depth == if evidence { RefComparisonDepth::Evidence } else { RefComparisonDepth::Counts })
        );
        complete(
            &mut queue,
            &task,
            Completion::Batch(Ok(RefComparisonBatch {
                target: target(8),
                results: vec![RefComparison {
                    subject: oid(1),
                    counts,
                    integration: RefIntegration::Pending,
                }],
                graph_reads: 0,
                cache_hits: 0,
            })),
        );
        assert!(take_task(&mut queue).is_none());
        let result = &queue.repositories[&1].cohorts[0].results[&oid(1)];
        assert_eq!(
            result.integration,
            RefIntegration::NotConfirmed(RefComparisonReason::IncompleteHistory)
        );
        assert_eq!(
            result.counts,
            if evidence {
                counts
            } else {
                RefComparisonCounts::Unavailable(RefComparisonReason::IncompleteHistory)
            }
        );
    }
}

#[test]
fn evidence_resource_exhaustion_preserves_already_published_exact_counts() {
    let (subscriber, _rx) = subscriber("/evidence-budget", &[oid(1)]);
    let owner = subscriber.request.lifetime;
    let mut work = RepositoryWork::default();
    work.targets.insert(owner, subscriber);
    let mut queue = Queue::default();
    queue.repositories.insert(1, work);
    resolve_all(&mut queue, &FxHashMap::from_iter([(owner, target(8))]));
    let exact = RefComparisonCounts::Exact {
        ahead: 2,
        behind: 5,
    };
    queue.repositories.get_mut(&1).unwrap().cohorts[0]
        .results
        .get_mut(&oid(1))
        .unwrap()
        .counts = exact;
    let task = take_task(&mut queue).unwrap();
    complete(
        &mut queue,
        &task,
        Completion::Batch(Ok(RefComparisonBatch {
            target: target(8),
            results: vec![RefComparison {
                subject: oid(1),
                counts: RefComparisonCounts::Unavailable(RefComparisonReason::Budget),
                integration: RefIntegration::NotConfirmed(RefComparisonReason::Budget),
            }],
            graph_reads: 0,
            cache_hits: 0,
        })),
    );
    let result = &queue.repositories[&1].cohorts[0].results[&oid(1)];
    assert_eq!(result.counts, exact);
    assert_eq!(
        result.integration,
        RefIntegration::NotConfirmed(RefComparisonReason::Budget)
    );
    assert!(take_task(&mut queue).is_none());
}
