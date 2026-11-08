/// Whether git accepts `name` for a new local branch, as
/// `git check-ref-format --branch` does.
pub fn is_valid_branch_name(name: &str) -> bool {
    // Git reads `@` as HEAD's branch, and a leading `-` as an option.
    !name.starts_with('-')
        && name != "@"
        && gix_validate::reference::branch_name(format!("refs/heads/{name}").as_str().into())
            .is_ok()
}

/// Whether git accepts `name` as a full ref name, as `git check-ref-format`
/// does.
pub fn is_valid_ref_name(name: &str) -> bool {
    gix_validate::reference::name(name.into()).is_ok()
}

#[cfg(test)]
mod tests {
    use super::{is_valid_branch_name, is_valid_ref_name};

    #[test]
    fn full_ref_names_follow_git_ref_rules() {
        for valid in ["refs/t3", "refs/jj/keep", "refs/review-1.2"] {
            assert!(is_valid_ref_name(valid), "{valid}");
        }
        for invalid in [
            "refs/a..b",
            "refs/x.lock",
            "refs/a.",
            "refs/@{x}",
            "refs/my review",
            "refs/a*b",
            "refs//a",
            "refs/.hidden",
        ] {
            assert!(!is_valid_ref_name(invalid), "{invalid}");
        }
    }

    #[test]
    fn branch_names_follow_git_ref_rules() {
        for valid in ["main", "feature/x", "fix-1.2", "a@b"] {
            assert!(is_valid_branch_name(valid), "{valid}");
        }
        for invalid in [
            "",
            "@",
            "my feature",
            "a..b",
            "x.lock",
            "-x",
            "HEAD",
            "a/",
            "/a",
            "a//b",
            "@{x}",
            "a~1",
            "a:b",
            "a^b",
            "a?b",
            "a*b",
            "a[b",
            ".hidden",
            "a\\b",
        ] {
            assert!(!is_valid_branch_name(invalid), "{invalid:?}");
        }
    }
}
