#!/usr/bin/env python3
"""Extract a revision and add the same diagnostic witnesses as the candidate.

This intentionally does not copy search, scheduling, notification or cache
optimizations. Record the archive revision and script hash with measurements.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[2]
UI = Path("crates/gitcomet-ui-gpui/src")


def instrument(destination):
    def edit(relative, before, after):
        path = destination / UI / relative
        source = path.read_text(encoding="utf-8")
        if source.count(before) != 1:
            raise ValueError(f"Baseline source changed: {relative}: {before!r}")
        path.write_text(source.replace(before, after), encoding="utf-8", newline="\n")
    for relative in ("ui_probe.rs", "kit/click.rs", "kit/text_input/render.rs"):
        (destination / UI / relative).write_bytes((ROOT / UI / relative).read_bytes())
    driver = Path("crates/gitcomet-git-gix/examples/interaction-probe.rs")
    (destination / driver).write_bytes((ROOT / driver).read_bytes())
    edit("kit/text_input/state.rs", "pub struct TextInput {", "pub struct TextInput {\n    pub(super) probe_action: u64,")
    edit("kit/text_input/editing.rs", "            focus_handle,", "            focus_handle,\n            probe_action: 0,")
    edit("kit/text_input/editing.rs", "        let inserted = self.content.replace_range(range, new_text);", """        self.probe_action = crate::ui_probe::begin_action("typing");
        let inserted = self.content.replace_range(range, new_text);
        crate::ui_probe::action_phase(self.probe_action, "applied", || {
            let snapshot = self.content.snapshot();
            serde_json::json!({"model":snapshot.model_id(), "revision":snapshot.revision(), "bytes":snapshot.len()})
        });""")
    for name in ("diff_search_probe_action", "diff_search_probe_render"):
        edit("view/panes/main/helpers.rs", "    pub(in crate::view) diff_search_debounce_seq: u64,",
             "    pub(in crate::view) diff_search_debounce_seq: u64,\n    pub(in crate::view) " + name + ": u64,")
        edit("view/panes/main/core_impl/init.rs", "            diff_search_debounce_seq: 0,",
             "            diff_search_debounce_seq: 0,\n            " + name + ": 0,")
    edit("view/panes/main/diff_search.rs", "        let seq = self.diff_search_debounce_seq;", """        self.diff_search_probe_action = crate::ui_probe::begin_action("diff_search");
        let seq = self.diff_search_debounce_seq;""")
    edit("view/panes/main/diff_search.rs", "        self.diff_search_recompute_matches_for_query_change(previous_query.as_ref());", """        self.diff_search_recompute_matches_for_query_change(previous_query.as_ref());
        self.diff_search_probe_render = self.diff_search_probe_action;
        crate::ui_probe::action_phase(self.diff_search_probe_action, "applied", || serde_json::json!({"revision": self.diff_search_debounce_seq, "matches": self.diff_search_matches.len(),
            "query_bytes":self.diff_search_query.len(), "target":self.rendered_diff_target().map(|target| format!("{target:?}"))}));""")
    candidate = (ROOT / UI / "view/panes/main.rs").read_text(encoding="utf-8")
    start = candidate.index("        let search_action =")
    end = candidate.index("        // The historical-browse", start)
    marker = "        // The historical-browse"
    edit("view/panes/main.rs", marker, candidate[start:end] + marker)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    revision = subprocess.check_output(["git", "rev-parse", args.revision], cwd=ROOT, text=True).strip()
    destination = args.output.resolve()
    destination.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory() as temp:
        archive = Path(temp) / "source.zip"
        subprocess.run(["git", "archive", "--format=zip", "--output", str(archive), revision], cwd=ROOT, check=True)
        with zipfile.ZipFile(archive) as source:
            if not all((destination / name).resolve().is_relative_to(destination) for name in source.namelist()):
                raise ValueError("Archive path outside destination")
            source.extractall(destination)
    instrument(destination)
    (destination / "measurement-source.json").write_text(json.dumps({"revision": revision,
        "instrumentation_script_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}, indent=2) + "\n")


if __name__ == "__main__":
    main()
