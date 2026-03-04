use super::diff_text::{DiffSyntaxMode, diff_syntax_language_for_path};
use super::*;
use crate::theme::AppTheme;
use crate::view::conflict_resolver::{
    self, ConflictBlock, ConflictChoice, ConflictSegment, ThreeWayVisibleItem,
};
use crate::view::history_graph;
use gitgpui_core::domain::{
    Branch, Commit, CommitDetails, CommitFileChange, CommitId, FileStatusKind, Remote,
    RemoteBranch, RepoSpec, Upstream, UpstreamDivergence,
};
use gitgpui_state::model::{Loadable, RepoId, RepoState};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

pub struct OpenRepoFixture {
    repo: RepoState,
    commits: Vec<Commit>,
    theme: AppTheme,
}

impl OpenRepoFixture {
    pub fn new(
        commits: usize,
        local_branches: usize,
        remote_branches: usize,
        remotes: usize,
    ) -> Self {
        let theme = AppTheme::zed_ayu_dark();
        let commits_vec = build_synthetic_commits(commits);
        let repo =
            build_synthetic_repo_state(local_branches, remote_branches, remotes, &commits_vec);
        Self {
            repo,
            commits: commits_vec,
            theme,
        }
    }

    pub fn run(&self) -> u64 {
        // Branch sidebar is the main "many branches" transformation.
        let rows = GitGpuiView::branch_sidebar_rows(&self.repo);

        // History graph is the main "long history" transformation.
        let branch_heads = HashSet::default();
        let graph = history_graph::compute_graph(&self.commits, self.theme, &branch_heads);

        let mut h = DefaultHasher::new();
        rows.len().hash(&mut h);
        graph.len().hash(&mut h);
        graph
            .iter()
            .take(128)
            .map(|r| (r.lanes_now.len(), r.lanes_next.len(), r.is_merge))
            .collect::<Vec<_>>()
            .hash(&mut h);
        h.finish()
    }
}

pub struct CommitDetailsFixture {
    details: CommitDetails,
}

impl CommitDetailsFixture {
    pub fn new(files: usize, depth: usize) -> Self {
        Self {
            details: build_synthetic_commit_details(files, depth),
        }
    }

    pub fn run(&self) -> u64 {
        // Approximation of the per-row work done by the commit files list:
        // kind->icon mapping and formatting the displayed path string.
        let mut h = DefaultHasher::new();
        self.details.id.as_ref().hash(&mut h);
        self.details.message.len().hash(&mut h);

        let mut counts = [0usize; 6];
        for f in &self.details.files {
            let icon: Option<&'static str> = match f.kind {
                FileStatusKind::Added => Some("+"),
                FileStatusKind::Modified => Some("✎"),
                FileStatusKind::Deleted => None,
                FileStatusKind::Renamed => Some("→"),
                FileStatusKind::Untracked => Some("?"),
                FileStatusKind::Conflicted => Some("!"),
            };
            icon.hash(&mut h);
            let kind_key: u8 = match f.kind {
                FileStatusKind::Added => 0,
                FileStatusKind::Modified => 1,
                FileStatusKind::Deleted => 2,
                FileStatusKind::Renamed => 3,
                FileStatusKind::Untracked => 4,
                FileStatusKind::Conflicted => 5,
            };
            kind_key.hash(&mut h);

            // This allocation is a real part of row construction today.
            let path_text = f.path.display().to_string();
            path_text.hash(&mut h);

            counts[kind_key as usize] = counts[kind_key as usize].saturating_add(1);
        }
        counts.hash(&mut h);
        h.finish()
    }
}

pub struct LargeFileDiffScrollFixture {
    lines: Vec<String>,
    language: Option<super::diff_text::DiffSyntaxLanguage>,
    theme: AppTheme,
}

impl LargeFileDiffScrollFixture {
    pub fn new(lines: usize) -> Self {
        let theme = AppTheme::zed_ayu_dark();
        let language = diff_syntax_language_for_path("src/lib.rs");
        Self {
            lines: build_synthetic_source_lines(lines),
            language,
            theme,
        }
    }

