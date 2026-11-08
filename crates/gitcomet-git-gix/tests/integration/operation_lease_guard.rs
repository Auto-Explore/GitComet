use std::collections::BTreeSet;
use syn::{ImplItem, Item, ReturnType, Type};

fn returns_result(signature: &syn::Signature) -> bool {
    matches!(
        &signature.output,
        ReturnType::Type(_, result)
            if matches!(result.as_ref(), Type::Path(path)
                if path.path.segments.last().is_some_and(|segment| segment.ident == "Result"))
    )
}

fn has_first_operation_lease(method: &syn::ImplItemFn) -> bool {
    let Some(syn::Stmt::Local(local)) = method.block.stmts.first() else {
        return false;
    };
    if !matches!(&local.pat, syn::Pat::Ident(_)) {
        return false;
    }
    let Some(init) = &local.init else {
        return false;
    };
    let syn::Expr::Try(guard) = init.expr.as_ref() else {
        return false;
    };
    matches!(
        guard.expr.as_ref(),
        syn::Expr::MethodCall(call)
            if call.method == "operation_lease"
                && call.args.is_empty()
                && matches!(call.receiver.as_ref(), syn::Expr::Path(path) if path.path.is_ident("self"))
    )
}

#[test]
fn every_repository_result_override_holds_an_operation_lease() {
    let source = include_str!("../../src/repo/mod.rs");
    let parsed = syn::parse_file(source).unwrap();
    let implementation = parsed
        .items
        .iter()
        .find_map(|item| match item {
            Item::Impl(item)
                if item
                    .trait_
                    .as_ref()
                    .is_some_and(|(path, _)| path.is_ident("GitRepository")) =>
            {
                Some(item)
            }
            _ => None,
        })
        .unwrap();
    let exclusive = BTreeSet::from([
        "disable_bare_mode".to_string(),
        "disable_bare_mode_cancellable".to_string(),
    ]);
    let mut missing = Vec::new();
    let mut implemented = BTreeSet::new();
    for item in &implementation.items {
        let ImplItem::Fn(method) = item else {
            continue;
        };
        if !returns_result(&method.sig) {
            continue;
        }
        let name = method.sig.ident.to_string();
        if !exclusive.contains(&name) && !has_first_operation_lease(method) {
            missing.push(name.clone());
        }
        implemented.insert(name);
    }
    assert!(
        missing.is_empty(),
        "repository calls without a lease: {missing:?}"
    );
    assert!(exclusive.is_subset(&implemented));

    let contract = syn::parse_file(include_str!("../../../gitcomet-core/src/services.rs")).unwrap();
    let methods = contract
        .items
        .iter()
        .find_map(|item| match item {
            Item::Trait(item) if item.ident == "GitRepository" => Some(&item.items),
            _ => None,
        })
        .unwrap();
    let inherited = methods
        .iter()
        .filter_map(|item| {
            let syn::TraitItem::Fn(method) = item else {
                return None;
            };
            let name = method.sig.ident.to_string();
            (returns_result(&method.sig) && !implemented.contains(&name)).then_some(name)
        })
        .collect::<Vec<_>>();
    assert!(
        inherited.is_empty(),
        "repository calls without a Gix lease owner: {inherited:?}"
    );
}

#[test]
fn operation_inventory_rejects_missing_or_late_leases() {
    for body in [
        "self.read()",
        "self.read()?; let _operation = self.operation_lease()?; Ok(())",
        "self.operation_lease()?; self.read()",
        "let _ = self.operation_lease()?; self.read()",
        "let _operation = other.operation_lease()?; self.read()",
    ] {
        let method: syn::ImplItemFn =
            syn::parse_str(&format!("fn read(&self) -> Result<()> {{ {body} }}")).unwrap();
        assert!(!has_first_operation_lease(&method), "{body}");
    }
    let method: syn::ImplItemFn = syn::parse_str(
        "fn read(&self) -> Result<()> { let _operation = self.operation_lease()?; self.read() }",
    )
    .unwrap();
    assert!(has_first_operation_lease(&method));
}
