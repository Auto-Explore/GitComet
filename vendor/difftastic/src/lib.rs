//! Difftastic is a syntactic diff tool.
//!
//! For usage instructions and advice on contributing, see [the
//! manual](http://difftastic.wilfred.me.uk/).
//!

// I frequently develop difftastic on a newer rustc than the MSRV, so
// these two aren't relevant.
#![allow(renamed_and_removed_lints)]
// This tends to trigger on larger tuples of simple types, and naming
// them would probably be worse for readability.
#![allow(clippy::type_complexity)]
// == "" is often clearer when dealing with strings.
#![allow(clippy::comparison_to_empty)]
// It's common to have pairs foo_lhs and foo_rhs, leading to double
// the number of arguments and triggering this lint.
#![allow(clippy::too_many_arguments)]
// Has false positives on else if chains that sometimes have the same
// body for readability.
#![allow(clippy::if_same_then_else)]
// Good practice in general, but a necessary evil for Syntax. Its Hash
// implementation does not consider the mutable fields, so it is still
// correct.
#![allow(clippy::mutable_key_type)]
// manual_unwrap_or_default was added in Rust 1.79, so earlier versions of
// clippy complain about allowing it.
#![allow(unknown_lints)]
// It's sometimes more readable to explicitly create a vec than to use
// the Default trait.
#![allow(clippy::manual_unwrap_or_default)]
// I find the explicit arithmetic clearer sometimes.
#![allow(clippy::implicit_saturating_sub)]
// It's helpful being super explicit about byte length versus Unicode
// character point length sometimes.
#![allow(clippy::needless_as_bytes)]
// .to_owned() is more explicit on string references.
#![warn(clippy::str_to_string)]
// .to_string() on a String is clearer as .clone().
#![warn(clippy::string_to_string)]
// Debugging features shouldn't be in checked-in code.
#![warn(clippy::todo)]
#![warn(clippy::dbg_macro)]

mod constants;
mod diff;
mod display;
mod hash;
mod lines;
mod options;
mod parse;
mod summary;
mod words;

#[macro_use]
extern crate log;

pub mod api;
mod interrupt;

use std::path::Path;

use typed_arena::Arena;

use crate::diff::changes::ChangeMap;
use crate::diff::shortest_path::SearchStop;
use crate::diff::sliders::fix_all_sliders;
use crate::diff::{shortest_path, unchanged};
use crate::interrupt::Interrupted;
use crate::options::DiffOptions;
use crate::parse::guess_language::guess;
use crate::parse::syntax::{self, MatchedPos};
use crate::parse::tree_sitter_parser as tsp;
use crate::shortest_path::mark_syntax;
use crate::summary::{DiffResult, FileFormat, TextFallbackReason};
use crate::syntax::init_next_prev;

// GitComet: callers show their own line diff for a fallback, so difftastic's
// line-based positions would be discarded.
fn fallback_positions() -> (Vec<MatchedPos>, Vec<MatchedPos>) {
    (vec![], vec![])
}

pub(crate) fn guess_language_for(
    path: &Path,
    lhs_src: &str,
    rhs_src: &str,
) -> Option<crate::parse::guess_language::Language> {
    let guess_src = if rhs_src.is_empty() { lhs_src } else { rhs_src };
    guess(path, guess_src)
}

