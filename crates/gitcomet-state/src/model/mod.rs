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
mod workspace;

pub use app::*;
pub use diff::*;
pub use history::*;
pub use loads::*;
pub use navigation::*;
pub use operations::*;
pub use repository::*;
pub use repository_preferences::*;
pub use signature_map::CommitSignatureMap;
pub use workspace::*;

pub type Shared<T> = Arc<T>;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum Loadable<T> {
    /// Nothing has been requested yet. This is also the `Default`, so a fresh
    /// state starts out not-loaded rather than loading or failed.
    #[default]
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
