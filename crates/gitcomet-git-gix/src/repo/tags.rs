use super::GixRepo;
use crate::util::{
    git_workdir_cmd_for, run_git_capture, run_git_with_output, validate_ref_like_arg,
};
use gitcomet_core::domain::{CommitId, RemoteTag, Tag};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CommandOutput, Result};
use gix::bstr::ByteSlice as _;
use rustc_hash::FxHashSet as HashSet;
use std::str;
use std::thread;

fn parse_ls_remote_tag_names(output: &str) -> HashSet<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let (_object, reference) = line.split_once('\t')?;
            reference.strip_prefix("refs/tags/").map(ToOwned::to_owned)
        })
        .collect()
}

fn parse_ls_remote_tags(output: &str, remote_name: &str) -> Vec<RemoteTag> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let (object, reference) = line.split_once('\t')?;
            let name = reference.strip_prefix("refs/tags/")?;
            Some(RemoteTag {
                remote: remote_name.to_string(),
                name: name.to_string(),
                target: CommitId(object.to_string()),
            })
        })
        .collect()
}

fn local_tags_to_prune(local_tags_output: &str, remote_tags: &HashSet<String>) -> Vec<String> {
    local_tags_output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !remote_tags.contains(*line))
        .map(ToOwned::to_owned)
        .collect()
}

fn delete_local_tag(repo: &gix::Repository, name: &str) -> Result<()> {
    let ref_name = format!("refs/tags/{name}");
    let reference = repo
        .find_reference(ref_name.as_str())
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix find tag reference: {e}"))))?;
    reference
        .delete()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix delete tag {name}: {e}"))))
}

impl GixRepo {
    pub(super) fn list_tags_impl(&self) -> Result<Vec<Tag>> {
        let repo = self._repo.to_thread_local();

        let refs = repo
            .references()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix references: {e}"))))?;

        let iter = refs
            .tags()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix tags: {e}"))))?
            .peeled()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix peel refs: {e}"))))?;

        let mut tags = Vec::new();
        for reference in iter {
            let reference = reference
                .map_err(|e| Error::new(ErrorKind::Backend(format!("gix ref iter: {e}"))))?;
            let name = reference.name().shorten().to_str_lossy().into_owned();
            let target = CommitId(reference.id().detach().to_string());
            tags.push(Tag { name, target });
        }

        tags.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(tags)
    }

