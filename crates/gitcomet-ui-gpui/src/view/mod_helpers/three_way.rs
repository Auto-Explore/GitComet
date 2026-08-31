use super::*;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ThreeWayColumn {
    Base,
    Ours,
    Theirs,
}

impl ThreeWayColumn {
    /// Index into `[base, ours, theirs]` arrays and the aligned map.
    pub(crate) fn side_index(self) -> usize {
        match self {
            ThreeWayColumn::Base => 0,
            ThreeWayColumn::Ours => 1,
            ThreeWayColumn::Theirs => 2,
        }
    }

    pub(crate) const ALL: [ThreeWayColumn; 3] = [
        ThreeWayColumn::Base,
        ThreeWayColumn::Ours,
        ThreeWayColumn::Theirs,
    ];
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ThreeWaySides<T> {
    pub(crate) base: T,
    pub(crate) ours: T,
    pub(crate) theirs: T,
}

impl<T> std::ops::Index<ThreeWayColumn> for ThreeWaySides<T> {
    type Output = T;
    fn index(&self, side: ThreeWayColumn) -> &T {
        match side {
            ThreeWayColumn::Base => &self.base,
            ThreeWayColumn::Ours => &self.ours,
            ThreeWayColumn::Theirs => &self.theirs,
        }
    }
}

impl<T> std::ops::IndexMut<ThreeWayColumn> for ThreeWaySides<T> {
    fn index_mut(&mut self, side: ThreeWayColumn) -> &mut T {
        match side {
            ThreeWayColumn::Base => &mut self.base,
            ThreeWayColumn::Ours => &mut self.ours,
            ThreeWayColumn::Theirs => &mut self.theirs,
        }
    }
}

pub(crate) fn deferred_line_starts_for_text(text: &str) -> Vec<usize> {
    let mut starts = Vec::with_capacity(text.len().saturating_div(64).saturating_add(1));
    starts.push(0);
    for (ix, byte) in text.as_bytes().iter().enumerate() {
        if *byte == b'\n' {
            starts.push(ix.saturating_add(1));
        }
    }
    starts
}

/// Lazily materialized line starts for one merge-input side.
///
/// Large conflict bootstrap only needs stable line counts up front. The full
/// byte-offset index is built on demand when a consumer actually needs random
/// line access for that side.
#[derive(Clone, Debug, Default)]
pub(crate) struct DeferredLineStarts {
    line_count: usize,
    starts: std::sync::Arc<std::sync::OnceLock<std::sync::Arc<[usize]>>>,
}

impl DeferredLineStarts {
    pub(crate) fn with_line_count(line_count: usize) -> Self {
        Self {
            line_count,
            starts: std::sync::Arc::new(std::sync::OnceLock::new()),
        }
    }

    pub(crate) fn line_count(&self) -> usize {
        self.line_count
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.line_count == 0
    }

    #[cfg(test)]
    pub(crate) fn is_materialized(&self) -> bool {
        self.starts.get().is_some()
    }

    pub(crate) fn starts<'a>(&'a self, text: &str) -> &'a [usize] {
        self.starts
            .get_or_init(|| std::sync::Arc::from(deferred_line_starts_for_text(text)))
            .as_ref()
    }

    pub(crate) fn shared_starts(&self, text: &str) -> std::sync::Arc<[usize]> {
        std::sync::Arc::clone(
            self.starts
                .get_or_init(|| std::sync::Arc::from(deferred_line_starts_for_text(text))),
        )
    }

    pub(crate) fn materialized_with_count(
        line_starts: std::sync::Arc<[usize]>,
        line_count: usize,
    ) -> Self {
        let starts = std::sync::OnceLock::new();
        assert!(
            starts.set(line_starts).is_ok(),
            "fresh OnceLock should accept line starts"
        );
        Self {
            line_count,
            starts: std::sync::Arc::new(starts),
        }
    }
}

impl From<Vec<usize>> for DeferredLineStarts {
    fn from(starts: Vec<usize>) -> Self {
        let line_count = starts.len();
        Self::materialized_with_count(std::sync::Arc::from(starts), line_count)
    }
}

impl From<std::sync::Arc<[usize]>> for DeferredLineStarts {
    fn from(starts: std::sync::Arc<[usize]>) -> Self {
        let line_count = starts.len();
        Self::materialized_with_count(starts, line_count)
    }
}
