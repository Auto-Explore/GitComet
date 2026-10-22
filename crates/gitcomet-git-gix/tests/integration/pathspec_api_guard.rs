//! gix walks that default to the process's working directory go through
//! `repo/status.rs`, which gives them a whole-repository pathspec.
use crate::source_audit;
use source_audit::{has_test_cfg, rust_source_files};
use std::{fs, path::PathBuf};
use syn::visit::{self, Visit};

#[derive(Default)]
struct Guard {
    calls: Vec<String>,
}
impl<'a> Visit<'a> for Guard {
    fn visit_item_mod(&mut self, n: &'a syn::ItemMod) {
        if !has_test_cfg(&n.attrs) {
            visit::visit_item_mod(self, n);
        }
    }
    fn visit_item_fn(&mut self, n: &'a syn::ItemFn) {
        if !has_test_cfg(&n.attrs) {
            visit::visit_item_fn(self, n);
        }
    }
    fn visit_item_impl(&mut self, n: &'a syn::ItemImpl) {
        if !has_test_cfg(&n.attrs) {
            visit::visit_item_impl(self, n);
        }
    }
    fn visit_impl_item_fn(&mut self, n: &'a syn::ImplItemFn) {
        if !has_test_cfg(&n.attrs) {
            visit::visit_impl_item_fn(self, n);
        }
    }
    fn visit_expr_method_call(&mut self, n: &'a syn::ExprMethodCall) {
        let method = n.method.to_string();
        if matches!(
            method.as_str(),
            "tree_index_status" | "pathspec" | "index_worktree_status"
        ) {
            self.calls.push(method);
        }
        visit::visit_expr_method_call(self, n);
    }
    fn visit_expr_call(&mut self, n: &'a syn::ExprCall) {
        if let syn::Expr::Path(path) = &*n.func {
            let segments: Vec<String> = path
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect();
            if segments.ends_with(&["Pathspec".to_owned(), "new".to_owned()]) {
                self.calls.push("Pathspec::new".to_owned());
            }
        }
        visit::visit_expr_call(self, n);
    }
}

#[test]
fn prefix_scoped_walks_stay_in_the_whole_repository_helpers() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut calls = Vec::new();
    for path in rust_source_files(&root) {
        let relative = path.strip_prefix(&root).unwrap();
        if relative.file_stem().is_some_and(|n| n == "tests") {
            continue;
        }
        let ast = syn::parse_file(&fs::read_to_string(&path).unwrap()).unwrap();
        let mut guard = Guard::default();
        guard.visit_file(&ast);
        calls.extend(
            guard
                .calls
                .into_iter()
                .map(|call| (relative.to_string_lossy().replace('\\', "/"), call)),
        );
    }
    assert_eq!(
        calls,
        [
            ("repo/status.rs".to_owned(), "Pathspec::new".to_owned()),
            ("repo/status.rs".to_owned(), "tree_index_status".to_owned()),
        ]
    );
}

#[test]
fn guard_recognizes_nested_calls_and_ignores_test_only_code() {
    let ast = syn::parse_file(
        "fn f() { repo.tree_index_status(a, b, None, r, cb).unwrap(); \
         let p = gix::Pathspec::new(repo, true, x, false, m); repo.pathspec(true, x, false, i, s); } \
         #[cfg(test)] mod tests { fn t() { repo.tree_index_status(a, b, None, r, cb); } }",
    )
    .unwrap();
    let mut guard = Guard::default();
    guard.visit_file(&ast);
    assert_eq!(
        guard.calls,
        ["tree_index_status", "Pathspec::new", "pathspec"]
    );
}
