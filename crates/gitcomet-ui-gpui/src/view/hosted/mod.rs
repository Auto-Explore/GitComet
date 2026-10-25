//! Panes extensions host: independently owned diff panes and file lists over
//! their own sessions. History keeps its own main pane.

use super::*;

pub(crate) mod diff_pane;
pub(in crate::view) mod environment;
pub(crate) mod file_list;
pub(crate) mod markdown;

pub(crate) mod raw_lines;
