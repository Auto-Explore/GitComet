use crate::model::RepoId;

/// Fetch operations share authentication, cancellation, and transfer reporting.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum FetchMsg {
    All {
        repo_id: RepoId,
    },
    Remote {
        repo_id: RepoId,
        remote: String,
    },
    Refspecs {
        repo_id: RepoId,
        remote: String,
        refspecs: Vec<String>,
    },
}
