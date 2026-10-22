//! gix walks that default to the process's working directory go through
//! `repo/status.rs`, which gives them a whole-repository pathspec.
use crate::source_audit::{Call, calls_in, production_calls};
use std::path::PathBuf;

fn prefix_scoped(call: Call<'_>) -> Option<String> {
    match call {
        Call::Method(call) => {
            let method = call.method.to_string();
            matches!(
                method.as_str(),
                "tree_index_status" | "pathspec" | "index_worktree_status"
            )
            .then_some(method)
        }
        Call::Function(call) => {
            let syn::Expr::Path(path) = &*call.func else {
                return None;
            };
            let mut segments = path.path.segments.iter().rev();
            let (Some(last), Some(previous)) = (segments.next(), segments.next()) else {
                return None;
            };
            (last.ident == "new" && previous.ident == "Pathspec")
                .then(|| "Pathspec::new".to_owned())
        }
    }
}

#[test]
fn prefix_scoped_walks_stay_in_the_whole_repository_helpers() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    assert_eq!(
        production_calls(&root, prefix_scoped),
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
    assert_eq!(
        calls_in(&ast, prefix_scoped),
        ["tree_index_status", "Pathspec::new", "pathspec"]
    );
}
