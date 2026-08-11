# Testing SSH Commit Signing on Windows

How to validate the SSH signing passphrase fix on a native Windows machine.

Source of truth:

- `crates/gitcomet-core/src/auth.rs` — the askpass helper scripts and the shared
  `SSH_PASSPHRASE_PROMPT_MARKER`
- `crates/gitcomet-git-gix/src/util.rs` — `command_may_require_auth`,
  `append_passphrase_prompt_to_stderr`
- `crates/gitcomet-state/src/store/reducer/util.rs` —
  `detect_auth_prompt_kind_from_message`

## What is being tested and why Windows specifically

With `gpg.format = ssh`, Git signs by running `ssh-keygen -Y sign`, which asks
for the signing key's passphrase through `SSH_ASKPASS`. GitComet answers that
with a generated helper script. On Windows that helper is a **batch file**, and
batch has expansion rules that cannot be verified by reading the Rust that emits
it:

- `echo %VAR%` with `VAR` unset collapses to a bare `echo`, which prints
  `ECHO is on.`. OpenSSH took that as the passphrase, so signing failed with
  `incorrect passphrase supplied to decrypt private key` even though the user was
  never asked for one.
- `%VAR%` substitutes before parsing, so `&`, `|`, `<`, `>` inside a passphrase
  were parsed as batch operators and truncated the answer.

Both are fixed by emitting every value as `echo(!VAR!`. The Linux and macOS
helpers are POSIX `sh` and were never affected, so **this behaviour has no
coverage anywhere except a Windows host**.

## Layer 1 — Automated

`crates/gitcomet-core/tests/askpass_script_windows.rs` writes the real helper to
disk and executes it through `cmd.exe`. The file is `#![cfg(windows)]`, so it is
inert elsewhere and runs automatically here:

```powershell
cargo test --workspace --no-default-features --features gix
```

To run just this file:

```powershell
cargo test -p gitcomet-core --test askpass_script_windows -- --nocapture
```

Expect 8 passing tests. What each one protects:

| Test | Protects against |
| --- | --- |
| `no_staged_secret_answers_with_an_empty_line` | The reported bug: `ECHO is on.` leaking in as the passphrase |
| `staged_passphrase_is_answered_verbatim` | The retry after the user types their passphrase |
| `secret_with_shell_metacharacters_is_answered_verbatim` | `&`, `\|`, `<`, `>`, `^`, `()` truncating the answer |
| `secret_with_exclamation_marks_is_answered_verbatim` | `!` being eaten by delayed expansion |
| `ssh_keygen_passphrase_prompt_is_logged` | The prompt log the failure classifier depends on |
| `host_verification_prompt_is_logged_and_answered` | Regression: the yes/no host key prompt |
| `username_password_answers_each_prompt_with_its_own_value` | Regression: HTTPS credential prompts |
| `cached_passphrase_answers_only_its_own_prompt` | Session caching, and not answering the wrong key |

**If only `secret_with_exclamation_marks_is_answered_verbatim` fails**, that is a
known-risk case rather than a broken fix: `!` is the one character delayed
expansion itself treats specially. The remedy is to `endlocal` (or
`setlocal DisableDelayedExpansion`) immediately before the final `echo` in
`ASKPASS_SCRIPT_WINDOWS`. Report the observed output rather than working around
it locally.

Also run the cross-platform suites, which cover the classifier and the wiring:

```powershell
cargo test -p gitcomet-state --lib auth_prompt
cargo test -p gitcomet-git-gix --lib util::
cargo test -p gitcomet-git-gix --test ssh_signing_passphrase_integration
```

The last one needs `ssh-keygen` on `PATH`; it self-skips if absent, so confirm it
reports 2 passing tests rather than a skip message.

## Layer 2 — Confirm the bug shape from the CLI

This reproduces the reporter's environment without involving GitComet, and
proves the machine is actually capable of hitting the bug.

```powershell
# A throwaway signing key with a passphrase
ssh-keygen -t ed25519 -C "gitcomet signing test" -f "$env:USERPROFILE\.ssh\gctest"
# ...enter a passphrase when prompted, twice

# A throwaway repo configured to sign
mkdir $env:TEMP\gcsign; cd $env:TEMP\gcsign
git init
git config user.name "Test"; git config user.email "test@example.com"
git config gpg.format ssh
git config user.signingkey "$env:USERPROFILE/.ssh/gctest"
git config commit.gpgsign true
git config tag.gpgsign true
"hello" | Out-File file.txt; git add .

# Simulate how GitComet runs git: no terminal prompt, stdin not a console.
# PowerShell has no `<` redirection, so borrow cmd.exe for the redirect.
$env:GIT_TERMINAL_PROMPT = "0"
cmd /c "git commit -m test < NUL"
```

