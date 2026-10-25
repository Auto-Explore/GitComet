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
    (
        Subscriber {
            request: RefComparisonsEffect {
                repo_id: RepoId(lifetime.get()),
                lifetime,
                generation: 1,
                priority_only: false,
                reference: Some("HEAD".into()),
                subjects: Arc::new(subjects.to_vec()),
                deep: false,
            },
            repo: Arc::new(UnconfiguredRepository::new(path)),
            tx: StoreWorkerSender::for_test_msg_sender(tx),
            target: None,
            subjects: Arc::new(subjects.iter().cloned().collect()),
            visible: subjects.iter().take(128).cloned().collect(),
            pending: FxHashSet::default(),
            last_publication: None,
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
