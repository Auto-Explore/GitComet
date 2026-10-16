use crate::model::RepoId;
use gitcomet_core::domain::HistoryMode;
use std::path::PathBuf;

#[derive(Clone, Debug, strum::IntoStaticStr)]
pub enum PersistenceEffect {
    UpdateRepositoryPreferences {
        repo_id: RepoId,
        key: crate::model::RepositoryKey,
        update: crate::model::RepositoryPreferenceUpdate,
    },
    PersistSession {
        repo_id: Option<RepoId>,
        action: &'static str,
    },
    PersistRecentRepo {
        repo_id: Option<RepoId>,
        workdir: PathBuf,
        action: &'static str,
    },
    PersistRepoHistoryMode {
        repo_id: Option<RepoId>,
        workdir: PathBuf,
        mode: HistoryMode,
        action: &'static str,
    },
    PersistRepoHistoryModesBatch {
        repo_id: Option<RepoId>,
        updates: Vec<(PathBuf, HistoryMode)>,
        action: &'static str,
    },
    PersistRepoHistoryAuthorFilter {
        repo_id: Option<RepoId>,
        workdir: PathBuf,
        author: Option<String>,
        action: &'static str,
    },
    PersistRepoMaintenanceSnooze {
        common_dir: std::path::PathBuf,
    },
}
