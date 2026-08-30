use super::*;
use crate::msg::{ConflictAutosolveMode, ConflictAutosolveStats};

#[test]
fn record_conflict_autosolve_telemetry_logs_mode_and_unresolved_deltas() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id: RepoId(1),
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RecordConflictAutosolveTelemetry {
            repo_id: RepoId(1),
            path: Some(PathBuf::from("src/lib.rs")),
            mode: ConflictAutosolveMode::Regex,
            total_conflicts_before: 7,
            total_conflicts_after: 9,
            unresolved_before: 7,
            unresolved_after: 2,
            stats: ConflictAutosolveStats {
                pass1: 2,
                pass2_split: 3,
                pass1_after_split: 1,
                regex: 2,
                history: 0,
            },
        },
    );
    assert!(effects.is_empty(), "telemetry should not schedule effects");

    let repo_state = state
        .repos
        .iter()
        .find(|repo| repo.id == RepoId(1))
        .expect("opened repo");
    assert_eq!(repo_state.feedback.command_log.len(), 1);
    let entry = &repo_state.feedback.command_log[0];
    assert!(entry.ok);
    assert_eq!(
        entry.command,
        "telemetry.conflict_autosolve.regex src/lib.rs"
    );
    assert!(entry.summary.contains("resolved 8"));
    assert!(entry.summary.contains("unresolved 7 -> 2"));
    assert!(entry.summary.contains("conflicts 7 -> 9"));
    assert!(entry.summary.contains("pass1=2"));
    assert!(entry.summary.contains("pass2_split=3"));
    assert!(entry.summary.contains("pass1_after_split=1"));
    assert!(entry.summary.contains("regex=2"));
}
