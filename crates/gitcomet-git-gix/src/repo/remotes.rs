use super::GixRepo;
use super::history::gix_head_id_or_none;
use super::porcelain::edit_local_config_strict;
use crate::util::{
    bytes_to_text_preserving_utf8, git_command_failed_error, run_git_capture, run_git_raw_output,
    run_git_simple, run_git_with_output, validate_hex_commit_id, validate_ref_like_arg,
};
use gitcomet_core::domain::{CommitId, Remote, RemoteBranch, Upstream};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::remote_url::{RemoteUrlPolicy, validate_remote_url_with_policy};
use gitcomet_core::services::{
    CancellationToken, CommandOutput, ForcePushLease, PullMode, RemoteUrlKind, Result,
    SafePushAfterCommitContext, SafePushAfterCommitDecision, SafePushAfterCommitTarget,
};
use gitcomet_core::text_utils::redact_url_userinfo;
use gix::bstr::ByteSlice as _;
use rustc_hash::FxHashSet;
use std::process::Command;
use std::str;

const PENDING_UPSTREAM_CONFIG_KEY: &str = "gitcometPendingUpstream";

/// Display label for `git remote add`; the URL is masked because the label
/// ends up in the command log and error toasts, unlike the argv.
fn remote_add_label(name: &str, url: &str) -> String {
    format!("git remote add {name} {}", redact_url_userinfo(url))
}

fn remote_set_url_label(name: &str, url: &str, kind: RemoteUrlKind) -> String {
    let url = redact_url_userinfo(url);
    match kind {
        RemoteUrlKind::Fetch => format!("git remote set-url {name} {url}"),
        RemoteUrlKind::Push => format!("git remote set-url --push {name} {url}"),
    }
}

fn parse_refname_set(output: &str) -> FxHashSet<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn branches_to_prune(
    branches_output: &str,
    merged: &FxHashSet<String>,
    existing_tracking_refs: &FxHashSet<String>,
    current_branch: Option<&str>,
) -> Vec<String> {
    let mut candidates = Vec::new();

    for line in branches_output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let (branch, upstream) = line.split_once('\t').unwrap_or((line, ""));
        if branch.is_empty() || upstream.is_empty() {
            continue;
        }
        if current_branch == Some(branch) {
            continue;
        }
        if !merged.contains(branch) {
            continue;
        }

        let tracking_ref = format!("refs/remotes/{upstream}");
        if existing_tracking_refs.contains(&tracking_ref) {
            continue;
        }
        candidates.push(branch.to_string());
    }

    candidates
}

/// Forward-map an exact remote branch through that remote's fetch refspecs.
/// This deliberately does not inspect any other remote, even when another
/// remote maps to the same local tracking ref.
pub(super) fn tracking_refs_for_remote_branch(
    repo: &gix::Repository,
    upstream: &Upstream,
) -> Result<Vec<String>> {
    let remote = repo.find_remote(upstream.remote.as_str()).map_err(|e| {
        Error::new(ErrorKind::Backend(format!(
            "gix find_remote {}: {e}",
            upstream.remote
        )))
    })?;
    let source_ref = format!("refs/heads/{}", upstream.branch);
    let null = repo.object_hash().null();
    let matches = gix::refspec::MatchGroup::from_fetch_specs(
        remote
            .refspecs(gix::remote::Direction::Fetch)
            .iter()
            .map(|spec| spec.to_ref()),
    )
    .match_lhs(
        Some(gix::refspec::match_group::Item {
            full_ref_name: source_ref.as_bytes().as_bstr(),
            target: &null,
            object: None,
        })
        .into_iter(),
    );

    let mut tracking_refs = matches
        .mappings
        .into_iter()
        .filter_map(|mapping| mapping.rhs)
        .map(|name| name.to_str_lossy().into_owned())
        .collect::<Vec<_>>();
    tracking_refs.sort_unstable();
    tracking_refs.dedup();
    Ok(tracking_refs)
}

pub(super) fn configured_upstream_of(reference: &gix::Reference<'_>) -> Option<Upstream> {
    let local_branch = reference
        .name()
        .as_bstr()
        .strip_prefix(b"refs/heads/")?
        .as_bstr();
    let config = reference.repo.config_snapshot();
    let mut configured_remote = None;
    let mut configured_merge = None;
    for section in config
        .plumbing()
        .sections_by_name("branch")?
        .filter(|section| section.header().subsection_name() == Some(local_branch))
    {
        if let Some(value) = section.value("remote") {
            configured_remote = Some(value);
        }
        if let Some(value) = section.value("merge") {
            configured_merge = Some(value);
        }
    }
    let remote = configured_remote?.to_str_lossy().into_owned();
    // A branch remote may also be `.` or a URL. Only a named, configured
    // remote can own one of the remote-tracking branches this API exposes.
    reference.repo.find_remote(remote.as_str()).ok()?;
    let branch = configured_merge?
        .as_bstr()
        .strip_prefix(b"refs/heads/")
        .map(|name| name.to_str_lossy().into_owned())
        .filter(|name| !name.is_empty())?;
    Some(Upstream { remote, branch })
}

/// Whether GitComet intentionally configured this upstream before the remote
/// branch existed. This distinguishes a future branch from a formerly-live
/// upstream that disappeared and remains eligible for fetch cleanup.
pub(super) fn configured_upstream_is_pending(reference: &gix::Reference<'_>) -> bool {
    let local_branch = match reference.name().as_bstr().strip_prefix(b"refs/heads/") {
        Some(branch) => branch.as_bstr(),
        None => return false,
    };
    let config = reference.repo.config_snapshot();
    config
        .plumbing()
        .sections_by_name("branch")
        .into_iter()
        .flatten()
        .filter(|section| section.header().subsection_name() == Some(local_branch))
        .filter_map(|section| section.value(PENDING_UPSTREAM_CONFIG_KEY))
        .filter_map(|value| gix::config::Boolean::try_from(value).ok())
        .map(bool::from)
        .last()
        .unwrap_or(false)
}

fn normalize_remote_url(url: &str) -> String {
    let Some(path) = url.strip_prefix("file://") else {
        return url.to_string();
    };
    let path_bytes = path.as_bytes();
    if path.starts_with('/')
        || path_bytes.len() < 3
        || !path_bytes[0].is_ascii_alphabetic()
        || path_bytes[1] != b':'
        || !matches!(path_bytes[2], b'/' | b'\\')
    {
        return url.to_string();
    }

    // gix serializes Windows drive-letter file remotes as `file://C:/...`.
    let normalized_path = path.replace('\\', "/");
    format!("file:///{normalized_path}")
}

fn safe_push_ref_display(remote: &str, branch: &str) -> String {
    format!("{remote}/{branch}")
}

fn output_mentions_missing_remote_ref(output: &std::process::Output) -> bool {
    let mut text = bytes_to_text_preserving_utf8(&output.stderr);
    text.push('\n');
    text.push_str(&bytes_to_text_preserving_utf8(&output.stdout));
    text.contains("couldn't find remote ref")
        || text.contains("could not find remote ref")
        || text.contains("couldn't find remote branch")
        || text.contains("could not find remote branch")
}

fn run_git_command<S, O>(
    cmd: Command,
    label: &str,
    capture_output: bool,
    run_simple: S,
    run_with_output: O,
) -> Result<CommandOutput>
where
    S: FnOnce(Command, &str) -> Result<()>,
    O: FnOnce(Command, &str) -> Result<CommandOutput>,
{
    if capture_output {
        return run_with_output(cmd, label);
    }

    run_simple(cmd, label)?;
    Ok(CommandOutput::empty_success(label))
}

fn run_git_command_with_optional_output(
    cmd: Command,
    label: &str,
    capture_output: bool,
) -> Result<CommandOutput> {
    run_git_command(
        cmd,
        label,
        capture_output,
        run_git_simple,
        run_git_with_output,
    )
}

fn combine_command_outputs(command: impl Into<String>, outputs: &[CommandOutput]) -> CommandOutput {
    CommandOutput {
        command: command.into(),
        stdout: outputs
            .iter()
            .map(|output| output.stdout.trim_end())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        stderr: outputs
            .iter()
            .map(|output| output.stderr.trim_end())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        exit_code: Some(0),
    }
}

/// A remote's configured fetch refspecs, grouped by destination namespace.
#[derive(Debug, Default)]
struct RemoteFetchRefspecs {
    /// Positive refspecs whose destination is under `refs/remotes/`.
    tracking: Vec<String>,
    /// Negative refspecs, which Git applies to the whole positive set.
    exclusions: Vec<String>,
    /// Whether a positive refspec writes outside `refs/remotes/`, such as the
    /// explicit `+refs/tags/*:refs/tags/*` some mirrors configure.
    has_non_tracking_destination: bool,
}