pub(crate) fn diff_file_content(
    path: &Path,
    lhs_src: &str,
    rhs_src: &str,
    diff_options: &DiffOptions,
) -> Result<DiffResult, Interrupted> {
    let language = guess_language_for(path, lhs_src, rhs_src);
    let lang_config = language.map(|lang| (lang, tsp::from_language(lang)));

    if lhs_src == rhs_src {
        let file_format = match language {
            Some(language) => FileFormat::SupportedLanguage(language),
            None => FileFormat::PlainText,
        };

        // If the two files are byte-for-byte identical, return early
        // rather than doing any more work.
        return Ok(DiffResult {
            file_format,
            lhs_src: String::new(),
            rhs_src: String::new(),
            lhs_positions: vec![],
            rhs_positions: vec![],
        });
    }

    let (file_format, lhs_positions, rhs_positions) = match lang_config {
        None => {
            let file_format = FileFormat::PlainText;
            let (lhs_positions, rhs_positions) = fallback_positions();
            (file_format, lhs_positions, rhs_positions)
        }
        Some((language, lang_config)) => {
            let arena = Arena::new();
            match tsp::to_tree_with_limit(diff_options, lang_config, lhs_src, rhs_src) {
                Ok((lhs_tree, rhs_tree)) => {
                    match tsp::to_syntax_with_limit(
                        lhs_src,
                        rhs_src,
                        &lhs_tree,
                        &rhs_tree,
                        &arena,
                        lang_config,
                        diff_options,
                    ) {
                        Ok((lhs, rhs)) => {
                            interrupt::check(diff_options.should_stop)?;
                            let mut change_map = ChangeMap::default();
                            let possibly_changed =
                                unchanged::mark_unchanged(&lhs, &rhs, &mut change_map);
                            interrupt::check(diff_options.should_stop)?;

                            let mut exceeded_graph_limit = false;

                            for (lhs_section_nodes, rhs_section_nodes) in possibly_changed {
                                init_next_prev(&lhs_section_nodes);
                                init_next_prev(&rhs_section_nodes);

                                match mark_syntax(
                                    lhs_section_nodes.first().copied(),
                                    rhs_section_nodes.first().copied(),
                                    &mut change_map,
                                    diff_options.graph_limit,
                                    diff_options.should_stop,
                                ) {
                                    Ok(()) => {}
                                    Err(SearchStop::Interrupted) => return Err(Interrupted),
                                    Err(SearchStop::GraphLimit) => {
                                        exceeded_graph_limit = true;
                                        break;
                                    }
                                }
                            }

                            if exceeded_graph_limit {
                                let (lhs_positions, rhs_positions) = fallback_positions();
                                (
                                    FileFormat::TextFallback {
                                        reason: TextFallbackReason::GraphLimit,
                                    },
                                    lhs_positions,
                                    rhs_positions,
                                )
                            } else {
                                fix_all_sliders(language, &lhs, &mut change_map);
                                fix_all_sliders(language, &rhs, &mut change_map);
                                interrupt::check(diff_options.should_stop)?;

                                let mut lhs_positions = syntax::change_positions(&lhs, &change_map);
                                interrupt::check(diff_options.should_stop)?;
                                let mut rhs_positions = syntax::change_positions(&rhs, &change_map);

                                if diff_options.ignore_comments {
                                    let lhs_comments = tsp::comment_positions(
                                        &lhs_tree,
                                        lhs_src,
                                        lang_config,
                                        diff_options,
                                    )?;
                                    lhs_positions.extend(lhs_comments);

                                    let rhs_comments = tsp::comment_positions(
                                        &rhs_tree,
                                        rhs_src,
                                        lang_config,
                                        diff_options,
                                    )?;
                                    rhs_positions.extend(rhs_comments);
                                }

                                (
                                    FileFormat::SupportedLanguage(language),
                                    lhs_positions,
                                    rhs_positions,
                                )
                            }
                        }
                        Err(tsp::SyntaxStop::Interrupted) => return Err(Interrupted),
                        Err(tsp::SyntaxStop::ParseErrors(tsp::ExceededParseErrorLimit {
                            error_count,
                            first_error_pos,
                        })) => {
                            debug!(
                                "{}: {error_count} parse errors, first at {first_error_pos:?}",
                                path.display()
                            );
                            let file_format = FileFormat::TextFallback {
                                reason: TextFallbackReason::ParseErrors,
                            };

                            let (lhs_positions, rhs_positions) = fallback_positions();
                            (file_format, lhs_positions, rhs_positions)
                        }
                    }
                }
                Err(tsp::TreeStop::Interrupted) => return Err(Interrupted),
                Err(tsp::TreeStop::NestingLimit) => {
                    debug!("{}: nesting exceeds the limit", path.display());
                    let file_format = FileFormat::TextFallback {
                        reason: TextFallbackReason::NestingLimit,
                    };

                    let (lhs_positions, rhs_positions) = fallback_positions();
                    (file_format, lhs_positions, rhs_positions)
                }
                Err(tsp::TreeStop::ByteLimit(num_bytes)) => {
                    debug!(
                        "{}: {num_bytes} bytes exceed the byte limit",
                        path.display()
                    );
                    let file_format = FileFormat::TextFallback {
                        reason: TextFallbackReason::ByteLimit,
                    };

                    let (lhs_positions, rhs_positions) = fallback_positions();
                    (file_format, lhs_positions, rhs_positions)
                }
            }
        }
    };

    Ok(DiffResult {
        file_format,
        lhs_src: lhs_src.to_owned(),
        rhs_src: rhs_src.to_owned(),
        lhs_positions,
        rhs_positions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diff_identical_content() {
        let s = "foo";
        let res = diff_file_content(Path::new("foo.el"), s, s, &DiffOptions::default()).unwrap();

        assert_eq!(res.lhs_positions, vec![]);
        assert_eq!(res.rhs_positions, vec![]);
    }
}
