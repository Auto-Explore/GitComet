use super::GixRepo;
use crate::util::{
    path_buf_from_git_bytes, run_git_capture_bytes_cancellable, run_git_with_output,
    validate_ref_like_arg,
};
use gitcomet_core::domain::{CommitId, Worktree};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::{CancellationToken, CommandOutput, Result, WorktreeCheckout};
use gix::bstr::ByteSlice as _;
use std::path::Path;
use std::process::Command;

mod native_config;
#[cfg(test)]
mod native_config_tests;

impl GixRepo {
    pub(super) fn list_worktrees_impl(&self) -> Result<Vec<Worktree>> {
        self.list_worktrees_cancellable_impl(&CancellationToken::new())
    }

    pub(super) fn list_worktrees_cancellable_impl(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Vec<Worktree>> {
        cancellation.check_cancelled()?;
        // The existing layout can only reject the optimization. In particular,
        // linked checkouts should not reparse configuration before invoking Git.
        // A positive still refreshes refs/config and rechecks the whole layout;
        // registrations and configuration can change while this handle is open.
        if !worktree_environment_overridden()
            && self.may_be_sole_primary_worktree()
            && let Some(repo) = native_config::reopen(self)
        {
            cancellation.check_cancelled()?;
            if let Some(worktree) = sole_primary_worktree(&repo) {
                cancellation.check_cancelled()?;
                return Ok(vec![worktree]);
            }
        }
        cancellation.check_cancelled()?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("worktree").arg("list").arg("--porcelain").arg("-z");
        let output = run_git_capture_bytes_cancellable(
            cmd,
            "git worktree list --porcelain -z",
            cancellation,
        )?;
        parse_git_worktree_list_porcelain_z(&output)
    }

    /// A stale negative only forgoes the native path; a positive is not evidence
    /// that a listing can be returned without the fresh authoritative checks.
    fn may_be_sole_primary_worktree(&self) -> bool {
        let common_dir = {
            let store = self.store.read().expect("repo store");
            let git_dir = store.repo.git_dir();
            let common_dir = store.repo.common_dir.as_deref().unwrap_or(git_dir);
            if git_dir != common_dir {
                return false;
            }
            common_dir.to_path_buf()
        };
        // The store guard must not outlive the path read: filesystem I/O and
        // the subsequent fresh open must not delay a store replacement.
        no_worktree_registrations(&common_dir)
    }

    pub(super) fn add_worktree_with_output_impl(
        &self,
        path: &Path,
        checkout: &WorktreeCheckout,
    ) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        let label = match checkout {
            WorktreeCheckout::Existing { reference } => {
                push_worktree_args(&mut cmd, &["add"], path, Some(reference))?
            }
            WorktreeCheckout::NewBranch { name, start } => {
                validate_ref_like_arg(name, "worktree branch")?;
                push_worktree_args(
                    &mut cmd,
                    &["add", "--no-track", "-b", name],
                    path,
                    start.as_deref(),
                )?
            }
            WorktreeCheckout::TrackRemote { name, upstream } => {
                validate_ref_like_arg(name, "worktree branch")?;
                validate_ref_like_arg(&upstream.remote, "remote name")?;
                validate_ref_like_arg(&upstream.branch, "branch name")?;
                let tracking_ref = self.live_tracking_ref(upstream, "git worktree add --track")?;
                push_worktree_args(
                    &mut cmd,
                    &["add", "--track", "-b", name],
                    path,
                    Some(&tracking_ref),
                )?
            }
        };
        run_git_with_output(cmd, &label)
    }

    pub(super) fn remove_worktree_with_output_impl(&self, path: &Path) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        let label = push_worktree_args(&mut cmd, &["remove"], path, None)?;
        run_git_with_output(cmd, &label)
    }

    pub(super) fn force_remove_worktree_with_output_impl(
        &self,
        path: &Path,
    ) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        let label = push_worktree_args(&mut cmd, &["remove", "--force"], path, None)?;
        run_git_with_output(cmd, &label)
    }
}

fn worktree_environment_overridden() -> bool {
    [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_NAMESPACE",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
    ]
    .iter()
    .any(|name| std::env::var_os(name).is_some())
}

fn no_worktree_registrations(common_dir: &Path) -> bool {
    let registrations = common_dir.join("worktrees");
    match std::fs::symlink_metadata(&registrations) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Ok(metadata) if plain_directory(&metadata) => {
            // An entry or a failed enumeration may represent another checkout.
            std::fs::read_dir(&registrations).is_ok_and(|mut entries| entries.next().is_none())
        }
        _ => false,
    }
}

