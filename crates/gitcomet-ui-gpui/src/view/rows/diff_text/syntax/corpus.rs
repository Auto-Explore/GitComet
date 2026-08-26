//! Regression tests driven by the external syntax-highlighting corpus.
//!
//! The `syntax_highlight_test` corpus is ~200 languages' worth of samples
//! written specifically to break highlighters: f-strings inside
//! f-strings, raw strings whose delimiter contains a quote, regex-versus-divide,
//! heredocs with comment markers inside them, RTL and zero-width identifiers.
//! The in-tree tests are hand-written five-line snippets; these files are what
//! the grammars actually meet.
//!
//! There is no colour ground truth here -- nobody has hand-labelled 250 hostile
//! files -- so nothing asserts "this byte is a string". What it asserts instead
//! is every property that must hold whatever the right answer is, and those are
//! exactly the ones whose failures are invisible in a screenshot: token ranges
//! that land inside a character, pairs that cross each other, a delimiter whose
//! partner does not point back at it, a file that colours nothing at all.
//!
//! # Running it
//!
//! The corpus is the `fixtures/syntax_test` submodule. An unfetched submodule is
//! an empty directory, so these tests **skip** rather than fail when it has not
//! been initialised -- a checkout without `--recurse-submodules` is normal, and
//! a red suite would only teach people to ignore it. The skip prints why, so a
//! run that silently checked nothing cannot be mistaken for a pass.
//!
//! ```sh
//! git submodule update --init fixtures/syntax_test
//! cargo test -p gitcomet-ui-gpui --lib syntax_corpus -- --nocapture
//! ```
//!
//! `$GITCOMET_SYNTAX_CORPUS` overrides the location, for running against a
//! working copy of the corpus that is ahead of the pinned commit.
//!
//! Files whose extension GitComet does not wire are skipped, not failed: the
//! corpus deliberately covers more languages than any one editor does.

use super::*;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Directories that hold generated output rather than samples.
const GENERATED_DIRS: &[&str] = &["build", "gallery", "tools", ".git", ".github"];

/// The corpus writes one of these per language folder; they are documentation,
/// not samples, and 200 identical-shaped Markdown files say nothing new.
///
/// `.git` is here because in a submodule it is a *file* -- a gitdir pointer --
/// rather than the directory [`GENERATED_DIRS`] skips.
const NON_SAMPLE_FILES: &[&str] = &[
    "README.md",
    "CONTRIBUTING.md",
    "FINDINGS.md",
    "LICENSE",
    ".git",
];

/// Samples that legitimately colour nothing, and why.
///
/// Written down rather than tolerated silently: "this file produced no tokens"
/// is exactly the symptom of a grammar that stopped being wired, so the check
/// stays on for everything else.
const NO_TOKENS_EXPECTED: &[(&str, &str)] = &[(
    "templates/blade/page.blade.php",
    "a Blade template carries no `<?php` tag, so tree-sitter-php sees the whole \
     file as one inline-HTML text node -- and GitComet wires no Blade grammar",
)];

/// Bounded so one pathological sample cannot turn the suite into a benchmark.
/// The corpus's largest sample is far under this; the ceiling is here so a
/// future addition fails loudly by being skipped rather than quietly by taking
/// a minute.
const MAX_SAMPLE_BYTES: usize = 256 * 1024;

/// Generous on purpose: this is a correctness sweep, not a budget test, and the
/// budget regime tests live next door. A grammar that cannot parse a 100 KB
/// file in this is a finding in itself.
const PARSE_BUDGET: Duration = Duration::from_secs(5);

/// Delimiter probes per file. Every bracket in a 3000-line file is thousands of
/// tree walks; a bug in the pair walk shows up in the first handful.
const MAX_PAIR_PROBES_PER_FILE: usize = 64;

/// The corpus submodule's path, relative to the repository root.
const CORPUS_SUBMODULE: &str = "fixtures/syntax_test";

