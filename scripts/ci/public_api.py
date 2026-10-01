#!/usr/bin/env python3
"""Snapshot the reachable extension API from rustdoc's type-checked JSON.

The pinned Rust toolchain produces the JSON. Private bodies, docs, source
locations and rustdoc's allocation-order IDs are deliberately excluded.
Run with --update after reviewing an intentional contract change.
"""
import argparse
import difflib
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
SNAPSHOT = ROOT / "crates/gitcomet-extension-api/public-api.json"


def public_api(doc):
    index = doc["index"]
    paths = {str(key): "::".join(value["path"]) for key, value in doc["paths"].items()}
    exported = {}
    external_reexports = {}

    def visit(item_id, path, force=False):
        item = index.get(str(item_id))
        if item is None or (not force and item["visibility"] != "public"):
            return
        kind, body = next(iter(item["inner"].items()))
        if kind == "use":
            if body["id"] is not None:
                alias = path if body["is_glob"] else path + [body["name"]]
                if str(body["id"]) not in index:
                    external_reexports["::".join(alias)] = paths.get(str(body["id"]), body.get("source", "external"))
                else:
                    visit(body["id"], alias, True)
        elif kind == "module":
            for child in body["items"]:
                child_item = index[str(child)]
                child_kind = next(iter(child_item["inner"]))
                visit(child, path if child_kind == "use" else path + [child_item["name"]])
        else:
            exported["::".join(path)] = item
            paths[str(item_id)] = "::".join(path)

    root = index[str(doc["root"])]
    visit(doc["root"], [root["name"]], True)
    # Describe each item once; public re-exports point at its canonical name.
    # This also gives signatures stable type paths regardless of traversal order.
    canonical = {}
    for path, item in sorted(exported.items(), key=lambda entry: (
            entry[0].rsplit("::", 1)[-1] != entry[1]["name"],
            entry[0].count("::"), entry[0])):
        canonical.setdefault(str(item["id"]), path)
    paths.update(canonical)

    def normalize(value, key=None):
        if isinstance(value, dict):
            result = {}
            for name, child in sorted(value.items()):
                if name in {"impls", "implementations", "span", "docs", "links", "crate_id", "has_body", "default_unstable", "provided_trait_methods"}:
                    continue
                if name == "id":
                    if child is not None:
                        result["type_path"] = paths.get(str(child), index.get(str(child), {}).get("name", "external"))
                elif name in {"items", "fields", "variants"} and isinstance(child, list):
                    result[name] = [declaration(index[str(entry)]) if entry is not None else None for entry in child]
                elif name == "tuple" and isinstance(child, list) and all(entry is None or isinstance(entry, int) for entry in child):
                    result[name] = [declaration(index[str(entry)]) if entry is not None else None for entry in child]
                else:
                    result[name] = normalize(child, name)
            return result
        if isinstance(value, list):
            return [normalize(child) for child in value]
        return value

    def declaration(item):
        return {"name": item["name"], "attrs": normalize(item.get("attrs", [])),
                "deprecation": item.get("deprecation"), "definition": normalize(item["inner"])}

    result = {path: {"reexport": target} for path, target in external_reexports.items()}
    for path, item in sorted(exported.items()):
        target = canonical[str(item["id"])]
        if target != path:
            result[path] = {"reexport": target}
            continue
        entry = declaration(item)
        body = next(iter(item["inner"].values()))
        implementations = []
        if isinstance(body, dict):
            for impl_id in body.get("impls", []):
                implementation = index[str(impl_id)]["inner"]["impl"]
                if implementation["is_synthetic"] or implementation["blanket_impl"] is not None:
                    continue
                public_items = [child for child in implementation["items"]
                                if implementation["trait"] is not None or index[str(child)]["visibility"] == "public"]
                if implementation["trait"] is None and not public_items:
                    continue
                if implementation["trait"] is not None:
                    # Trait methods inherit their signatures from the trait.
                    # Record the implementation, bounds and associated values,
                    # without repeating Debug/Clone/etc. methods on every type.
                    public_items = [child for child in public_items
                                    if "function" not in index[str(child)]["inner"]]
                implementations.append(normalize({**implementation, "items": public_items}))
        if implementations:
            entry["impls"] = sorted(implementations, key=lambda value: json.dumps(value, sort_keys=True))
        result[path] = entry
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--update", action="store_true")
    parser.add_argument("--json", type=Path, help="Use already generated rustdoc JSON")
    args = parser.parse_args()
    source = args.json
    if source is None:
        env = {**os.environ, "RUSTC_BOOTSTRAP": "gitcomet_extension_api"}
        subprocess.run(["cargo", "rustdoc", "--locked", "-p", "gitcomet-extension-api", "--lib", "--",
                        "-Z", "unstable-options", "--output-format", "json"], cwd=ROOT, env=env, check=True)
        metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], cwd=ROOT))
        source = Path(metadata["target_directory"]) / "doc/gitcomet_extension_api.json"
    actual = json.dumps(public_api(json.loads(source.read_text())), indent=2, ensure_ascii=False, sort_keys=True) + "\n"
    if args.update:
        SNAPSHOT.write_text(actual)
        print(f"Updated {SNAPSHOT.relative_to(ROOT)}")
        return 0
    expected = SNAPSHOT.read_text() if SNAPSHOT.exists() else ""
    if expected == actual:
        print("Extension public API matches the snapshot")
        return 0
    sys.stdout.writelines(difflib.unified_diff(expected.splitlines(True), actual.splitlines(True), fromfile="public-api.json", tofile="current API"))
    print("Review the contract change, then run python3 scripts/ci/public_api.py --update", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