/// Only the conventional primary checkout with a resolved HEAD is handled
/// here. Git remains authoritative for linked, bare and unusual layouts.
fn sole_primary_worktree(repo: &gix::Repository) -> Option<Worktree> {
    if repo.git_dir_trust() != gix::sec::Trust::Full
        || repo.is_bare()
        || repo.git_dir() != repo.common_dir()
        || crate::refs::backend(repo).ok()? != crate::refs::RefBackend::Files
        || repo.config_snapshot().string("core.worktree").is_some()
    {
        return None;
    }
    let workdir = repo.workdir()?;
    let dot_git = workdir.join(".git");
    if !plain_directory(&std::fs::symlink_metadata(workdir).ok()?)
        || !plain_directory(&std::fs::symlink_metadata(&dot_git).ok()?)
        || !same_file::is_same_file(repo.git_dir(), &dot_git).ok()?
    {
        return None;
    }

    if !no_worktree_registrations(repo.common_dir()) {
        return None;
    }

    let refs = crate::refs::view(repo).ok()?;
    let (id, branch, detached) = match refs.find_exact(b"HEAD").ok()??.target {
        gix::refs::Target::Symbolic(name) => {
            let branch = name.as_bstr().strip_prefix(b"refs/heads/")?;
            // Symbolic chains and unborn refs retain Git's existing behavior.
            let gix::refs::Target::Object(id) = refs.find_exact(name.as_bstr()).ok()??.target
            else {
                return None;
            };
            (id, Some(branch.to_str_lossy().into_owned()), false)
        }
        gix::refs::Target::Object(id) => (id, None, true),
    };
    // Like Git's porcelain listing, return the ref's OID without loading its
    // object. Missing objects and detached tag targets still have a worktree row.
    let mut worktree = Worktree {
        path: workdir.to_path_buf(),
        bare: false,
        head: Some(CommitId(super::oid_to_arc_str(&id))),
        branch,
        detached,
    };
    canonicalize_worktree_path(&mut worktree);
    Some(worktree)
}

fn plain_directory(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        // Junctions and other reparse points are not all exposed as symlinks.
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    metadata.is_dir() && !metadata.file_type().is_symlink()
}

/// Append `git worktree <args> -- <path> [<reference>]` and return the display
/// label.
///
/// `path` and `reference` are typed by the user, so `--` keeps a value that
/// starts with `-` out of the option parser, and the reference is checked the
/// way every other ref argument in this crate is.
fn push_worktree_args(
    cmd: &mut Command,
    args: &[&str],
    path: &Path,
    reference: Option<&str>,
) -> Result<String> {
    if let Some(reference) = reference {
        validate_ref_like_arg(reference, "worktree reference")?;
    }
    // The label is a human-readable summary that the UI also inspects to
    // recover the worktree path, so `--` stays an argv concern only.
    cmd.arg("worktree").args(args).arg("--").arg(path);
    let mut label = format!("git worktree {} {}", args.join(" "), path.display());
    if let Some(reference) = reference {
        cmd.arg(reference);
        label.push(' ');
        label.push_str(reference);
    }
    Ok(label)
}

pub(super) fn parse_git_worktree_list_porcelain_z(output: &[u8]) -> Result<Vec<Worktree>> {
    let mut out = Vec::new();
    let mut current: Option<Worktree> = None;

    for field in output.split(|b| *b == b'\0') {
        if field.is_empty() {
            if let Some(mut wt) = current.take() {
                canonicalize_worktree_path(&mut wt);
                out.push(wt);
            }
            continue;
        }

        if let Some(rest) = field.strip_prefix(b"worktree ") {
            if let Some(mut wt) = current.take() {
                canonicalize_worktree_path(&mut wt);
                out.push(wt);
            }
            current = Some(Worktree {
                path: path_buf_from_git_bytes(rest, "git worktree list path")?,
                bare: false,
                head: None,
                branch: None,
                detached: false,
            });
            continue;
        }

        let Some(wt) = current.as_mut() else {
            continue;
        };

        if let Some(rest) = field.strip_prefix(b"HEAD ") {
            if !rest.is_empty() {
                wt.head = Some(CommitId(String::from_utf8_lossy(rest).into_owned().into()));
            }
        } else if let Some(rest) = field.strip_prefix(b"branch ") {
            let branch = String::from_utf8_lossy(rest);
            if let Some(stripped) = branch.strip_prefix("refs/heads/") {
                wt.branch = Some(stripped.to_string());
            } else if !branch.is_empty() {
                wt.branch = Some(branch.into_owned());
            }
        } else if field == b"bare" {
            wt.bare = true;
        } else if field == b"detached" {
            wt.detached = true;
            wt.branch = None;
        }
    }

    if let Some(mut wt) = current.take() {
        canonicalize_worktree_path(&mut wt);
        out.push(wt);
    }

    Ok(out)
}

