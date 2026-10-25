These small binary tables were generated with Git 2.55.0 on 2026-09-29.
They are independent parser fixtures, tested without running Git.

For each object format (`sha1`, `sha256`), in a fresh directory:

```sh
GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null git init --initial-branch=main --ref-format=reftable --object-format=<format>
git config user.name 'Reader Test'
git config user.email reader@example.invalid
GIT_AUTHOR_DATE='1700000000 +0000' GIT_COMMITTER_DATE='1800000000 +0530' git commit --allow-empty -m root
git pack-refs --all
```

The single table named by `.git/reftable/tables.list` is copied verbatim.
The `.expected` file is `git for-each-ref --include-root-refs
--format='%(refname) %(objectname) %(symref)'`. SHA-1 uses v1; SHA-256
uses v2 with `s256`. Both HEAD and main have one reflog entry at timestamp
1800000000, offset +0530, message `commit (initial): root`.
