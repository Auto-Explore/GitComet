use crate::domain::CommitId;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum TagPushMode {
    FollowAnnotated,
    All,
}

impl TagPushMode {
    pub const ALL: [Self; 2] = [Self::FollowAnnotated, Self::All];

    pub fn index(self) -> usize {
        match self {
            Self::FollowAnnotated => 0,
            Self::All => 1,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::FollowAnnotated => "Push with annotated tags",
            Self::All => "Push with all tags",
        }
    }

    pub fn flag(self) -> &'static str {
        match self {
            Self::FollowAnnotated => "--follow-tags",
            Self::All => "--tags",
        }
    }
}

/// A branch push and its tag policy, resolved before showing a preview or
/// starting authentication. Retries retain both the destination and the policy.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct TagPushRequest {
    pub mode: TagPushMode,
    pub remote: String,
    pub branch: String,
    pub local_branch: String,
    pub head: CommitId,
    pub set_upstream: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TagPushPreview {
    /// Unique names of tags that would be created on at least one push URL.
    pub new_tags: Vec<String>,
    /// Tags rejected on at least one destination, kept out of the new-tag count.
    pub conflicting_tags: Vec<String>,
    pub rejected_branches: Vec<String>,
    pub destinations: Vec<String>,
}
