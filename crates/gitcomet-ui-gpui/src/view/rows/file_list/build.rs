use super::*;
use crate::view::rows::{CommitFileKindCounts, CommitFileSort};
use gitcomet_core::domain::FileStatusKind;

#[derive(Clone, Copy, Debug)]
pub(in crate::view) struct FileTreeItem<'a> {
    pub(in crate::view) path: &'a Path,
    pub(in crate::view) kind: Option<FileStatusKind>,
    pub(in crate::view) additions: Option<u32>,
    pub(in crate::view) deletions: Option<u32>,
}

impl<'a> FileTreeItem<'a> {
    /// A path alone, for tests that only exercise the grouping.
    #[cfg(test)]
    pub(in crate::view) fn new(path: &'a Path) -> Self {
        Self {
            path,
            kind: None,
            additions: None,
            deletions: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum TreeEntry {
    Dir(u32),
    File(u32),
}

#[derive(Debug)]
struct DirNode {
    segment: Arc<str>,
    path: Arc<Path>,
    /// Insertion order, which is first-appearance order in the projection —
    /// that is what makes tree order a refinement of flat order.
    entries: Vec<TreeEntry>,
    dirs: DirLookup,
}

impl DirNode {
    fn new(segment: Arc<str>, path: Arc<Path>) -> Self {
        Self {
            segment,
            path,
            entries: Vec::new(),
            dirs: DirLookup::default(),
        }
    }

    fn only_child_dir(&self) -> Option<u32> {
        match self.entries.as_slice() {
            [TreeEntry::Dir(child)] => Some(*child),
            _ => None,
        }
    }
}

/// A path trie over one list's projection, independent of what is collapsed.
#[derive(Debug)]
pub(in crate::view) struct FileTree {
    nodes: Vec<DirNode>,
    stats: Vec<Option<(u32, u32)>>,
    kinds: Vec<Option<FileStatusKind>>,
    sort: CommitFileSort,
}

impl FileTree {
    pub(in crate::view) fn build<'a>(
        items: impl Iterator<Item = FileTreeItem<'a>>,
        sort: CommitFileSort,
    ) -> Self {
        let root = DirNode::new(Arc::from(""), arc_path(PathBuf::new()));
        let mut tree = Self {
            nodes: vec![root],
            stats: Vec::new(),
            kinds: Vec::new(),
            sort,
        };

        for item in items {
            let ordinal = tree.stats.len() as u32;
            tree.kinds.push(item.kind);
            // Both sides or neither, the way the backend reports counts: half
            // a pair folded into a directory subtotal would state a zero
            // nobody measured.
            tree.stats.push(match (item.additions, item.deletions) {
                (Some(additions), Some(deletions)) => Some((additions, deletions)),
                _ => None,
            });

            let mut node_ix = 0usize;
            let mut components = item.path.components().peekable();
            let mut prefix = PathBuf::new();
            while let Some(component) = components.next() {
                // Directory identities must retain the bytes used by folder actions.
                let segment = component.as_os_str();
                prefix.push(segment);
                if components.peek().is_none() {
                    break;
                }
                node_ix = tree.child_dir(node_ix, segment, &prefix);
            }
            tree.nodes[node_ix].entries.push(TreeEntry::File(ordinal));
        }

        tree
    }

    fn child_dir(&mut self, parent: usize, segment: &std::ffi::OsStr, path: &Path) -> usize {
        if let Some(existing) = self.nodes[parent].dirs.get(segment) {
            return *existing;
        }
        let key = Arc::from(segment);
        let child = self.nodes.len();
        self.nodes.push(DirNode::new(
            Arc::from(segment.to_string_lossy().as_ref()),
            Arc::from(path),
        ));
        self.nodes[parent].dirs.insert(key, child);
        self.nodes[parent]
            .entries
            .push(TreeEntry::Dir(child as u32));
        child
    }

    pub(in crate::view) fn flatten(&self, collapsed: &CollapsedDirs) -> FileListPlan {
        let mut out = Flatten {
            rows: Vec::new(),
            ordered: Vec::with_capacity(self.stats.len()),
            row_ix_by_ordinal: vec![HIDDEN; self.stats.len()],
            collapsed_spans: Vec::new(),
        };
        self.emit(0, 0, false, collapsed, &mut out);
        // Inverted here, once, because `display_position` is a per-visible-row
        // lookup during render: scanning `ordered` there made one section's
        // frame O(visible rows x files).
        let mut display_ix_by_ordinal = vec![0usize; self.stats.len()];
        for (display_ix, ordinal) in out.ordered.iter().enumerate() {
            display_ix_by_ordinal[*ordinal] = display_ix;
        }
        FileListPlan::Tree {
            rows: out.rows.into(),
            ordered: out.ordered.into(),
            row_ix_by_ordinal: out.row_ix_by_ordinal.into(),
            display_ix_by_ordinal: display_ix_by_ordinal.into(),
            collapsed_spans: out.collapsed_spans.into(),
        }
    }

    /// Returns its subtree totals so a parent folds them in as the recursion
    /// unwinds. Rescanning `ordered[start..end]` per directory would be
    /// O(N x depth), and this runs on every chevron click.
    fn emit(
        &self,
        node_ix: usize,
        depth: usize,
        hidden: bool,
        collapsed: &CollapsedDirs,
        out: &mut Flatten,
    ) -> SubtreeTotals {
        let mut totals = SubtreeTotals::default();
        for entry in self.emit_order(node_ix) {
            match entry {
                TreeEntry::File(ordinal) => {
                    if !hidden {
                        out.row_ix_by_ordinal[ordinal as usize] = out.rows.len();
                        out.rows.push(FileListRow::File {
                            ordinal: FileOrdinal(ordinal as usize),
                            depth,
                        });
                    }
                    out.ordered.push(ordinal as usize);
                    totals.add_file(self.kinds[ordinal as usize], self.stats[ordinal as usize]);
                }
                TreeEntry::Dir(child) => {
                    let (deepest, chain, label) = self.fold(child as usize);
                    let chain: DirChain = chain.into();
                    let is_collapsed = chain
                        .iter()
                        .any(|segment| collapsed.set().contains(segment.as_ref() as &Path));
                    let row_ix = (!hidden).then(|| {
                        let row_ix = out.rows.len();
                        out.rows.push(FileListRow::Directory {
                            key: Arc::clone(&self.nodes[deepest].path),
                            label: label.into(),
                            depth,
                            collapsed: is_collapsed,
                            chain: Arc::clone(&chain),
                            subtree: 0..0,
                            counts: CommitFileKindCounts::default(),
                            additions: None,
                            deletions: None,
                        });
                        row_ix
                    });

                    let start = out.ordered.len();
                    let child =
                        self.emit(deepest, depth + 1, hidden || is_collapsed, collapsed, out);
                    let end = out.ordered.len();
                    totals.merge(&child);
                    // Recorded even inside a hidden subtree: revealing a file
                    // has to expand every collapsed folder above it at once,
                    // and the nested ones never got a row.
                    if is_collapsed {
                        out.collapsed_spans.push((start..end, chain));
                    }

                    if let Some(row_ix) = row_ix
                        && let FileListRow::Directory {
                            subtree,
                            counts,
                            additions,
                            deletions,
                            ..
                        } = &mut out.rows[row_ix]
                    {
                        *subtree = start..end;
                        *counts = child.counts;
                        if let Some((a, d)) = child.sums {
                            *additions = Some(a);
                            *deletions = Some(d);
                        }
                    }
                }
            }
        }
        totals
    }

    /// Directories before files under a path sort, mirrored for Z-A. Edit-size
    /// sorts interleave instead: grouping directories first would float a folder
    /// holding a three-line change above a root file with nine hundred.
    fn emit_order(&self, node_ix: usize) -> Vec<TreeEntry> {
        let entries = &self.nodes[node_ix].entries;
        match self.sort {
            CommitFileSort::EditSizeAscending | CommitFileSort::EditSizeDescending => {
                entries.clone()
            }
            CommitFileSort::PathAscending | CommitFileSort::PathDescending => {
                let dirs_first = self.sort == CommitFileSort::PathAscending;
                let is_dir = |entry: &TreeEntry| matches!(entry, TreeEntry::Dir(_));
                let mut ordered = Vec::with_capacity(entries.len());
                ordered.extend(entries.iter().filter(|e| is_dir(e) == dirs_first).copied());
                ordered.extend(entries.iter().filter(|e| is_dir(e) != dirs_first).copied());
                ordered
            }
        }
    }

    /// Walk a single-child chain down to the node whose children actually
    /// render, collecting every segment on the way so a collapse survives the
    /// chain later splitting or merging.
    fn fold(&self, start: usize) -> (usize, Vec<Arc<Path>>, String) {
        let mut node_ix = start;
        let mut chain = vec![Arc::clone(&self.nodes[start].path)];
        let mut label = String::from(self.nodes[start].segment.as_ref());
        while let Some(child) = self.nodes[node_ix].only_child_dir() {
            node_ix = child as usize;
            chain.push(Arc::clone(&self.nodes[node_ix].path));
            label.push('/');
            label.push_str(self.nodes[node_ix].segment.as_ref());
        }
        (node_ix, chain, label)
    }
}

/// What one subtree contributes to its parent: kind counts and `+/-` sums.
#[derive(Clone, Copy, Default)]
struct SubtreeTotals {
    counts: CommitFileKindCounts,
    sums: Option<(u64, u64)>,
}

impl SubtreeTotals {
    fn add_file(&mut self, kind: Option<FileStatusKind>, stats: Option<(u32, u32)>) {
        self.counts.all += 1;
        // Buckets match `build_commit_file_projection`, so a badge and the
        // filter tabs above it cannot disagree.
        match kind {
            Some(FileStatusKind::Untracked | FileStatusKind::Added) => self.counts.added += 1,
            Some(FileStatusKind::Modified | FileStatusKind::Conflicted) => {
                self.counts.modified += 1
            }
            Some(FileStatusKind::Deleted) => self.counts.removed += 1,
            Some(FileStatusKind::Renamed) => self.counts.renamed += 1,
            None => {}
        }
        if let Some((a, d)) = stats {
            let entry = self.sums.get_or_insert((0, 0));
            entry.0 += u64::from(a);
            entry.1 += u64::from(d);
        }
    }

    fn merge(&mut self, child: &Self) {
        self.counts.all += child.counts.all;
        self.counts.modified += child.counts.modified;
        self.counts.added += child.counts.added;
        self.counts.removed += child.counts.removed;
        self.counts.renamed += child.counts.renamed;
        if let Some((a, d)) = child.sums {
            let entry = self.sums.get_or_insert((0, 0));
            entry.0 += a;
            entry.1 += d;
        }
    }
}

struct Flatten {
    rows: Vec<FileListRow>,
    ordered: Vec<usize>,
    row_ix_by_ordinal: Vec<usize>,
    collapsed_spans: Vec<CollapsedSpan>,
}
