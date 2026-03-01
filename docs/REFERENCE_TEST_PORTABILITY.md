# Reference Implementation Test Portability Plan

Analysis of tests from git, meld, and kdiff3 that can be reimplemented in
gitgpui to verify merge/conflict resolution correctness.

Sources analyzed:

- git: `/home/sampo/git/git/t/t6403-merge-file.sh`, `t6427-diff3-conflict-markers.sh`, `t7610-mergetool.sh`
- meld: `/home/sampo/git/meld/test/test_matchers.py`, `meld/matchers/merge.py`, `test/test_misc.py`
- kdiff3: `/home/sampo/git/kdiff3/test/alignmenttest.cpp`, `test/testdata/`, `src/autotests/DiffTest.cpp`, `src/autotests/Diff3LineTest.cpp`, `test/generate_testdata_from_permutations.py`, `test/generate_testdata_from_git_merges.py`

---

## Phase 1 — Core 3-Way Merge Algorithm (P0)

### 1A. Port git `t6403-merge-file.sh` (23 of 37 tests are algorithm-focused)

Create a new test module (e.g. `crates/gitgpui-core/tests/merge_algorithm.rs` or
wherever the merge implementation lives) containing these test groups:

#### Identity and clean merge

```
merge_identity:
  input:  base="Psalm 23 Latin text", local=base, remote=base
  expect: output == base, exit 0

merge_nonoverlapping_clean:
  input:  base="Psalm 23 Latin text"
          local=base + 3 appended lines
          remote=first two lines joined into one
  expect: both changes applied, exit 0
```

#### Conflict detection and marker format

```
merge_overlapping_conflict:
  input:  base="Psalm 23 Latin text"
          local=first two lines joined
          remote=first word uppercased ("DOMINUS")
  expect: exit != 0, output contains <<<<<<< / ======= / >>>>>>>
          local section: joined line
          remote section: uppercased + original line 2

merge_conflict_markers_with_labels:
  input:  same conflict, labels -L "1" -L "2" (third defaults to filename)
  expect: <<<<<<< 1 / >>>>>>> <filename>

merge_delete_vs_modify_conflict:
  input:  base has 3 appended lines
          local deletes them
          remote modifies "tu" -> "TU"
  expect: conflict with empty local section, remote has modified lines
```

#### Conflict resolution strategies

```
merge_ours:       same conflict -> resolved with local side, exit 0
merge_theirs:     same conflict -> resolved with remote side, exit 0
merge_union:      same conflict -> both sides concatenated, no markers, exit 0
merge_ours_eof:   conflict at EOF without trailing LF -> local chosen
merge_theirs_eof: conflict at EOF without trailing LF -> remote chosen
merge_union_eof:  conflict at EOF without trailing LF -> both, newline between
```

#### Trailing newline / EOF edge cases

```
merge_missing_lf_at_eof:
  input:  remote lacks trailing LF, change is at head
  expect: merge succeeds (git marks this as expected-failure — we can do better)

merge_preserves_missing_lf:
  input:  local lacks trailing LF, change in remote is far from EOF
  expect: output preserves absence of trailing LF

merge_no_spurious_lf:
  input:  same as above
  expect: output ends without newline (byte-level check)
```

#### CRLF handling

```
merge_crlf_conflict_markers:
  input:  CRLF files base="1\r\n2\r\n3", local="1\r\n2\r\n4", remote="1\r\n2\r\n5"
  expect: conflict markers also use \r\n line endings

  re-run with LF-only files:
  expect: conflict markers use \n line endings
```

#### Zealous merge optimization

```
merge_zealous_coalesces_adjacent:
  input:  two adjacent conflicting regions
  expect: single ======= marker (conflicts coalesced)

merge_zealous_alnum_coalesces_across_blank:
  input:  two conflicting regions separated by blank lines only
  expect: single ======= marker
```

#### diff3 conflict style

```
merge_diff3_output:
  input:  same zealous conflict scenario
  expect: output includes ||||||| section showing base content
```

#### Configurable marker width

