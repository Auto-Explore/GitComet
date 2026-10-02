//! Every production ref-dependent gix API must pass through the storage adapter.
#[path = "support/source_audit.rs"]
mod source_audit;
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
        let banned = matches!(
            method.as_str(),
            "head"
                | "head_id"
                | "head_name"
                | "head_commit"
                | "head_tree_id"
                | "head_tree_id_or_empty"
                | "find_reference"
                | "try_find_reference"
                | "references"
                | "log_iter"
                | "is_dirty"
                | "find_fetch_remote"
                | "submodules"
                | "modules"
        ) || method.starts_with("rev_parse")
            || method.starts_with("index_or_load_from_head")
            || (method == "reference" && n.args.len() == 4)
            || (method == "status" && n.args.len() == 1)
            || (matches!(method.as_str(), "delete" | "state") && n.args.is_empty());
        if banned {
            self.calls.push(method.clone());
        }
        if method == "join"
            && let Some(syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(value),
                ..
            })) = n.args.first()
        {
            let value = value.value();
            if matches!(value.as_str(), "HEAD" | "refs" | "packed-refs") || value.ends_with("_HEAD")
            {
                self.calls.push(format!("join({value})"));
            }
        }
        visit::visit_expr_method_call(self, n);
    }
}

#[test]
fn ref_dependent_calls_stay_in_the_storage_adapter() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut calls = Vec::new();
    for path in rust_source_files(&root) {
        let relative = path.strip_prefix(&root).unwrap();
        if relative.starts_with("refs") || relative.file_stem().is_some_and(|n| n == "tests") {
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
    // Administrative-directory discovery tests the placeholder's existence,
    // not the value of HEAD. This is the sole permanent filesystem exception.
    assert_eq!(calls, [("ignore.rs".to_owned(), "join(HEAD)".to_owned())]);
}

#[test]
fn guard_recognizes_nested_calls_and_ignores_test_only_code() {
    let ast = syn::parse_file("fn f() { repo.head().unwrap(); repo.status(progress); repo.state(); repo.reference(a,b,c,d); repo.path().join(\"REVERT_HEAD\"); } #[cfg(test)] mod tests { fn t() { repo.head(); } }").unwrap();
    let mut guard = Guard::default();
    guard.visit_file(&ast);
    assert_eq!(
        guard.calls,
        ["head", "status", "state", "reference", "join(REVERT_HEAD)"]
    );
}
