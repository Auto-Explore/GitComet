/// Whether git accepts `name` for a new local branch, as
/// `git check-ref-format --branch` does.
pub fn is_valid_branch_name(name: &str) -> bool {
    // Git reads `@` as HEAD's branch, and a leading `-` as an option.
    !name.starts_with('-')
        && name != "@"
        && gix_validate::reference::branch_name(format!("refs/heads/{name}").as_str().into())
            .is_ok()
}

#[cfg(test)]
mod tests {
    use super::is_valid_branch_name;

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
