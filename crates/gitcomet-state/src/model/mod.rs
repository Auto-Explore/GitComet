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