impl RemoteFetchRefspecs {
    /// Refspecs to hand `git fetch --prune` so that it can only delete
    /// remote-tracking refs. Git applies negative refspecs to the complete
    /// positive set regardless of their configuration order, so appending them
    /// preserves the remote's exclusions. Negative-only command lines are
    /// invalid and cannot prune anything.
    fn into_prune_command_refspecs(mut self) -> Vec<String> {
        if self.tracking.is_empty() {
            return Vec::new();
        }
        self.tracking.extend(self.exclusions);
        self.tracking
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ConfiguredRemoteUpstream {
    local_branch: String,
    remote: String,
    remote_branch: String,
    tracking_ref: Option<String>,
    pending: bool,
}

enum UpstreamCleanupScope<'a> {
    Remotes(&'a [String]),
    Remote(&'a str),
    RemoteBranches {
        remote: &'a str,
        branches: &'a [String],
    },
}

impl UpstreamCleanupScope<'_> {
    fn includes(&self, upstream: &ConfiguredRemoteUpstream) -> bool {
        match self {
            Self::Remotes(remotes) => remotes.contains(&upstream.remote),
            Self::Remote(remote) => upstream.remote == *remote,
            Self::RemoteBranches { remote, branches } => {
                upstream.remote == *remote
                    && branches
                        .iter()
                        .any(|branch| branch == &upstream.remote_branch)
            }
        }
    }
}

/// Outcome of unlinking configured upstreams. Cleanup runs after the remote has
/// already changed, so what it could not do belongs in the command output: an
/// error here would report an operation that already succeeded as a failure.
#[derive(Debug, Default)]
struct UnlinkedUpstreams {
    branches: Vec<String>,
    failures: Vec<String>,
}

impl UnlinkedUpstreams {
    fn failed(failure: String) -> Self {
        Self {
            branches: Vec::new(),
            failures: vec![failure],
        }
    }
}

fn append_unlinked_upstreams(
    mut output: CommandOutput,
    unlinked: &UnlinkedUpstreams,
) -> CommandOutput {
    if !unlinked.branches.is_empty() {
        if !output.stdout.is_empty() && !output.stdout.ends_with('\n') {
            output.stdout.push('\n');
        }
        output
            .stdout
            .push_str("Unlinked deleted upstream branches:\n");
        for branch in &unlinked.branches {
            output.stdout.push_str("- ");
            output.stdout.push_str(branch);
            output.stdout.push('\n');
        }
    }
    if !unlinked.failures.is_empty() {
        if !output.stderr.is_empty() && !output.stderr.ends_with('\n') {
            output.stderr.push('\n');
        }
        output.stderr.push_str(&format!(
            "Could not unlink some deleted upstreams: {}\n",
            unlinked.failures.join("; ")
        ));
    }
    output
}

/// A remote name can be interpolated into a `remote.<name>.<key>` config key -
/// on a `git -c` argument or in a lookup - only when it cannot be mistaken for
/// the start of the value.
fn remote_name_is_config_key_safe(remote: &str) -> bool {
    !remote.is_empty() && !remote.contains(['=', '\n', '\r'])
}

/// Read a boolean directly from the matching `[remote "..."]` sections. This
/// avoids interpolating the remote name into a dotted config key, where names
/// containing `=` can be parsed as part of the value instead.
fn remote_config_boolean(
    config: &gix::config::Snapshot<'_>,
    remote_name: &str,
    value_name: &str,
) -> Option<bool> {
    let remote_name = remote_name.as_bytes().as_bstr();
    let value = config
        .sections_by_name("remote")?
        .filter(|section| section.header().subsection_name() == Some(remote_name))
        .filter_map(|section| section.value_implicit(value_name))
        .last()?;
    match value {
        // A key without `=` is Git's implicit true spelling.
        None => Some(true),
        Some(value) => gix::config::Boolean::try_from(value).ok().map(Into::into),
    }
}

/// The `branch.<name>.remote`/`merge` pair a local branch reference configures.
/// `tracking_ref` is populated only when the remote's positive and negative
/// fetch refspecs map this upstream locally.
fn configured_remote_upstream_of(
    reference: &gix::Reference<'_>,
) -> Option<ConfiguredRemoteUpstream> {
    let local_branch = reference.name().shorten().to_str_lossy().into_owned();
    let Upstream {
        remote,
        branch: remote_branch,
    } = configured_upstream_of(reference)?;
    let tracking_ref = tracking_refs_for_remote_branch(
        reference.repo,
        &Upstream {
            remote: remote.clone(),
            branch: remote_branch.clone(),
        },
    )
    .ok()
    .and_then(|refs| match refs.as_slice() {
        [tracking_ref] => Some(tracking_ref.clone()),
        _ => None,
    });

    Some(ConfiguredRemoteUpstream {
        local_branch,
        remote,
        remote_branch,
        tracking_ref,
        pending: configured_upstream_is_pending(reference),
    })
}

fn remote_branch_gone_after_fetch_error(remote: &str, branch: &str) -> Error {
    Error::new(ErrorKind::Backend(format!(
        "Remote branch {remote}/{branch} no longer exists. Fetch completed and removed its stale reference."
    )))
}

impl GixRepo {
    fn best_effort_delete_reference(&self, ref_name: &str) {
        let Ok(repo) = self.reopen_repo() else {
            return;
        };
        let Ok(Some(reference)) = repo.try_find_reference(ref_name) else {
            return;
        };
        let _ = reference.delete();
    }

    fn reference_exists(&self, ref_name: &str) -> Result<bool> {
        let repo = self.reopen_repo()?;
        Ok(repo
            .try_find_reference(ref_name)
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix try_find_reference: {e}"))))?
            .is_some())
    }

    fn remote_fetch_and_push_urls_match(&self, remote_name: &str) -> bool {
        let Ok(repo) = self.reopen_repo() else {
            return false;
        };
        let Ok(remote) = repo.find_remote(remote_name) else {
            return false;
        };
        let fetch_urls = remote
            .urls(gix::remote::Direction::Fetch)
            .map(|url| url.to_bstring())
            .collect::<Vec<_>>();
        let push_urls = remote
            .urls(gix::remote::Direction::Push)
            .map(|url| url.to_bstring())
            .collect::<Vec<_>>();
        !fetch_urls.is_empty() && fetch_urls == push_urls
    }

    fn preferred_remote_name(&self) -> Result<Option<String>> {
        let remotes = self.list_remotes_impl()?;
        if remotes.is_empty() {
            return Ok(None);
        }
        if remotes.iter().any(|r| r.name == "origin") {
            return Ok(Some("origin".to_string()));
        }
        Ok(Some(remotes[0].name.clone()))
    }

    fn current_branch_name(&self) -> Result<Option<String>> {
        let head = self.current_branch_impl()?;
        let head = head.trim();
        if head.is_empty() || head == "HEAD" {
            return Ok(None);
        }
        Ok(Some(head.to_string()))
    }

    /// A remote's configured fetch refspecs, grouped by what `--prune` would be
    /// allowed to delete through them.
    fn remote_fetch_refspecs(&self, remote_name: &str) -> Result<RemoteFetchRefspecs> {
        let repo = self.reopen_repo()?;
        let remote = repo.find_remote(remote_name).map_err(|e| {
            Error::new(ErrorKind::Backend(format!(
                "gix find_remote {remote_name}: {e}"
            )))
        })?;
        let mut refspecs = RemoteFetchRefspecs::default();

        for refspec in remote.refspecs(gix::remote::Direction::Fetch) {
            let refspec = refspec.to_ref();
            let serialized = refspec.to_bstring();
            if serialized.starts_with(b"^") {
                refspecs
                    .exclusions
                    .push(serialized.to_str_lossy().into_owned());
                continue;
            }
            match refspec.destination() {
                Some(destination) if destination.starts_with(b"refs/remotes/") => refspecs
                    .tracking
                    .push(serialized.to_str_lossy().into_owned()),
                // A refspec without a destination only writes FETCH_HEAD, so it
                // maps nothing `--prune` could delete.
                Some(_) => refspecs.has_non_tracking_destination = true,
                None => {}
            }
        }

        Ok(refspecs)
    }

    /// Return only configured fetch refspecs whose destinations are in the
    /// remote-tracking namespace, plus the negative refspecs that constrain
    /// them. Supplying these refspecs explicitly keeps `--prune` from applying
    /// to configured destinations such as `refs/tags/*` or `refs/notes/*`.
    fn remote_tracking_fetch_refspecs(&self, remote_name: &str) -> Result<Vec<String>> {
        Ok(self
            .remote_fetch_refspecs(remote_name)?
            .into_prune_command_refspecs())
    }

    /// Whether `git fetch --prune <remote>` can only delete remote-tracking
    /// refs. A remote that also fetches into `refs/tags/*` (or, when mirroring,
    /// `refs/*`) would have its local tags and branches pruned along with them.
    fn remote_prunes_only_tracking_refs(&self, remote_name: &str) -> Result<bool> {
        Ok(!self
            .remote_fetch_refspecs(remote_name)?
            .has_non_tracking_destination)
    }

    /// Whether the remote's tracking refspecs map every branch it publishes, so
    /// a completed prune fetch says as much about one branch as an explicit
    /// per-branch fetch would.
    fn remote_tracking_refspecs_cover_all_branches(&self, remote_name: &str) -> Result<bool> {
        let refspecs = self.remote_fetch_refspecs(remote_name)?;
        Ok(refspecs.exclusions.is_empty()
            && !refspecs.has_non_tracking_destination
            && refspecs.tracking.len() == 1
            && refspecs.tracking[0] == format!("+refs/heads/*:refs/remotes/{remote_name}/*"))
    }

    /// Remotes `git fetch --all` contacts, in Git's own order. A remote it
    /// skips says nothing about whether that remote's branches still exist.
    fn fetch_all_remote_names(&self) -> Result<Vec<String>> {
        let repo = self.reopen_repo()?;
        let config = repo.config_snapshot();
        let mut names = Vec::new();
        for name in repo.remote_names() {
            let name = name.to_str_lossy().into_owned();
            let skipped = ["skipFetchAll", "skipDefaultUpdate"]
                .into_iter()
                .any(|key| remote_config_boolean(&config, &name, key).unwrap_or(false));
            if !skipped {
                names.push(name);
            }
        }
        Ok(names)
    }

    /// Configured branch upstreams. `tracking_ref` is populated only when the
    /// remote's positive and negative fetch refspecs map this upstream locally.
    fn configured_remote_upstreams(
        &self,
        scope: UpstreamCleanupScope<'_>,
    ) -> Result<Vec<ConfiguredRemoteUpstream>> {
        let repo = self.reopen_repo()?;
        let refs = repo
            .references()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix references: {e}"))))?;
        let iter = refs
            .local_branches()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix local_branches: {e}"))))?;
        let mut upstreams = Vec::new();

        for reference in iter {
            let reference = reference
                .map_err(|e| Error::new(ErrorKind::Backend(format!("gix ref iter: {e}"))))?;
            let Some(upstream) = configured_remote_upstream_of(&reference) else {
                continue;
            };
            if !scope.includes(&upstream) {
                continue;
            }
            upstreams.push(upstream);
        }

        upstreams.sort_unstable_by(|left, right| left.local_branch.cmp(&right.local_branch));
        Ok(upstreams)
    }

    fn configured_upstreams_with_tracking_presence(
        &self,
        upstreams: Vec<ConfiguredRemoteUpstream>,
        expected_to_exist: bool,
    ) -> Result<Vec<ConfiguredRemoteUpstream>> {
        let repo = self.reopen_repo()?;
        let mut matching = Vec::new();
        for mut upstream in upstreams {
            // Without a matching fetch refspec, a fetch was not authoritative
            // for this configured upstream. It is neither present nor missing
            // for automatic cleanup purposes.
            let Some(tracking_ref) = upstream.tracking_ref.as_deref() else {
                continue;
            };
            let tracking_exists = repo
                .try_find_reference(tracking_ref)
                .map_err(|e| {
                    Error::new(ErrorKind::Backend(format!(
                        "gix try_find upstream reference: {e}"
                    )))
                })?
                .is_some();
            if tracking_exists && upstream.pending {
                self.clear_pending_upstream_if_matches(
                    &upstream.local_branch,
                    &Upstream {
                        remote: upstream.remote.clone(),
                        branch: upstream.remote_branch.clone(),
                    },
                );
                upstream.pending = false;
            }
            if !expected_to_exist && upstream.pending {
                continue;
            }
            if tracking_exists == expected_to_exist {
                matching.push(upstream);
            }
        }
        Ok(matching)
    }

    fn best_effort_unlink_upstreams_pruned_during_failed_fetch(
        &self,
        tracked_before_fetch: Vec<ConfiguredRemoteUpstream>,
    ) {
        if tracked_before_fetch.is_empty() {
            return;
        }
        if let Ok(disappeared) =
            self.configured_upstreams_with_tracking_presence(tracked_before_fetch, false)
        {
            self.unlink_remote_upstreams(disappeared);
        }
    }

    /// Configured branch upstreams whose authoritative remote-tracking
    /// destination no longer exists locally.
    fn missing_configured_remote_upstreams(
        &self,
        scope: UpstreamCleanupScope<'_>,
    ) -> Result<Vec<ConfiguredRemoteUpstream>> {
        let upstreams = self.configured_remote_upstreams(scope)?;
        self.configured_upstreams_with_tracking_presence(upstreams, false)
    }

    fn unlink_remote_upstreams(
        &self,
        upstreams: Vec<ConfiguredRemoteUpstream>,
    ) -> UnlinkedUpstreams {
        let mut result = UnlinkedUpstreams {
            branches: Vec::with_capacity(upstreams.len()),
            failures: Vec::new(),
        };

        for upstream in upstreams {
            if let Err(error) = validate_ref_like_arg(&upstream.local_branch, "branch name") {
                result
                    .failures
                    .push(format!("{}: {error}", upstream.local_branch));
                continue;
            }
            let label = format!("git branch --unset-upstream {}", upstream.local_branch);
            let mut cmd = self.git_workdir_cmd();
            cmd.arg("branch")
                .arg("--unset-upstream")
                .arg("--")
                .arg(&upstream.local_branch);
            match run_git_simple(cmd, &label) {
                Ok(()) => result.branches.push(upstream.local_branch),
                Err(error) => result
                    .failures
                    .push(format!("{}: {error}", upstream.local_branch)),
            }
        }

        result
    }

    fn unlink_missing_remote_upstreams(
        &self,
        scope: UpstreamCleanupScope<'_>,
    ) -> UnlinkedUpstreams {
        match self.missing_configured_remote_upstreams(scope) {
            Ok(missing) => self.unlink_remote_upstreams(missing),
            Err(error) => {
                UnlinkedUpstreams::failed(format!("could not read configured upstreams: {error}"))
            }
        }
    }

    /// Unlink an exact, explicitly deleted remote destination. Unlike fetch
    /// pruning, a successful delete (or an `ls-remote` confirmation) remains
    /// authoritative even when a narrow fetch refspec excludes the branch.
    fn unlink_configured_remote_upstreams(
        &self,
        scope: UpstreamCleanupScope<'_>,
    ) -> UnlinkedUpstreams {
        match self.configured_remote_upstreams(scope) {
            Ok(configured) => self.unlink_remote_upstreams(configured),
            Err(error) => {
                UnlinkedUpstreams::failed(format!("could not read configured upstreams: {error}"))
            }
        }
    }

    /// Return branches confirmed absent from the endpoint represented by the
    /// remote-tracking refs, removing those refs as a side effect. A successful
    /// push is sufficient proof only when fetch and push resolve to the same
    /// endpoint; otherwise query the fetch endpoint before unlinking anything.
    fn prune_tracking_refs_after_successful_push_delete(
        &self,
        remote: &str,
        branches: &[String],
    ) -> Vec<String> {
        if self.remote_fetch_and_push_urls_match(remote) {
            for branch in branches {
                self.best_effort_delete_reference(&format!("refs/remotes/{remote}/{branch}"));
            }
            return branches.to_vec();
        }

        self.prune_missing_remote_tracking_refs(remote, branches)
    }

    /// The upstream `branch.<name>.remote`/`merge` configures, whether or not
    /// its remote-tracking ref still exists locally. Callers deciding what to
    /// merge need the configured intent: treating a branch whose upstream was
    /// deleted as unconfigured would silently retarget it at another branch.
    fn configured_branch_upstream(&self, branch_name: &str) -> Result<Option<Upstream>> {
        validate_ref_like_arg(branch_name, "branch name")?;

        let repo = self.reopen_repo()?;
        let ref_name = format!("refs/heads/{branch_name}");
        let Some(reference) = repo
            .try_find_reference(ref_name.as_str())
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix try_find_reference: {e}"))))?
        else {
            return Ok(None);
        };

        Ok(
            configured_remote_upstream_of(&reference).map(|upstream| Upstream {
                remote: upstream.remote,
                branch: upstream.remote_branch,
            }),
        )
    }

    /// The configured upstream, but only while its remote-tracking ref is still
    /// present and resolvable.
    fn branch_upstream(&self, branch_name: &str) -> Result<Option<Upstream>> {
        validate_ref_like_arg(branch_name, "branch name")?;

        let repo = self.reopen_repo()?;
        let ref_name = format!("refs/heads/{branch_name}");
        let Some(reference) = repo
            .try_find_reference(ref_name.as_str())
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix try_find_reference: {e}"))))?
        else {
            return Ok(None);
        };

        let Some(upstream) = configured_upstream_of(&reference) else {
            return Ok(None);
        };
        let tracking_refs = tracking_refs_for_remote_branch(&repo, &upstream)?;
        let [tracking_ref_name] = tracking_refs.as_slice() else {
            return Ok(None);
        };

        let Some(mut tracking_ref) = repo
            .try_find_reference(tracking_ref_name.as_str())
            .map_err(|e| {
                Error::new(ErrorKind::Backend(format!(
                    "gix try_find upstream reference: {e}"
                )))
            })?
        else {
            return Ok(None);
        };
        if tracking_ref.peel_to_id().is_err() {
            return Ok(None);
        }

        Ok(Some(upstream))
    }

    /// Clear the future-upstream marker after a successful push to that exact
    /// destination. Best effort is deliberate: the network operation already
    /// succeeded, so a transient config-lock failure must not report the push
    /// itself as failed.
    fn clear_pending_upstream_if_matches(&self, local_branch: &str, upstream: &Upstream) {
        let Ok(repo) = self.reopen_repo() else {
            return;
        };
        let _ = edit_local_config_strict(&repo, |config| {
            let Ok(mut section) =
                config.section_mut("branch", Some(gix::bstr::BStr::new(local_branch)))
            else {
                return Ok(false);
            };
            let remote_matches = section
                .value("remote")
                .is_some_and(|value| value.as_bstr() == upstream.remote.as_bytes().as_bstr());
            let merge_ref = format!("refs/heads/{}", upstream.branch);
            let branch_matches = section
                .value("merge")
                .is_some_and(|value| value.as_bstr() == merge_ref.as_bytes().as_bstr());
            if !remote_matches || !branch_matches {
                return Ok(false);
            }
            Ok(section.remove(PENDING_UPSTREAM_CONFIG_KEY).is_some())
        });
    }

    pub(super) fn list_remotes_impl(&self) -> Result<Vec<Remote>> {
        let repo = self.reopen_repo()?;
        let mut remotes = Vec::new();

        for name in repo.remote_names() {
            let remote = repo.find_remote(&name).map_err(|e| {
                Error::new(ErrorKind::Backend(format!(
                    "gix find_remote {}: {e}",
                    name.to_str_lossy()
                )))
            })?;

            let url = remote
                .url(gix::remote::Direction::Fetch)
                .map(|url| {
                    normalize_remote_url(&bytes_to_text_preserving_utf8(url.to_bstring().as_ref()))
                })
                .filter(|url| !url.is_empty());

            remotes.push(Remote {
                name: name.to_str_lossy().into_owned(),
                url,
            });
        }

        remotes.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(remotes)
    }

    pub(super) fn list_remote_branches_impl(&self) -> Result<Vec<RemoteBranch>> {
        self.list_remote_branches_cancellable_impl(&CancellationToken::new())
    }

    pub(super) fn list_remote_branches_cancellable_impl(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Vec<RemoteBranch>> {
        cancellation.check_cancelled()?;
        // Fetch and prune run through the Git CLI, outside gix. Reopen the
        // repository so this refresh cannot reuse a packed-refs snapshot from
        // before the command and resurrect already-pruned tracking branches in
        // the UI.
        let repo = self.reopen_repo()?;
        let refs = repo
            .references()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix references: {e}"))))?;
        let iter = refs
            .remote_branches()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix remote_branches: {e}"))))?;
        let remotes = repo
            .remote_names()
            .into_iter()
            .map(|name| {
                repo.find_remote(&name).map_err(|e| {
                    Error::new(ErrorKind::Backend(format!(
                        "gix find_remote {}: {e}",
                        name.to_str_lossy()
                    )))
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let mut tracking_refs = Vec::new();
        for reference in iter {
            cancellation.check_cancelled()?;
            let mut reference = reference
                .map_err(|e| Error::new(ErrorKind::Backend(format!("gix ref iter: {e}"))))?;

            let target = match reference.try_id() {
                Some(id) => id.detach(),
                None => reference
                    .peel_to_id()
                    .map_err(|e| Error::new(ErrorKind::Backend(format!("gix peel branch: {e}"))))?
                    .detach(),
            };
            // Existence only: decoding every tip would inflate one object per
            // remote-tracking ref, and this list is rebuilt after every fetch.
            if !repo.has_object(target) {
                continue;
            }
            tracking_refs.push((reference.name().to_owned(), target));
        }

        // Reverse-map the complete tracking-ref set through each remote once.
        // A local ref can legitimately produce more than one row when remote
        // namespaces overlap; the remote and branch boundary remains explicit
        // in every result.
        let mut branches = Vec::new();
        for remote in &remotes {
            cancellation.check_cancelled()?;
            let Some(remote_name) = remote.name() else {
                continue;
            };
            let matches = gix::refspec::MatchGroup::from_fetch_specs(
                remote
                    .refspecs(gix::remote::Direction::Fetch)
                    .iter()
                    .map(|spec| spec.to_ref()),
            )
            .match_rhs(tracking_refs.iter().map(|(name, target)| {
                gix::refspec::match_group::Item {
                    full_ref_name: name.as_bstr(),
                    target: target.as_ref(),
                    object: None,
                }
            }));

            for mapping in matches.mappings {
                let Some(item_index) = mapping.item_index else {
                    continue;
                };
                let gix::refspec::match_group::SourceRef::FullName(source) = mapping.lhs else {
                    continue;
                };
                let Some(name) = source.as_ref().strip_prefix(b"refs/heads/") else {
                    continue;
                };
                let name = name.to_str_lossy();
                if name.is_empty() || name == "HEAD" {
                    continue;
                }
                let Some((_, target)) = tracking_refs.get(item_index) else {
                    continue;
                };
                branches.push(RemoteBranch {
                    remote: remote_name.as_bstr().to_str_lossy().into_owned(),
                    name: name.into_owned(),
                    target: CommitId(target.to_string().into()),
                });
            }
        }

        branches.sort_by(|a, b| a.remote.cmp(&b.remote).then_with(|| a.name.cmp(&b.name)));
        branches.dedup_by(|a, b| a.remote == b.remote && a.name == b.name);
        cancellation.check_cancelled()?;
        Ok(branches)
    }

    fn fetch_all_command_impl(&self, prune: bool, capture_output: bool) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("fetch").arg("--all");
        if prune {
            cmd.arg("--prune");
        } else {
            cmd.arg("--no-prune");
        }
        cmd.arg("--no-prune-tags");
        run_git_command_with_optional_output(
            cmd,
            if prune {
                "git fetch --all --prune --no-prune-tags"
            } else {
                "git fetch --all --no-prune --no-prune-tags"
            },
            capture_output,
        )
    }

    fn fetch_all_command_with_optional_output_impl(
        &self,
        remotes: &[String],
        prune: bool,
        capture_output: bool,
    ) -> Result<CommandOutput> {
        if !prune {
            return self.fetch_all_command_impl(false, capture_output);
        }

        // `--prune` deletes every destination the remote's own refspecs map, so
        // a remote that also fetches into refs/tags/* (or refs/* when it
        // mirrors) would lose local tags and branches along with stale
        // remote-tracking refs. A remote whose refspecs cannot be read takes
        // the same scoped path rather than risk the broad prune.
        let needs_scoped_prune = remotes.iter().any(|remote| {
            !self
                .remote_prunes_only_tracking_refs(remote)
                .unwrap_or(false)
        });
        if !needs_scoped_prune {
            return self.fetch_all_command_impl(true, capture_output);
        }

        let mut outputs = vec![self.fetch_all_command_impl(false, capture_output)?];
        for remote in remotes {
            outputs.push(
                self.prune_remote_tracking_refs_command_with_optional_output_impl(
                    remote,
                    capture_output,
                )?,
            );
        }
        Ok(combine_command_outputs(
            "git fetch --all --no-prune --no-prune-tags && git fetch --prune per remote",
            &outputs,
        ))
    }

    fn fetch_all_with_optional_output_impl(
        &self,
        prune: bool,
        capture_output: bool,
    ) -> Result<CommandOutput> {
        // Only the remotes this fetch contacts are authoritative about their
        // upstreams. A remote it skips can have a configured upstream with no
        // remote-tracking ref for reasons this fetch says nothing about.
        let remotes = self.fetch_all_remote_names()?;
        let tracked_before_fetch = if prune {
            let configured =
                self.configured_remote_upstreams(UpstreamCleanupScope::Remotes(&remotes))?;
            self.configured_upstreams_with_tracking_presence(configured, true)?
        } else {
            Vec::new()
        };
        let output =
            match self.fetch_all_command_with_optional_output_impl(&remotes, prune, capture_output)
            {
                Ok(output) => output,
                Err(error) => {
                    self.best_effort_unlink_upstreams_pruned_during_failed_fetch(
                        tracked_before_fetch,
                    );
                    return Err(error);
                }
            };
        if !prune {
            return Ok(output);
        }
        let unlinked =
            self.unlink_missing_remote_upstreams(UpstreamCleanupScope::Remotes(&remotes));
        Ok(append_unlinked_upstreams(output, &unlinked))
    }

    fn prune_remote_tracking_refs_command_with_optional_output_impl(
        &self,
        remote: &str,
        capture_output: bool,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(remote, "remote name")?;
        let label = format!("git fetch {remote} --prune --no-prune-tags");
        let refspecs = self.remote_tracking_fetch_refspecs(remote)?;
        if refspecs.is_empty() {
            // With no configured destination under refs/remotes, this remote
            // cannot authoritatively prune a remote-tracking ref.
            return Ok(CommandOutput::empty_success(label));
        }

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("fetch")
            .arg("--prune")
            .arg("--no-prune-tags")
            .arg("--")
            .arg(remote);
        for refspec in refspecs {
            cmd.arg(refspec);
        }
        run_git_command_with_optional_output(cmd, &label, capture_output)
    }

    fn prune_remote_tracking_refs_with_optional_output_impl(
        &self,
        remote: &str,
        capture_output: bool,
    ) -> Result<CommandOutput> {
        let configured = self.configured_remote_upstreams(UpstreamCleanupScope::Remote(remote))?;
        let tracked_before_fetch =
            self.configured_upstreams_with_tracking_presence(configured, true)?;
        let output = match self
            .prune_remote_tracking_refs_command_with_optional_output_impl(remote, capture_output)
        {
            Ok(output) => output,
            Err(error) => {
                self.best_effort_unlink_upstreams_pruned_during_failed_fetch(tracked_before_fetch);
                return Err(error);
            }
        };
        let unlinked = self.unlink_missing_remote_upstreams(UpstreamCleanupScope::Remote(remote));
        Ok(append_unlinked_upstreams(output, &unlinked))
    }

    pub(super) fn fetch_all_impl(&self, prune: bool) -> Result<()> {
        self.fetch_all_with_optional_output_impl(prune, false)
            .map(|_| ())
    }

    pub(super) fn fetch_all_with_output_impl(&self, prune: bool) -> Result<CommandOutput> {
        self.fetch_all_with_optional_output_impl(prune, true)
    }

    /// Apply the pull mode's integration flags. Being explicit about
    /// fast-forward behavior keeps a user's `pull.ff` config from turning a
    /// fast-forwardable pull into a merge commit.
    fn append_pull_mode_args(cmd: &mut Command, mode: PullMode) {
        match mode {
            PullMode::Default | PullMode::FastForwardIfPossible => {
                cmd.arg("--ff");
            }
            PullMode::Merge => {
                cmd.arg("--no-rebase");
                cmd.arg("--ff");
            }
            PullMode::FastForwardOnly => {
                cmd.arg("--ff-only");
            }
            PullMode::Rebase => {
                cmd.arg("--rebase");
            }
        }
    }

    /// `git pull` with tag pruning switched off. Git prunes tags whenever
    /// `fetch.pruneTags` or `remote.<name>.pruneTags` is set, which this pull
    /// must never do; `git pull` has no `--no-prune-tags` of its own.
    fn pull_cmd(&self, remote: Option<&str>, mode: PullMode) -> Command {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("-c").arg("fetch.pruneTags=false");
        if let Some(remote) = remote.filter(|remote| remote_name_is_config_key_safe(remote)) {
            cmd.arg("-c")
                .arg(format!("remote.{remote}.pruneTags=false"));
        }
        cmd.arg("pull");
        Self::append_pull_mode_args(&mut cmd, mode);
        cmd
    }

    fn pull_with_optional_output_impl(
        &self,
        mode: PullMode,
        prune: bool,
        capture_output: bool,
    ) -> Result<CommandOutput> {
        let branch = self.current_branch_name()?;
        // Read the configuration rather than the tracking ref: a branch whose
        // upstream was deleted on the remote still has one configured, and
        // treating it as unconfigured would pull from - and then set the
        // upstream to - a different remote branch.
        let upstream = match branch.as_deref() {
            Some(branch) => self.configured_branch_upstream(branch)?,
            None => None,
        };

        match (branch, upstream) {
            (Some(branch), None) => {
                self.pull_setting_upstream(&branch, mode, prune, capture_output)
            }
            (branch, upstream) => self.pull_configured_upstream(
                branch.as_deref(),
                upstream.as_ref(),
                mode,
                prune,
                capture_output,
            ),
        }
    }

    /// Pull a branch that already has an upstream configured, or a detached
    /// HEAD, where Git's own diagnostics apply.
    fn pull_configured_upstream(
        &self,
        branch: Option<&str>,
        upstream: Option<&Upstream>,
        mode: PullMode,
        prune: bool,
        capture_output: bool,
    ) -> Result<CommandOutput> {
        let remote = upstream.map(|upstream| upstream.remote.as_str());
        let prune_remote = prune.then_some(remote).flatten();
        // Pruning inside the pull costs one network round trip instead of two,
        // but `--prune` deletes every destination the remote's refspecs map, so
        // it is only safe while they all land under refs/remotes/ and
        // `pull_cmd` can address this remote's tag pruning by name. Otherwise
        // prune through a separate, refspec-scoped fetch.
        let mut prune_in_pull = None;
        let mut scoped_prune_remote = None;
        if let Some(remote) = prune_remote {
            if remote_name_is_config_key_safe(remote)
                && self.remote_prunes_only_tracking_refs(remote)?
            {
                prune_in_pull = Some(remote);
            } else {
                scoped_prune_remote = Some(remote);
            }
        }

        let mut outputs = Vec::new();
        if let Some(remote) = scoped_prune_remote {
            outputs.push(
                self.prune_remote_tracking_refs_with_optional_output_impl(remote, capture_output)?,
            );
            // That fetch is authoritative: if it removed the upstream this
            // branch tracks, `git pull` would fail with Git's raw "no tracking
            // information" hint after the configuration was already unlinked.
            if let (Some(branch), Some(upstream)) = (branch, upstream)
                && self.branch_upstream(branch)?.is_none()
            {
                return Err(remote_branch_gone_after_fetch_error(
                    &upstream.remote,
                    &upstream.branch,
                ));
            }
        }

        let tracked_before_pull = match prune_in_pull {
            Some(remote) => {
                let configured =
                    self.configured_remote_upstreams(UpstreamCleanupScope::Remote(remote))?;
                self.configured_upstreams_with_tracking_presence(configured, true)?
            }
            None => Vec::new(),
        };
        let current_tracking_ref_before_pull = match (branch, upstream) {
            (Some(branch), Some(upstream)) => tracked_before_pull
                .iter()
                .find(|tracked| {
                    tracked.local_branch == branch
                        && tracked.remote == upstream.remote
                        && tracked.remote_branch == upstream.branch
                })
                .and_then(|tracked| tracked.tracking_ref.clone()),
            _ => None,
        };

        let mut cmd = self.pull_cmd(remote, mode);
        let label = if prune_in_pull.is_some() {
            cmd.arg("--prune");
            "git pull --prune".to_string()
        } else {
            // Integrate with pruning disabled after the dedicated fetch above. A
            // configured non-tracking destination (notably refs/tags/*) is still
            // fetched normally, but can never be deleted by this pull.
            cmd.arg("--no-prune");
            "git pull --no-prune".to_string()
        };

        let output = match run_git_command_with_optional_output(cmd, &label, capture_output) {
            Ok(output) => output,
            Err(error) => {
                // `git pull --prune` deletes the tracking ref before it reports
                // that it has nothing to merge. Only refs observed before this
                // pull can be attributed to that failed fetch phase; a missing
                // pre-state may instead accompany an auth or transport error.
                let current_upstream_disappeared = current_tracking_ref_before_pull
                    .as_deref()
                    .is_some_and(|tracking_ref| {
                        matches!(self.reference_exists(tracking_ref), Ok(false))
                    });
                self.best_effort_unlink_upstreams_pruned_during_failed_fetch(tracked_before_pull);
                if let Some(upstream) = upstream
                    && current_upstream_disappeared
                {
                    return Err(remote_branch_gone_after_fetch_error(
                        &upstream.remote,
                        &upstream.branch,
                    ));
                }
                return Err(error);
            }
        };

        let output = match prune_in_pull {
            Some(remote) => {
                let unlinked =
                    self.unlink_missing_remote_upstreams(UpstreamCleanupScope::Remote(remote));
                append_unlinked_upstreams(output, &unlinked)
            }
            None => output,
        };

        match scoped_prune_remote {
            None => Ok(output),
            Some(remote) => {
                outputs.push(output);
                Ok(combine_command_outputs(
                    format!("git fetch {remote} --prune --no-prune-tags && {label}"),
                    &outputs,
                ))
            }
        }
    }

    /// Pull a branch with no configured upstream: integrate from the preferred
    /// remote and record it as the upstream afterwards.
    fn pull_setting_upstream(
        &self,
        branch: &str,
        mode: PullMode,
        prune: bool,
        capture_output: bool,
    ) -> Result<CommandOutput> {
        let Some(remote) = self.preferred_remote_name()? else {
            // No remote to name; let Git report why it cannot pull.
            let mut cmd = self.pull_cmd(None, mode);
            cmd.arg("--no-prune");
            return run_git_command_with_optional_output(
                cmd,
                "git pull --no-prune",
                capture_output,
            );
        };
        validate_ref_like_arg(&remote, "remote name")?;
        validate_ref_like_arg(branch, "branch name")?;

        let mut outputs = Vec::new();
        if prune {
            outputs.push(
                self.prune_remote_tracking_refs_with_optional_output_impl(&remote, capture_output)?,
            );
        }

        let mut cmd = self.pull_cmd(Some(&remote), mode);
        cmd.arg("--no-prune").arg("--").arg(&remote).arg(branch);
        let pull_label = format!("git pull --no-prune {remote} {branch}");
        let output = run_git_command_with_optional_output(cmd, &pull_label, capture_output)?;

        let mut set_upstream = self.git_workdir_cmd();
        set_upstream
            .arg("branch")
            .arg("--set-upstream-to")
            .arg(format!("{remote}/{branch}"))
            .arg("--")
            .arg(branch);
        run_git_simple(set_upstream, "git branch --set-upstream-to")?;

        if outputs.is_empty() {
            return Ok(output);
        }
        outputs.push(output);
        Ok(combine_command_outputs(
            format!("git fetch {remote} --prune --no-prune-tags && {pull_label}"),
            &outputs,
        ))
    }

    pub(super) fn pull_impl(&self, mode: PullMode) -> Result<()> {
        self.pull_with_optional_output_impl(mode, true, false)
            .map(|_| ())
    }

    pub(super) fn pull_with_output_impl(&self, mode: PullMode) -> Result<CommandOutput> {
        self.pull_with_optional_output_impl(mode, true, true)
    }

    pub(super) fn pull_with_output_prune_impl(
        &self,
        mode: PullMode,
        prune: bool,
    ) -> Result<CommandOutput> {
        self.pull_with_optional_output_impl(mode, prune, true)
    }

    fn push_set_upstream_with_optional_output_impl(
        &self,
        remote: &str,
        branch: &str,
        capture_output: bool,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(remote, "remote name")?;
        validate_ref_like_arg(branch, "branch name")?;

        let command_label = format!("git push --set-upstream {remote} HEAD:refs/heads/{branch}");
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("push")
            .arg("--set-upstream")
            .arg("--")
            .arg(remote)
            .arg(format!("HEAD:refs/heads/{branch}"));
        let output = run_git_command_with_optional_output(cmd, &command_label, capture_output)?;
        if let Ok(Some(local_branch)) = self.current_branch_name() {
            self.clear_pending_upstream_if_matches(
                &local_branch,
                &Upstream {
                    remote: remote.to_string(),
                    branch: branch.to_string(),
                },
            );
        }
        Ok(output)
    }

    fn push_head_to_branch_with_optional_output_impl(
        &self,
        remote: &str,
        branch: &str,
        force_with_lease: bool,
        capture_output: bool,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(remote, "remote name")?;
        validate_ref_like_arg(branch, "branch name")?;

        let command_label = if force_with_lease {
            format!("git push --force-with-lease {remote} HEAD:refs/heads/{branch}")
        } else {
            format!("git push {remote} HEAD:refs/heads/{branch}")
        };

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("push");
        if force_with_lease {
            cmd.arg("--force-with-lease");
        }
        cmd.arg("--")
            .arg(remote)
            .arg(format!("HEAD:refs/heads/{branch}"));
        run_git_command_with_optional_output(cmd, &command_label, capture_output)
    }

    fn push_head_to_branch_with_oid_lease_with_output_impl(
        &self,
        lease: &ForcePushLease,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(&lease.remote, "remote name")?;
        validate_ref_like_arg(&lease.branch, "branch name")?;
        validate_hex_commit_id(&lease.expected)?;
        validate_ref_like_arg(&lease.local_branch, "local branch name")?;
        validate_hex_commit_id(&lease.local_head)?;

        let current_branch = self.current_branch_name()?.ok_or_else(|| {
            Error::new(ErrorKind::Backend(format!(
                "stale force-push lease: expected branch {}, but HEAD is detached",
                lease.local_branch
            )))
        })?;
        if current_branch != lease.local_branch {
            return Err(Error::new(ErrorKind::Backend(format!(
                "stale force-push lease: expected branch {}, but current branch is {}",
                lease.local_branch, current_branch
            ))));
        }

        let current_head = self.head_commit_id_impl()?.ok_or_else(|| {
            Error::new(ErrorKind::Backend(
                "stale force-push lease: current HEAD does not point to a commit".to_string(),
            ))
        })?;
        if current_head != lease.local_head {
            return Err(Error::new(ErrorKind::Backend(format!(
                "stale force-push lease: expected HEAD {}, but current HEAD is {}",
                lease.local_head, current_head
            ))));
        }

        let lease_ref = format!("refs/heads/{}", lease.branch);
        let lease_arg = format!("--force-with-lease={}:{}", lease_ref, lease.expected);
        let source_ref = format!("{}:{lease_ref}", lease.local_head);
        let command_label = format!("git push {lease_arg} {} {source_ref}", lease.remote);

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("push")
            .arg(&lease_arg)
            .arg("--")
            .arg(&lease.remote)
            .arg(source_ref);
        run_git_with_output(cmd, &command_label)
    }

    pub(super) fn head_commit_id_impl(&self) -> Result<Option<CommitId>> {
        let repo = self.reopen_repo()?;
        gix_head_id_or_none(&repo).map(|id| id.map(|id| CommitId(id.to_string().into())))
    }

    fn validate_push_after_commit_target(&self, target: &SafePushAfterCommitTarget) -> Result<()> {
        validate_ref_like_arg(&target.remote, "remote name")?;
        validate_ref_like_arg(&target.branch, "branch name")?;
        validate_ref_like_arg(&target.local_branch, "local branch name")?;
        validate_hex_commit_id(&target.local_head)?;

        let current_branch = self.current_branch_name()?.ok_or_else(|| {
            Error::new(ErrorKind::Backend(format!(
                "stale push-after-commit target: expected branch {}, but HEAD is detached",
                target.local_branch
            )))
        })?;
        if current_branch != target.local_branch {
            return Err(Error::new(ErrorKind::Backend(format!(
                "stale push-after-commit target: expected branch {}, but current branch is {}",
                target.local_branch, current_branch
            ))));
        }

        let current_head = self.head_commit_id_impl()?.ok_or_else(|| {
            Error::new(ErrorKind::Backend(
                "stale push-after-commit target: current HEAD does not point to a commit"
                    .to_string(),
            ))
        })?;
        if current_head != target.local_head {
            return Err(Error::new(ErrorKind::Backend(format!(
                "stale push-after-commit target: expected HEAD {}, but current HEAD is {}",
                target.local_head, current_head
            ))));
        }

        Ok(())
    }

    fn push_after_commit_target_with_optional_output_impl(
        &self,
        target: &SafePushAfterCommitTarget,
        set_upstream: bool,
        capture_output: bool,
    ) -> Result<CommandOutput> {
        self.validate_push_after_commit_target(target)?;

        let source = if set_upstream {
            format!("refs/heads/{}", target.local_branch)
        } else {
            target.local_head.to_string()
        };
        let refspec = format!("{source}:refs/heads/{}", target.branch);
        let command_label = if set_upstream {
            format!(
                "git push --set-upstream {} {}:refs/heads/{}",
                target.remote, source, target.branch
            )
        } else {
            format!(
                "git push {} {}:refs/heads/{}",
                target.remote, source, target.branch
            )
        };

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("push");
        if set_upstream {
            cmd.arg("--set-upstream");
        }
        cmd.arg("--").arg(&target.remote).arg(refspec);
        let output = run_git_command_with_optional_output(cmd, &command_label, capture_output)?;
        self.clear_pending_upstream_if_matches(
            &target.local_branch,
            &Upstream {
                remote: target.remote.clone(),
                branch: target.branch.clone(),
            },
        );
        Ok(output)
    }

    pub(super) fn push_after_commit_with_output_impl(
        &self,
        target: &SafePushAfterCommitTarget,
    ) -> Result<CommandOutput> {
        self.push_after_commit_target_with_optional_output_impl(target, false, true)
    }

    pub(super) fn push_after_commit_set_upstream_with_output_impl(
        &self,
        target: &SafePushAfterCommitTarget,
    ) -> Result<CommandOutput> {
        self.push_after_commit_target_with_optional_output_impl(target, true, true)
    }

    fn fetch_remote_branch_tip_with_output(
        &self,
        remote: &str,
        branch: &str,
    ) -> Result<Option<(CommitId, CommandOutput)>> {
        validate_ref_like_arg(remote, "remote name")?;
        validate_ref_like_arg(branch, "branch name")?;

        let remote_ref = format!("refs/heads/{branch}");
        let label = format!("git fetch --no-prune --refmap= {remote} {remote_ref}");
        let mut cmd = self.git_workdir_cmd();
        cmd.env("LC_ALL", "C")
            .arg("fetch")
            .arg("--no-prune")
            .arg("--no-prune-tags")
            .arg("--no-tags")
            .arg("--refmap=")
            .arg("--")
            .arg(remote)
            .arg(&remote_ref);
        let output = run_git_raw_output(cmd, &label)?;
        if !output.status.success() {
            if output_mentions_missing_remote_ref(&output) {
                return Ok(None);
            }
            return Err(git_command_failed_error(&label, output));
        }
        let fetch_output = CommandOutput {
            command: label,
            stdout: bytes_to_text_preserving_utf8(&output.stdout),
            stderr: bytes_to_text_preserving_utf8(&output.stderr),
            exit_code: output.status.code(),
        };

        let label = "git rev-parse --verify FETCH_HEAD^{commit}";
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("rev-parse")
            .arg("--verify")
            .arg("FETCH_HEAD^{commit}");
        let output = run_git_raw_output(cmd, label)?;
        if !output.status.success() {
            return Err(git_command_failed_error(label, output));
        }

        let tip = bytes_to_text_preserving_utf8(&output.stdout)
            .trim()
            .to_string();
        let tip = CommitId(tip.into());
        validate_hex_commit_id(&tip)?;
        Ok(Some((tip, fetch_output)))
    }

    fn fetch_remote_branch_tip_for_safe_push(
        &self,
        remote: &str,
        branch: &str,
    ) -> Result<Option<CommitId>> {
        Ok(self
            .fetch_remote_branch_tip_with_output(remote, branch)?
            .map(|(tip, _)| tip))
    }

    fn commit_is_ancestor(&self, ancestor: &CommitId, descendant: &CommitId) -> Result<bool> {
        validate_hex_commit_id(ancestor)?;
        validate_hex_commit_id(descendant)?;

        let label = format!("git merge-base --is-ancestor {ancestor} {descendant}");
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("merge-base")
            .arg("--is-ancestor")
            .arg(ancestor.as_ref())
            .arg(descendant.as_ref());
        let output = run_git_raw_output(cmd, &label)?;
        if output.status.success() {
            return Ok(true);
        }
        if output.status.code() == Some(1) {
            return Ok(false);
        }
        Err(git_command_failed_error(&label, output))
    }

    fn safe_push_decision_for_target(
        &self,
        context: &SafePushAfterCommitContext,
        local_branch: &str,
        remote: String,
        branch: String,
        has_upstream: bool,
    ) -> Result<SafePushAfterCommitDecision> {
        let display_ref = safe_push_ref_display(&remote, &branch);
        let Some(post_head) = context.post_head.as_ref() else {
            return Ok(SafePushAfterCommitDecision::Blocked {
                summary: "No commit was created to push.".to_string(),
                lease: None,
            });
        };
        let target = SafePushAfterCommitTarget {
            remote,
            branch,
            local_branch: local_branch.to_string(),
            local_head: post_head.clone(),
        };

        let Some(remote_tip) =
            self.fetch_remote_branch_tip_for_safe_push(&target.remote, &target.branch)?
        else {
            return if has_upstream {
                Ok(SafePushAfterCommitDecision::Blocked {
                    summary: format!(
                        "The configured upstream branch {display_ref} was not found on the remote."
                    ),
                    lease: None,
                })
            } else {
                Ok(SafePushAfterCommitDecision::PushSetUpstream { target })
            };
        };

        if self.commit_is_ancestor(&remote_tip, post_head)? {
            return if has_upstream {
                Ok(SafePushAfterCommitDecision::Push { target })
            } else {
                Ok(SafePushAfterCommitDecision::PushSetUpstream { target })
            };
        }

        if context.amend && context.pre_head.as_ref() == Some(&remote_tip) {
            return Ok(SafePushAfterCommitDecision::Blocked {
                summary: format!(
                    "The amended commit appears to be published at {display_ref}. Use Force push with lease to update it without overwriting newer remote work."
                ),
                lease: Some(ForcePushLease {
                    remote: target.remote,
                    branch: target.branch,
                    expected: remote_tip,
                    local_branch: local_branch.to_string(),
                    local_head: post_head.clone(),
                }),
            });
        }

        Ok(SafePushAfterCommitDecision::Blocked {
            summary: format!(
                "Remote branch {display_ref} changed while committing. Pull or rebase manually, then push again."
            ),
            lease: None,
        })
    }

    fn validate_safe_push_after_commit_context(
        &self,
        local_branch: &str,
        post_head: &CommitId,
    ) -> Result<Option<SafePushAfterCommitDecision>> {
        validate_ref_like_arg(local_branch, "local branch name")?;
        validate_hex_commit_id(post_head)?;

        let Some(current_branch) = self.current_branch_name()? else {
            return Ok(Some(SafePushAfterCommitDecision::Blocked {
                summary: format!(
                    "Current branch changed from {local_branch} to detached HEAD after committing. Check out {local_branch} and push manually."
                ),
                lease: None,
            }));
        };
        if current_branch != local_branch {
            return Ok(Some(SafePushAfterCommitDecision::Blocked {
                summary: format!(
                    "Current branch changed from {local_branch} to {current_branch} after committing. Check out {local_branch} and push manually."
                ),
                lease: None,
            }));
        }

        let Some(current_head) = self.head_commit_id_impl()? else {
            return Ok(Some(SafePushAfterCommitDecision::Blocked {
                summary: format!(
                    "Current HEAD no longer points to the commit created on {local_branch}. Push manually."
                ),
                lease: None,
            }));
        };
        if &current_head != post_head {
            return Ok(Some(SafePushAfterCommitDecision::Blocked {
                summary: format!(
                    "Current HEAD changed after committing on {local_branch}. Expected {post_head}, but current HEAD is {current_head}. Push manually."
                ),
                lease: None,
            }));
        }

        Ok(None)
    }

    pub(super) fn safe_push_after_commit_impl(
        &self,
        context: &SafePushAfterCommitContext,
    ) -> Result<SafePushAfterCommitDecision> {
        let Some(post_head) = context.post_head.as_ref() else {
            return Ok(SafePushAfterCommitDecision::Blocked {
                summary: "No commit was created to push.".to_string(),
                lease: None,
            });
        };
        let Some(local_branch) = context.local_branch.as_deref() else {
            return Ok(SafePushAfterCommitDecision::Blocked {
                summary: "Push after commit needs a checked-out branch.".to_string(),
                lease: None,
            });
        };

        if let Some(decision) =
            self.validate_safe_push_after_commit_context(local_branch, post_head)?
        {
            return Ok(decision);
        }

        if let Some(upstream) = self.configured_branch_upstream(local_branch)? {
            let has_live_upstream = self.branch_upstream(local_branch)?.is_some();
            return self.safe_push_decision_for_target(
                context,
                local_branch,
                upstream.remote,
                upstream.branch,
                has_live_upstream,
            );
        }

        let Some(remote) = self.preferred_remote_name()? else {
            return Ok(SafePushAfterCommitDecision::Blocked {
                summary: "No git remote is configured for push after commit.".to_string(),
                lease: None,
            });
        };

        self.safe_push_decision_for_target(
            context,
            local_branch,
            remote,
            local_branch.to_string(),
            false,
        )
    }

    fn push_with_optional_output_impl(&self, capture_output: bool) -> Result<CommandOutput> {
        if let Some(branch) = self.current_branch_name()? {
            if let Some(upstream) = self.configured_branch_upstream(&branch)? {
                let output = self.push_head_to_branch_with_optional_output_impl(
                    &upstream.remote,
                    &upstream.branch,
                    false,
                    capture_output,
                )?;
                self.clear_pending_upstream_if_matches(&branch, &upstream);
                return Ok(output);
            }

            if let Some(remote) = self.preferred_remote_name()? {
                return self.push_set_upstream_with_optional_output_impl(
                    &remote,
                    &branch,
                    capture_output,
                );
            }
        }

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("push");
        run_git_command_with_optional_output(cmd, "git push", capture_output)
    }

    pub(super) fn push_impl(&self) -> Result<()> {
        self.push_with_optional_output_impl(false).map(|_| ())
    }

    pub(super) fn push_with_output_impl(&self) -> Result<CommandOutput> {
        self.push_with_optional_output_impl(true)
    }

    fn push_force_with_optional_output_impl(&self, capture_output: bool) -> Result<CommandOutput> {
        if let Some(branch) = self.current_branch_name()?
            && let Some(upstream) = self.branch_upstream(&branch)?
        {
            let output = self.push_head_to_branch_with_optional_output_impl(
                &upstream.remote,
                &upstream.branch,
                true,
                capture_output,
            )?;
            self.clear_pending_upstream_if_matches(&branch, &upstream);
            return Ok(output);
        }

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("push").arg("--force-with-lease");
        run_git_command_with_optional_output(cmd, "git push --force-with-lease", capture_output)
    }

    pub(super) fn push_force_impl(&self) -> Result<()> {
        self.push_force_with_optional_output_impl(false).map(|_| ())
    }

    pub(super) fn push_force_with_output_impl(&self) -> Result<CommandOutput> {
        self.push_force_with_optional_output_impl(true)
    }

    pub(super) fn push_force_with_lease_with_output_impl(
        &self,
        lease: &ForcePushLease,
    ) -> Result<CommandOutput> {
        self.push_head_to_branch_with_oid_lease_with_output_impl(lease)
    }

    pub(super) fn pull_branch_with_output_impl(
        &self,
        remote: &str,
        branch: &str,
    ) -> Result<CommandOutput> {
        self.pull_branch_with_output_prune_impl(remote, branch, false)
    }

    pub(super) fn pull_branch_with_output_prune_impl(
        &self,
        remote: &str,
        branch: &str,
        prune: bool,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(remote, "remote name")?;
        validate_ref_like_arg(branch, "branch name")?;

        // `.` is Git's special local-repository source, used by the branch
        // menu to pull another local branch into the current one. It is not a
        // configured remote, so remote pruning and refspec lookup do not apply.
        if prune && remote != "." {
            let prune_output =
                self.prune_remote_tracking_refs_with_optional_output_impl(remote, true)?;

            // With a refspec that maps every branch on the remote, the prune
            // fetch above already updated - or removed - this branch's tracking
            // ref, so a second fetch would only repeat it.
            if self.remote_tracking_refspecs_cover_all_branches(remote)? {
                let tracking_ref = format!("refs/remotes/{remote}/{branch}");
                if !self.reference_exists(&tracking_ref)? {
                    return Err(remote_branch_gone_after_fetch_error(remote, branch));
                }
                let merge_output = self.merge_ref_with_output_impl(&tracking_ref)?;
                return Ok(combine_command_outputs(
                    format!("git fetch {remote} --prune && git merge {tracking_ref}"),
                    &[prune_output, merge_output],
                ));
            }

            let Some((tip, fetch_output)) =
                self.fetch_remote_branch_tip_with_output(remote, branch)?
            else {
                // The explicit fetch is authoritative even when configured
                // refspecs exclude this branch. Remove a stale row and unlink
                // any branch configured to track the now-confirmed deletion.
                self.best_effort_delete_reference(&format!("refs/remotes/{remote}/{branch}"));
                let deleted = [branch.to_string()];
                self.unlink_configured_remote_upstreams(UpstreamCleanupScope::RemoteBranches {
                    remote,
                    branches: &deleted,
                });
                return Err(remote_branch_gone_after_fetch_error(remote, branch));
            };
            let merge_output = self.merge_ref_with_output_impl(tip.as_ref())?;
            return Ok(combine_command_outputs(
                format!(
                    "git fetch {remote} --prune && git fetch {remote} refs/heads/{branch} && git merge {tip}"
                ),
                &[prune_output, fetch_output, merge_output],
            ));
        }

        let command_str = format!("git pull --no-rebase --ff --no-prune {remote} {branch}");
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("-c")
            .arg("color.ui=false")
            .arg("-c")
            .arg("fetch.pruneTags=false");
        if remote_name_is_config_key_safe(remote) {
            cmd.arg("-c")
                .arg(format!("remote.{remote}.pruneTags=false"));
        }
        cmd.arg("--no-pager")
            .arg("pull")
            .arg("--no-rebase")
            .arg("--ff")
            .arg("--no-prune")
            .arg("--")
            .arg(remote)
            .arg(branch);
        run_git_with_output(cmd, &command_str)
    }

    pub(super) fn merge_ref_with_output_impl(&self, reference: &str) -> Result<CommandOutput> {
        validate_ref_like_arg(reference, "reference")?;

        let command_str = format!("git merge --ff --no-edit {reference}");
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("-c")
            .arg("color.ui=false")
            .arg("--no-pager")
            .arg("merge")
            .arg("--ff")
            .arg("--no-edit")
            .arg("--")
            .arg(reference);
        run_git_with_output(cmd, &command_str)
    }

    pub(super) fn squash_ref_with_output_impl(&self, reference: &str) -> Result<CommandOutput> {
        validate_ref_like_arg(reference, "reference")?;

        let command_str = format!("git merge --squash --no-commit {reference}");
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("-c")
            .arg("color.ui=false")
            .arg("--no-pager")
            .arg("merge")
            .arg("--squash")
            .arg("--no-commit")
            .arg("--")
            .arg(reference);
        run_git_with_output(cmd, &command_str)
    }

    pub(super) fn add_remote_with_output_impl(
        &self,
        name: &str,
        url: &str,
        remote_url_policy: RemoteUrlPolicy,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(name, "remote name")?;
        validate_remote_url_with_policy(url, remote_url_policy)?;

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("remote").arg("add").arg("--").arg(name).arg(url);
        run_git_with_output(cmd, &remote_add_label(name, url))
    }

    pub(super) fn remove_remote_with_output_impl(&self, name: &str) -> Result<CommandOutput> {
        validate_ref_like_arg(name, "remote name")?;

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("remote").arg("remove").arg("--").arg(name);
        run_git_with_output(cmd, &format!("git remote remove {name}"))
    }

    pub(super) fn set_remote_url_with_output_impl(
        &self,
        name: &str,
        url: &str,
        kind: RemoteUrlKind,
        remote_url_policy: RemoteUrlPolicy,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(name, "remote name")?;
        validate_remote_url_with_policy(url, remote_url_policy)?;

        let mut cmd = self.git_workdir_cmd();
        cmd.arg("remote").arg("set-url");
        match kind {
            RemoteUrlKind::Fetch => {}
            RemoteUrlKind::Push => {
                cmd.arg("--push");
            }
        }
        cmd.arg("--").arg(name).arg(url);
        run_git_with_output(cmd, &remote_set_url_label(name, url, kind))
    }

    pub(super) fn push_set_upstream_impl(&self, remote: &str, branch: &str) -> Result<()> {
        self.push_set_upstream_with_optional_output_impl(remote, branch, false)
            .map(|_| ())
    }

    pub(super) fn push_set_upstream_with_output_impl(
        &self,
        remote: &str,
        branch: &str,
    ) -> Result<CommandOutput> {
        self.push_set_upstream_with_optional_output_impl(remote, branch, true)
    }

    pub(super) fn set_upstream_branch_with_output_impl(
        &self,
        branch: &str,
        upstream: &Upstream,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(branch, "branch name")?;
        validate_ref_like_arg(&upstream.remote, "remote name")?;
        validate_ref_like_arg(&upstream.branch, "branch name")?;

        let repo = self.reopen_repo()?;
        let local_ref = format!("refs/heads/{branch}");
        if repo
            .try_find_reference(local_ref.as_str())
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix try_find_reference: {e}"))))?
            .is_none()
        {
            return Err(Error::new(ErrorKind::Backend(format!(
                "local branch {branch} does not exist"
            ))));
        }

        let tracking_refs = tracking_refs_for_remote_branch(&repo, upstream)?;
        let [tracking_ref] = tracking_refs.as_slice() else {
            let detail = if tracking_refs.is_empty() {
                "is not mapped by that remote's fetch refspecs"
            } else {
                "maps to multiple local tracking refs"
            };
            return Err(Error::new(ErrorKind::Backend(format!(
                "remote branch {}/{} {detail}",
                upstream.remote, upstream.branch
            ))));
        };
        let tracking_ref_exists = match repo
            .try_find_reference(tracking_ref.as_str())
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix try_find_reference: {e}"))))?
        {
            Some(mut reference) => reference.peel_to_id().map(|_| true).map_err(|e| {
                Error::new(ErrorKind::Backend(format!(
                    "resolve remote branch {}/{}: {e}",
                    upstream.remote, upstream.branch
                )))
            })?,
            None => false,
        };
        // The mapping, rather than a local tracking ref's existence, is the
        // authority here. Git permits branch.<name>.remote/merge to name a
        // branch that will be created by a later push. Existing-target picker
        // actions validate liveness against state before reaching this method;
        // the explicit "Create new" flow intentionally has no tracking ref yet.
        let merge_ref = format!("refs/heads/{}", upstream.branch);
        let label = format!(
            "set upstream {branch} -> {} / {}",
            upstream.remote, upstream.branch
        );
        edit_local_config_strict(&repo, |config| {
            let mut section = config
                .section_mut_or_create_new("branch", Some(gix::bstr::BStr::new(branch)))
                .map_err(|e| {
                    Error::new(ErrorKind::Backend(format!(
                        "create local config section for branch {branch}: {e}"
                    )))
                })?;
            section
                .set("remote", upstream.remote.as_str())
                .map_err(|e| {
                    Error::new(ErrorKind::Backend(format!(
                        "set local config upstream remote for branch {branch}: {e}"
                    )))
                })?;
            section.set("merge", merge_ref.as_str()).map_err(|e| {
                Error::new(ErrorKind::Backend(format!(
                    "set local config upstream merge ref for branch {branch}: {e}"
                )))
            })?;
            while section.remove(PENDING_UPSTREAM_CONFIG_KEY).is_some() {}
            if !tracking_ref_exists {
                section
                    .set(PENDING_UPSTREAM_CONFIG_KEY, "true")
                    .map_err(|e| {
                        Error::new(ErrorKind::Backend(format!(
                            "mark new upstream for branch {branch}: {e}"
                        )))
                    })?;
            }
            Ok(true)
        })?;

        Ok(CommandOutput::empty_success(label))
    }

    pub(super) fn unset_upstream_branch_with_output_impl(
        &self,
        branch: &str,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(branch, "branch name")?;

        let label = format!("git branch --unset-upstream {branch}");
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("branch")
            .arg("--unset-upstream")
            .arg("--")
            .arg(branch);
        let output = run_git_with_output(cmd, &label)?;
        if let Ok(repo) = self.reopen_repo() {
            let _ = edit_local_config_strict(&repo, |config| {
                let Ok(mut section) =
                    config.section_mut("branch", Some(gix::bstr::BStr::new(branch)))
                else {
                    return Ok(false);
                };
                Ok(section.remove(PENDING_UPSTREAM_CONFIG_KEY).is_some())
            });
        }
        Ok(output)
    }

    pub(super) fn delete_remote_branch_with_output_impl(
        &self,
        remote: &str,
        branch: &str,
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(remote, "remote name")?;
        validate_ref_like_arg(branch, "branch name")?;

        let label = format!("git push --delete {remote} {branch}");
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("push")
            .arg("--delete")
            .arg("--")
            .arg(remote)
            .arg(branch);
        let output = run_git_with_output(cmd, &label)?;

        let deleted = [branch.to_string()];
        let deleted_from_fetch =
            self.prune_tracking_refs_after_successful_push_delete(remote, &deleted);
        let unlinked =
            self.unlink_configured_remote_upstreams(UpstreamCleanupScope::RemoteBranches {
                remote,
                branches: &deleted_from_fetch,
            });

        Ok(append_unlinked_upstreams(output, &unlinked))
    }

    /// Delete several branches on `remote` with one `git push --delete`.
    ///
    /// `git push --delete` accepts any number of refspecs, so the whole batch
    /// costs a single network round trip instead of one per branch.
    pub(super) fn delete_remote_branches_with_output_impl(
        &self,
        remote: &str,
        branches: &[String],
    ) -> Result<CommandOutput> {
        validate_ref_like_arg(remote, "remote name")?;
        for branch in branches {
            validate_ref_like_arg(branch, "branch name")?;
        }
        if branches.is_empty() {
            return Ok(CommandOutput::empty_success("git push --delete"));
        }

        let label = format!("git push --delete {remote} {}", branches.join(" "));
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("push").arg("--delete").arg("--").arg(remote);
        for branch in branches {
            cmd.arg(branch);
        }

        match run_git_with_output(cmd, &label) {
            Ok(output) => {
                let deleted_from_fetch =
                    self.prune_tracking_refs_after_successful_push_delete(remote, branches);
                let unlinked =
                    self.unlink_configured_remote_upstreams(UpstreamCleanupScope::RemoteBranches {
                        remote,
                        branches: &deleted_from_fetch,
                    });
                Ok(append_unlinked_upstreams(output, &unlinked))
            }
            // How much a failed batch deleted depends on why it failed: a
            // refspec naming a ref the remote does not have is rejected before
            // anything is pushed, while a per-ref hook rejection lets the other
            // deletes through. The output does not say which, so ask the remote
            // instead of guessing — guessing either way is wrong, and pruning
            // blindly would erase rows for branches that still exist.
            Err(batch_error) => {
                let missing = self.prune_missing_remote_tracking_refs(remote, branches);
                self.unlink_configured_remote_upstreams(UpstreamCleanupScope::RemoteBranches {
                    remote,
                    branches: &missing,
                });
                Err(batch_error)
            }
        }
    }

    /// Drop the local remote-tracking ref for each of `branches` that no longer
    /// exists on `remote`, and refresh refs that still exist there.
    ///
    /// The refresh matters when `remote.<name>.pushurl` differs: Git itself
    /// removes a remote-tracking ref after a successful push deletion, even
    /// though that ref represents the fetch endpoint where the branch may still
    /// be live. Best-effort throughout: a failure here must never turn a partial
    /// push success into an error.
    fn prune_missing_remote_tracking_refs(&self, remote: &str, branches: &[String]) -> Vec<String> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("ls-remote").arg("--heads").arg("--").arg(remote);
        for branch in branches {
            cmd.arg(format!("refs/heads/{branch}"));
        }
        let Ok(output) = run_git_with_output(cmd, "git ls-remote --heads") else {
            return Vec::new();
        };

        let mut missing = Vec::new();
        let mut present = Vec::new();
        for branch in branches {
            let refname = format!("refs/heads/{branch}");
            let still_on_remote = output.stdout.lines().any(|line| {
                line.split_once('\t')
                    .is_some_and(|(_, name)| name.trim() == refname)
            });
            if !still_on_remote {
                self.best_effort_delete_reference(&format!("refs/remotes/{remote}/{branch}"));
                missing.push(branch.clone());
            } else {
                present.push(branch);
            }
        }

        if !present.is_empty() {
            let mut cmd = self.git_workdir_cmd();
            cmd.arg("fetch")
                .arg("--no-prune")
                .arg("--no-prune-tags")
                .arg("--no-tags")
                .arg("--")
                .arg(remote);
            for branch in present {
                cmd.arg(format!(
                    "+refs/heads/{branch}:refs/remotes/{remote}/{branch}"
                ));
            }
            let _ = run_git_with_output(cmd, "git fetch live remote-tracking branches");
        }
        missing
    }

    pub(super) fn prune_merged_branches_with_output_impl(&self) -> Result<CommandOutput> {
        // Keep configured upstreams intact until merged local branches have
        // been selected: that command intentionally uses a missing upstream as
        // one of its deletion criteria. Surviving branches are unlinked below.
        let remotes = self.fetch_all_remote_names()?;
        let fetch_output =
            self.fetch_all_command_with_optional_output_impl(&remotes, true, true)?;

        let mut merged_cmd = self.git_workdir_cmd();
        merged_cmd
            .arg("for-each-ref")
            .arg("--format=%(refname:short)")
            .arg("--merged=HEAD")
            .arg("refs/heads");
        let merged_output =
            run_git_capture(merged_cmd, "git for-each-ref --merged=HEAD refs/heads")?;
        let merged = parse_refname_set(&merged_output);

        let mut branches_cmd = self.git_workdir_cmd();
        branches_cmd
            .arg("for-each-ref")
            .arg("--format=%(refname:short)\t%(upstream:short)")
            .arg("refs/heads");
        let branches_output = run_git_capture(
            branches_cmd,
            "git for-each-ref --format=%(refname:short)\\t%(upstream:short) refs/heads",
        )?;

        let mut refs_cmd = self.git_workdir_cmd();
        refs_cmd
            .arg("for-each-ref")
            .arg("--format=%(refname)")
            .arg("refs/remotes");
        let tracking_refs_output = run_git_capture(
            refs_cmd,
            "git for-each-ref --format=%(refname) refs/remotes",
        )?;
        let existing_tracking_refs = parse_refname_set(&tracking_refs_output);

        let current_branch = self.current_branch_name()?;
        let prune_candidates = branches_to_prune(
            &branches_output,
            &merged,
            &existing_tracking_refs,
            current_branch.as_deref(),
        );
        let mut deleted: Vec<String> = Vec::new();
        let mut deleted_outputs: Vec<CommandOutput> = Vec::new();

        for branch in prune_candidates {
            let mut delete_cmd = self.git_workdir_cmd();
            delete_cmd.arg("branch").arg("-d").arg("--").arg(&branch);
            let output = run_git_with_output(delete_cmd, &format!("git branch -d {branch}"))?;
            deleted.push(branch);
            deleted_outputs.push(output);
        }

        let mut stdout = String::new();
        let mut stderr = String::new();
        if !fetch_output.stdout.is_empty() {
            stdout.push_str(&fetch_output.stdout);
        }
        if !fetch_output.stderr.is_empty() {
            stderr.push_str(&fetch_output.stderr);
        }
        for output in &deleted_outputs {
            if !output.stdout.is_empty() {
                stdout.push_str(&output.stdout);
            }
            if !output.stderr.is_empty() {
                stderr.push_str(&output.stderr);
            }
        }
        if deleted.is_empty() {
            if !stdout.ends_with('\n') && !stdout.is_empty() {
                stdout.push('\n');
            }
            stdout.push_str("No merged local branches to prune.\n");
        } else {
            if !stdout.ends_with('\n') && !stdout.is_empty() {
                stdout.push('\n');
            }
            stdout.push_str("Pruned merged local branches:\n");
            for branch in deleted {
                stdout.push_str("- ");
                stdout.push_str(&branch);
                stdout.push('\n');
            }
        }

        let output = CommandOutput {
            command: "git prune merged branches".to_string(),
            stdout,
            stderr,
            exit_code: Some(0),
        };
        let unlinked =
            self.unlink_missing_remote_upstreams(UpstreamCleanupScope::Remotes(&remotes));
        Ok(append_unlinked_upstreams(output, &unlinked))
    }
}

#[cfg(test)]
mod tests {
    use super::{branches_to_prune, normalize_remote_url, parse_refname_set, run_git_command};
    use gitcomet_core::services::CommandOutput;
    use rustc_hash::FxHashSet;
    use std::{cell::Cell, process::Command};

    #[test]
    fn parse_refname_set_trims_and_deduplicates_lines() {
        let output =
            "refs/remotes/origin/main\n\n refs/remotes/origin/main \nrefs/remotes/upstream/dev\n";
        let refs = parse_refname_set(output);
        assert_eq!(refs.len(), 2);
        assert!(refs.contains("refs/remotes/origin/main"));
        assert!(refs.contains("refs/remotes/upstream/dev"));
    }

    #[test]
    fn branches_to_prune_filters_by_merge_state_tracking_and_current_branch() {
        let branches_output = "\
feature/stale\torigin/feature/stale\n\
feature/tracked\torigin/feature/tracked\n\
feature/unmerged\torigin/feature/unmerged\n\
feature/current\torigin/feature/current\n\
feature/no-upstream\t\n";
        let merged: FxHashSet<String> = ["feature/stale", "feature/tracked", "feature/current"]
            .into_iter()
            .map(ToOwned::to_owned)
            .collect();
        let tracking_refs: FxHashSet<String> = ["refs/remotes/origin/feature/tracked".to_string()]
            .into_iter()
            .collect();

        let prune = branches_to_prune(
            branches_output,
            &merged,
            &tracking_refs,
            Some("feature/current"),
        );
        assert_eq!(prune, vec!["feature/stale".to_string()]);
    }

    #[test]
    fn normalize_remote_url_preserves_non_drive_letter_urls() {
        assert_eq!(
            normalize_remote_url("https://example.com/repo.git"),
            "https://example.com/repo.git"
        );
        assert_eq!(
            normalize_remote_url("file:///tmp/repo.git"),
            "file:///tmp/repo.git"
        );
        assert_eq!(
            normalize_remote_url("file://server/share/repo.git"),
            "file://server/share/repo.git"
        );
    }

    #[test]
    fn normalize_remote_url_fixes_windows_drive_letter_file_urls() {
        assert_eq!(
            normalize_remote_url("file://C:/Users/example/repo.git"),
            "file:///C:/Users/example/repo.git"
        );
        assert_eq!(
            normalize_remote_url(r"file://D:\Users\example\repo.git"),
            "file:///D:/Users/example/repo.git"
        );
    }

    #[test]
    fn run_git_command_discard_mode_uses_simple_runner_and_returns_empty_success() {
        let simple_called = Cell::new(false);
        let with_output_called = Cell::new(false);

        let output = run_git_command(
            Command::new("git"),
            "git push",
            false,
            |_, label| {
                simple_called.set(true);
                assert_eq!(label, "git push");
                Ok(())
            },
            |_, _| {
                with_output_called.set(true);
                Ok(CommandOutput::empty_success("unexpected"))
            },
        )
        .expect("discard mode should execute the simple runner");

        assert!(simple_called.get());
        assert!(!with_output_called.get());
        assert_eq!(output, CommandOutput::empty_success("git push"));
    }

    #[test]
    fn run_git_command_capture_mode_uses_output_runner() {
        let simple_called = Cell::new(false);
        let with_output_called = Cell::new(false);
        let expected = CommandOutput {
            command: "git push".to_string(),
            stdout: "stdout".to_string(),
            stderr: "stderr".to_string(),
            exit_code: Some(0),
        };

        let output = run_git_command(
            Command::new("git"),
            "git push",
            true,
            |_, _| {
                simple_called.set(true);
                Ok(())
            },
            |_, label| {
                with_output_called.set(true);
                assert_eq!(label, "git push");
                Ok(expected.clone())
            },
        )
        .expect("capture mode should execute the output runner");

        assert!(!simple_called.get());
        assert!(with_output_called.get());
        assert_eq!(output, expected);
    }
}

#[cfg(test)]
mod remote_label_tests {
    use super::{RemoteUrlKind, remote_add_label, remote_set_url_label};

    #[test]
    fn remote_labels_mask_credentials_in_urls() {
        assert_eq!(
            remote_add_label("origin", "https://user:s3cret@example.com/org/repo.git"),
            "git remote add origin https://user:***@example.com/org/repo.git"
        );
        assert_eq!(
            remote_set_url_label(
                "origin",
                "https://ghp_token@github.com/org/repo.git",
                RemoteUrlKind::Fetch
            ),
            "git remote set-url origin https://***@github.com/org/repo.git"
        );
        assert_eq!(
            remote_set_url_label("origin", "git@github.com:org/repo.git", RemoteUrlKind::Push),
            "git remote set-url --push origin git@github.com:org/repo.git"
        );
    }
}