fn canonicalize_worktree_path(worktree: &mut Worktree) {
    worktree.path = canonicalize_or_original(worktree.path.clone());
}

#[cfg(test)]
mod tests {
    use super::{
        GixRepo, Worktree, parse_git_worktree_list_porcelain_z, push_worktree_args,
        sole_primary_worktree,
    };
    use crate::test_support::{git_output, git_stdout, git_success, init_test_repo, open_repo};
    use gitcomet_core::path_utils::canonicalize_or_original;
    use gitcomet_core::services::{CancellationToken, GitRepository as _};
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn committed_primary() -> (tempfile::TempDir, GixRepo) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("repository space");
        std::fs::create_dir(&path).unwrap();
        init_test_repo(&path);
        git_success(&path, &["commit", "--allow-empty", "-m", "initial"]);
        git_success(&path, &["branch", "-M", "main"]);
        let repo = open_repo(&path);
        (root, repo)
    }

    fn oracle(repo: &GixRepo) -> Vec<Worktree> {
        let output = git_output(
            &repo.spec.workdir,
            &["worktree", "list", "--porcelain", "-z"],
        );
        assert!(output.status.success(), "{output:?}");
        parse_git_worktree_list_porcelain_z(&output.stdout).unwrap()
    }

    fn assert_listing(repo: &GixRepo, commands: usize) {
        let (actual, trace) =
            crate::command_trace::capture(|| repo.list_worktrees(&CancellationToken::new()));
        assert_eq!(actual.unwrap(), oracle(repo));
        assert_eq!(trace.len(), commands, "{trace:?}");
    }

    #[test]
    fn native_single_worktree_matches_git_after_head_changes() {
        let (_root, repo) = committed_primary();
        assert_listing(&repo, 0);
        git_success(&repo.spec.workdir, &["checkout", "-b", "topic/nested"]);
        git_success(
            &repo.spec.workdir,
            &["commit", "--allow-empty", "-m", "second"],
        );
        git_success(&repo.spec.workdir, &["pack-refs", "--all"]);
        assert_listing(&repo, 0);
        git_success(&repo.spec.workdir, &["checkout", "--detach", "HEAD~1"]);
        assert_listing(&repo, 0);
        git_success(&repo.spec.workdir, &["checkout", "main"]);
        assert_listing(&repo, 0);
    }

    #[test]
    fn native_single_worktree_rechecks_linked_registrations_each_time() {
        let (root, repo) = committed_primary();
        assert!(repo.may_be_sole_primary_worktree());
        assert_listing(&repo, 0);
        let linked = root.path().join("linked checkout");
        git_success(
            &repo.spec.workdir,
            &[
                "worktree",
                "add",
                "--detach",
                linked.to_str().unwrap(),
                "HEAD",
            ],
        );
        assert!(!repo.may_be_sole_primary_worktree());
        assert_listing(&repo, 1);
        let linked_repo = open_repo(&linked);
        assert!(!linked_repo.may_be_sole_primary_worktree());
        assert_listing(&linked_repo, 1);
        drop(linked_repo);
        git_success(
            &repo.spec.workdir,
            &["worktree", "lock", linked.to_str().unwrap()],
        );
        assert_listing(&repo, 1);
        git_success(
            &repo.spec.workdir,
            &["worktree", "unlock", linked.to_str().unwrap()],
        );
        git_success(
            &repo.spec.workdir,
            &["worktree", "remove", linked.to_str().unwrap()],
        );
        assert!(repo.may_be_sole_primary_worktree());
        assert_listing(&repo, 0);
    }

    #[test]
    fn native_single_worktree_keeps_git_for_unborn_and_separate_git_dirs() {
        let unborn = tempfile::tempdir().unwrap();
        init_test_repo(unborn.path());
        assert_listing(&open_repo(unborn.path()), 1);

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("checkout");
        std::fs::create_dir(&path).unwrap();
        let separate = root.path().join("separate metadata");
        git_success(
            &path,
            &["init", "--separate-git-dir", separate.to_str().unwrap()],
        );
        git_success(
            &path,
            &[
                "-c",
                "user.name=Test User",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-m",
                "initial",
            ],
        );
        assert_listing(&open_repo(&path), 1);
    }

    #[test]
    fn native_single_worktree_reads_oid_without_requiring_its_object() {
        let (_root, repo) = committed_primary();
        git_success(&repo.spec.workdir, &["tag", "-a", "tagged", "-m", "tag"]);
        let tag = git_stdout(&repo.spec.workdir, &["rev-parse", "refs/tags/tagged"]);
        std::fs::write(repo.spec.workdir.join(".git/HEAD"), format!("{tag}\n")).unwrap();
        assert_listing(&repo, 0);
        let absent = "1111111111111111111111111111111111111111";
        std::fs::write(repo.spec.workdir.join(".git/HEAD"), format!("{absent}\n")).unwrap();
        assert_listing(&repo, 0);
        std::fs::write(
            repo.spec.workdir.join(".git/refs/heads/main"),
            format!("{absent}\n"),
        )
        .unwrap();
        std::fs::write(
            repo.spec.workdir.join(".git/HEAD"),
            b"ref: refs/heads/main\n",
        )
        .unwrap();
        assert_listing(&repo, 0);
    }

    #[test]
    fn native_single_worktree_defers_reduced_trust_to_git() {
        let (_root, repo) = committed_primary();
        let reduced = gix::open_opts(
            repo.spec.workdir.join(".git"),
            gix::open::Options::default()
                .permissions(gix::open::Permissions::isolated())
                .with(gix::sec::Trust::Reduced),
        )
        .unwrap();
        assert_eq!(reduced.git_dir_trust(), gix::sec::Trust::Reduced);
        assert!(sole_primary_worktree(&reduced).is_none());
    }

    #[test]
    fn native_single_worktree_defers_uncertain_layouts_and_symbolic_chains() {
        let (_root, repo) = committed_primary();
        let registrations = repo.spec.workdir.join(".git/worktrees");
        std::fs::write(&registrations, b"not a registration directory").unwrap();
        assert!(!repo.may_be_sole_primary_worktree());
        assert!(sole_primary_worktree(&repo.reopen_repo().unwrap()).is_none());
        let (_, trace) =
            crate::command_trace::capture(|| repo.list_worktrees(&CancellationToken::new()));
        assert_eq!(trace.len(), 1);
        std::fs::remove_file(&registrations).unwrap();
        std::fs::create_dir(&registrations).unwrap();
        std::fs::create_dir(registrations.join("unknown checkout")).unwrap();
        assert!(!repo.may_be_sole_primary_worktree());
        assert!(sole_primary_worktree(&repo.reopen_repo().unwrap()).is_none());
        std::fs::remove_dir(registrations.join("unknown checkout")).unwrap();
        assert!(repo.may_be_sole_primary_worktree());
        assert_listing(&repo, 0);

        git_success(
            &repo.spec.workdir,
            &["symbolic-ref", "refs/heads/alias", "refs/heads/main"],
        );
        git_success(
            &repo.spec.workdir,
            &["symbolic-ref", "HEAD", "refs/heads/alias"],
        );
        assert_listing(&repo, 1);
        git_success(
            &repo.spec.workdir,
            &["symbolic-ref", "HEAD", "refs/heads/main"],
        );
        git_success(&repo.spec.workdir, &["config", "core.worktree", "."]);
        // The layout-only preflight remains positive, but the fresh config
        // must still reject the native path after a same-handle change.
        assert!(repo.may_be_sole_primary_worktree());
        assert!(sole_primary_worktree(&repo.reopen_repo().unwrap()).is_none());
        let (actual, trace) =
            crate::command_trace::capture(|| repo.list_worktrees(&CancellationToken::new()));
        assert_eq!(actual.unwrap(), oracle(&repo));
        assert_eq!(
            trace
                .iter()
                .map(|command| command.label.as_str())
                .collect::<Vec<_>>(),
            [
                "git rev-parse --is-bare-repository",
                "git worktree list --porcelain -z"
            ]
        );
        assert_listing(&repo, 1);
    }

    #[test]
    fn native_single_worktree_pre_cancelled_load_never_starts_git() {
        let (_root, repo) = committed_primary();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let (result, trace) = crate::command_trace::capture(|| repo.list_worktrees(&cancellation));
        assert!(matches!(
            result.unwrap_err().kind(),
            gitcomet_core::error::ErrorKind::Cancelled
        ));
        assert!(trace.is_empty());
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn native_single_worktree_does_not_follow_registration_directory_links() {
        let (root, repo) = committed_primary();
        let target = root.path().join("registration target");
        std::fs::create_dir(&target).unwrap();
        let registrations = repo.spec.workdir.join(".git/worktrees");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &registrations).unwrap();
        #[cfg(windows)]
        {
            let output = Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&registrations)
                .arg(&target)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
        }
        assert!(!repo.may_be_sole_primary_worktree());
        assert_listing(&repo, 1);
    }

    #[test]
    fn worktree_add_places_separator_before_path_and_reference() {
        let mut cmd = Command::new("git");
        let label = push_worktree_args(&mut cmd, &["add"], Path::new("-linked"), Some("main"))
            .expect("valid worktree arguments");
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(
            args,
            [
                OsStr::new("worktree"),
                OsStr::new("add"),
                OsStr::new("--"),
                OsStr::new("-linked"),
                OsStr::new("main"),
            ]
        );
        assert_eq!(label, "git worktree add -linked main");
    }

    #[test]
    fn worktree_add_rejects_option_like_reference() {
        let mut cmd = Command::new("git");
        let err = push_worktree_args(&mut cmd, &["add"], Path::new("linked"), Some("--detach"))
            .expect_err("an option-looking reference must be refused");
        assert!(err.to_string().contains("worktree reference"), "{err}");
        assert_eq!(
            cmd.get_args().count(),
            0,
            "nothing should be appended on refusal"
        );
    }

    #[test]
    fn worktree_remove_keeps_flags_before_separator() {
        let mut cmd = Command::new("git");
        let label = push_worktree_args(&mut cmd, &["remove", "--force"], Path::new("linked"), None)
            .expect("valid worktree arguments");
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(
            args,
            [
                OsStr::new("worktree"),
                OsStr::new("remove"),
                OsStr::new("--force"),
                OsStr::new("--"),
                OsStr::new("linked"),
            ]
        );
        assert_eq!(label, "git worktree remove --force linked");
    }

    #[test]
    fn parse_git_worktree_list_porcelain_z_parses_regular_and_detached_entries() {
        let parsed = parse_git_worktree_list_porcelain_z(
            b"worktree /repo\0HEAD 1111111111111111111111111111111111111111\0branch refs/heads/main\0\0worktree /repo-linked\0HEAD 2222222222222222222222222222222222222222\0detached\0\0",
        )
        .unwrap();

        assert_eq!(parsed.len(), 2);

        assert_eq!(parsed[0].path, PathBuf::from("/repo"));
        assert_eq!(
            parsed[0].head.as_ref().map(|id| id.as_ref()),
            Some("1111111111111111111111111111111111111111")
        );
        assert_eq!(parsed[0].branch.as_deref(), Some("main"));
        assert!(!parsed[0].detached);

        assert_eq!(parsed[1].path, PathBuf::from("/repo-linked"));
        assert_eq!(
            parsed[1].head.as_ref().map(|id| id.as_ref()),
            Some("2222222222222222222222222222222222222222")
        );
        assert!(parsed[1].branch.is_none());
        assert!(parsed[1].detached);
    }

    #[test]
    fn parse_git_worktree_list_porcelain_z_ignores_noise_before_first_worktree() {
        let parsed = parse_git_worktree_list_porcelain_z(
            b"HEAD deadbeef\0branch refs/heads/ignored\0\0worktree /repo\0branch feature/topic\0\0",
        )
        .unwrap();

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].path, PathBuf::from("/repo"));
        assert_eq!(parsed[0].branch.as_deref(), Some("feature/topic"));
        assert!(parsed[0].head.is_none());
    }

    #[test]
    fn parse_git_worktree_list_porcelain_z_skips_empty_head_values() {
        let parsed = parse_git_worktree_list_porcelain_z(
            b"worktree /repo\0HEAD \0branch refs/heads/main\0\0",
        )
        .unwrap();

        assert_eq!(parsed.len(), 1);
        assert!(parsed[0].head.is_none());
        assert_eq!(parsed[0].branch.as_deref(), Some("main"));
    }

    #[test]
    fn parse_git_worktree_list_porcelain_z_preserves_newlines_in_paths() {
        let parsed = parse_git_worktree_list_porcelain_z(
            b"worktree /repo\nlinked\0HEAD 1111111111111111111111111111111111111111\0detached\0\0",
        )
        .unwrap();

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].path, PathBuf::from("/repo\nlinked"));
        assert!(parsed[0].detached);
    }

    #[test]
    fn parse_git_worktree_list_porcelain_z_canonicalizes_existing_worktree_paths() {
        let root = std::env::temp_dir().join(format!(
            "gitcomet-worktree-parse-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let nested = root.join("repo");
        std::fs::create_dir_all(&nested).unwrap();

        let input = format!(
            "worktree {}\0HEAD 1111111111111111111111111111111111111111\0branch refs/heads/main\0\0",
            nested.join("..").join("repo").display()
        );
        let parsed = parse_git_worktree_list_porcelain_z(input.as_bytes()).unwrap();

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].path, canonicalize_or_original(nested.clone()));
    }
}
