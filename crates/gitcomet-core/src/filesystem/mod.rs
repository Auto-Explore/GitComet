//! Filesystem operations shared by all windows. This service never stages Git
//! changes. Call it on a worker; saves and journal operations share its lock.
mod engine;
mod io;
mod trash;

pub use engine::{Filesystem, global};
pub use io::{DiskVersion, absolute_identity, validate_name};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Identity of a live filesystem document, independent of repository tabs.
/// Revision previews retain their separate Git revision identities.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct DocumentIdentity(pub PathBuf);

impl DocumentIdentity {
    pub fn resolve(path: &std::path::Path) -> std::io::Result<Self> {
        absolute_identity(path).map(Self)
    }

    pub fn retarget(&self, changes: &[PathChange]) -> Self {
        let mut path = self.0.clone();
        for change in changes {
            if let Some(next) = change.retarget(&path) {
                path = next;
            }
        }
        Self(path)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct OperationId(pub u64);

impl OperationId {
    pub fn allocate() -> Self {
        static NEXT: std::sync::OnceLock<AtomicU64> = std::sync::OnceLock::new();
        let next = NEXT.get_or_init(|| {
            let time = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64;
            AtomicU64::new((time ^ u64::from(std::process::id()).rotate_left(32)).max(1))
        });
        Self(next.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Clone, Debug, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferIntent {
    Copy,
    Move,
}

#[derive(Clone, Debug)]
pub enum Operation {
    CompleteOutbound {
        receipt: OutboundReceipt,
        intent: Option<TransferIntent>,
        source_removed: bool,
    },
    Save {
        path: PathBuf,
        contents: Arc<[u8]>,
        expected: Option<DiskVersion>,
        overwrite: bool,
    },
    CreateFile {
        path: PathBuf,
    },
    CreateDirectory {
        path: PathBuf,
    },
    Transfer {
        sources: Vec<PathBuf>,
        destination: PathBuf,
        intent: TransferIntent,
    },
    Duplicate {
        sources: Vec<PathBuf>,
    },
    Rename {
        source: PathBuf,
        name: OsString,
    },
    Trash {
        sources: Vec<PathBuf>,
    },
    DeletePermanently {
        sources: Vec<PathBuf>,
        confirmed: bool,
    },
    Undo,
    Redo,
}

impl Operation {
    pub fn sources(&self) -> &[PathBuf] {
        match self {
            Self::Transfer { sources, .. }
            | Self::Duplicate { sources }
            | Self::Trash { sources }
            | Self::DeletePermanently { sources, .. } => sources,
            Self::CompleteOutbound { receipt, .. } => &receipt.paths,
            Self::Rename { source, .. } => std::slice::from_ref(source),
            _ => &[],
        }
    }

    pub fn removes_sources(&self) -> bool {
        matches!(
            self,
            Self::Trash { .. }
                | Self::DeletePermanently { .. }
                | Self::CompleteOutbound {
                    intent: Some(TransferIntent::Move),
                    source_removed: false,
                    ..
                }
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictChoice {
    KeepBoth,
    Replace,
    Merge,
    Skip,
    Cancel,
}

/// A decision applies only to the object that was shown in the prompt.
#[derive(Clone, Debug)]
pub struct ConflictResolution {
    pub expected: DiskVersion,
    pub choice: ConflictChoice,
}

#[derive(Clone, Debug)]
pub struct Request {
    pub id: OperationId,
    pub logical_id: OperationId,
    pub operation: Operation,
    pub resolutions: BTreeMap<PathBuf, ConflictResolution>,
    pub cancellation: Cancellation,
    /// A native sender owns removal after this copy succeeds. The resulting
    /// transfer is completed by that application and is outside our journal.
    pub native_source_move: bool,
}

impl Request {
    pub fn new(operation: Operation) -> Self {
        let id = OperationId::allocate();
        Self {
            id,
            logical_id: id,
            operation,
            resolutions: BTreeMap::new(),
            cancellation: Cancellation::default(),
            native_source_move: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Conflict {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub version: DiskVersion,
    pub can_merge: bool,
    /// Resume the enclosing directory merge after resolving a child collision.
    pub continuation: Option<Box<Request>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathChange {
    pub old: Option<PathBuf>,
    pub new: Option<PathBuf>,
}

impl PathChange {
    pub fn retarget(&self, path: &std::path::Path) -> Option<PathBuf> {
        self.new
            .as_ref()
            .zip(self.old.as_ref())
            .and_then(|(new, old)| path.strip_prefix(old).ok().map(|suffix| new.join(suffix)))
    }
}

#[derive(Clone, Debug)]
pub enum ItemOutcome {
    Completed,
    Skipped,
    Cancelled,
    Conflict(Conflict),
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct ItemResult {
    pub source: PathBuf,
    pub destination: Option<PathBuf>,
    pub outcome: ItemOutcome,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub id: OperationId,
    pub completed_items: usize,
    pub total_items: usize,
    pub current_path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct OperationResult {
    pub saved_version: Option<DiskVersion>,
    /// Disk identities captured while the operation still owns the filesystem
    /// lock. Editors adopt these only when their saved contents still match.
    pub moved_versions: BTreeMap<PathBuf, DiskVersion>,
    pub id: OperationId,
    pub items: Vec<ItemResult>,
    pub changes: Vec<PathChange>,
    pub undo_available: bool,
    pub redo_available: bool,
}

impl OperationResult {
    pub fn succeeded(&self) -> bool {
        self.items
            .iter()
            .all(|item| matches!(item.outcome, ItemOutcome::Completed | ItemOutcome::Skipped))
    }
}

/// Saved-disk snapshot taken before offering items to another application.
#[derive(Clone, Debug)]
pub struct OutboundReceipt {
    pub id: OperationId,
    pub paths: Vec<PathBuf>,
    pub directories: Vec<bool>,
    pub(crate) versions: Vec<DiskVersion>,
}
