"""Disposable, witnessed Git/LFS/annex workloads shared by CLI, backend and UI."""
from contextlib import contextmanager
import hashlib
import importlib.util
from pathlib import Path
import shutil
import subprocess

from perf_transport import serve_git


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


lfs = module("performance_lfs", "lfs-performance.py")


def git(repo, env, *args):
    return lfs.git(repo, env, *args)


def configure(repo, env):
    for key, value in {"user.name": "Performance", "user.email": "perf@example.invalid",
                       "commit.gpgsign": "false", "core.autocrlf": "false", "gc.auto": "0"}.items():
        git(repo, env, "config", key, value)


def add_history(repo, env, count=256):
    branch = git(repo, env, "symbolic-ref", "HEAD").decode().strip()
    parent = git(repo, env, "rev-parse", "HEAD").decode().strip()
    lines = []
    for index in range(count):
        body = f"revision {index}\n"
        lines.append(f"commit {branch}\nmark :{index + 1}\ncommitter Performance <perf@example.invalid> 1600000000 +0000\ndata 7\nhistory\nfrom {parent}\nM 100644 inline history.txt\ndata {len(body)}\n{body}\n")
        parent = f":{index + 1}"
    subprocess.run(["git", "-C", str(repo), "fast-import", "--quiet"], env=env,
                   input="".join(lines).encode(), capture_output=True, check=True)
    git(repo, env, "read-tree", "HEAD")
    (repo / "history.txt").write_text(body, encoding="utf-8", newline="\n")


def cli_command(operation):
    return {"fetch": ["fetch", "--all"], "fetch-noop": ["fetch", "--all"],
            "pull": ["pull", "--no-rebase"], "pull-noop": ["pull", "--no-rebase"],
            "pull-merge": ["pull", "--no-rebase"], "pull-rebase": ["pull", "--rebase"],
            "push": ["push"], "push-noop": ["push"],
            "lfs-fetch": ["lfs", "fetch", "--all", "origin"],
            "lfs-pull": ["lfs", "pull"], "lfs-push": ["lfs", "push", "--all", "origin"],
            "annex-get": ["annex", "get", "--from=backup", "."],
            "annex-copy": ["annex", "copy", "--to=backup", "."],
            "annex-pull": ["annex", "pull", "--content"],
            "annex-push": ["annex", "push", "--content"],
            "annex-sync": ["annex", "sync", "--no-commit", "--content"]}[operation]


