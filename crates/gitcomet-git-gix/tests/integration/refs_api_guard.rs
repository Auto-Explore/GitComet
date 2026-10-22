//! Every production ref-dependent gix API must pass through the storage adapter.
use crate::source_audit::{Site, production_sites, sites_in};
use std::path::PathBuf;

fn ref_dependent(site: Site<'_>) -> Option<String> {
    let Site::Method(n) = site else {
        return None;
    };
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
        return Some(method);
    }
    if method == "join"
        && let Some(syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(value),
            ..
        })) = n.args.first()
    {
        let value = value.value();
        if matches!(value.as_str(), "HEAD" | "refs" | "packed-refs") || value.ends_with("_HEAD") {
            return Some(format!("join({value})"));
        }
    }
    None
}

#[test]
fn ref_dependent_calls_stay_in_the_storage_adapter() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let calls: Vec<(String, String)> = production_sites(&root, ref_dependent)
        .into_iter()
        .filter(|(path, _)| !path.starts_with("refs/"))
        .collect();
    // Administrative-directory discovery tests the placeholder's existence,
    // not the value of HEAD. This is the sole permanent filesystem exception.
    assert_eq!(calls, [("ignore.rs".to_owned(), "join(HEAD)".to_owned())]);
}

#[test]
fn guard_recognizes_nested_calls_and_ignores_test_only_code() {
    let ast = syn::parse_file("fn f() { repo.head().unwrap(); repo.status(progress); repo.state(); repo.reference(a,b,c,d); repo.path().join(\"REVERT_HEAD\"); } #[cfg(test)] mod tests { fn t() { repo.head(); } }").unwrap();
    assert_eq!(
        sites_in(&ast, ref_dependent),
        ["head", "status", "state", "reference", "join(REVERT_HEAD)"]
    );
}
