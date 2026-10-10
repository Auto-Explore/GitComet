#!/usr/bin/env python3
"""Rows of one table exported from an Instruments trace, with id/ref resolved.

  xctrace_table.py TRACE.trace SCHEMA [--limit N]      prints JSON lines

`xcrun xctrace export` writes each distinct value once with an `id` and later
occurrences as `<tag ref="id"/>`. Each row becomes {column mnemonic: value},
where value is the element's `fmt` text plus its raw text for numbers.
"""
import argparse
import json
import subprocess
import xml.etree.ElementTree as ET


def export(trace, schema):
    xpath = f'/trace-toc/run[@number="1"]/data/table[@schema="{schema}"]'
    return subprocess.run(["xcrun", "xctrace", "export", "--input", str(trace), "--xpath", xpath],
                          check=True, capture_output=True).stdout


def rows(xml_bytes):
    root = ET.fromstring(xml_bytes)
    node = root.find("node")
    if node is None:
        return
    schema = node.find("schema")
    columns = [col.findtext("mnemonic") for col in schema.findall("col")]
    seen = {}

    def resolve(element):
        ref = element.get("ref")
        if ref is not None:
            return seen.get(ref)
        value = {"fmt": element.get("fmt"), "text": (element.text or "").strip() or None}
        # Nested values (a thread's process, a process's pid) are kept by tag.
        for child in element:
            value[child.tag] = resolve(child)
        if element.get("id") is not None:
            seen[element.get("id")] = value
        return value

    for row in node.findall("row"):
        cells = [resolve(child) for child in row]
        yield dict(zip(columns, cells))


def number(cell):
    if cell is None:
        return None
    try:
        return int(cell.get("text"))
    except (TypeError, ValueError):
        try:
            return float(cell.get("text"))
        except (TypeError, ValueError):
            return None


def pid(cell):
    """The pid inside a process/thread cell."""
    while isinstance(cell, dict):
        if "pid" in cell and isinstance(cell["pid"], dict):
            return number(cell["pid"])
        cell = cell.get("process")
    return None


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("trace")
    parser.add_argument("schema")
    parser.add_argument("--limit", type=int)
    args = parser.parse_args()
    for index, row in enumerate(rows(export(args.trace, args.schema))):
        if args.limit is not None and index >= args.limit:
            break
        print(json.dumps({key: (value or {}).get("fmt") for key, value in row.items()}))
