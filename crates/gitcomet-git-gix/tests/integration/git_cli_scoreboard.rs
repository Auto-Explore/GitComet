use crate::source_audit::{Site, production_sites, rust_source_files};
use std::path::PathBuf;
use syn::{Expr, ExprCall, ExprLit, Lit};

const GIT_COMMAND_BUDGET: usize = 105;

#[test]
fn production_git_cli_call_scoreboard_with_budget_gate() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src_root = crate_root.join("src");
    let files = rust_source_files(&src_root);

    assert!(
        !files.is_empty(),
        "expected Rust source files under {}",
        src_root.display()
    );

    let mut per_file: Vec<(String, usize)> = Vec::new();
    for (path, _) in production_sites(&src_root, |site| match site {
        Site::Function(call) if is_git_command_new_call(call) => Some(String::new()),
        _ => None,
    }) {
        match per_file.last_mut() {
            Some((last, count)) if *last == path => *count += 1,
            _ => per_file.push((path, 1)),
        }
    }
    let total: usize = per_file.iter().map(|(_, count)| count).sum();

    per_file.sort_by(|(a_path, a_count), (b_path, b_count)| {
        b_count.cmp(a_count).then_with(|| a_path.cmp(b_path))
    });

    eprintln!("git CLI migration scoreboard: production Command::new(\"git\") calls = {total}");
    for (path, count) in &per_file {
        eprintln!("  {count:>3} src/{path}");
    }
    eprintln!("budget gate: <= {GIT_COMMAND_BUDGET}");

    assert!(
        total <= GIT_COMMAND_BUDGET,
        "production Command::new(\"git\") call count increased to {total} (budget {GIT_COMMAND_BUDGET}). \
         Reduce CLI usage or explicitly lower the budget after migration progress."
    );
}

/// A user's file path is a pathspec even after `--`, so `[a].txt` also
/// matches `a.txt`. Only `push_literal_paths` and `push_literal_pathspec_file`
/// may switch literal pathspecs on; a second way to do it lets the next
/// command that takes a path forget it, as the discard, stage, submodule and
/// history commands once did.
#[test]
fn literal_pathspecs_are_switched_on_only_by_the_path_helpers() {
    let src_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders: Vec<String> = production_sites(&src_root, |site| match site {
        Site::Literal(literal)
            if matches!(
                literal.value().as_str(),
                "GIT_LITERAL_PATHSPECS" | "--literal-pathspecs"
            ) =>
        {
            Some(String::new())
        }
        _ => None,
    })
    .into_iter()
    .map(|(path, _)| path)
    .filter(|path| path != "util/process.rs")
    .collect();
    offenders.dedup();
    assert!(
        offenders.is_empty(),
        "pass user paths through push_literal_paths/push_literal_pathspec_file instead: {offenders:#?}"
    );
}

fn is_git_command_new_call(expr: &ExprCall) -> bool {
    let Expr::Path(path_expr) = expr.func.as_ref() else {
        return false;
    };

    let mut segments = path_expr.path.segments.iter().rev();
    let Some(last) = segments.next() else {
        return false;
    };
    let Some(previous) = segments.next() else {
        return false;
    };
    if last.ident != "new" || previous.ident != "Command" {
        return false;
    }

    let Some(first_arg) = expr.args.first() else {
        return false;
    };
    let Expr::Lit(ExprLit {
        lit: Lit::Str(lit_str),
        ..
    }) = first_arg
    else {
        return false;
    };

    lit_str.value() == "git"
}