    pub(super) fn list_remote_tags_impl(&self) -> Result<Vec<RemoteTag>> {
        let remotes = self.list_remotes_impl()?;
        let workdir = self.spec.workdir.clone();
        let mut handles = Vec::new();

        for remote in remotes {
            if validate_ref_like_arg(&remote.name, "remote name").is_err() {
                continue;
            }

            let workdir = workdir.clone();
            let remote_name = remote.name;
            handles.push(thread::spawn(move || {
                let mut cmd = git_workdir_cmd_for(&workdir);
                cmd.arg("ls-remote")
                    .arg("--tags")
                    .arg("--refs")
                    .arg("--")
                    .arg(&remote_name);
                match run_git_capture(cmd, &format!("git ls-remote --tags --refs {remote_name}")) {
                    Ok(output) => Some(parse_ls_remote_tags(&output, &remote_name)),
                    // Remote tag presence is best-effort metadata for UI menus.
                    // If one remote is unavailable, keep partial results from others.
                    Err(_) => None,
                }
            }));
        }

        let mut remote_tags = Vec::new();
        for handle in handles {
            let Ok(maybe_tags) = handle.join() else {
                continue;
            };
            if let Some(mut tags) = maybe_tags {
                remote_tags.append(&mut tags);
            }
        }

        remote_tags.sort_by(|a, b| {
            a.remote
                .cmp(&b.remote)
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.target.as_ref().cmp(b.target.as_ref()))
        });
        Ok(remote_tags)
    }

    pub(super) fn create_tag_with_output_impl(
        &self,
        name: &str,
        target: &str,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(name, "tag name")?;
        validate_ref_like_arg(target, "tag target")?;

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("-c")
            .arg("alias.tag=")
            .arg("tag")
            .arg("-m")
            .arg(name)
            .arg("--")
            .arg(name)
            .arg(target);
        run_git_with_output(cmd, &format!("git tag -m {name} -- {name} {target}"))
    }

    pub(super) fn delete_tag_with_output_impl(&self, name: &str) -> Result<CommandOutput> {
        validate_ref_like_arg(name, "tag name")?;

        let repo = self._repo.to_thread_local();
        delete_local_tag(&repo, name)?;
        Ok(CommandOutput::empty_success(format!("git tag -d {name}")))
    }

    pub(super) fn push_tag_with_output_impl(
        &self,
        remote: &str,
        name: &str,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(remote, "remote name")?;
        validate_ref_like_arg(name, "tag name")?;

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("push")
            .arg("--")
            .arg(remote)
            .arg(format!("refs/tags/{name}"));
        run_git_with_output(cmd, &format!("git push {remote} refs/tags/{name}"))
    }

    pub(super) fn delete_remote_tag_with_output_impl(
        &self,
        remote: &str,
        name: &str,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(remote, "remote name")?;
        validate_ref_like_arg(name, "tag name")?;

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("push")
            .arg("--delete")
            .arg("--")
            .arg(remote)
            .arg(format!("refs/tags/{name}"));
        run_git_with_output(cmd, &format!("git push {remote} --delete refs/tags/{name}"))
    }

    pub(super) fn prune_local_tags_with_output_impl(&self) -> Result<CommandOutput> {
        let remotes = self.list_remotes_impl()?;
        if remotes.is_empty() {
            return Ok(CommandOutput {
                command: "git prune local tags".to_string(),
                stdout: "No remotes configured; skipping tag prune.\n".to_string(),
                stderr: String::new(),
                exit_code: Some(0),
            });
        }

        let mut remote_tags: HashSet<String> = HashSet::default();
        for remote in remotes {
            validate_ref_like_arg(&remote.name, "remote name")?;

            let mut cmd = self.git_workdir_cmd();
            cmd.arg("ls-remote")
                .arg("--tags")
                .arg("--refs")
                .arg("--")
                .arg(&remote.name);
            let output =
                run_git_capture(cmd, &format!("git ls-remote --tags --refs {}", remote.name))?;
            remote_tags.extend(parse_ls_remote_tag_names(&output));
        }

        let mut list_cmd = self.git_workdir_cmd();
        list_cmd.arg("tag").arg("--list");
        let local_tags = run_git_capture(list_cmd, "git tag --list")?;

        let deleted = local_tags_to_prune(&local_tags, &remote_tags);

        let mut stdout = String::new();
        let mut stderr = String::new();
        if !deleted.is_empty() {
            let mut delete_cmd = self.git_workdir_cmd();
            delete_cmd
                .arg("-c")
                .arg("alias.tag=")
                .arg("tag")
                .arg("-d")
                .arg("--");
            for local_tag in &deleted {
                delete_cmd.arg(local_tag);
            }

            let output =
                run_git_with_output(delete_cmd, &format!("git tag -d {}", deleted.join(" ")))?;
            if !output.stdout.is_empty() {
                stdout.push_str(&output.stdout);
            }
            if !output.stderr.is_empty() {
                stderr.push_str(&output.stderr);
            }
        }
        if deleted.is_empty() {
            stdout.push_str("No local tags to prune.\n");
        } else {
            if !stdout.ends_with('\n') && !stdout.is_empty() {
                stdout.push('\n');
            }
            stdout.push_str("Pruned local tags:\n");
            for name in deleted {
                stdout.push_str("- ");
                stdout.push_str(&name);
                stdout.push('\n');
            }
        }

        Ok(CommandOutput {
            command: "git prune local tags".to_string(),
            stdout,
            stderr,
            exit_code: Some(0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{local_tags_to_prune, parse_ls_remote_tag_names, parse_ls_remote_tags};
    use rustc_hash::FxHashSet as HashSet;

    #[test]
    fn parse_ls_remote_tag_names_skips_invalid_lines() {
        let output = "\
1111111111111111111111111111111111111111\trefs/tags/v1.0.0\n\
bad-line\n\
2222222222222222222222222222222222222222\trefs/heads/main\n\
3333333333333333333333333333333333333333\trefs/tags/v2.0.0\n";
        let names = parse_ls_remote_tag_names(output);
        assert_eq!(names.len(), 2);
        assert!(names.contains("v1.0.0"));
        assert!(names.contains("v2.0.0"));
    }

    #[test]
    fn parse_ls_remote_tags_assigns_remote_name() {
        let output = "\
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\trefs/tags/release\n\
bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\trefs/tags/hotfix\n";
        let tags = parse_ls_remote_tags(output, "origin");
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].remote, "origin");
        assert_eq!(tags[0].name, "release");
        assert_eq!(
            tags[0].target.as_ref(),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(tags[1].name, "hotfix");
    }

    #[test]
    fn local_tags_to_prune_only_returns_tags_missing_from_remotes() {
        let local_output = "v1.0.0\nv1.1.0\nv2.0.0\n";
        let remote_tags: HashSet<String> = ["v1.0.0".to_string(), "v2.0.0".to_string()]
            .into_iter()
            .collect();
        let prune = local_tags_to_prune(local_output, &remote_tags);
        assert_eq!(prune, vec!["v1.1.0".to_string()]);
    }
}
