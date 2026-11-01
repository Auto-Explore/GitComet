use crate::source_audit;
use source_audit::{has_test_cfg, rust_source_files};
use std::fs;
use std::path::PathBuf;
use syn::visit::{self, Visit};
use syn::{Expr, ExprCall, ExprLit, File, ImplItemFn, ItemFn, ItemImpl, ItemMod, Lit};

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

    let mut per_file = Vec::new();
    let mut total = 0usize;

    for file in files {
        let source = fs::read_to_string(&file)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", file.display()));
        let parsed: File = syn::parse_file(&source)
            .unwrap_or_else(|err| panic!("failed to parse {}: {err}", file.display()));

        let mut counter = GitCommandCounter::default();
        counter.visit_file(&parsed);

        if counter.count > 0 {
            let rel = file
                .strip_prefix(&crate_root)
                .unwrap_or(file.as_path())
                .to_path_buf();
            per_file.push((rel, counter.count));
            total += counter.count;
        }
    }

    per_file.sort_by(|(a_path, a_count), (b_path, b_count)| {
        b_count.cmp(a_count).then_with(|| a_path.cmp(b_path))
    });

    eprintln!("git CLI migration scoreboard: production Command::new(\"git\") calls = {total}");
    for (path, count) in &per_file {
        eprintln!("  {count:>3} {}", path.display());
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
    let mut offenders = Vec::new();
    for file in rust_source_files(&src_root) {
        let relative = file.strip_prefix(&src_root).unwrap().to_path_buf();
        if relative.as_path() == std::path::Path::new("util/process.rs") {
            continue;
        }
        let source = fs::read_to_string(&file)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", file.display()));
        let parsed: File = syn::parse_file(&source)
            .unwrap_or_else(|err| panic!("failed to parse {}: {err}", file.display()));
        let mut finder = LiteralPathspecSwitchFinder::default();
        finder.visit_file(&parsed);
        if finder.found {
            offenders.push(relative);
        }
    }
    assert!(
        offenders.is_empty(),
        "pass user paths through push_literal_paths/push_literal_pathspec_file instead: {offenders:#?}"
    );
}

#[derive(Default)]
struct LiteralPathspecSwitchFinder {
    found: bool,
}

impl<'ast> Visit<'ast> for LiteralPathspecSwitchFinder {
    fn visit_item_mod(&mut self, node: &'ast ItemMod) {
        if !has_test_cfg(&node.attrs) {
            visit::visit_item_mod(self, node);
        }
    }

    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        if !has_test_cfg(&node.attrs) {
            visit::visit_item_fn(self, node);
        }
    }

    fn visit_lit_str(&mut self, node: &'ast syn::LitStr) {
        if matches!(
            node.value().as_str(),
            "GIT_LITERAL_PATHSPECS" | "--literal-pathspecs"
        ) {
            self.found = true;
        }
    }
}

#[derive(Default)]
struct GitCommandCounter {
    count: usize,
}

impl<'ast> Visit<'ast> for GitCommandCounter {
    fn visit_item_mod(&mut self, node: &'ast ItemMod) {
        if has_test_cfg(&node.attrs) {
            return;
        }
        visit::visit_item_mod(self, node);
    }

    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        if has_test_cfg(&node.attrs) {
            return;
        }
        visit::visit_item_fn(self, node);
    }

    fn visit_item_impl(&mut self, node: &'ast ItemImpl) {
        if has_test_cfg(&node.attrs) {
            return;
        }
        visit::visit_item_impl(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast ImplItemFn) {
        if has_test_cfg(&node.attrs) {
            return;
        }
        visit::visit_impl_item_fn(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast ExprCall) {
        if is_git_command_new_call(node) {
            self.count += 1;
        }
        visit::visit_expr_call(self, node);
    }
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
