"""Owned, pinned performance fixtures. Originals are read-only seeds."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

SOURCES = {name: str(Path.home() / path) for name, path in
           {"git": "git/git", "bun": "git/bun", "chromium": "chromium/src"}.items()}


def git(path, *args, env=None):
    return subprocess.run(["git", "-C", str(path), *map(str, args)], env=env,
                          capture_output=True, check=True).stdout


def identity(path):
    refs = git(path, "for-each-ref", "--format=%(refname) %(objectname)")
    common = Path(git(path, "rev-parse", "--git-common-dir").decode().strip())
    if not common.is_absolute():
        common = Path(path) / common
    graphs = [common / "objects/info/commit-graph"]
    graphs += sorted((common / "objects/info/commit-graphs").glob("*"))
    return {"head": git(path, "rev-parse", "HEAD").decode().strip(),
            "refs_sha256": hashlib.sha256(refs).hexdigest(),
            "object_format": git(path, "rev-parse", "--show-object-format").decode().strip(),
            "commit_graphs": {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in graphs if p.is_file()},
            "packs": {p.name: p.stat().st_size for p in sorted((common / "objects/pack").glob("*.pack"))}}


def worktree_identity(path):
    """Include intentional dirty fixtures; refs alone cannot pin a diff workload."""
    env = {**os.environ, "GIT_OPTIONAL_LOCKS": "0"}
    digest = hashlib.sha256(git(path, "diff", "--binary", "--no-ext-diff", "--no-textconv", "HEAD", env=env))
    untracked = git(path, "ls-files", "--others", "--exclude-standard", "-z", env=env)
    digest.update(untracked)
    for name in untracked.split(b"\0"):
        if name:
            file = Path(path) / os.fsdecode(name)
            if file.is_symlink():
                digest.update(os.fsencode(os.readlink(file)))
            else:
                with file.open("rb") as stream:
                    for block in iter(lambda: stream.read(1024 * 1024), b""):
                        digest.update(block)
    return digest.hexdigest()


def read(root):
    manifest = json.loads((Path(root) / "corpus.json").read_text(encoding="utf-8"))
    if manifest.get("version") != 1:
        raise ValueError("unsupported corpus version")
    return manifest


def write(root, manifest):
    path = Path(root) / "corpus.json"
    temporary = path.with_suffix(".json.tmp")
    temporary.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)


def prepare(root, suite, live, sources=None, real=False, sizes=None):
    root = Path(root).resolve()
    root.mkdir(parents=True, exist_ok=True)
    manifest = read(root) if (root / "corpus.json").exists() else {"version": 1, "fixtures": {}}
    chosen = sizes or ([20_000] if suite == "smoke" else [20_000, 100_000, 2_000_000])
    for size in chosen:
        name, path = f"history-{size}", root / f"history-{size}"
        if name in manifest["fixtures"]:
            if identity(path) != manifest["fixtures"][name]["identity"]:
                raise ValueError(f"fixture changed: {path}")
            continue
        if path.exists():
            raise ValueError(f"unregistered destination exists: {path}; choose a new corpus root")
        print(f"Preparing {name}", flush=True)
        live.create_fixture(path, size, 300 if size < 100_000 else 4000)
        manifest["fixtures"][name] = {"path": name, "kind": "generated", "requested_commits": size,
                                      "identity": identity(path)}
        write(root, manifest)
    if real:
        for name, source in (sources or SOURCES).items():
            if name in manifest["fixtures"]:
                continue
            source = Path(source).resolve()
            before = identity(source)
            # A mirror and checkout can each need a copy of the source. Be
            # conservative; this is preflight, outside every timed sample.
            needed = sum(before["packs"].values()) * 3 + 2 * 1024**3
            if shutil.disk_usage(root).free < needed:
                raise ValueError(f"{name} needs at least {needed // 1024**3} GiB free for snapshot and scratch")
            mirror, checkout = root / f"{name}.git", root / name
            if mirror.exists() or checkout.exists():
                raise ValueError(f"destination already exists for {name}")
            print(f"Snapshotting {source} at {before['head']}", flush=True)
            subprocess.run(["git", "clone", "--quiet", "--mirror", "--no-hardlinks", str(source), str(mirror)],
                           env=live.fixture_env(), check=True)
            live.clone_fixture(mirror, checkout, before["head"])
            if identity(source) != before:
                raise ValueError(f"source changed during preparation: {source}")
            manifest["fixtures"][name] = {"path": name, "mirror": mirror.name, "kind": "real",
                                          "source": str(source), "source_identity": before,
                                          "identity": identity(checkout)}
            write(root, manifest)
    return manifest


def validate(root, entry):
    path = Path(root) / entry["path"]
    if identity(path) != entry["identity"]:
        raise ValueError(f"corpus fixture changed: {path}; prepare a new snapshot")
    return path
