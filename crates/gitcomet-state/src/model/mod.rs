use std::sync::Arc;

mod app;
mod diff;
mod history;
mod loads;
mod navigation;
mod operations;
mod repository;
mod repository_preferences;
mod signature_map;

pub use app::*;
pub use diff::*;
pub use history::*;
pub use loads::*;
pub use navigation::*;
pub use operations::*;
pub use repository::*;
pub use repository_preferences::*;
pub use signature_map::CommitSignatureMap;

pub type Shared<T> = Arc<T>;

/// The entries of a feedback queue appended after entry number `after`.
///
/// Notifications, diagnostics and command results are numbered from 1 in the
/// order they are queued, and `seq` is the newest entry's number. The store
/// drops entries from the front once the window acknowledges them, so a
/// queue's length says nothing about which entries are new. An `after` past
/// `seq` predates a restart of the numbering (a new repository lifetime), so
/// every queued entry is newer.
fn feedback_after<T>(entries: &[T], seq: u64, after: u64) -> &[T] {
    let newer = usize::try_from(seq.wrapping_sub(after))
        .unwrap_or(usize::MAX)
        .min(entries.len());
    &entries[entries.len() - newer..]
}

/// Drops the entries numbered `through` or lower, keeping any queued after the
/// acknowledgement was sent.
fn acknowledge_feedback<T>(entries: &mut Vec<T>, seq: u64, through: u64) {
    let unacknowledged = feedback_after(entries, seq, through).len();
    entries.drain(..entries.len() - unacknowledged);
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Loadable<T> {
    NotLoaded,
    Loading,
    Ready(T),
    Error(String),
}

impl<T> Loadable<T> {
    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading)
    }

    /// The loaded value, if there is one.
    ///
    /// Exists so the ~160 sites that only care about the `Ready` arm can say so
    /// in one line instead of spelling out a `match` with a `_ => ..` fallback,
    /// which is how the same five-line block ended up copied across the pickers.
    pub fn ready(&self) -> Option<&T> {
        match self {
            Self::Ready(value) => Some(value),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
