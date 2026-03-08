use super::*;
use std::cell::RefCell;

const MAX_LINES_FOR_SYNTAX_HIGHLIGHTING: usize = 4_000;
const MAX_TREESITTER_LINE_BYTES: usize = 512;

thread_local! {
    static LINE_NUMBER_STRINGS: RefCell<Vec<SharedString>> =
        RefCell::new(vec![SharedString::default()]);
}

fn line_number_string(n: Option<u32>) -> SharedString {
    let Some(n) = n else {
        return SharedString::default();
    };
    let ix = n as usize;
    LINE_NUMBER_STRINGS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() <= ix {
            let start = cache.len();
            cache.reserve(ix + 1 - start);
            for v in start..=ix {
                cache.push(v.to_string().into());
            }
        }
        cache[ix].clone()
    })
}

mod canvas;
#[cfg(test)]
mod canvas_tests;
mod conflict_canvas;
mod conflict_resolver;
mod diff;
mod diff_canvas;
mod diff_text;
mod history;
mod history_canvas;
mod history_graph_paint;
mod sidebar;
mod status;

pub(crate) mod benchmarks;

pub(super) use diff_text::{
    DiffSyntaxLanguage, DiffSyntaxMode, diff_syntax_language_for_path, syntax_highlights_for_line,
};