    pub fn run_scroll_step(&self, start: usize, window: usize) -> u64 {
        // Approximate "a scroll step": style the newly visible rows in a window.
        let end = (start + window).min(self.lines.len());
        let mut h = DefaultHasher::new();
        for line in &self.lines[start..end] {
            let styled = super::diff_text::build_cached_diff_styled_text(
                self.theme,
                line,
                &[],
                "",
                self.language,
                DiffSyntaxMode::Auto,
                None,
            );
            styled.text.len().hash(&mut h);
            styled.highlights.len().hash(&mut h);
        }
        h.finish()
    }
}

pub struct ConflictThreeWayScrollFixture {
    base_lines: Vec<SharedString>,
    ours_lines: Vec<SharedString>,
    theirs_lines: Vec<SharedString>,
    base_word_highlights: conflict_resolver::WordHighlights,
    ours_word_highlights: conflict_resolver::WordHighlights,
    theirs_word_highlights: conflict_resolver::WordHighlights,
    base_line_conflict_map: Vec<Option<usize>>,
    ours_line_conflict_map: Vec<Option<usize>>,
    theirs_line_conflict_map: Vec<Option<usize>>,
    visible_map: Vec<ThreeWayVisibleItem>,
    conflict_count: usize,
    language: Option<super::diff_text::DiffSyntaxLanguage>,
    syntax_mode: DiffSyntaxMode,
    theme: AppTheme,
}

impl ConflictThreeWayScrollFixture {
    pub fn new(lines: usize, conflict_blocks: usize) -> Self {
        let theme = AppTheme::zed_ayu_dark();
        let segments = build_synthetic_three_way_segments(lines, conflict_blocks);
        let (base_text, ours_text, theirs_text) = materialize_three_way_side_texts(&segments);
        let base_lines = split_lines_shared(&base_text);
        let ours_lines = split_lines_shared(&ours_text);
        let theirs_lines = split_lines_shared(&theirs_text);
        let three_way_len = base_lines
            .len()
            .max(ours_lines.len())
            .max(theirs_lines.len());
        let conflict_maps = conflict_resolver::build_three_way_conflict_maps(
            &segments,
            base_lines.len(),
            ours_lines.len(),
            theirs_lines.len(),
        );
        let visible_map = conflict_resolver::build_three_way_visible_map(
            three_way_len,
            &conflict_maps.conflict_ranges,
            &segments,
            false,
        );
        let (base_word_highlights, ours_word_highlights, theirs_word_highlights) =
            conflict_resolver::compute_three_way_word_highlights(
                &base_lines,
                &ours_lines,
                &theirs_lines,
                &segments,
            );
        let syntax_mode = if three_way_len > 4_000 {
            DiffSyntaxMode::HeuristicOnly
        } else {
            DiffSyntaxMode::Auto
        };

        Self {
            base_lines,
            ours_lines,
            theirs_lines,
            base_word_highlights,
            ours_word_highlights,
            theirs_word_highlights,
            base_line_conflict_map: conflict_maps.base_line_conflict_map,
            ours_line_conflict_map: conflict_maps.ours_line_conflict_map,
            theirs_line_conflict_map: conflict_maps.theirs_line_conflict_map,
            visible_map,
            conflict_count: conflict_maps.conflict_ranges.len(),
            language: diff_syntax_language_for_path("src/conflict.rs"),
            syntax_mode,
            theme,
        }
    }

