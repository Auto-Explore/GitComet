# Commit Signature Verification

GitComet shows a badge next to signed commits in the history list and in the commit details. GitComet does not check signatures itself. It asks Git, and Git runs:

- **gpg** for GPG and X.509 signatures
- **ssh-keygen** for SSH signatures

A badge appears only when the matching program is installed and knows the signer's public key. **Settings → Executables** shows which programs GitComet found. When a program is **Not found**, commits signed in that format get no badge and GitComet does not try to verify them.

To turn verification off, disable **Settings → Git log → Verify commit signatures**.

## Badges

| Badge | Git code | Meaning |
| --- | --- | --- |
| **Verified** | `G` | Good signature from a trusted key. |
| **Untrusted key** | `U` | Good signature, but you have not trusted the key. See [Trust a GPG key](#trust-a-gpg-key). |
| **Expired** | `X` | Good signature that has expired. |
| **Expired key** | `Y` | Good signature made by a key that has since expired. |
| **Bad signature** | `B` | The signature does not match the commit. |
| **Revoked key** | `R` | Good signature made by a revoked key. |
| No badge | `N`, `E` | The commit is unsigned, the signer's public key is not imported, or the verifier is not installed. |

To see what Git reports for a commit, run this inside the repository:

```bash
git log -1 --format='%G? %GK %GS' <commit>
```

GitComet keeps a commit's result until it reloads the history. After you import or trust a key, reopen the repository to refresh its badges.

## GPG signatures

Commits created or merged on GitHub's website are signed with GitHub's own GPG key, so most repositories hosted on GitHub contain GPG-signed commits.

### Install GnuPG

| Platform | Command |
| --- | --- |
| macOS | `brew install gnupg` |
| Debian / Ubuntu | `sudo apt install gnupg` |
| Fedora | `sudo dnf install gnupg2` |
| Windows | Included with Git for Windows. Run the commands on this page in Git Bash. |

On macOS, GitComet opened from Finder or the Dock does not inherit your shell's `PATH`, so it may not find a Homebrew `gpg`. Tell Git where it is:

```bash
git config --global gpg.program "$(command -v gpg)"
```

Then check that **Settings → Executables** shows **GPG** as **Found**.

### Import public keys

Import GitHub's signing keys:

```bash
curl -fsSL https://github.com/web-flow.gpg | gpg --import
```

Import the keys a GitHub user signs with (replace `USERNAME`):

```bash
curl -fsSL https://github.com/USERNAME.gpg | gpg --import
```

Without the signer's public key, Git reports `E` and GitComet shows no badge.

GitHub's previous key `4AEE18F83AFDEB23` expired on 2024-01-16, so commits GitHub signed with it show **Expired key**.

### Trust a GPG key

An imported key is not trusted yet, so its commits show **Untrusted key**. List imported keys and their fingerprints:

```bash
gpg --list-keys --keyid-format long
```

GitHub's current key has the fingerprint `968479A1AFF927E37D1A566BB5690EEEBB952194`.

**Recommended: sign the key locally with your own key.** The signature stays on your machine and is never exported:

```bash
gpg --quick-lsign-key 968479A1AFF927E37D1A566BB5690EEEBB952194
```

This needs a GPG key of your own. Without one, the command fails with `No secret key`. Create a key first, then run the command above:

```bash
gpg --quick-generate-key "Your Name <you@example.com>"
```

Note that `gpg --lsign-key` without a key argument only prints its usage.

**Alternative without a key of your own: mark the key as ultimately trusted.**

```bash
echo "968479A1AFF927E37D1A566BB5690EEEBB952194:6:" | gpg --import-ownertrust
```

Ultimate trust is meant for your own keys: GPG also accepts every key that an ultimately trusted key has signed. Prefer the local signature when you can.

Repeat either step with a contributor's fingerprint to verify their commits.

## SSH signatures

Git only trusts SSH keys listed in an allowed signers file. Until one is configured, SSH-signed commits get no badge. Create the file and point Git at it:

```bash
mkdir -p ~/.config/git
touch ~/.config/git/allowed_signers
git config --global gpg.ssh.allowedSignersFile ~/.config/git/allowed_signers
```

Add one line per signer, pairing the email address used in their commits with their public key:

```text
alice@example.com namespaces="git" ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA...
```

A commit signed by a key that is not in the file shows **Untrusted key**.

## Troubleshooting

| Symptom | Fix |
| --- | --- |
| **Settings → Executables** shows **GPG** or **ssh-keygen** as **Not found** | Install the program, or set `gpg.program` or `gpg.ssh.program` to its full path. |
| No badges on commits you know are signed | Import the signer's public key, then reopen the repository. |
| Yellow **Untrusted key** badges | [Trust the key](#trust-a-gpg-key). |
| Badges did not change after importing or trusting a key | Reopen the repository. |
