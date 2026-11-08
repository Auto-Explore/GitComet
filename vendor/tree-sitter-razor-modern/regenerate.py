from pathlib import Path
import os
import re
import shutil
import subprocess
import sys


root = Path(__file__).resolve().parent
cli = sys.argv[1] if len(sys.argv) == 2 else "tree-sitter"
version = subprocess.check_output([cli, "--version"], text=True).strip()
if version != "tree-sitter 0.26.5":
    raise SystemExit(f"Expected tree-sitter 0.26.5, found {version}")
subprocess.run(
    [cli, "generate", "--abi", "15"],
    cwd=root,
    env={**os.environ, "TS_SMALL_STATE_THRESHOLD": "128"},
    check=True,
)
parser = (root / "src/parser.c").read_text()
large_states = re.search(r"^#define LARGE_STATE_COUNT (\d+)$", parser, re.M)
if large_states is None or int(large_states[1]) > 2818:
    raise SystemExit("Expected the patched generator's threshold 128 retune; stock CLI is unsupported")
tables = re.findall(
    r"static const TSCharacterRange sym__identifier_token_character_set_\d+\[\] = \{(.*?)\n\};",
    parser,
    re.S,
)
if len(tables) != 2:
    raise SystemExit("Expected identifier start and continuation tables from the generated C# lexer")
parts = []
for name, table in zip(("start", "continue"), tables, strict=True):
    parts.append(f"static const TSCharacterRange razor_identifier_{name}_ranges[] = {{{table}\n}};\n")
(root / "src/identifier_ranges.h").write_text("\n".join(parts))
shutil.rmtree(root / "src/tree_sitter")
