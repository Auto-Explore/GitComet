#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RepoExternalChange {
    pub worktree: bool,
    pub index: bool,
    pub git_state: bool,
    pub tags: bool,
    /// Configuration/watch-policy inputs changed. A routine full refresh does
    /// not imply a changed verification context.
    pub verification_context: bool,
    /// Attribute inputs changed; independent of ordinary content/index edits.
    pub text_attributes: bool,
}

impl RepoExternalChange {
    #[allow(non_upper_case_globals)]
    pub const Worktree: Self = Self::worktree();

    /// Every kind of change either side reports.
    pub const fn union(self, other: Self) -> Self {
        Self {
            worktree: self.worktree || other.worktree,
            index: self.index || other.index,
            git_state: self.git_state || other.git_state,
            tags: self.tags || other.tags,
            verification_context: self.verification_context || other.verification_context,
            text_attributes: self.text_attributes || other.text_attributes,
        }
    }

    #[allow(non_upper_case_globals)]
    pub const Index: Self = Self::index();
    #[allow(non_upper_case_globals)]
    pub const GitState: Self = Self::git_state();
    #[allow(non_upper_case_globals)]
    pub const Both: Self = Self::all();

    pub const fn worktree() -> Self {
        Self {
            worktree: true,
            index: false,
            git_state: false,
            tags: false,
            verification_context: false,
            text_attributes: false,
        }
    }

    pub const fn index() -> Self {
        Self {
            worktree: false,
            index: true,
            git_state: false,
            tags: false,
            verification_context: false,
            text_attributes: false,
        }
    }

    pub const fn git_state() -> Self {
        Self {
            worktree: false,
            index: false,
            git_state: true,
            tags: false,
            verification_context: false,
            text_attributes: false,
        }
    }

    pub const fn all() -> Self {
        Self {
            worktree: true,
            index: true,
            git_state: true,
            tags: true,
            verification_context: false,
            text_attributes: true,
        }
    }

    pub const fn is_empty(self) -> bool {
        !self.worktree
            && !self.index
            && !self.git_state
            && !self.tags
            && !self.verification_context
            && !self.text_attributes
    }
}
