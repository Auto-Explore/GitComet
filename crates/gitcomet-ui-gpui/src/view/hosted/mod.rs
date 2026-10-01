//! Panes extensions host: independently owned diff panes and file lists over
//! their own sessions. History keeps its own main pane.

use super::*;

pub(crate) mod diff_pane;
pub(crate) mod file_list;
pub(crate) mod projection;
pub(crate) mod rows;

pub(crate) mod raw_lines;
