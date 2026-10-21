//! One retained status-footer position, keyed by immutable source and order.
//!
//! The caller resolves source readiness before querying. It supplies immutable
//! order inputs rather than a materialized drawn-order vector. A cache hit asks
//! for the previously resolved section's inputs, so split-untracked selection
//! does not search the source just to decide whether a cached result is usable.

use crate::view::{ChangeTrackingView, StatusSection};
use gitcomet_core::domain::{DiffArea, FileStatus};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) struct OrderSnapshot<P> {
    pub(crate) projection: Arc<[usize]>,
    pub(crate) plan: Arc<P>,
}

impl<P> Clone for OrderSnapshot<P> {
    fn clone(&self) -> Self {
        Self {
            projection: Arc::clone(&self.projection),
            plan: Arc::clone(&self.plan),
        }
    }
}

impl<P> OrderSnapshot<P> {
    fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.projection, &other.projection) && Arc::ptr_eq(&self.plan, &other.plan)
    }
}

struct Entry<P> {
    repo: u64,
    source: Arc<Vec<FileStatus>>,
    path: PathBuf,
    area: DiffArea,
    view: ChangeTrackingView,
    section: Option<StatusSection>,
    order: Option<OrderSnapshot<P>>,
    position: Option<(usize, usize)>,
}

pub(crate) struct StatusPositionCache<P> {
    entry: Option<Entry<P>>,
}

impl<P> Default for StatusPositionCache<P> {
    fn default() -> Self {
        Self { entry: None }
    }
}

pub(crate) struct PositionQuery<'a> {
    pub(crate) repo: u64,
    pub(crate) source: Option<&'a Arc<Vec<FileStatus>>>,
    pub(crate) path: &'a Path,
    pub(crate) area: DiffArea,
    pub(crate) view: ChangeTrackingView,
}

impl<P> StatusPositionCache<P> {
    pub(crate) fn lookup(
        &mut self,
        query: PositionQuery<'_>,
        mut order_for: impl FnMut(StatusSection) -> Option<OrderSnapshot<P>>,
        resolve_section: impl FnOnce() -> Option<StatusSection>,
        compute: impl FnOnce(StatusSection, Option<&OrderSnapshot<P>>) -> Option<(usize, usize)>,
    ) -> Option<(StatusSection, usize, usize)> {
        let PositionQuery {
            repo,
            source,
            path,
            area,
            view,
        } = query;
        let Some(source) = source else {
            self.entry = None;
            return None;
        };
        if let Some(entry) = self.entry.as_ref()
            && entry.repo == repo
            && Arc::ptr_eq(&entry.source, source)
            && entry.area == area
            && entry.view == view
            && entry.path == path
        {
            let section = entry.section?;
            let order = order_for(section);
            let same_order = match (&entry.order, &order) {
                (None, None) => true,
                (Some(previous), Some(current)) => previous.same(current),
                _ => false,
            };
            if same_order {
                return entry.position.map(|(index, count)| (section, index, count));
            }
        }

        let section = resolve_section();
        let order = section.and_then(&mut order_for);
        let position = section.and_then(|section| compute(section, order.as_ref()));
        self.entry = Some(Entry {
            repo,
            source: Arc::clone(source),
            path: path.to_owned(),
            area,
            view,
            section,
            order,
            position,
        });
        section.and_then(|section| position.map(|(index, count)| (section, index, count)))
    }
}
