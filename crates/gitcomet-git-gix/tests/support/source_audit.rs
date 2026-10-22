use std::{
    fs,
    path::{Path, PathBuf},
};
use syn::Attribute;

pub fn rust_source_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_rust_source_files(root, &mut files);
    files.sort();
    files
}

fn collect_rust_source_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let mut entries = fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("failed to read source directory {}: {err}", dir.display()))
        .map(|entry| entry.unwrap_or_else(|err| panic!("failed to read dir entry: {err}")))
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_source_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}

pub fn has_test_cfg(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("cfg") {
            return false;
        }
        match &attr.meta {
            syn::Meta::List(list) => list
                .tokens
                .to_string()
                .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
                .any(|token| token == "test"),
            _ => false,
        }
    })
}

/// A place in the source a guard may name: a call or a string literal.
pub enum Site<'a> {
    Method(&'a syn::ExprMethodCall),
    Function(&'a syn::ExprCall),
    Literal(&'a syn::LitStr),
}

/// The sites `pick` names in production code under `root`, as
/// `(file relative to root, name)` in source order. Files named `tests` and
/// items marked `#[cfg(test)]` are skipped.
pub fn production_sites(
    root: &Path,
    pick: impl Fn(Site<'_>) -> Option<String>,
) -> Vec<(String, String)> {
    let mut calls = Vec::new();
    for path in rust_source_files(root) {
        let relative = path.strip_prefix(root).unwrap();
        if relative.file_stem().is_some_and(|stem| stem == "tests") {
            continue;
        }
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        let file = syn::parse_file(&source)
            .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()));
        let relative = relative.to_string_lossy().replace('\\', "/");
        calls.extend(
            sites_in(&file, &pick)
                .into_iter()
                .map(|call| (relative.clone(), call)),
        );
    }
    calls
}

/// The sites `pick` names in `file`, outside items marked `#[cfg(test)]`.
pub fn sites_in(file: &syn::File, pick: impl Fn(Site<'_>) -> Option<String>) -> Vec<String> {
    let mut finder = SiteFinder {
        pick,
        sites: Vec::new(),
    };
    syn::visit::Visit::visit_file(&mut finder, file);
    finder.sites
}

struct SiteFinder<F> {
    pick: F,
    sites: Vec<String>,
}

impl<'a, F: Fn(Site<'_>) -> Option<String>> syn::visit::Visit<'a> for SiteFinder<F> {
    fn visit_item_mod(&mut self, node: &'a syn::ItemMod) {
        if !has_test_cfg(&node.attrs) {
            syn::visit::visit_item_mod(self, node);
        }
    }
    fn visit_item_fn(&mut self, node: &'a syn::ItemFn) {
        if !has_test_cfg(&node.attrs) {
            syn::visit::visit_item_fn(self, node);
        }
    }
    fn visit_item_impl(&mut self, node: &'a syn::ItemImpl) {
        if !has_test_cfg(&node.attrs) {
            syn::visit::visit_item_impl(self, node);
        }
    }
    fn visit_impl_item_fn(&mut self, node: &'a syn::ImplItemFn) {
        if !has_test_cfg(&node.attrs) {
            syn::visit::visit_impl_item_fn(self, node);
        }
    }
    fn visit_expr_method_call(&mut self, node: &'a syn::ExprMethodCall) {
        self.sites.extend((self.pick)(Site::Method(node)));
        syn::visit::visit_expr_method_call(self, node);
    }
    fn visit_expr_call(&mut self, node: &'a syn::ExprCall) {
        self.sites.extend((self.pick)(Site::Function(node)));
        syn::visit::visit_expr_call(self, node);
    }
    fn visit_lit_str(&mut self, node: &'a syn::LitStr) {
        self.sites.extend((self.pick)(Site::Literal(node)));
    }
}
