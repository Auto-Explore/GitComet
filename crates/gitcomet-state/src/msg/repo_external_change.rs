#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RepoExternalChange {
    pub worktree: bool,
    pub index: bool,
    pub git_state: bool,
}

impl RepoExternalChange {
    #[allow(non_upper_case_globals)]
    pub const Worktree: Self = Self::worktree();
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
        }
    }

    pub const fn index() -> Self {
        Self {
            worktree: false,
            index: true,
            git_state: false,
        }
    }

    pub const fn git_state() -> Self {
        Self {
            worktree: false,
            index: false,
            git_state: true,
        }
    }

    pub const fn all() -> Self {
        Self {
            worktree: true,
            index: true,
            git_state: true,
        }
    }

    pub const fn is_empty(self) -> bool {
        !self.worktree && !self.index && !self.git_state
    }
}
