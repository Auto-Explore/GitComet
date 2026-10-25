#!/usr/bin/env python3
import argparse
import contextlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import threading
import time


class ComparisonBenchError(Exception):
    pass


NATIVE_PROBES_SUPPORTED = (
    sys.platform == "linux" and hasattr(os, "pidfd_open") and hasattr(signal, "pidfd_send_signal")
)


def _positive(value: str) -> int:
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("value must be positive")
    return number


def _git_environment() -> dict[str, str]:
    environment = {
        name: value for name, value in os.environ.items()
        if not name.upper().startswith("GIT_")
    }
    environment.update({"GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull})
    return environment


def _git(repo: Path, *args: str, input_data: bytes | None = None) -> bytes:
    result = subprocess.run(
        ["git", "-C", str(repo), *args],
        input=input_data,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=_git_environment(),
        check=False,
    )
    if result.returncode:
        raise ComparisonBenchError(result.stderr.decode(errors="replace").strip())
    return result.stdout


def _history(root: Path, commits: int, branches: int) -> tuple[Path, list[str], str]:
    repo = root / "repo"
    repo.mkdir()
    _git(repo, "init", "-q", "-b", "history", "--template=")
    _git(repo, "config", "gc.auto", "0")
    marks = root / "marks"
    with (root / "fast-import.log").open("wb") as errors:
        with subprocess.Popen(
            ["git", "-C", str(repo), "fast-import", "--quiet", f"--export-marks={marks}"],
            stdin=subprocess.PIPE,
            stdout=subprocess.DEVNULL,
            stderr=errors,
            env=_git_environment(),
        ) as process:
            if process.stdin is None:
                raise ComparisonBenchError("fast-import input pipe is unavailable")
            for number in range(1, commits + 1):
                message = f"{number}\n"
                commit = (
                    f"commit refs/heads/history\nmark :{number}\n"
                    f"committer Fixture <fixture@example.invalid> {1700000000 + number % 997} +0000\n"
                    f"data {len(message)}\n{message}"
                )
                if number > 1:
                    commit += f"from :{number - 1}\n"
                process.stdin.write((commit + "\n").encode("ascii"))
            process.stdin.close()
            if process.wait():
                raise ComparisonBenchError(f"fast-import failed; see {errors.name}")
    positions = [1 + index * (commits - 1) // (branches - 1) for index in range(branches - 1)]
    wanted = set(positions) | {commits}
    selected: dict[int, str] = {}
    with marks.open(encoding="ascii") as records:
        for line in records:
            mark, oid = line.split()
            number = int(mark.removeprefix(":"))
            if number in wanted:
                selected[number] = oid
    if len(selected) != len(wanted):
        raise ComparisonBenchError("fast-import did not export every required immutable OID")
    target = selected[commits]
    _git(repo, "update-ref", "refs/remotes/origin/dev", target)
    _git(repo, "symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/dev")
    return repo, [selected[position] for position in positions], target


def _worktrees(repo: Path, root: Path, worktrees: int, target: str) -> list[Path]:
    admin = repo / ".git" / "worktrees"
    admin.mkdir()
    checkouts = root / "checkouts"
    checkouts.mkdir()
    heads = []
    for index in range(worktrees - 1):
        name = f"checkout-{index:05}"
        checkout = checkouts / name
        checkout.mkdir()
        private = admin / name
        private.mkdir()
        (checkout / ".git").write_text(f"gitdir: {private}\n", encoding="utf-8")
        (private / "gitdir").write_text(f"{checkout / '.git'}\n", encoding="utf-8")
        (private / "commondir").write_text("../..\n", encoding="ascii")
        head = private / "HEAD"
        head.write_text(f"{target}\n", encoding="ascii")
        heads.append(head)
    return heads


def _tips(repo: Path, heads: list[Path], distinct: list[str], target: str, shared: bool) -> None:
    updates = ["start\n"]
    for index, oid in enumerate(distinct):
        updates.append(f"update refs/heads/branch-{index:05} {target if shared else oid}\n")
    updates.extend(["prepare\n", "commit\n"])
    _git(repo, "update-ref", "--stdin", input_data="".join(updates).encode("ascii"))
    for index, head in enumerate(heads):
        head.write_text(f"{target if shared else distinct[index % len(distinct)]}\n", encoding="ascii")


def _children(pid: int) -> set[int]:
    children: set[int] = set()
    try:
        tasks = list(Path(f"/proc/{pid}/task").iterdir())
    except (FileNotFoundError, PermissionError):
        return children
    for task in tasks:
        try:
            children.update(int(value) for value in (task / "children").read_text().split())
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            continue
    return children


class _MemorySampler:
    def __init__(self, pid: int) -> None:
        self.pid = pid
        self.peak_git_rss = 0
        self.peak_probe_rss = 0
        self.owned: dict[int, int] = {}
        self.measuring = threading.Event()
        self.measuring.set()
        self.stopped = threading.Event()
        self.thread = threading.Thread(target=self._run, daemon=True)

    def _run(self) -> None:
        while not self.stopped.is_set():
            self.capture()
            self.stopped.wait(0.01)

    def capture(self) -> None:
        page_bytes = os.sysconf("SC_PAGE_SIZE")
        frontier = [self.pid]
        seen: set[int] = set()
        git_rss = 0
        probe_rss = 0
        while frontier:
            pid = frontier.pop()
            if pid in seen:
                continue
            seen.add(pid)
            frontier.extend(_children(pid))
            try:
                resident = int(Path(f"/proc/{pid}/statm").read_text().split()[1]) * page_bytes
                name = Path(f"/proc/{pid}/comm").read_text().strip()
                started = int(Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[19])
                arguments = Path(f"/proc/{pid}/cmdline").read_bytes().split(b"\0")
            except (FileNotFoundError, PermissionError, ProcessLookupError):
                continue
            self.owned[pid] = started
            if name == "git" and b"--left-right" in arguments and b"--count" in arguments:
                continue
            if name == "git":
                git_rss += resident
            else:
                probe_rss += resident
        if self.measuring.is_set() and not self.stopped.is_set():
            self.peak_git_rss = max(self.peak_git_rss, git_rss)
            self.peak_probe_rss = max(self.peak_probe_rss, probe_rss)

    def freeze_measurement(self) -> None:
        self.measuring.clear()

    def capture_session(self) -> None:
        for path in Path("/proc").iterdir():
            if not path.name.isdecimal():
                continue
            try:
                fields = (path / "stat").read_text().rsplit(")", 1)[1].split()
                if int(fields[3]) == self.pid:
                    self.owned[int(path.name)] = int(fields[19])
            except (FileNotFoundError, PermissionError, ProcessLookupError):
                continue

    def finish(self) -> None:
        self.stopped.set()
        self.thread.join()


def _stop_probe(process: subprocess.Popen[str], sampler: _MemorySampler) -> None:
    sampler.finish()
    if process.returncode is None:
        sampler.capture()
        with contextlib.suppress(ProcessLookupError):
            os.killpg(process.pid, signal.SIGKILL)
        sampler.capture_session()
    for pid, started in sampler.owned.items():
        if pid == process.pid:
            continue
        with contextlib.suppress(FileNotFoundError, ProcessLookupError, PermissionError):
            descriptor = os.pidfd_open(pid)
            try:
                current = int(Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[19])
                if current == started:
                    signal.pidfd_send_signal(descriptor, signal.SIGKILL)
            finally:
                os.close(descriptor)
    process.wait(timeout=10)


def _probe(binary: Path, repo: Path, root: Path, case: str) -> None:
    if not NATIVE_PROBES_SUPPORTED:
        raise ComparisonBenchError("Native probes require Linux process handles; omit --probe to generate fixtures.")
    environment = _git_environment()
    environment.update({
        "GITCOMET_COMPARISON_PROBE_CHILD": "1",
        "GITCOMET_DISABLE_SESSION_PERSIST": "1",
        "GITCOMET_PROFILE_ROOT": str(root / "profile"),
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": os.devnull,
        "XDG_CONFIG_HOME": str(root / "settings"),
        "XDG_STATE_HOME": str(root / "state"),
        "XDG_DATA_HOME": str(root / "data"),
        "APPDATA": str(root / "settings"),
        "LOCALAPPDATA": str(root / "state"),
    })
    environment.pop("GITCOMET_SESSION_FILE", None)
    with (root / f"{case}.stderr").open("w", encoding="utf-8") as errors:
        with subprocess.Popen(
            [str(binary), str(repo)], stdout=subprocess.PIPE, stderr=errors,
            text=True, env=environment, start_new_session=True,
        ) as process:
            sampler = _MemorySampler(process.pid)
            sampler.thread.start()
            try:
                if process.stdout is None:
                    raise ComparisonBenchError("probe output pipe is unavailable")
                with (root / f"{case}.jsonl").open("w", encoding="utf-8") as records:
                    for line in process.stdout:
                        try:
                            event = json.loads(line)
                        except json.JSONDecodeError as error:
                            raise ComparisonBenchError("probe emitted invalid JSON") from error
                        if not isinstance(event, dict) or not isinstance(event.get("event"), str):
                            raise ComparisonBenchError("probe emitted an invalid measurement")
                        records.write(line)
                        records.flush()
                        print(f"{case}: {line.rstrip()}", flush=True)
                        if event["event"] == "cold_complete":
                            sampler.freeze_measurement()
                    sampler.finish()
                    sampler.capture_session()
                    status = process.wait()
                    measurement = {"event": "native_memory", "sample_period_ms": 10, "sampled_git_peak_rss": sampler.peak_git_rss, "sampled_probe_peak_rss": sampler.peak_probe_rss, "returncode": status}
                    records.write(json.dumps(measurement) + "\n")
                    print(f"{case}: {json.dumps(measurement)}", flush=True)
                    if status:
                        raise ComparisonBenchError(f"{case} failed; see {errors.name}")
            finally:
                _stop_probe(process, sampler)


def main() -> int:
    parser = argparse.ArgumentParser(description="Generate local-branch/worktree comparison fixtures and record isolated cold/warm probes")
    parser.add_argument("--output", type=Path, default=Path("target/comparison-scale"))
    parser.add_argument("--probe", type=Path)
    parser.add_argument("--commits", type=_positive, default=1_000_000)
    parser.add_argument("--branches", type=_positive, default=20_000)
    parser.add_argument("--worktrees", type=_positive, default=5_000)
    options = parser.parse_args()
    if options.branches < 2 or options.commits < options.branches:
        parser.error("require 2 <= branches <= commits")
    if options.probe is not None and not NATIVE_PROBES_SUPPORTED:
        parser.error("--probe requires Linux process handles; omit it to generate fixtures")
    options.output.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix="comparison-", dir=options.output)).resolve()
    started = time.monotonic()
    repo, distinct, target = _history(root, options.commits, options.branches)
    heads = _worktrees(repo, root, options.worktrees, target)
    metadata = {"commits": options.commits, "local_branches": options.branches, "worktrees": options.worktrees, "repository": str(repo), "generation_seconds": time.monotonic() - started, "git_version": _git(repo, "--version").decode().strip()}
    (root / "fixture.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(metadata), flush=True)
    for graph in (False, True):
        _git(repo, "config", "core.commitGraph", str(graph).lower())
        if graph:
            _git(repo, "commit-graph", "write", "--reachable")
        for shared in (True, False):
            _tips(repo, heads, distinct, target, shared)
            case = f"{'graph' if graph else 'no-graph'}-{'shared' if shared else 'distinct'}"
            if options.probe is not None:
                _probe(options.probe.resolve(), repo, root, case)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ComparisonBenchError, OSError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