```
merge_marker_size_10:
  input:  same conflict, marker-size=10
  expect: markers are 10 chars wide (<<<<<<<<<< / ========== / >>>>>>>>>>)
```

#### Diff algorithm impact

```
merge_myers_spurious_conflict:
  input:  base.c with f() and g()
          ours.c deletes f(), adds h()
          theirs.c modifies g() body
  expect: with Myers: spurious conflicts in h() body

merge_histogram_clean:
  input:  same files
  expect: with histogram: clean merge, exit 0
```

#### Binary detection

```
merge_binary_rejected:
  input:  one of the three files is a PNG
  expect: error indicating binary files cannot be merged
```

### 1B. Port git `t6427` zdiff3 tests (4 sub-cases)

These are the single highest-value portable tests — pure text in, text out:

```
zdiff3_basic:
  base:   1 2 3 4 5 6 7 8 9
  left:   1 2 3 4 A B C D E 7 8 9
  right:  1 2 3 4 A X C Y E 7 8 9
  expect: common prefix "A" and suffix "E" extracted outside markers
          conflict contains only B/C/D vs X/C/Y

zdiff3_middle_common:
  two disjoint change regions with common material ("4 5") between
  expect: two separate conflict hunks, common "4 5" preserved outside markers

zdiff3_interesting:
  left adds D/E/F then G/H/I/J; right adds 5/6 then G/H/I/J
  expect: common prefix "A B C" and suffix "G H I J" extracted
          conflict region is D/E/F vs 5/6

zdiff3_evil:
  tricky case with common trailing "B C"
  expect: "B C" pulled out as common suffix after conflict block
```

### 1C. Conflict marker label formatting

Test that labels are generated correctly for different merge-base scenarios:

```
label_no_base:             "empty tree"
label_unique_base:         "<short-sha>:<path>"
label_unique_base_rename:  "<short-sha>:<original-path>"
label_merged_ancestors:    "merged common ancestors:<path>"
label_rebase_parent:       "parent of <desc>"
```

---

## Phase 2 — KDiff3-Style Fixture Harness (P0)

### 2A. Fixture format

Adopt the KDiff3 naming convention for test data files:

```
tests/fixtures/merge/
  1_simpletest_base.txt
  1_simpletest_contrib1.txt
  1_simpletest_contrib2.txt
  1_simpletest_expected_result.txt
  2_prefer_identical_base.txt
  ...
```

Expected result format: one row per visual line, three space-separated integers
(line index in base/contrib1/contrib2, -1 for gaps).

### 2B. Test runner

Write a Rust test that:

1. Auto-discovers all `*_base.*` files in the fixtures directory.
2. For each fixture, loads base, contrib1, contrib2.
3. Runs the merge/alignment algorithm.
4. Applies two algorithm-independent invariant checks:
   - **Sequence monotonicity**: line numbers in each column are strictly increasing.
   - **Consistency**: when lines are marked "equal", actual content matches.
5. Compares output against expected result (when present).
6. On failure, writes `*_actual_result.*` for manual comparison.

### 2C. Seed test cases

Port the two existing KDiff3 hand-crafted cases:

```
1_simpletest:
  base:     "same everywhere"
  contrib1: "same in b and c" / "only in b" / "again same in b and c" /
            "same in b and c except for space" / "same everywhere"
  contrib2: "same in b and c" / "again same in b and c" /
            "same  in b and c except for space" / "same everywhere"
  tests:    insertions, deletions, whitespace differences, lines common to all

2_prefer_identical_to_space_differences:
  base:     "aaa"
  contrib1: "bbb" / "    aaa"
  contrib2: "aaa" / "    aaa"
  tests:    algorithm should prefer exact match over whitespace-different match
```

---

## Phase 3 — Permutation Corpus (P1)

### 3A. Port the KDiff3 permutation option table

The 11 options per line cover all meaningful 3-way merge scenarios:

