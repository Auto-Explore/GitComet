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

pub use gitcomet_base::Loadable;

#[cfg(test)]
mod tests;
