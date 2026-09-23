#!/usr/bin/env python3
"""Controlled loopback LFS transfers and local operations on disposable repos.

No system/user Git configuration is changed. All concurrency overrides apply
only to the measured command. Latency is per object request, not an emulation
of a WAN's bandwidth, packet loss or TLS overhead.
"""
import argparse
from contextlib import contextmanager
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tempfile
import threading
import time

SHAPES = {"small": (60, 1024 * 1024, 0), "dense": (512, 64 * 1024, 0),
          "large": (4, 64 * 1024 * 1024, 0), "mixed": (60, 1024 * 1024, 4000),
          "smoke": (4, 4096, 8)}


def remove_owned(root, path):
    root, path = root.resolve(), path.resolve()
    if path == root or not path.is_relative_to(root):
        raise ValueError(f"Not a disposable child of {root}: {path}")
    if path.exists():
        shutil.rmtree(path)


def isolated_environment(root):
    env = {k: v for k, v in os.environ.items() if not k.startswith(("GIT_", "GITCOMET_"))}
    for name in ("home", "xdg", "appdata", "gnupg"):
        (root / name).mkdir()
    (root / "gitconfig").write_text("", encoding="utf-8")
    env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=str(root / "gitconfig"),
               HOME=str(root / "home"), USERPROFILE=str(root / "home"), XDG_CONFIG_HOME=str(root / "xdg"),
               LOCALAPPDATA=str(root / "appdata"), GNUPGHOME=str(root / "gnupg"), GIT_TERMINAL_PROMPT="0",
               GIT_LFS_FORCE_PROGRESS="1", GITCOMET_DISABLE_SESSION_PERSIST="1")
    env.update(GIT_AUTHOR_DATE="2020-01-01T00:00:00Z", GIT_COMMITTER_DATE="2020-01-01T00:00:00Z")
    return env


def git(repo, env, *args):
    result = subprocess.run(["git", "-C", str(repo), *map(str, args)], env=env,
                            capture_output=True, timeout=300)
    if result.returncode:
        raise RuntimeError(f"git {args}: {result.stderr.decode(errors='replace')}")
    return result.stdout


class LfsServer(ThreadingHTTPServer):
    daemon_threads = True
    def __init__(self, root, expected, latency_ms):
        super().__init__(("127.0.0.1", 0), LfsHandler)
        self.root, self.expected = root, expected
        self.delay = latency_ms / 1000
        self.lock = threading.Lock()
        self.records = []
        self.active = self.max_active = 0
        self.url = f"http://127.0.0.1:{self.server_port}"


class LfsHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def handle(self):
        try:
            super().handle()
        except ConnectionResetError:
            # git-lfs closes its idle keep-alive sockets when the command exits.
            # Interrupted transfers still have a failed request record.
            pass

    def log_message(self, *_):
        pass

    def reply(self, status, payload):
        body = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/vnd.git-lfs+json")
        self.send_header("Content-Length", str(len(body)))
        if self.close_connection:
            self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        if self.path != "/objects/batch":
            # git-lfs probes /locks/verify before a normal push. Its unread
            # request body must not become the next keep-alive request line.
            self.close_connection = True
            self.reply(404, {"message": "unknown endpoint"})
            return
        size = int(self.headers.get("Content-Length", "0"))
        if not 0 < size < 2 * 1024 * 1024:
            self.reply(413, {"message": "invalid batch size"})
            self.close_connection = True
            return
        request = json.loads(self.rfile.read(size))
        operation = request.get("operation")
        if operation not in ("upload", "download"):
            self.reply(400, {"message": "invalid operation"})
            return
        objects = []
        for obj in request.get("objects", []):
            oid = obj["oid"]
            entry = {"oid": oid, "size": obj["size"]}
            if self.server.expected.get(oid) != obj["size"] or (operation == "download" and not (self.server.root / oid).is_file()):
                entry["error"] = {"code": 404, "message": "unknown object"}
            elif operation == "download" or not (self.server.root / oid).exists():
                entry["actions"] = {operation: {"href": self.server.url + "/objects/" + oid}}
            objects.append(entry)
        self.reply(200, {"transfer": "basic", "objects": objects})

    def transfer(self, upload):
        match = re.fullmatch(r"/objects/([a-f0-9]{64})", self.path)
        oid = match[1] if match else None
        if oid not in self.server.expected:
            self.reply(404, {"message": "unknown object"})
            self.close_connection = True
            return
        size = self.server.expected[oid]
        if upload and int(self.headers.get("Content-Length", "-1")) != size:
            self.reply(400, {"message": "incorrect size"})
            self.close_connection = True
            return
        started = time.perf_counter()
        started_unix_ms = time.time_ns() // 1_000_000
        success = False
        with self.server.lock:
            self.server.active += 1
            self.server.max_active = max(self.server.active, self.server.max_active)
        try:
            time.sleep(self.server.delay)
            path = self.server.root / oid
            if upload:
                remaining, digest = size, hashlib.sha256()
                with tempfile.NamedTemporaryFile(dir=self.server.root, delete=False) as output:
                    temporary = Path(output.name)
                    while remaining:
                        data = self.rfile.read(min(1024 * 1024, remaining))
                        if not data:
                            raise ValueError("incomplete upload")
                        digest.update(data)
                        output.write(data)
                        remaining -= len(data)
                if digest.hexdigest() != oid:
                    temporary.unlink()
                    self.reply(422, {"message": "hash mismatch"})
                    return
                temporary.replace(path)
                self.reply(200, {})
            else:
                if not path.is_file():
                    self.reply(404, {"message": "missing object"})
                    return
                self.send_response(200)
                self.send_header("Content-Type", "application/octet-stream")
                self.send_header("Content-Length", str(size))
                self.end_headers()
                with path.open("rb") as source:
                    shutil.copyfileobj(source, self.wfile, 1024 * 1024)
            success = True
        finally:
            with self.server.lock:
                self.server.active -= 1
                self.server.records.append({"operation": "upload" if upload else "download", "oid": oid,
                    "bytes": size, "success": success, "milliseconds": (time.perf_counter() - started) * 1000,
                    "start_unix_ms":started_unix_ms, "end_unix_ms":time.time_ns() // 1_000_000})

    def do_GET(self):
        self.transfer(False)

    def do_PUT(self):
        self.transfer(True)


@contextmanager
def server_at(root, expected, latency_ms):
    root.mkdir()
    server = LfsServer(root, expected, latency_ms)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield server
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


def fixture(root, env, shape):
    count, size, plain = SHAPES[shape]
    repo = root / "repo"
    repo.mkdir()
    git(repo, env, "init", "-q", "-b", "fixture")
    for key, value in {"user.name": "Performance fixture", "user.email": "probe@example.invalid",
                       "core.autocrlf": "false", "commit.gpgsign": "false"}.items():
        git(repo, env, "config", key, value)
    git(repo, env, "lfs", "install", "--local")
    # Keep a real upstream, with the asset commit still unpublished. A remote
    # tracking ref that already contains the assets makes the pre-push hook
    # correctly skip those objects, even if its LFS server is empty.
    git(repo, env, "commit", "--allow-empty", "-qm", "empty upstream")
    git(root, env, "init", "--bare", "remote.git")
    git(repo, env, "remote", "add", "origin", root / "remote.git")
    git(repo, env, "push", "-u", "origin", "fixture")
    (repo / ".gitattributes").write_text("*.bin filter=lfs diff=lfs merge=lfs -text\n")
    expected = {}
    for ix in range(count):
        data = hashlib.shake_256(f"gitcomet-lfs-{shape}-{ix}".encode()).digest(size)
        oid = hashlib.sha256(data).hexdigest()
        expected[oid] = size
        (repo / f"asset-{ix:04}.bin").write_bytes(data)
    for ix in range(plain):
        (repo / f"plain-{ix:04}.txt").write_text("fixture\n")
    git(repo, env, "add", ".")
    git(repo, env, "commit", "-qm", "fixture")
    git(repo, env, "branch", "alternate")
    return repo, expected


def verify_objects(directory, expected, nested=False):
    for oid, size in expected.items():
        path = directory / oid[:2] / oid[2:4] / oid if nested else directory / oid
        with path.open("rb") as source:
            digest = hashlib.file_digest(source, "sha256").hexdigest()
        if path.stat().st_size != size or digest != oid:
            raise ValueError(f"Incorrect object: {oid}")


def run(args):
    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=False)
    env = isolated_environment(root)
    repo, expected = fixture(root, env, args.shape)
    report = {"version": 1, "machine": platform.node(), "shape": args.shape, "latency_ms": args.latency_ms,
              "git": git(repo, env, "--version").decode().strip(), "lfs": git(repo, env, "lfs", "version").decode().strip(),
              "objects": len(expected), "payload_bytes": sum(expected.values()), "samples": [], "complete": False}
    try:
        with server_at(root / "server", expected, args.latency_ms) as server:
            git(repo, env, "config", "lfs.url", server.url)
            if args.serve_seconds:
                # A normal application push will advance the branch and upload
                # the fixture's complete LFS payload through its pre-push hook.
                (root / "ready.json").write_text(json.dumps({"repository":str(repo), "url":server.url}))
                deadline = time.monotonic() + args.serve_seconds
                while time.monotonic() < deadline and not (root / "stop").exists():
                    with server.lock:
                        records = list(server.records)
                    if not report["complete"] and len(records) == len(expected) and all(r["success"] for r in records):
                        remote = subprocess.run(["git", "-C", str(root / "remote.git"), "rev-parse", "--verify", "refs/heads/fixture"],
                                                env=env, capture_output=True, timeout=30)
                        if remote.returncode == 0 and remote.stdout.strip() == git(repo, env, "rev-parse", "HEAD").strip():
                            verify_objects(server.root, expected)
                            report.update(complete=True, hashes_verified=True)
                    report["requests"] = records
                    report["max_concurrent_requests"] = server.max_active
                    (root / "transfer.json").write_text(json.dumps(report), encoding="utf-8")
                    time.sleep(.1)
                return
            # Every upload starts with an empty server; every download uses a
            # fresh LFS storage directory. OS file caches are intentionally warm.
            for round_ix in range(args.rounds):
                order = args.workers if round_ix % 2 == 0 else list(reversed(args.workers))
                for workers in order:
                    for operation in ("upload", "download"):
                        storage = root / "download"
                        if operation == "upload":
                            for path in server.root.iterdir():
                                path.unlink()
                            command = ["lfs", "push", "--all", "origin"]
                        else:
                            remove_owned(root, storage)
                            command = ["-c", f"lfs.storage={storage}", "lfs", "fetch", "origin", "fixture"]
                        server.records.clear()
                        server.max_active = 0
                        started = time.perf_counter()
                        git(repo, env, "-c", f"lfs.concurrenttransfers={workers}", *command)
                        elapsed = (time.perf_counter() - started) * 1000
                        # Count + hash every transferred object outside timing.
                        records = list(server.records)
                        if len(records) != len(expected) or not all(record["success"] for record in records):
                            raise ValueError("Missing, repeated or failed transfer; capture is not comparable")
                        verify_objects(server.root if operation == "upload" else storage / "objects", expected, nested=operation == "download")
                        sample = {"round": round_ix, "workers": workers, "operation": operation,
                                  "milliseconds": elapsed, "max_concurrent_requests": server.max_active, "requests": records}
                        report["samples"].append(sample)
                        print(f"{args.shape} {operation} workers={workers}: {elapsed:.1f} ms", flush=True)
                remove_owned(root, root / "download")
            report["local_operations"] = []
            if args.backend:
                # Each local operation has an explicit correctness witness.
                backend = str(args.backend.resolve())
                original_asset = hashlib.sha256((repo / "asset-0000.bin").read_bytes()).hexdigest()
                changed_asset = hashlib.sha256(b"changed LFS payload\n").hexdigest()
                (repo / "asset-0000.bin").write_bytes(b"changed LFS payload\n")
                for operation, path in (("status", "asset-0000.bin"), ("diff", "asset-0000.bin"),
                                        ("stage", "asset-0000.bin"), ("checkout", "alternate"), ("push", "-")):
                    if operation == "checkout":
                        pointer = git(repo, env, "show", ":asset-0000.bin")
                        assert b"oid sha256:" + hashlib.sha256(b"changed LFS payload\n").hexdigest().encode() in pointer
                        git(repo, env, "commit", "-qm", "changed asset")
                    if operation == "push":
                        git(repo, env, "checkout", "fixture")
                        changed = b"changed LFS payload\n"
                        expected[hashlib.sha256(changed).hexdigest()] = len(changed)
                    result = subprocess.run([backend, str(repo), operation, path, "1", "0", "context"],
                                            env=env, capture_output=True, text=True, check=True, timeout=300)
                    report["local_operations"].append(json.loads(result.stdout))
                    if operation == "status":
                        assert report["local_operations"][-1]["samples"][0]["witness"] == {"staged": 0, "unstaged": 1}
                    if operation == "diff":
                        source = report["local_operations"][-1]["samples"][0]["witness"]["new"]
                        assert b"oid sha256:" + changed_asset.encode() in Path(source).read_bytes(), "LFS diff must show the correct normalized pointer"
                    if operation == "checkout":
                        assert hashlib.sha256((repo / "asset-0000.bin").read_bytes()).hexdigest() == original_asset
                    if operation == "push":
                        verify_objects(server.root, expected)
            report["complete"] = True
    finally:
        (root / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    # Retain the repo/server for inspection; the caller owns these new fixtures.


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--shape", choices=SHAPES, default="small")
    parser.add_argument("--latency-ms", type=float, default=0)
    parser.add_argument("--workers", nargs="+", type=int, default=[1, 2, 4, 8, 16])
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--backend", type=Path, help="Optional interaction-probe executable for local operations and push")
    parser.add_argument("--serve-seconds", type=int, default=0, help="Serve a fixture for a GUI push until stop file or deadline")
    args = parser.parse_args()
    if args.rounds < 1 or not 0 <= args.latency_ms <= 1000 or any(not 1 <= value <= 32 for value in args.workers):
        parser.error("Invalid rounds, latency or concurrency")
    run(args)


if __name__ == "__main__":
    main()