| # | Base    | Contrib1  | Contrib2  | Description                         |
|---|---------|-----------|-----------|-------------------------------------|
| 1 | original | original | original | unchanged everywhere                |
| 2 | original | original | modified | changed in contrib2 only            |
| 3 | original | original | deleted  | deleted in contrib2 only            |
| 4 | original | modified | original | changed in contrib1 only            |
| 5 | original | modified | same-mod | both changed identically            |
| 6 | original | modified | diff-mod | both changed differently (conflict) |
| 7 | original | modified | deleted  | modify vs delete                    |
| 8 | absent  | added    | added    | added identically in both           |
| 9 | absent  | added    | diff-add | added differently (conflict)        |
| 10| absent  | added    | absent   | added in contrib1 only              |
| 11| absent  | absent   | added    | added in contrib2 only              |

With 5 default lines and all 11 options: 11^5 = 161,051 test cases (full),
or use `-r 3` for 3^5 = 243 sampled cases.

### 3B. Implementation approach

Either:
- Port the Python generator to a Rust `build.rs` or `#[test]` that generates
  fixtures at test time (preferred — no file system bloat).
- Or generate once and commit a subset as golden files.

For each generated case, validate the two algorithm-independent invariants
(sequence monotonicity + consistency). Expected alignment results are
algorithm-specific and must be generated by the tool under test.

### 3C. Real-world merge extraction

Port `generate_testdata_from_git_merges.py` concept:

- Walk merge commits in any git repo.
- For each merge with 2 parents, find merge-base, extract base/contrib1/contrib2.
- Skip trivial merges (base == either contrib, or contribs identical).
- Use extracted files as regression test data.

Can run against gitgpui's own repo or linux kernel for diverse scenarios.

---

## Phase 4 — Mergetool / Difftool E2E (P1)

### 4A. Port critical t7610-mergetool.sh scenarios

These require spawning `git mergetool` with gitgpui configured as the tool.

#### trustExitCode behavior (partially covered — extend)

Already tested: boolean parsing in `mergetool.rs` (9 tests).

Add integration tests:
```
mergetool_trust_exit_0:   tool exits 0 with trustExitCode=true -> file resolved
mergetool_trust_exit_1:   tool exits 1 with trustExitCode=true -> file unresolved
mergetool_no_trust:       tool exits 0 with trustExitCode=false -> git prompts
```

#### Custom tool invocation

```
mergetool_custom_cmd:     cat "$REMOTE" > "$MERGED" resolves conflict
mergetool_gui_tool:       --gui selects merge.guitool over merge.tool
mergetool_gui_fallback:   --gui with no guitool falls back to merge.tool
mergetool_nonexistent:    --tool=absent -> error "cmd not set for tool"
mergetool_tool_help:      --tool-help lists known tools
```

#### Edge cases

```
mergetool_spaced_path:    file "spaced name" is passed correctly to tool
mergetool_subdirectory:   invocation from subdirectory resolves paths correctly
mergetool_write_to_temp:  writeToTemp=true uses temp dir for stage files
mergetool_path_prefix:    filenames start with "./" when writeToTemp=false
mergetool_order_file:     diff.orderFile controls tool invocation order
```

#### Delete/delete conflicts

```
mergetool_delete_delete_d: answer "d" -> file deleted
mergetool_delete_delete_m: answer "m" -> file at new location
mergetool_delete_delete_a: answer "a" (abort) -> non-zero exit
mergetool_delete_keep_backup: keepBackup=true, answer "d" -> no stderr errors
mergetool_delete_keep_temps: keepTemporaries=true, abort -> temp files remain
```

#### Submodule conflicts

```
mergetool_submod_local:       answer "l" -> keep local submodule commit
mergetool_submod_remote:      answer "r" -> keep remote submodule commit
mergetool_deleted_submod:     one side deletes, other modifies -> d/r/l choices
mergetool_file_vs_submod:     one side replaces submod with file -> conflict files
mergetool_dir_vs_submod:      one side replaces submod with directory
mergetool_submod_in_subdir:   submodule inside subdirectory
```

#### No-base conflicts

```
mergetool_no_base_file:   both-added file, tool receives empty $BASE
```

### 4B. Port critical t7800-difftool.sh scenarios