Expected: the exact failure from the report —

```
error: Load key "C:\Users\...\gctest": incorrect passphrase supplied to decrypt private key
fatal: failed to write commit object
```

Keep this repo; Layer 3 uses it.

## Layer 3 — GitComet end to end

Build and run against the repo from Layer 2:

```powershell
cargo run -p gitcomet --features ui-gpui,gix -- $env:TEMP\gcsign
```

| # | Scenario | Expected result |
| --- | --- | --- |
| 1 | Stage a file, commit | **Passphrase prompt appears** instead of a "Commit failed" banner. Prompt title is "Passphrase required" |
| 2 | Enter the correct passphrase, confirm | Commit succeeds and is signed |
| 3 | Commit again in the same session | **No prompt** — the passphrase is reused from the session cache |
| 4 | Enter a *wrong* passphrase at step 1 | Commit fails, prompt reappears; no crash, no prompt loop |
| 5 | Cancel the prompt | Commit is abandoned cleanly, error banner shown, app stays usable |
| 6 | Create an **annotated** tag with `tag.gpgsign=true` | Passphrase prompt appears; tag is created and signed |
| 7 | Create a **lightweight** tag | No prompt (signing is explicitly disabled for these) |
| 8 | Restart GitComet, commit again | Prompt appears once more (the cache is per-session, by design) |
| 9 | Amend a commit | Prompt appears if the cache is empty; amend succeeds |
| 10 | Commit with "push after commit" enabled | Signing prompt resolves, then the push proceeds |

Verify a signature landed (`%G?` reports `N` without an allowed-signers file, so
read the object instead):

```powershell
git cat-file commit HEAD | Select-String "gpgsig"
git cat-file tag v1 | Select-String "SSH SIGNATURE"
```

Confirm the passphrase never appears in the action log or error banners.

## Layer 4 — Regressions

The askpass helper is shared by every authenticating Git command, and this change
touched all of it, including de-duplicating a second copy that the clone path
used. Re-check the paths that already worked:

| # | Scenario | Expected result |
| --- | --- | --- |
| 11 | Clone over SSH with a passphrase-protected key | Passphrase prompt appears; clone succeeds |
| 12 | Push/fetch over SSH with a passphrase-protected key | Passphrase prompt appears; operation succeeds |
| 13 | Fetch from an unknown host | Host verification prompt ("yes"/fingerprint), not a passphrase prompt |
| 14 | Clone/push over HTTPS needing credentials | Username + password prompt |
| 15 | Any SSH operation with a key **without** a passphrase | No prompt at all |
| 16 | Commit in a repo with signing **disabled** | No prompt, commit succeeds |
| 17 | Commit with `gpg.format=openpgp` (GPG) and a passphrase | GPG's own pinentry window appears — GitComet must **not** show its own prompt, since it cannot answer pinentry |
| 18 | Ordinary read operations (status, log, diff, blame) | Unchanged; no console windows flash |

Scenario 18 is worth a deliberate look: `command_may_require_auth` grew to cover
`tag`, `merge`, `rebase`, `cherry-pick`, `revert`, `commit-tree` and `am`, so
those commands now create a temporary askpass script per invocation. Watch for
stray console windows or a visible slowdown during a rebase or merge.

## Reporting back

For any failure, capture:

- the failing test name and its `--nocapture` output, or the scenario number
- the GitComet error banner text and the matching entry from the action log
- `git --version`, `ssh -V`, and `(Get-Command ssh-keygen).Source` (Git for
  Windows bundles its own OpenSSH, which may differ from the one in
  `C:\Windows\System32\OpenSSH`)
- whether `gpg.ssh.program` is set, and to which binary

The two most informative things to know are whether the prompt **appeared** and
whether the passphrase you typed was **accepted**, since they fail in different
layers: the first is classification, the second is the batch script.

## Known limitation

The real ssh-keygen prompt contains double quotes:

```
Enter passphrase for "C:\Users\dev\.ssh\id_ed25519_signing":
```

Those quotes pass through `cmd.exe` argument parsing before the script sees them.
The retry path does not care — it answers whatever it is asked. The session cache
does, because it matches on exact prompt text. If scenario 3 prompts every time
instead of once, that is this limitation and not a broken fix; report it as such.
