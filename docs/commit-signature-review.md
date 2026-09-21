Commit signature review, 2026-09-11

The review was checked against the worktree, including the staged fixes from the previous pass. Existing fixes were retained. Remaining failures were reproduced in tests before their fixes. The following table maps all 15 findings to the resulting behavior and regression coverage.

Tests live in [core signature tests](../crates/gitcomet-core/tests/commit_signatures.rs), [backend integration tests](../crates/gitcomet-git-gix/tests/commit_signature_integration.rs), [backend cache/parser tests](../crates/gitcomet-git-gix/src/repo/signatures.rs), [state signature tests](../crates/gitcomet-state/src/store/tests/commit_signatures.rs), and the UI test modules beside the affected views.

| # | Finding and disposition | Regression coverage |
| --- | --- | --- |
| 1 | Confirmed. `U` now has an **Untrusted key** warning badge and does not satisfy `is_verified()`. | Core identity-status test; real SSH commit with empty allowed signers; badge label, icon and palette assertions. |
| 2 | Confirmed. SSH `B` requires an actual verifier rejection before displaying a bad-signature badge. Missing or unsupported helpers yield no badge. | Missing executable, `/bin/false`, and signing-only helper tests; genuine tampered and revoked signatures still report bad. |
| 3 | Confirmed. Armor detection now uses raw prefixes and includes `PGP MESSAGE`. | All four accepted armor prefixes, suffixes, and rejected leading spaces/newlines. |
| 4 | Already fixed in the staged work. Signed verdicts are rechecked on refresh, including unchanged history. Metadata caching cannot preserve an old trust verdict. | Trust added, trust removed, revocation-file contents changed, refresh and preference-toggle regressions. |
| 5 | Already fixed. Disabled and obsolete-epoch replies are discarded; disabling also cancels running verification. | Replies after disabling, after re-enabling, and after refresh. |
| 6 | Already fixed. Successful reveal resolution schedules verification for the full commit ID even outside the log. | Reveal-resolution tests with verification enabled and disabled. |
| 7 | Confirmed. Each repository has one running batch of at most 16 commits. Requests are deduplicated, completed no-badge commits are remembered until refresh, and pagination submits only newly appended IDs. Verification has a dedicated executor. | Repeated-details deduplication, no-badge memoization, 5,000-commit bounded scheduling, queue draining, and primary-worker isolation tests. |
| 8 | Unchanged-history retry was already fixed. Verification now has its own cancellation lifetime, so staging and tab switches cannot silently drop replies through the repository-load guard. Closing repositories and shutting down the store cancel signature work. | Repository-load cancellation while primary workers are occupied; single/bulk close cancellation; real running-verifier cancellation. |
| 9 | Oversized-batch failure confirmed. The process helper already retained partial stdout in its error; verification now consumes it. Backend calls are divided into small batches, and unfinished commits receive bounded individual retries. | An 80-commit slow-verifier test that previously hit the ten-second timeout; complete/incomplete output-record tests. |
| 10 | Confirmed, including the staged cache-hit counting regression. A bounded LRU evicts cold metadata entries individually. State memoization prevents repeated unsigned/unverifiable work on every page append. | A 3,000-entry cached page must retain its hits; hot entries survive overflow; completed no-badge results are not resubmitted. |
| 11 | Confirmed. Verification explicitly requests UTF-8 output. | Real SSH verification with `i18n.logOutputEncoding=UTF-16`. |
| 12 | Already fixed. Wheel scrolling, programmatic scrolling, list replacement and empty lists retract the owning tooltip. | Five visual tests covering date and signature tooltips and removal of the owning row. |
| 13 | Already fixed by invalidating hover ownership when the presented row mapping changes. Reusing the same list index cannot retain the old commit's text. | Replacement at the same row index retracts the old signature tooltip before any mouse movement. |
| 14 | Confirmed. Signature revisions repaint history independently of the content fingerprint that dismisses refs menus. | An open refs card and item menu survive a signature update, then close on a real history-page change. |
| 15 | Confirmed. Git Log search includes the setting's label and verification terms. | Searches for `signature`, `Verify commit signatures`, and `verification`. |

The trust-status and armor rules were cross-checked against [Git's pretty-format documentation](https://git-scm.com/docs/pretty-formats) and [Git 2.55's verifier implementation](https://github.com/git/git/blob/v2.55.0/gpg-interface.c). For SSH, Git initializes the result to `B` before interpreting the helper's output; the exit status of `git log` alone therefore cannot distinguish verifier failure from signature rejection.

The additional observations were checked too:

- The two weakened effect assertions now check the exact expected effect sequence.
- The history-append fixture keeps its zero-follow-up-effect expectation; bounded scheduling no longer starts an overlapping verification batch. Its existing structural regression test passes.
- Leaving a revealed commit prunes its off-page badge; late results cannot restore it. Revisiting that commit can request its discarded badge again.
- Row hover checks the tooltip host's actual text, so an external clear cannot leave the row mirror suppressing a new tooltip.
- Removing or replacing a signature badge clears its previous verdict tooltip even under a resting pointer.
- Signature snapshots share 64 map buckets. Updating one badge copies only affected buckets, while prior snapshots remain unchanged. The regression test originally measured 1,024 cloned signatures and now requires fewer than 64.
- Repeating the three `GitLogSettings` fields in the message is an API design preference, not a demonstrated correctness defect. The message shape was retained.
- The redundant backend format lookup was removed during record parsing changes.
- The bad-signature tooltip reads `Bad GPG signature`.
- The details icon follows UI scale; the visual test checks a change from 12 to 24 pixels.
- Cache coverage now verifies actual cache reuse and eviction behavior, so removing the cache fails tests.

Signed verdicts intentionally remain fresh per backend request. Repeated work is avoided in state, which remembers queued, running and completed attempts—including `E`/`N` results—until a refresh invalidates that epoch. This preserves both trust updates and pagination performance without attempting to fingerprint arbitrary keyrings, verifier programs and trust files.

The optional benchmark suite has existing failures outside this review. The serial `view::rows::benchmarks::tests` run produced 213 passes and 24 failures in an isolated snapshot of the starting worktree, and 214 passes and 23 failures after these changes. The only changed outcome was `history_load_more_append_fixture_appends_requested_page`, which now passes. The other failure names were identical; no assertions or performance budgets were relaxed. The separate `real_repo_large_diff_fixture_paints_first_window` benchmark fixture also fails the same byte-count assertion in both snapshots.

Validation commands and results:

| Command | Result |
| --- | --- |
| `cargo test -p gitcomet-core --lib --test commit_signatures` | 552 passed; 2 ignored. |
| `cargo test -p gitcomet-state` | 918 passed, including 901 unit tests and 17 integration tests. |
| `env GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null cargo test -p gitcomet-git-gix --lib --test commit_signature_integration` | 294 passed; 5 ignored. Git configuration was isolated from the user's automatic commit signing; socket-based tests ran outside the sandbox. |
| `cargo test -p gitcomet-ui-gpui --lib` | 3,766 passed; 5 ignored. |
| `cargo clippy -p gitcomet-core -p gitcomet-state -p gitcomet-git-gix -p gitcomet-ui-gpui -- -D warnings` | Passed. |
| `cargo fmt --all -- --check` and `git diff --check` | Passed. |