    pub fn run_scroll_step(&self, start: usize, window: usize) -> u64 {
        if self.visible_map.is_empty() || window == 0 {
            return 0;
        }
        let start = start % self.visible_map.len();
        let end = (start + window).min(self.visible_map.len());

        let mut h = DefaultHasher::new();
        for visible_item in &self.visible_map[start..end] {
            let line_ix = match *visible_item {
                ThreeWayVisibleItem::Line(ix) => ix,
                ThreeWayVisibleItem::CollapsedBlock(conflict_ix) => {
                    conflict_ix.hash(&mut h);
                    continue;
                }
            };

            self.base_line_conflict_map
                .get(line_ix)
                .copied()
                .flatten()
                .hash(&mut h);
            self.ours_line_conflict_map
                .get(line_ix)
                .copied()
                .flatten()
                .hash(&mut h);
            self.theirs_line_conflict_map
                .get(line_ix)
                .copied()
                .flatten()
                .hash(&mut h);

            if let Some(line) = self.base_lines.get(line_ix) {
                let styled = super::diff_text::build_cached_diff_styled_text(
                    self.theme,
                    line.as_ref(),
                    word_ranges_for_line(&self.base_word_highlights, line_ix),
                    "",
                    self.language,
                    self.syntax_mode,
                    None,
                );
                styled.text_hash.hash(&mut h);
                styled.highlights_hash.hash(&mut h);
            }
            if let Some(line) = self.ours_lines.get(line_ix) {
                let styled = super::diff_text::build_cached_diff_styled_text(
                    self.theme,
                    line.as_ref(),
                    word_ranges_for_line(&self.ours_word_highlights, line_ix),
                    "",
                    self.language,
                    self.syntax_mode,
                    None,
                );
                styled.text_hash.hash(&mut h);
                styled.highlights_hash.hash(&mut h);
            }
            if let Some(line) = self.theirs_lines.get(line_ix) {
                let styled = super::diff_text::build_cached_diff_styled_text(
                    self.theme,
                    line.as_ref(),
                    word_ranges_for_line(&self.theirs_word_highlights, line_ix),
                    "",
                    self.language,
                    self.syntax_mode,
                    None,
                );
                styled.text_hash.hash(&mut h);
                styled.highlights_hash.hash(&mut h);
            }
        }

        h.finish()
    }

    pub fn visible_rows(&self) -> usize {
        self.visible_map.len()
    }

    pub fn conflict_count(&self) -> usize {
        self.conflict_count
    }
}

fn build_synthetic_repo_state(
    local_branches: usize,
    remote_branches: usize,
    remotes: usize,
    commits: &[Commit],
) -> RepoState {
    let id = RepoId(1);
    let spec = RepoSpec {
        workdir: std::path::PathBuf::from("/tmp/bench"),
    };
    let mut repo = RepoState::new_opening(id, spec);

    let head = "main".to_string();
    repo.head_branch = Loadable::Ready(head.clone());

    let target = commits
        .first()
        .map(|c| c.id.clone())
        .unwrap_or_else(|| CommitId("0".repeat(40)));

    let mut branches = Vec::with_capacity(local_branches.max(1));
    branches.push(Branch {
        name: head.clone(),
        target: target.clone(),
        upstream: Some(Upstream {
            remote: "origin".to_string(),
            branch: head.clone(),
        }),
        divergence: Some(UpstreamDivergence {
            ahead: 1,
            behind: 2,
        }),
    });
    for ix in 0..local_branches.saturating_sub(1) {
        branches.push(Branch {
            name: format!("feature/{}/topic/{ix}", ix % 100),
            target: target.clone(),
            upstream: None,
            divergence: None,
        });
    }
    repo.branches = Loadable::Ready(Arc::new(branches));

    let mut remotes_vec = Vec::with_capacity(remotes.max(1));
    for r in 0..remotes.max(1) {
        remotes_vec.push(Remote {
            name: if r == 0 {
                "origin".to_string()
            } else {
                format!("remote{r}")
            },
            url: None,
        });
    }
    repo.remotes = Loadable::Ready(Arc::new(remotes_vec.clone()));

    let mut remote = Vec::with_capacity(remote_branches);
    for ix in 0..remote_branches {
        let remote_name = if remotes <= 1 || ix % remotes == 0 {
            "origin".to_string()
        } else {
            format!("remote{}", ix % remotes)
        };
        remote.push(RemoteBranch {
            remote: remote_name,
            name: format!("feature/{}/topic/{ix}", ix % 100),
            target: target.clone(),
        });
    }
    repo.remote_branches = Loadable::Ready(Arc::new(remote));

    // Minimal "repo is open" status.
    repo.open = Loadable::Ready(());

    repo
}