/// The corpus root, or the reason to print and skip.
fn corpus_root() -> Result<PathBuf, String> {
    let overridden = std::env::var_os("GITCOMET_SYNTAX_CORPUS").is_some();
    let candidate = match std::env::var_os("GITCOMET_SYNTAX_CORPUS") {
        Some(path) => PathBuf::from(path),
        // `CARGO_MANIFEST_DIR` is `<repo>/crates/gitcomet-ui-gpui`.
        None => Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .ok_or_else(|| "cannot locate the repository root".to_string())?
            .join(CORPUS_SUBMODULE),
    };
    // `manifest.json` is the corpus's own index, so its absence separates "the
    // submodule is not fetched" from "this path is something else entirely".
    if !candidate.join("manifest.json").is_file() {
        return Err(if overridden {
            format!(
                "GITCOMET_SYNTAX_CORPUS points at {}, which holds no manifest.json",
                candidate.display()
            )
        } else {
            format!(
                "the {CORPUS_SUBMODULE} submodule is not fetched -- run \
                 `git submodule update --init {CORPUS_SUBMODULE}`"
            )
        });
    }
    Ok(candidate)
}

/// Every sample file under `root`, in a stable order.
fn sample_files(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
        entries.sort();
        for path in entries {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            if path.is_dir() {
                if !GENERATED_DIRS.contains(&name) {
                    walk(&path, out);
                }
                continue;
            }
            if NON_SAMPLE_FILES.contains(&name) {
                continue;
            }
            out.push(path);
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out
}

/// One sample, ready to assert against.
struct Sample {
    path: PathBuf,
    language: DiffSyntaxLanguage,
    text: String,
    line_starts: Arc<[usize]>,
    document: PreparedSyntaxDocument,
}

impl Sample {
    /// The number of lines the document's own index reports, which is one more
    /// than `str::lines` for a file ending in a newline. Every offset here goes
    /// through that index so the two never disagree.
    fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// One line's text, without its terminator -- the same span the pair and
    /// occurrence lookups measure display columns against.
    fn line(&self, ix: usize) -> &str {
        let start = self.line_starts.get(ix).copied().unwrap_or(self.text.len());
        let end = self
            .line_starts
            .get(ix + 1)
            .copied()
            .unwrap_or(self.text.len())
            .min(self.text.len());
        let line = self.text.get(start..end).unwrap_or_default();
        line.strip_suffix('\n')
            .unwrap_or(line)
            .strip_suffix('\r')
            .unwrap_or_else(|| line.strip_suffix('\n').unwrap_or(line))
    }

    /// The path as the corpus names it, with `/` on every platform so the
    /// tables above read the same everywhere.
    fn relative(&self, root: &Path) -> String {
        self.path
            .strip_prefix(root)
            .unwrap_or(&self.path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    fn label(&self, root: &Path) -> String {
        format!("{} ({:?})", self.relative(root), self.language)
    }
}

/// Parse every sample GitComet claims to support and hand each one to `check`.
///
/// Streamed rather than collected: the prepared-document cache evicts, and 200
/// documents do not fit in it, so a pass that loaded them all first would find
/// every early sample's tokens gone by the time it looked. Each file is
/// checked while it is still the most recent thing parsed.
///
/// Returns how many samples were checked, so a caller can refuse to pass on an
/// empty sweep.
fn for_each_sample(
    root: &Path,
    failures: &mut Vec<String>,
    mut check: impl FnMut(&Sample, &mut Vec<String>),
) -> usize {
    let mut checked = 0usize;
    for path in sample_files(root) {
        let Some(sample) = parse_sample(path, failures) else {
            continue;
        };
        checked += 1;
        check(&sample, failures);
    }
    checked
}

/// Parse one file the way the diff panes would, or `None` if it is not a sample
/// GitComet claims to support.
///
/// Split out of [`for_each_sample`] so the single-file dump below shares exactly
/// this path -- a dump that parsed differently from the sweep would answer
/// questions about itself rather than about the highlighter.
fn parse_sample(path: PathBuf, failures: &mut Vec<String>) -> Option<Sample> {
    let language = diff_syntax_language_for_path(&path)?;
    let bytes = std::fs::read(&path).ok()?;
    if bytes.len() > MAX_SAMPLE_BYTES {
        return None;
    }
    // The corpus includes deliberately invalid encodings; a highlighter never
    // sees those, because the diff indexer rejects them first.
    let text = String::from_utf8(bytes).ok()?;
    if text.is_empty() {
        return None;
    }

    let input = treesitter_document_input_from_text(&text);
    let line_starts = Arc::clone(&input.line_starts);
    let document = match prepare_treesitter_document_with_budget_reuse_text(
        language,
        DiffSyntaxMode::Auto,
        input.text,
        Arc::clone(&line_starts),
        DiffSyntaxBudget {
            foreground_parse: PARSE_BUDGET,
        },
        None,
        None,
    ) {
        PrepareTreesitterDocumentResult::Ready(document) => document,
        // `Unsupported` is the honest answer for a language wired for heuristic
        // highlighting only, so it is not a failure. A timeout on a file this
        // small is.
        PrepareTreesitterDocumentResult::Unsupported => return None,
        PrepareTreesitterDocumentResult::TimedOut => {
            failures.push(format!(
                "{} ({language:?}) did not parse within {PARSE_BUDGET:?}",
                path.display()
            ));
            return None;
        }
    };

    Some(Sample {
        path,
        language,
        text,
        line_starts,
        document,
    })
}

/// Skip with a printed reason, or hand back the corpus.
macro_rules! corpus_or_skip {
    () => {
        match corpus_root() {
            Ok(root) => root,
            Err(reason) => {
                println!("skipping syntax corpus test: {reason}");
                return;
            }
        }
    };
}

/// Every token a grammar emits must be a slice of the line it was emitted for.
///
/// A range that ends past the line, starts inside a multi-byte character, or
/// overlaps its neighbour is not a colour bug -- it is a panic or a mangled row
/// the moment the canvas shapes that line. The hand-written tests check this on
/// snippets they chose; the corpus checks it on text chosen to break the
/// grammars.
#[test]
fn syntax_corpus_tokens_are_slices_of_their_own_line() {
    let root = corpus_or_skip!();
    let mut failures: Vec<String> = Vec::new();
    let mut coloured_files = 0usize;
    let checked = for_each_sample(&root, &mut failures, |sample, failures| {
        let mut tokens_in_file = 0usize;
        for line_ix in 0..sample.line_count() {
            let Some(tokens) = syntax_tokens_for_prepared_document_line(sample.document, line_ix)
            else {
                continue;
            };
            tokens_in_file += tokens.len();
            let line = sample.line(line_ix);
            let mut previous_end = 0usize;
            for token in tokens.iter() {
                let at = format!("{}:{}", sample.label(&root), line_ix + 1);
                if token.range.start > token.range.end {
                    failures.push(format!("{at}: inverted range {:?}", token.range));
                    continue;
                }
                if token.range.end > line.len() {
                    failures.push(format!(
                        "{at}: {:?} ends past the {}-byte line",
                        token.range,
                        line.len()
                    ));
                    continue;
                }
                if !line.is_char_boundary(token.range.start)
                    || !line.is_char_boundary(token.range.end)
                {
                    failures.push(format!(
                        "{at}: {:?} splits a character in {line:?}",
                        token.range
                    ));
                    continue;
                }
                if token.range.start < previous_end {
                    failures.push(format!(
                        "{at}: {:?} overlaps the token ending at {previous_end}",
                        token.range
                    ));
                }
                previous_end = token.range.end;
            }
        }
        if tokens_in_file > 0 {
            coloured_files += 1;
        } else {
            let relative = sample.relative(&root);
            match NO_TOKENS_EXPECTED
                .iter()
                .find(|(path, _)| *path == relative)
            {
                Some((_, why)) => println!("  (expected, no tokens) {relative}: {why}"),
                None => failures.push(format!(
                    "{}: parsed but emitted no tokens at all",
                    sample.label(&root)
                )),
            }
        }
    });
    assert!(
        checked > 0,
        "the corpus at {} yielded no supported samples",
        root.display()
    );

    println!(
        "syntax corpus: {checked} samples, {coloured_files} coloured, from {}",
        root.display()
    );
    assert!(
        failures.is_empty(),
        "{} corpus token failures:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

/// A pair must point back at itself.
///
/// Clicking one end and clicking the other must name the same two spans. That
/// single property is what a crossing pair violates: with `( [ ) ]` flattened
/// under one node, a stack that removes the matched opener from the middle
/// leaves `[` live, so `]` pairs across the `)` and the two ends disagree about
/// each other. It also catches an end projected onto the wrong line, since the
/// return trip starts from the projected position.
#[test]
fn syntax_corpus_pairs_agree_from_both_ends() {
    let root = corpus_or_skip!();
    let mut failures: Vec<String> = Vec::new();
    let mut probes = 0usize;
    let mut matched = 0usize;
    let checked = for_each_sample(&root, &mut failures, |sample, failures| {
        let mut probes_in_file = 0usize;
        for line_ix in 0..sample.line_count() {
            if probes_in_file >= MAX_PAIR_PROBES_PER_FILE {
                break;
            }
            let Some(tokens) = syntax_tokens_for_prepared_document_line(sample.document, line_ix)
            else {
                continue;
            };
            let line = sample.line(line_ix);
            for token in tokens.iter() {
                if probes_in_file >= MAX_PAIR_PROBES_PER_FILE {
                    break;
                }
                if !matches!(
                    token.kind,
                    SyntaxTokenKind::PunctuationBracket | SyntaxTokenKind::Tag
                ) {
                    continue;
                }
                if token.range.end > line.len() || !line.is_char_boundary(token.range.start) {
                    continue; // Already reported by the token-shape test.
                }
                probes_in_file += 1;
                probes += 1;

                let column = display_offset_for_raw_offset(line, token.range.start);
                // No pair is a legitimate answer: the corpus is full of
                // deliberately unbalanced and malformed text.
                let Some(hit) = prepared_document_syntax_pair_at_display_offset(
                    sample.document,
                    line_ix,
                    column,
                ) else {
                    continue;
                };
                matched += 1;

                let at = format!("{}:{}", sample.label(&root), line_ix + 1);
                let spans: Vec<_> = hit.open.iter().chain(hit.close.iter()).collect();
                for span in &spans {
                    if span.line_ix >= sample.line_count() {
                        failures.push(format!(
                            "{at}: an end landed on line {} of a {}-line file",
                            span.line_ix + 1,
                            sample.line_count()
                        ));
                        continue;
                    }
                    let width =
                        crate::view::diff_utils::diff_text_display_len(sample.line(span.line_ix));
                    if span.display_range.start >= span.display_range.end
                        || span.display_range.end > width
                    {
                        failures.push(format!(
                            "{at}: end {:?} is not inside line {}'s {width} columns",
                            span.display_range,
                            span.line_ix + 1
                        ));
                    }
                }

                // The return trip, from the far end's *last* column.
                //
                // Not its first: a caret sits between two characters, so the
                // column a delimiter starts at is equally the column after
                // whatever precedes it, and the adjacency rule may rightly
                // prefer that one -- clicking the `<` of `</h1>` in
                // `<h1>{title}</h1>` is also clicking just after the `}`, and
                // the braces are the nearer answer. The last column of the
                // delimiter has no such rival to its left.
                let Some(far) = hit.close.first() else {
                    failures.push(format!("{at}: a pair with no closing end"));
                    continue;
                };
                let far_column = far
                    .display_range
                    .end
                    .saturating_sub(1)
                    .max(far.display_range.start);
                let back = prepared_document_syntax_pair_at_display_offset(
                    sample.document,
                    far.line_ix,
                    far_column,
                );
                match back {
                    Some(back) if back.open == hit.open && back.close == hit.close => {}
                    Some(back) => failures.push(format!(
                        "{at}: clicking the open end gave {:?}/{:?}, clicking its close gave {:?}/{:?}",
                        hit.open, hit.close, back.open, back.close
                    )),
                    None => failures.push(format!(
                        "{at}: the open end paired at line {} column {far_column}, which pairs \
                         with nothing",
                        far.line_ix + 1,
                    )),
                }
            }
        }
    });
    assert!(checked > 0, "no supported samples");

    println!("syntax corpus: {checked} samples, {probes} delimiter probes, {matched} paired");
    assert!(
        failures.is_empty(),
        "{} corpus pair failures:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

/// The corpus names each file after the construct it stresses, which is the one
/// piece of ground truth in it: `strings.rs` is a file of strings, so a
/// highlighter that colours no string in it is broken however plausible the
/// rest looks.
///
/// Only the stems the corpus uses as construct names count, and only where the
/// language can express the construct at all -- there is no `comments.json`,
/// because JSON has no comments.
const STEM_MUST_COLOUR: &[(&str, SyntaxTokenKind)] = &[
    ("strings", SyntaxTokenKind::String),
    ("comments", SyntaxTokenKind::Comment),
    ("numbers", SyntaxTokenKind::Number),
    ("interpolation", SyntaxTokenKind::String),
    ("heredocs", SyntaxTokenKind::String),
];

#[test]
fn syntax_corpus_files_colour_the_construct_they_are_named_for() {
    let root = corpus_or_skip!();
    let mut failures: Vec<String> = Vec::new();
    let mut named = 0usize;
    let checked = for_each_sample(&root, &mut failures, |sample, failures| {
        let stem = sample
            .path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default();
        let Some((_, required)) = STEM_MUST_COLOUR.iter().find(|(name, _)| *name == stem) else {
            return;
        };
        named += 1;

        let mut seen = false;
        for line_ix in 0..sample.line_count() {
            let Some(tokens) = syntax_tokens_for_prepared_document_line(sample.document, line_ix)
            else {
                continue;
            };
            // A doc comment is a comment, and an escape or a regex literal is
            // part of the string it sits in; the file is named for the
            // construct, not for the finest label the grammar puts on it.
            seen = tokens.iter().any(|token| match required {
                SyntaxTokenKind::String => matches!(
                    token.kind,
                    SyntaxTokenKind::String
                        | SyntaxTokenKind::StringEscape
                        | SyntaxTokenKind::StringRegex
                        | SyntaxTokenKind::StringSpecial
                ),
                SyntaxTokenKind::Comment => matches!(
                    token.kind,
                    SyntaxTokenKind::Comment | SyntaxTokenKind::CommentDoc
                ),
                other => token.kind == *other,
            });
            if seen {
                break;
            }
        }
        if !seen {
            failures.push(format!(
                "{} is a file of {stem} and none were coloured {required:?}",
                sample.label(&root)
            ));
        }
    });
    assert!(checked > 0, "no supported samples");

    println!("syntax corpus: {named} construct-named samples checked");
    assert!(
        failures.is_empty(),
        "{} corpus construct failures:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

/// Print every token of one sample, line by line.
///
/// The corpus sweeps above assert invariants, and invariants are silent about the
/// one report that keeps arriving: "the highlighting breaks around line N". That
/// question needs the actual tokens, and reading the grammar's query cannot answer
/// it -- an upstream query that captures nothing for a construct looks identical
/// in source to one that captures it correctly.
///
/// Off unless `$GITCOMET_SYNTAX_DUMP` names a file, either absolute or relative to
/// the corpus root:
///
/// ```sh
/// GITCOMET_SYNTAX_DUMP=config/makefile/Makefile \
///   cargo test -p gitcomet-ui-gpui --lib syntax_corpus_dump -- --nocapture
/// ```
///
/// `..` marks a line the grammar coloured nothing on, which is what a break looks
/// like from here.
#[test]
fn syntax_corpus_dump() {
    let Some(requested) = std::env::var_os("GITCOMET_SYNTAX_DUMP") else {
        println!("skipping syntax corpus dump: set $GITCOMET_SYNTAX_DUMP to a sample path");
        return;
    };
    let requested = PathBuf::from(requested);
    let path = if requested.is_absolute() {
        requested
    } else {
        corpus_or_skip!().join(requested)
    };
    assert!(path.is_file(), "no such sample: {}", path.display());

    let language = diff_syntax_language_for_path(&path)
        .unwrap_or_else(|| panic!("no wired language for {}", path.display()));

    let mut failures: Vec<String> = Vec::new();
    // `None` here is a language wired for heuristic highlighting only, which is a
    // thing worth dumping rather than an error: `Conf` and the other grammarless
    // ones are exactly where "it colours nothing" is the report.
    let sample = parse_sample(path.clone(), &mut failures);
    assert!(
        failures.is_empty(),
        "{} did not parse: {}",
        path.display(),
        failures.join("; ")
    );
    let text = match sample.as_ref() {
        Some(sample) => sample.text.clone(),
        None => std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("cannot read {}: {err}", path.display())),
    };

    println!(
        "{} ({language:?}{})",
        path.display(),
        if sample.is_some() {
            ""
        } else {
            ", heuristic only"
        }
    );
    let mut uncoloured = 0usize;
    let line_count = match sample.as_ref() {
        Some(sample) => sample.line_count(),
        None => text.lines().count(),
    };
    for line_ix in 0..line_count {
        let heuristic_line;
        let (line, tokens) = match sample.as_ref() {
            Some(sample) => (
                sample.line(line_ix),
                syntax_tokens_for_prepared_document_line(sample.document, line_ix)
                    .map(|tokens| tokens.to_vec())
                    .unwrap_or_default(),
            ),
            None => {
                heuristic_line = text.lines().nth(line_ix).unwrap_or_default();
                (
                    heuristic_line,
                    syntax_tokens_for_line(heuristic_line, language, DiffSyntaxMode::Auto).to_vec(),
                )
            }
        };
        let rendered = if tokens.is_empty() {
            if !line.trim().is_empty() {
                uncoloured += 1;
            }
            "..".to_string()
        } else {
            tokens
                .iter()
                .map(|token| {
                    let text = line.get(token.range.clone()).unwrap_or("<out of range>");
                    format!("{:?}={text:?}", token.kind)
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        println!("{:>5}  {line:<72}  {rendered}", line_ix + 1);
    }
    println!("\n{uncoloured} non-empty lines coloured nothing");
}

/// For one line, what every column resolves to when clicked.
///
/// The token dump above answers "what colour is this"; this answers "what does
/// clicking here select", which is the other half of what a reader interacts
/// with and is invisible in a screenshot. A caret sits *between* characters, so
/// an off-by-one in the click path looks like "the bracket highlights only when
/// I click just before it" -- a shape that is obvious in this table and almost
/// impossible to reason about from the source.
///
/// `$GITCOMET_SYNTAX_PAIRS=<path>:<1-based line>`, path absolute or relative to
/// the corpus root:
///
/// ```sh
/// GITCOMET_SYNTAX_PAIRS=languages/php/templating.php:12 \
///   cargo test -p gitcomet-ui-gpui --lib syntax_pair_probe -- --nocapture
/// ```
#[test]
fn syntax_pair_probe() {
    let Some(requested) = std::env::var_os("GITCOMET_SYNTAX_PAIRS") else {
        println!("skipping pair probe: set $GITCOMET_SYNTAX_PAIRS to <path>:<line>");
        return;
    };
    let requested = requested.to_string_lossy().into_owned();
    let (path, line_no) = requested
        .rsplit_once(':')
        .unwrap_or_else(|| panic!("expected <path>:<line>, got {requested:?}"));
    let line_ix: usize = line_no
        .parse::<usize>()
        .unwrap_or_else(|err| panic!("bad line number {line_no:?}: {err}"))
        .saturating_sub(1);

    let path = PathBuf::from(path);
    let path = if path.is_absolute() {
        path
    } else {
        corpus_or_skip!().join(path)
    };
    let mut failures: Vec<String> = Vec::new();
    let sample = parse_sample(path.clone(), &mut failures)
        .unwrap_or_else(|| panic!("{} has no prepared document", path.display()));

    // Build the line's tokens first. A row has to be *drawn* before it can be
    // clicked, and drawing is what populates the injection cache that
    // `injected_syntax_pair_at` reads -- so probing without this would report
    // the host grammar's answer for a region an injected grammar owns.
    let _ = syntax_tokens_for_prepared_document_line(sample.document, line_ix);

    let line = sample.line(line_ix);
    println!(
        "{} ({:?}) line {}",
        path.display(),
        sample.language,
        line_ix + 1
    );
    println!("  {line}");
    let width = crate::view::diff_utils::diff_text_display_len(line);
    for column in 0..=width {
        let at = line
            .chars()
            .nth(column)
            .map_or_else(|| "<eol>".to_string(), |ch| format!("{ch:?}"));
        let rendered =
            match prepared_document_syntax_pair_at_display_offset(sample.document, line_ix, column)
            {
                Some(hit) => {
                    let span = |s: &PreparedSyntaxPairSpan| {
                        format!(
                            "{}:{}..{}",
                            s.line_ix + 1,
                            s.display_range.start,
                            s.display_range.end
                        )
                    };
                    format!(
                        "{:?} open={} close={}",
                        hit.kind,
                        hit.open.iter().map(span).collect::<Vec<_>>().join(","),
                        hit.close.iter().map(span).collect::<Vec<_>>().join(","),
                    )
                }
                None => "-".to_string(),
            };
        println!("  col {column:>3} {at:<8} {rendered}");
    }
}