@contextmanager
def transfer(root, seed, operation, live, shape="smoke", latency_ms=0, bandwidth_mib=0, fault="none", content_cache="cold"):
    if content_cache not in ("cold", "mixed", "warm"):
        raise ValueError(f"unknown content cache state: {content_cache}")
    root = Path(root).resolve()
    root.mkdir(parents=True, exist_ok=False)
    env = lfs.isolated_environment(root)
    env.update(GIT_AUTHOR_DATE="2020-01-01T00:00:00Z", GIT_COMMITTER_DATE="2020-01-01T00:00:00Z")
    if operation.startswith("lfs-"):
        repo, expected = lfs.fixture(root, env, shape)
        add_history(repo, env)
        with lfs.server_at(root / "server", expected, latency_ms) as server:
            server.bandwidth = bandwidth_mib * 1024 * 1024
            git(repo, env, "config", "lfs.url", server.url)
            # Warm means all content is already at the transfer destination;
            # mixed means half. Keep this setup outside the measured command.
            ordered = sorted(expected)
            present = set(ordered[:0 if content_cache == "cold" else len(ordered) // 2 if content_cache == "mixed" else len(ordered)])
            def object_path(oid):
                return repo / ".git/lfs/objects" / oid[:2] / oid[2:4] / oid
            if operation != "lfs-push":
                git(repo, env, "lfs", "push", "--all", "origin")
                for oid in expected.keys() - present:
                    object_path(oid).unlink()
                # A native status scan may clean full working-tree content
                # back into .git/lfs/objects before the measured fetch. Pointer
                # worktrees ensure the requested download is still necessary.
                for file in repo.glob("asset-*.bin"):
                    file.write_bytes(git(repo, env, "show", f"HEAD:{file.name}"))
            else:
                for oid in present:
                    shutil.copyfile(object_path(oid), server.root / oid)
            server.records.clear()
            server.fault = fault
            def verify():
                lfs.verify_objects(server.root if operation == "lfs-push" else repo / ".git/lfs/objects",
                                   expected, nested=operation != "lfs-push")
                if operation == "lfs-pull":
                    for path in repo.glob("asset-*.bin"):
                        assert hashlib.sha256(path.read_bytes()).hexdigest() in expected, "LFS checkout still contains pointers"
                # Completed payload requests must match exactly the initially
                # missing objects; a warm run proves zero redundant transfers.
                requests = list(server.records)
                assert len(requests) == len(expected) - len(present), "unexpected LFS payload transfer count"
                assert {r["oid"] for r in requests} == expected.keys() - present, "wrong LFS objects transferred"
                assert all(r["success"] for r in requests), "failed LFS payload transfer"
                return {"objects": len(expected), "bytes": sum(expected.values()), "content_cache": content_cache,
                        "required_content_bytes": sum(size for oid, size in expected.items() if oid not in present)}
            yield {"repo": repo, "env": env, "verify": verify, "cli": cli_command(operation),
                   "operation": operation, "server": server, "remote": "origin", "bytes": sum(expected.values())}
        return
    if operation.startswith("annex-"):
        repo, backup = root / "repo", root / "backup"
        repo.mkdir(); backup.mkdir()
        git(repo, env, "init", "-q", "-b", "main")
        configure(repo, env)
        git(repo, env, "annex", "init", "-q", "performance")
        git(repo, env, "annex", "initremote", "backup", "type=directory", f"directory={backup}", "encryption=none")
        count, size, _ = lfs.SHAPES[shape]
        expected = {}
        for index in range(count):
            data = hashlib.shake_256(f"annex-{shape}-{index}".encode()).digest(size)
            name = f"asset-{index:04}.bin"
            (repo / name).write_bytes(data)
            expected[name] = hashlib.sha256(data).hexdigest()
        git(repo, env, "annex", "add", "-q", ".")
        git(repo, env, "commit", "-qm", "annex fixture")
        add_history(repo, env)
        git(repo, env, "annex", "wanted", "here", "anything")
        git(repo, env, "annex", "wanted", "backup", "anything")
        ordered = sorted(expected)
        present = ordered[:0 if content_cache == "cold" else count // 2 if content_cache == "mixed" else count]
        if operation in ("annex-get", "annex-pull"):
            git(repo, env, "annex", "copy", "--to=backup", ".")
            missing = ordered[len(present):]
            if missing:
                git(repo, env, "annex", "drop", "--", *missing)
        elif present:
            git(repo, env, "annex", "copy", "--to=backup", "--", *present)
        # Git branch exchange uses a disposable peer; directory remote holds content.
        git(root, env, "init", "--bare", "remote.git")
        git(repo, env, "remote", "add", "origin", root / "remote.git")
        git(repo, env, "push", "-u", "origin", "main")
        def verify():
            for name, digest in expected.items():
                assert hashlib.sha256((repo / name).read_bytes()).hexdigest() == digest, f"missing annex content: {name}"
            if operation in ("annex-copy", "annex-push", "annex-sync"):
                found = {hashlib.sha256(p.read_bytes()).hexdigest() for p in backup.rglob("*") if p.is_file() and p.stat().st_size == size}
                assert set(expected.values()) <= found, "annex copies missing from backup"
            return {"objects": count, "bytes": count * size, "content_cache": content_cache,
                    "required_content_bytes": (count - len(present)) * size}
        yield {"repo": repo, "env": env, "verify": verify, "cli": cli_command(operation),
               "operation": operation, "remote": "backup", "bytes": count * size}
        return
    # Smart HTTP guarantees clone/fetch use pack transport rather than local hardlinks.
    remote, repo = root / "remote.git", root / "client"
    subprocess.run(["git", "clone", "--quiet", "--bare", str(seed), str(remote)], env=env, check=True)
    git(remote, env, "config", "http.receivepack", "true")
    if subprocess.run(["git", "-C", str(remote), "symbolic-ref", "-q", "HEAD"], env=env, capture_output=True).returncode:
        # Real snapshots are pinned detached checkouts. Give only this remote
        # fixture a branch so clone/pull witnesses have an unambiguous tip.
        head = git(remote, env, "rev-parse", "HEAD").decode().strip()
        git(remote, env, "update-ref", "refs/heads/performance-main", head)
        git(remote, env, "symbolic-ref", "HEAD", "refs/heads/performance-main")
    subprocess.run(["git", "clone", "--quiet", str(remote), str(repo)], env=env, check=True)
    configure(repo, env)
    branch = git(repo, env, "symbolic-ref", "--short", "HEAD").decode().strip()
    payload = hashlib.shake_256(b"gitcomet-transfer").digest(4 * 1024**2 if shape == "smoke" else 64 * 1024**2)
    digest = hashlib.sha256(payload).hexdigest()
    changed = operation not in ("fetch-noop", "pull-noop", "push-noop")
    producer = repo
    if changed and operation != "push":
        # Publish from another object database. Resetting the client after a
        # push leaves both the objects and remote-tracking ref locally, making
        # an alleged fetch benchmark a no-op with a pre-satisfied witness.
        producer = root / "publisher"
        subprocess.run(["git", "clone", "--quiet", str(remote), str(producer)], env=env, check=True)
        configure(producer, env)
    if changed:
        (producer / "performance-payload.bin").write_bytes(payload)
        git(producer, env, "add", "performance-payload.bin")
        git(producer, env, "commit", "-qm", "transfer payload")
    expected = git(producer, env, "rev-parse", "HEAD").decode().strip()
    if producer != repo:
        git(producer, env, "push", "origin", branch)
    if operation in ("pull-merge", "pull-rebase"):
        (repo / "local-only.txt").write_text("local change\n", encoding="utf-8")
        git(repo, env, "add", "local-only.txt")
        git(repo, env, "commit", "-qm", "local change")
    with serve_git(root, env, latency_ms=latency_ms, bandwidth_mib=bandwidth_mib, fault=fault) as server:
        git(repo, env, "remote", "set-url", "origin", server.url)
        dest = root / "cloned"
        def verify():
            if operation == "clone":
                observed = git(dest, env, "rev-parse", "HEAD").decode().strip()
            elif operation.startswith("push"):
                observed = git(remote, env, "rev-parse", f"refs/heads/{branch}").decode().strip()
            elif operation.startswith("fetch"):
                observed = git(repo, env, "rev-parse", f"refs/remotes/origin/{branch}").decode().strip()
            elif operation in ("pull-merge", "pull-rebase"):
                git(repo, env, "merge-base", "--is-ancestor", expected, "HEAD")
                assert (repo / "local-only.txt").read_text() == "local change\n"
                observed = expected
            else:
                observed = git(repo, env, "rev-parse", "HEAD").decode().strip()
            assert observed == expected, f"incorrect resulting ref: {observed} != {expected}"
            if changed:
                if operation.startswith(("fetch", "push")):
                    payload_repo = remote if operation.startswith("push") else repo
                    content = git(payload_repo, env, "show", f"{expected}:performance-payload.bin")
                else:
                    content = ((dest if operation == "clone" else repo) / "performance-payload.bin").read_bytes()
                assert hashlib.sha256(content).hexdigest() == digest, "incorrect transferred content"
            return {"expected_head": expected, "bytes": len(payload) if changed else 0}
        yield {"repo": repo, "env": env, "verify": verify,
               "cli": ["clone", "--progress", server.url, str(dest)] if operation == "clone" else cli_command(operation),
               "operation": operation.removesuffix("-noop"), "server": server, "url": server.url,
               "dest": dest, "bytes": len(payload) if changed else 0}


def transfer_scenario(fixture, cancel=False, timeout_ms=300_000):
    start = {"do": "start_operation", "operation": fixture["operation"],
             "path": "." if fixture["operation"] in ("annex-get", "annex-copy") else "",
             "remote": fixture.get("remote", ""), "url": fixture.get("url", ""),
             "dest": str(fixture["dest"]) if fixture.get("dest") else ""}
    gestures = [step for index in range(4) for step in (
        {"do": "scroll", "target": "history", "delta_px": -32 if index % 2 == 0 else 32,
         "repeat": 15, "interval_ms": 16, "witness": {"kind": "history_scrolled"}},
        {"do": "move_pointer", "positions": [[.55, .1], [.55, .2], [.55, .3]], "repeat": 30, "interval_ms": 8},
    )]
    return {"version": 1, "steps": [
        {"do": "wait_ready", "timeout_ms": 180_000}, {"do": "settle", "ms": 1000},
        {"do": "phase", "name": "transfer"}, start,
        # Large-file commands open the normal activity dialog. Minimize with
        # its production handler and verify that history is unobstructed.
        *([{"do": "minimize_activity"}] if fixture["operation"].startswith(("lfs-", "annex-")) else []),
        *gestures,
        *([{"do": "cancel_operation"}] if cancel else []),
        {"do": "wait_operation", "timeout_ms": 15_000 if cancel else timeout_ms, "cancelled": cancel},
        {"do": "phase", "name": "refresh"}, {"do": "wait_ready", "timeout_ms": 180_000},
        *([{"do": "scroll", "target": "history", "delta_px": -32,
            "repeat": 10, "interval_ms": 16, "witness": {"kind": "history_scrolled"}}] if cancel else []),
        {"do": "settle", "ms": 1000}]}