fn build_synthetic_commits(count: usize) -> Vec<Commit> {
    if count == 0 {
        return Vec::new();
    }

    let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let mut commits = Vec::with_capacity(count);

    for ix in 0..count {
        let id = CommitId(format!("{:040x}", ix));

        let mut parent_ids = Vec::new();
        if ix > 0 {
            parent_ids.push(CommitId(format!("{:040x}", ix - 1)));
        }
        // Synthetic merge-like commits at a fixed cadence.
        if ix >= 40 && ix % 50 == 0 {
            parent_ids.push(CommitId(format!("{:040x}", ix - 40)));
        }

        commits.push(Commit {
            id,
            parent_ids,
            summary: format!("Commit {ix} - synthetic benchmark history entry"),
            author: format!("Author {}", ix % 10),
            time: base + Duration::from_secs(ix as u64),
        });
    }

    commits
}

fn build_synthetic_commit_details(files: usize, depth: usize) -> CommitDetails {
    let id = CommitId("d".repeat(40));
    let mut out = Vec::with_capacity(files);
    for ix in 0..files {
        let kind = match ix % 23 {
            0 => FileStatusKind::Deleted,
            1 | 2 => FileStatusKind::Renamed,
            3..=5 => FileStatusKind::Added,
            6 => FileStatusKind::Conflicted,
            7 => FileStatusKind::Untracked,
            _ => FileStatusKind::Modified,
        };

        let mut path = std::path::PathBuf::new();
        let depth = depth.max(1);
        for d in 0..depth {
            path.push(format!("dir{}_{}", d, ix % 128));
        }
        path.push(format!("file_{ix}.rs"));

        out.push(CommitFileChange { path, kind });
    }

    CommitDetails {
        id,
        message: "Synthetic benchmark commit details message\n\nWith body.".to_string(),
        committed_at: "2024-01-01T00:00:00Z".to_string(),
        parent_ids: vec![CommitId("c".repeat(40))],
        files: out,
    }
}

fn build_synthetic_source_lines(count: usize) -> Vec<String> {
    let mut lines = Vec::with_capacity(count);
    for ix in 0..count {
        let indent = " ".repeat((ix % 8) * 2);
        let line = match ix % 10 {
            0 => format!("{indent}fn func_{ix}(x: usize) -> usize {{ x + {ix} }}"),
            1 => format!("{indent}let value_{ix} = \"string {ix}\";"),
            2 => format!("{indent}// comment {ix} with some extra words and tokens"),
            3 => format!("{indent}if value_{ix} > 10 {{ return value_{ix}; }}"),
            4 => format!(
                "{indent}for i in 0..{r} {{ sum += i; }}",
                r = (ix % 100) + 1
            ),
            5 => format!("{indent}match tag_{ix} {{ Some(v) => v, None => 0 }}"),
            6 => format!("{indent}struct S{ix} {{ a: i32, b: String }}"),
            7 => format!(
                "{indent}impl S{ix} {{ fn new() -> Self {{ Self {{ a: 0, b: String::new() }} }} }}"
            ),
            8 => format!("{indent}const CONST_{ix}: u64 = {v};", v = ix as u64 * 31),
            _ => format!("{indent}println!(\"{ix} {{}}\", value_{ix});"),
        };
        lines.push(line);
    }
    lines
}