```
difftool_basic:           git difftool opens with LOCAL and REMOTE set
difftool_trust_exit:      --trust-exit-code honored
difftool_gui_default:     difftool.guiDefault with auto + DISPLAY detection
difftool_dir_diff:        --dir-diff mode
difftool_spaced_path:     file paths with spaces
difftool_subdirectory:    invocation from repo subdirectory
```

---

## Phase 5 — Meld-Derived Algorithm Tests (P2)

### 5A. Myers matching blocks

Port the 6 test cases from `test_matchers.py` as unit tests for whatever diff
engine gitgpui uses. Input/output pairs:

```
myers_basic:
  a = "abcbdefgabcdefg" (15 chars)
  b = "gfabcdefcd" (10 chars)
  expect blocks: [(0,2,3), (4,5,3), (10,8,2)]

myers_postprocess:
  a = "abcfabgcd"
  b = "afabcgabgcabcd"
  expect blocks: [(0,2,3), (4,6,3), (7,12,2)]

myers_inline_trigram:
  a = "red, blue, yellow, white"
  b = "black green, hue, white"
  expect blocks: [(17,16,7)]

sync_point_none:  no sync points -> same as basic
sync_point_one:   sync at (3,6) -> forces different alignment
sync_point_two:   syncs at (3,2) and (8,6)
```

### 5B. Interval merging

Port 6 cases for chunk coalescing utility:

```
intervals_dominated:     [(1,5),(5,9),(10,11),(0,20)] -> [(0,20)]
intervals_disjoint:      [(1,5),(6,9),(10,11)] -> unchanged
intervals_two_groups:    [(1,5),(5,9),(10,12),(11,20)] -> [(1,9),(10,20)]
intervals_unsorted:      same as above but unsorted input
intervals_duplicate:     [(1,5),(7,8),(1,5)] -> [(1,5),(7,8)]
intervals_chain:         [(1,5),(4,10),(9,15)] -> [(1,15)]
```

### 5C. Newline-aware operations

Port the concept (not GTK impl) from `test_chunk_actions.py`:

```
delete_last_line_crlf:      "ree\r\neee"    -> "ree"
delete_last_line_crlf_trail: "ree\r\neee\r\n" -> "ree\r\neee"
delete_last_line_lf:        "ree\neee"      -> "ree"
delete_last_line_lf_trail:  "ree\neee\n"    -> "ree\neee"
delete_last_line_cr:        "ree\reee"      -> "ree"
delete_last_line_cr_trail:  "ree\reee\r"    -> "ree\reee"
delete_last_line_mixed:     "ree\r\neee\nqqq" -> "ree\r\neee"
```

---

## Existing GitGpui Coverage (No Action Needed)

These areas are already well-tested:

- trustExitCode boolean parsing: 9 tests in `mergetool.rs`
- All conflict stage shapes (BothDeleted, AddedByUs/Them, DeletedByThem/Us): `status_integration.rs`
- Conflict side checkout resolution: `status_integration.rs`
- Binary conflict handling: `status_integration.rs` + `conflict_base_checkout_integration.rs`
- Modify-delete and add-add detection: `status_integration.rs`
- Conflict session management (regions, bulk choice, autosolve): 23 tests in `conflict_session.rs`
- Conflict base checkout: `conflict_base_checkout_integration.rs`

---

## Implementation Priority Summary

| Phase | Effort | Tests Added | Value |
|-------|--------|-------------|-------|
| 1A: t6403 merge-file core | Medium | ~23 | Defines correctness contract |
| 1B: t6427 zdiff3 | Small | 4 | Highest value-per-test ratio |
| 1C: Label formatting | Small | 5 | Conflict marker correctness |
| 2: Fixture harness | Medium | 2 + framework | Long-term regression protection |
| 3A: Permutation corpus | Medium | 243-161K | Exhaustive edge case coverage |
| 3C: Git merge extraction | Small | Variable | Real-world regression data |
| 4A: Mergetool E2E | Large | ~20 | Integration contract with git |
| 4B: Difftool E2E | Medium | ~6 | Integration contract with git |
| 5: Meld algorithm | Small | ~19 | Diff engine verification |
