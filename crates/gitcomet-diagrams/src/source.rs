use crate::DiagramKind;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct DiagramKey(DiagramKind, Arc<str>, u64);

impl PartialEq for DiagramKey {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
            && self.2 == other.2
            && (Arc::ptr_eq(&self.1, &other.1) || self.1 == other.1)
    }
}

impl Eq for DiagramKey {}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static SOURCES_HASHED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(any(test, feature = "test-support"))]
pub fn take_sources_hashed_for_tests() -> usize {
    SOURCES_HASHED.with(|hashed| hashed.replace(0))
}

impl DiagramKey {
    pub fn new(kind: DiagramKind, source: Arc<str>) -> Self {
        #[cfg(any(test, feature = "test-support"))]
        SOURCES_HASHED.with(|hashed| hashed.set(hashed.get() + 1));
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        source.hash(&mut hasher);
        Self(kind, source, hasher.finish())
    }

    pub fn kind(&self) -> DiagramKind {
        self.0
    }

    pub fn source(&self) -> &Arc<str> {
        &self.1
    }
}

impl Hash for DiagramKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
        self.2.hash(state);
    }
}