fn build_synthetic_three_way_segments(
    total_lines: usize,
    requested_conflict_blocks: usize,
) -> Vec<ConflictSegment> {
    let total_lines = total_lines.max(1);
    let conflict_blocks = requested_conflict_blocks.max(1).min(total_lines);
    let context_lines = total_lines.saturating_sub(conflict_blocks);
    let context_slots = conflict_blocks.saturating_add(1);
    let context_per_slot = context_lines / context_slots;
    let context_remainder = context_lines % context_slots;

    let mut segments: Vec<ConflictSegment> = Vec::with_capacity(conflict_blocks * 2 + 1);
    for slot_ix in 0..context_slots {
        let slot_lines = context_per_slot + usize::from(slot_ix < context_remainder);
        if slot_lines > 0 {
            let mut text = String::with_capacity(slot_lines * 64);
            for line_ix in 0..slot_lines {
                let seed = slot_ix * 1_000 + line_ix;
                let line = match seed % 5 {
                    0 => {
                        format!(
                            "fn ctx_{slot_ix}_{line_ix}(value: usize) -> usize {{ value + {seed} }}"
                        )
                    }
                    1 => format!("let ctx_{slot_ix}_{line_ix} = \"context line {seed}\";"),
                    2 => {
                        format!("if ctx_{slot_ix}_{line_ix}.len() > 3 {{ println!(\"{seed}\"); }}")
                    }
                    3 => format!("match opt_{slot_ix}_{line_ix} {{ Some(v) => v, None => 0 }}"),
                    _ => format!("// context {seed} repeated words for highlight coverage"),
                };
                text.push_str(&line);
                text.push('\n');
            }
            segments.push(ConflictSegment::Text(text));
        }

        if slot_ix < conflict_blocks {
            let choice = match slot_ix % 4 {
                0 => ConflictChoice::Base,
                1 => ConflictChoice::Ours,
                2 => ConflictChoice::Theirs,
                _ => ConflictChoice::Both,
            };
            segments.push(ConflictSegment::Block(ConflictBlock {
                base: Some(format!("let shared_{slot_ix} = compute_base({slot_ix});\n")),
                ours: format!("let shared_{slot_ix} = compute_local({slot_ix});\n"),
                theirs: format!("let shared_{slot_ix} = compute_remote({slot_ix});\n"),
                choice,
                resolved: slot_ix % 5 == 0,
            }));
        }
    }

    segments
}

fn materialize_three_way_side_texts(segments: &[ConflictSegment]) -> (String, String, String) {
    let mut base = String::new();
    let mut ours = String::new();
    let mut theirs = String::new();
    for segment in segments {
        match segment {
            ConflictSegment::Text(text) => {
                base.push_str(text);
                ours.push_str(text);
                theirs.push_str(text);
            }
            ConflictSegment::Block(block) => {
                base.push_str(block.base.as_deref().unwrap_or_default());
                ours.push_str(&block.ours);
                theirs.push_str(&block.theirs);
            }
        }
    }
    (base, ours, theirs)
}

fn split_lines_shared(text: &str) -> Vec<SharedString> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(text.as_bytes().iter().filter(|&&b| b == b'\n').count() + 1);
    out.extend(text.lines().map(|line| line.to_string().into()));
    out
}

fn word_ranges_for_line(
    highlights: &conflict_resolver::WordHighlights,
    line_ix: usize,
) -> &[Range<usize>] {
    highlights
        .get(line_ix)
        .and_then(|ranges| ranges.as_deref())
        .unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflict_three_way_fixture_tracks_requested_conflict_blocks() {
        let fixture = ConflictThreeWayScrollFixture::new(120, 12);
        assert_eq!(fixture.conflict_count(), 12);
        assert_eq!(fixture.visible_rows(), 120);
    }

    #[test]
    fn conflict_three_way_fixture_wraps_start_offsets() {
        let fixture = ConflictThreeWayScrollFixture::new(180, 18);
        let hash_a = fixture.run_scroll_step(17, 40);
        let hash_b = fixture.run_scroll_step(17 + fixture.visible_rows() * 3, 40);
        assert_eq!(hash_a, hash_b);
    }
}
