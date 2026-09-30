#!/usr/bin/env python3
"""Drive the real GitComet application on Linux and measure it, or compare runs.

The app runs normally (native window, live store, real workers, normal
rendering) with the opt-in scenario driver and UI probe enabled. The driver
dispatches scripted input through production handlers and records, per input,
the stages the UI probe traces:

  scheduled input -> dispatch -> store queue -> reducer -> worker tasks
  -> state publication -> UI application -> draw -> submission

This harness seeds an isolated profile, launches a frozen binary, samples the
process from /proc, and turns the records into per-phase distributions.

  live-ui.py fixture DIR [--commits N]           synthetic repository
  live-ui.py clone SOURCE DIR [--revision REV]   pinned disposable copy
  live-ui.py run --binary B --repository R --scenario S --output DIR
  live-ui.py measure --baseline B --candidate C --repository R --scenarios S...
                     --session NAME --pairs N --output DIR [--reverse]
  live-ui.py summarize DIR
  live-ui.py report SESSION_DIR...

Measure alternates baseline and candidate within each pair and reverses the
order on odd pairs; pass --reverse in a second session.

Each run gets its own headless mutter unless --display desktop (see
scripts/profiling/README.md). Input is always dispatched inside the app.
"""

import argparse
import collections
import hashlib
import json
import math
import os
from pathlib import Path
import random
import re
import shlex
import signal
import statistics
import subprocess
import sys
import time
import uuid

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
import perf_metadata  # noqa: E402

SURVEY_SOURCE = ROOT / "crates/gitcomet-ui-gpui/src/view/user_survey.rs"
SAVE_FILE = "save-target.txt"
SEARCH_FILE = "search-target.txt"
IGNORED_DIR = "build"
WINDOW_SIZE = (1400, 900)
REFRESH_HZ = 60
# Runs share the drivers' pipeline cache, or every launch would be a first
# launch; --cold-gpu-cache measures that one.
GPU_SHADER_CACHE = ROOT / "target/profiling/gpu-shader-cache"


# ---------------------------------------------------------------- fixtures

def git(repo, *args, env=None, input=None):
    return subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True,
                          env=env, input=input)


def fixture_env():
    env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull, GIT_TERMINAL_PROMPT="0")
    return env


def create_fixture(path, commits, files):
    """History with merges and branches, a large file with local edits for diff
    search, and a small file for save-to-status propagation."""
    path = path.resolve()
    path.mkdir(parents=True, exist_ok=False)
    env = fixture_env()
    git(path, "init", "-q", "-b", "main", env=env)
    for key, value in {"user.name": "Probe", "user.email": "probe@example.invalid",
                       "core.autocrlf": "false", "commit.gpgsign": "false"}.items():
        git(path, "config", key, value, env=env)
    # fast-import: a linear main line touching a rotating set of `files`
    # files, with a side-branch commit merged back every 50 commits.
    stream = []
    mark = 0
    main_head = None

    def commit(branch, parents, index, name):
        nonlocal mark
        mark += 1
        when = 1_600_000_000 + index
        message = f"change {index}"
        body = f"commit {index}\n"
        stream.append(f"commit refs/heads/{branch}\nmark :{mark}\n"
                      f"author Probe <probe@example.invalid> {when} +0000\n"
                      f"committer Probe <probe@example.invalid> {when} +0000\n"
                      f"data {len(message)}\n{message}\n")
        if parents:
            stream.append(f"from :{parents[0]}\n")
            stream.extend(f"merge :{parent}\n" for parent in parents[1:])
        stream.append(f"M 100644 inline {name}\ndata {len(body)}\n{body}\n")
        return mark

    for index in range(commits):
        name = f"src/module{index % files:05}.txt"
        if main_head and index % 50 == 49:
            side = commit("side", [main_head], index, f"side/topic{index % files:05}.txt")
            main_head = commit("main", [main_head, side], index, name)
        else:
            main_head = commit("main", [main_head] if main_head else [], index, name)
    git(path, "fast-import", "--quiet", env=env, input="".join(stream).encode())
    git(path, "checkout", "-q", "-f", "main", env=env)
    # 100,000 rows; every 1000th holds the needle. The work-tree copy edits
    # 100 of them so the file has a real diff to search.
    lines = [f"row {row:06}: " + ("needle" if row % 1000 == 0 else "plain content") + "\n"
             for row in range(100_000)]
    (path / SEARCH_FILE).write_text("".join(lines), encoding="utf-8", newline="\n")
    (path / SAVE_FILE).write_text("saved content\n", encoding="utf-8", newline="\n")
    (path / ".gitignore").write_text(f"/{IGNORED_DIR}/\n", encoding="utf-8", newline="\n")
    git(path, "add", SEARCH_FILE, SAVE_FILE, ".gitignore", env=env)
    git(path, "commit", "-qm", "live fixture files",
        env={**env, "GIT_AUTHOR_DATE": "2020-01-01T00:00:00Z", "GIT_COMMITTER_DATE": "2020-01-01T00:00:00Z"})
    for row in range(500, 100_000, 1000):
        lines[row] = f"row {row:06}: edited needle\n"
    (path / SEARCH_FILE).write_text("".join(lines), encoding="utf-8", newline="\n")
    return path


def clone_fixture(source, path, revision):
    """A disposable copy pinned to one revision; objects are hard-linked, so
    the source is never written."""
    path = path.resolve()
    subprocess.run(["git", "clone", "-q", "--local", "--no-checkout", str(source), str(path)],
                   check=True, env=fixture_env())
    git(path, "checkout", "-q", "--detach", revision or "HEAD", env=fixture_env())
    return path


# ---------------------------------------------------------------- scenarios

def tracked_files(repository, count):
    """`count` tracked files for a burst, spread across the tree."""
    listed = git(repository, "ls-files", "-z").stdout.decode().split("\0")
    listed = [path for path in listed if path and not path.startswith(".")]
    step = max(1, len(listed) // count)
    return listed[::step][:count]


def lifecycle_cycle(secondary):
    """Open a second repository, select through its history, close it."""
    # Select only once its history is shown; `open_repo` witnesses the open alone.
    return [{"do": "open_repo", "path": str(secondary)},
            {"do": "wait_ready", "timeout_ms": 60_000},
            {"do": "focus", "target": "history"},
            {"do": "keys", "key": "down", "repeat": 3, "interval_ms": 60,
             "witness": {"kind": "commit_details"}},
            # Witnessed, or the next step reads the snapshot from before the close.
            {"do": "command", "id": "close-repo-tab",
             "witness": {"kind": "repo_closed", "path": str(secondary)}},
            {"do": "wait_ready", "timeout_ms": 60_000}]


def file_text(repository, path):
    """A file's exact contents (line endings included), so writing them back
    changes nothing."""
    try:
        return (Path(repository) / path).read_bytes().decode("utf-8")
    except (FileNotFoundError, UnicodeDecodeError) as error:
        raise ValueError(f"{path} must be an existing UTF-8 file: {error}") from error


def scenario(name, repository, save_file=SAVE_FILE, secondary=None, cycles=100):
    """Scenario files for the in-app driver (view/scenario_driver.rs)."""
    ready = [{"do": "wait_ready", "timeout_ms": 180_000}, {"do": "settle", "ms": 3000}]
    if name == "lifecycle":
        if secondary is None:
            raise ValueError("lifecycle needs --secondary-repository")
        # Warm caches first; resources should then plateau across cycles.
        return {"steps": ready
                + [{"do": "phase", "name": "warmup_cycles"}]
                + [step for _ in range(10) for step in lifecycle_cycle(secondary)]
                + [{"do": "phase", "name": "cycles"}]
                + [step for _ in range(cycles) for step in lifecycle_cycle(secondary)]
                + [{"do": "phase", "name": "after_cycles"}, {"do": "settle", "ms": 10_000}]}
    if name == "status-touch":
        # Saves that rewrite identical bytes: an editor's save-all. The
        # watcher fires, but nothing should change or redraw.
        return {"steps": ready + [
            {"do": "phase", "name": "touch"},
            {"do": "write_files", "paths": [save_file], "contents": file_text(repository, save_file),
             "rounds": 40, "interval_ms": 500, "expect_status": False},
            {"do": "settle", "ms": 3000}]}
    if name == "status-burst":
        # A 50-file save burst (formatter, branch switch), restored each round.
        return {"steps": ready + [
            {"do": "phase", "name": "burst"},
            {"do": "write_files", "paths": tracked_files(repository, 50),
             "contents": "burst\n", "rounds": 20, "interval_ms": 2500}]}
    steps = {
        # Process start to usable status and history, then quit.
        "startup": [{"do": "wait_ready", "timeout_ms": 180_000}],
        # Build output churning in an ignored directory should cost nothing.
        "ignored-churn": ready + [
            {"do": "phase", "name": "ignored_churn"},
            {"do": "write_files", "paths": [f"{IGNORED_DIR}/out-{ix:03}.o" for ix in range(100)],
             "contents": "object\n", "rounds": 60, "interval_ms": 500, "expect_status": False},
            {"do": "settle", "ms": 3000}],
        # Sustained terminal output while the history list scrolls.
        "terminal-output": ready + [
            {"do": "command", "id": "toggle-terminal"},
            {"do": "settle", "ms": 3000},
            {"do": "type", "text": "yes GitComet-terminal-output | head -n 3000000\n", "interval_ms": 20},
            {"do": "phase", "name": "scroll_with_output"},
            {"do": "scroll", "target": "history", "delta_px": -96, "repeat": 400, "interval_ms": 16,
             "flip_every": 100, "witness": {"kind": "history_scrolled"}},
            {"do": "phase", "name": "output_settling"},
            {"do": "settle", "ms": 10_000}],
        # A terminal opened, then hidden, must stop costing anything.
        "idle-hidden-terminal": ready + [
            {"do": "command", "id": "toggle-terminal"},
            {"do": "settle", "ms": 3000},
            {"do": "command", "id": "toggle-terminal"},
            {"do": "settle", "ms": 2000},
            {"do": "phase", "name": "idle_hidden_terminal"},
            {"do": "settle", "ms": 60_000}],
        # A second (Home) window beside the repository window.
        "two-windows-idle": ready + [
            {"do": "command", "id": "new-window"},
            {"do": "settle", "ms": 5000},
            {"do": "phase", "name": "idle_two_windows"},
            {"do": "settle", "ms": 60_000}],
        "idle": ready + [{"do": "phase", "name": "idle"}, {"do": "settle", "ms": 60_000}],
        "idle-minimized": ready + [{"do": "minimize"}, {"do": "settle", "ms": 2000},
                                   {"do": "phase", "name": "idle_minimized"}, {"do": "settle", "ms": 60_000}],
        # Selections faster than details load: latest wins, obsolete loads
        # should be dropped or cancelled.
        "history-select-burst": ready + [
            {"do": "focus", "target": "history"},
            {"do": "phase", "name": "select_burst"},
            {"do": "keys", "key": "down", "repeat": 600, "interval_ms": 16,
             "witness": {"kind": "commit_details"}}],
        "history-select": ready + [
            {"do": "focus", "target": "history"},
            {"do": "phase", "name": "select"},
            {"do": "keys", "key": "down", "repeat": 240, "interval_ms": 120,
             "witness": {"kind": "commit_details"}}],
        "history-scroll": ready + [
            {"do": "phase", "name": "scroll"},
            {"do": "scroll", "target": "history", "delta_px": -96, "repeat": 1200, "interval_ms": 16,
             "flip_every": 150, "witness": {"kind": "history_scrolled"}}],
        "status-save": ready + [
            {"do": "phase", "name": "save"},
            {"do": "write_files", "paths": [save_file], "contents": "edited by the scenario\n",
             "rounds": 40, "interval_ms": 1500}],
        "diff-search": ready + [
            {"do": "phase", "name": "open_diff"},
            {"do": "click", "target": {"list": "unstaged_row", "index": 0},
             "witness": {"kind": "diff_loaded"}},
            {"do": "settle", "ms": 1500},
            {"do": "keys", "key": "secondary-f", "repeat": 1, "interval_ms": 0},
            {"do": "settle", "ms": 500},
            {"do": "phase", "name": "first_search"},
            {"do": "type", "text": "needle", "interval_ms": 150,
             "witness": {"kind": "search_settled"}},
            {"do": "settle", "ms": 1000},
            {"do": "phase", "name": "repeated_search"}]
        + [step for _ in range(20) for step in (
            {"do": "keys", "key": "backspace", "repeat": 6, "interval_ms": 100,
             "witness": {"kind": "search_settled"}},
            {"do": "type", "text": "needle", "interval_ms": 100,
             "witness": {"kind": "search_settled"}})],
    }
    if name not in steps:
        raise ValueError(f"unknown scenario {name}; choose from {', '.join(steps)}")
    return {"steps": steps[name]}


SCENARIOS = ("startup", "idle", "idle-minimized", "idle-hidden-terminal", "two-windows-idle", "history-select",
             "history-select-burst", "history-scroll", "status-save", "status-burst", "status-touch",
             "ignored-churn",
             "diff-search", "terminal-output", "lifecycle")


# ---------------------------------------------------------------- one run

def survey_id():
    match = re.search(r'SURVEY_ID: &str = "([^"]+)"', SURVEY_SOURCE.read_text(encoding="utf-8"))
    return match.group(1) if match else "unknown"


def gpu_cache_environment(output, cold):
    cache = (output / "gpu-shader-cache") if cold else GPU_SHADER_CACHE
    cache.mkdir(parents=True, exist_ok=True)
    return {"__GL_SHADER_DISK_CACHE": "1", "__GL_SHADER_DISK_CACHE_PATH": str(cache),
            "__GL_SHADER_DISK_CACHE_SKIP_CLEANUP": "1", "MESA_SHADER_CACHE_DIR": str(cache)}


def smaps_breakdown(pid):
    """Proportional set size by mapping kind: separates the allocator heap
    from memory-mapped pack files and GPU driver mappings."""
    kinds = collections.Counter()
    kind = "other"
    try:
        with open(f"/proc/{pid}/smaps") as stream:
            for line in stream:
                if line[0] in "0123456789abcdef" and "-" in line.split(" ", 1)[0]:
                    parts = line.split(None, 5)
                    name = parts[5].strip() if len(parts) > 5 else ""
                    if name == "[anon:mimalloc]":
                        kind = "heap_mimalloc"
                    elif name in ("[heap]", "") or name.startswith("[anon"):
                        kind = "heap_other"
                    elif "/objects/pack/" in name:
                        kind = "git_packs_mapped"
                    elif "nvidia" in name or name.startswith("/dev/dri") or "libdrm" in name or "/dev/nvidia" in name:
                        kind = "gpu_driver"
                    elif ".so" in name:
                        kind = "shared_libraries"
                    elif name.startswith("["):
                        kind = "kernel_special"
                    elif os.path.basename(name).startswith("gitcomet"):
                        kind = "binary"
                    else:
                        kind = "files_mapped"
                elif line.startswith("Pss:"):
                    kinds[kind] += int(line.split()[1])
    except (FileNotFoundError, ProcessLookupError, PermissionError):
        return None
    return dict(kinds)


def seed_profile(output, repository):
    """An isolated profile: sandboxed XDG dirs so the user's session, crash
    reports and desktop entries are never touched, and a seeded session."""
    home = output / "profile"
    dirs = {name: home / name for name in ("config", "data", "state", "cache")}
    for directory in dirs.values():
        directory.mkdir(parents=True)
    session_file = output / "session.json"
    session_file.write_text(json.dumps({
        "version": 3, "open_repos": [str(repository)], "active_repo": str(repository),
        "window_width": WINDOW_SIZE[0], "window_height": WINDOW_SIZE[1], "ui_scale_percent": 100,
        "history_verify_commit_signatures": False, "history_verify_commit_signatures_opt_in": False,
        "history_tag_fetch_mode": "disabled", "check_for_updates_on_startup": False,
        "survey_prompt": {"survey_id": survey_id(), "opened_at_unix_seconds": 1},
    }), encoding="utf-8")
    gitconfig = output / "gitconfig"
    gitconfig.write_text("", encoding="utf-8")
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(("GIT_", "GITCOMET_", "MIMALLOC_"))}
    env.update(XDG_CONFIG_HOME=str(dirs["config"]), XDG_DATA_HOME=str(dirs["data"]),
               XDG_STATE_HOME=str(dirs["state"]), XDG_CACHE_HOME=str(dirs["cache"]),
               GITCOMET_NO_DESKTOP_INSTALL="1", GITCOMET_SESSION_FILE=str(session_file),
               GITCOMET_DISABLE_SESSION_PERSIST="1", GIT_CONFIG_NOSYSTEM="1",
               GIT_CONFIG_GLOBAL=str(gitconfig), GIT_TERMINAL_PROMPT="0")
    return env


def read_proc(pid):
    """One process sample; None once the process is gone."""
    try:
        status = Path(f"/proc/{pid}/status").read_text()
        stat = Path(f"/proc/{pid}/stat").read_text()
        rollup = Path(f"/proc/{pid}/smaps_rollup").read_text()
        fds = len(os.listdir(f"/proc/{pid}/fd"))
        tasks = os.listdir(f"/proc/{pid}/task")
    except (FileNotFoundError, ProcessLookupError, PermissionError):
        return None
    fields = {line.split(":")[0]: line.split(":", 1)[1].strip() for line in status.splitlines() if ":" in line}
    kib = lambda text: int(text.split()[0]) if text else None  # noqa: E731
    pss = next((kib(line.split(":", 1)[1]) for line in rollup.splitlines() if line.startswith("Pss:")), None)
    # utime, stime, cutime, cstime follow the parenthesised command name.
    after = stat.rsplit(")", 1)[1].split()
    ticks = os.sysconf("SC_CLK_TCK")
    switches = 0
    for task in tasks:
        try:
            task_status = Path(f"/proc/{pid}/task/{task}/status").read_text()
        except (FileNotFoundError, ProcessLookupError):
            # The thread exited after the listing (ESRCH once it is reaped).
            continue
        for line in task_status.splitlines():
            if line.startswith("voluntary_ctxt_switches:"):
                switches += int(line.split(":")[1])
    return {"unix_ms": time.time() * 1000, "rss_kib": kib(fields.get("VmRSS")),
            "pss_kib": pss, "threads": len(tasks), "fds": fds,
            "cpu_s": (int(after[11]) + int(after[12])) / ticks,
            "children_cpu_s": (int(after[13]) + int(after[14])) / ticks,
            "voluntary_switches": switches}


def find_app_pid(parent, binary):
    """The measured app under a wrapper: the descendant running `binary`."""
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            # Same session as the wrapper, which leads its own session.
            session = int((entry / "stat").read_text().rsplit(")", 1)[1].split()[3])
            if session == parent and os.readlink(entry / "exe") == str(binary):
                return int(entry.name)
        except (OSError, ValueError, IndexError):
            continue
    return None


def load_average():
    return Path("/proc/loadavg").read_text().split()[:3]


def wait_for_quiet(max_load, timeout_s=4 * 3600, poll_s=5.0):
    """Blocks until the 1-minute load average drops below `max_load`: on a
    shared machine other builds would otherwise land inside a run."""
    deadline = time.monotonic() + timeout_s
    while float(load_average()[0]) >= max_load:
        if time.monotonic() > deadline:
            raise TimeoutError(f"load stayed at or above {max_load} for {timeout_s} s")
        time.sleep(poll_s)


class HeadlessCompositor:
    """A private headless mutter with one virtual monitor, in its own D-Bus
    session so it never touches the desktop's display configuration.

    A headless mutter has no input devices, so no window ever gets keyboard
    focus, and GitComet (rightly) accepts text only in an active window. A
    RemoteDesktop session adds a virtual keyboard and pointer; they must exist
    before the app binds its seat. Nothing is typed or clicked through them."""

    def __init__(self, output):
        from gi.repository import Gio, GLib
        self.name = f"gitcomet-perf-{uuid.uuid4().hex[:8]}"
        self.log = open(output / "compositor.log", "wb")
        address_file = output / "compositor-bus.address"
        self.process = subprocess.Popen(
            ["dbus-run-session", "--", "sh", "-c",
             'printf %s "$DBUS_SESSION_BUS_ADDRESS" > "$1"; shift; exec "$@"', "sh", str(address_file),
             "mutter", "--headless", "--no-x11",
             "--virtual-monitor", f"{WINDOW_SIZE[0] + 200}x{WINDOW_SIZE[1] + 200}@{REFRESH_HZ}",
             "--wayland-display", self.name],
            stdin=subprocess.DEVNULL, stdout=self.log, stderr=subprocess.STDOUT, start_new_session=True)
        socket = Path(os.environ["XDG_RUNTIME_DIR"]) / self.name
        deadline = time.time() + 20

        def wait(condition, what):
            while not condition():
                if self.process.poll() is not None or time.time() > deadline:
                    self.close()
                    raise RuntimeError(f"headless mutter: no {what}; see {output / 'compositor.log'}")
                time.sleep(0.05)

        wait(lambda: socket.exists() and address_file.exists() and address_file.read_text(), "socket")
        self.bus = Gio.DBusConnection.new_for_address_sync(
            address_file.read_text(),
            Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,
            None, None)
        service, root = "org.gnome.Mutter.RemoteDesktop", "/org/gnome/Mutter/RemoteDesktop"

        def call(path, interface, method, args=None, reply=None):
            result = self.bus.call_sync(service if path != "/org/freedesktop/DBus" else "org.freedesktop.DBus",
                                        path, interface, method, args,
                                        GLib.VariantType(reply) if reply else None,
                                        Gio.DBusCallFlags.NONE, 5000, None)
            return result.unpack() if result is not None else None

        wait(lambda: call("/org/freedesktop/DBus", "org.freedesktop.DBus", "NameHasOwner",
                          GLib.Variant("(s)", (service,)), "(b)")[0], "RemoteDesktop service")
        session = call(root, service, "CreateSession", reply="(o)")[0]
        session_interface = service + ".Session"
        call(session, session_interface, "Start")
        # Shift press/release creates the keyboard; the pointer is parked in
        # the monitor's corner, away from the window.
        call(session, session_interface, "NotifyKeyboardKeysym", GLib.Variant("(ub)", (0xFFE1, True)))
        call(session, session_interface, "NotifyKeyboardKeysym", GLib.Variant("(ub)", (0xFFE1, False)))
        call(session, session_interface, "NotifyPointerMotionRelative", GLib.Variant("(dd)", (5000.0, 5000.0)))
        time.sleep(0.3)

    def environment(self, env):
        env = dict(env, WAYLAND_DISPLAY=self.name, XDG_SESSION_TYPE="wayland")
        env.pop("DISPLAY", None)
        return env

    def close(self):
        if getattr(self, "bus", None) is not None:
            self.bus.close_sync(None)
            self.bus = None
        if self.process.poll() is None:
            os.killpg(self.process.pid, signal.SIGTERM)
            try:
                self.process.wait(10)
            except subprocess.TimeoutExpired:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait()
        self.log.close()


def run_once(binary, repository, name, output, timeout, metadata=True, display="headless",
             ping_ms=None, save_file=SAVE_FILE, wrap=None, cold_gpu_cache=False, secondary=None,
             cycles=100, extra_env=None):
    # Absolute: the app runs with its working directory in `output`.
    binary, repository, output = binary.resolve(), repository.resolve(), output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    run_id = str(uuid.uuid4())
    env = seed_profile(output, repository)
    scenario_file = output / "scenario.json"
    secondary = secondary.resolve() if secondary else None
    scenario_file.write_text(json.dumps(scenario(name, repository, save_file, secondary, cycles), indent=2),
                             encoding="utf-8")
    frames = output / "frames.jsonl"
    env.update(GITCOMET_UI_PROBE="1", GITCOMET_UI_PROBE_JSONL=str(frames),
               GITCOMET_UI_PROBE_LOG=str(output / "ui.log"), GITCOMET_UI_SCENARIO=str(scenario_file),
               GITCOMET_PERF_RUN_ID=run_id)
    for key in ("MIMALLOC_PURGE_DELAY", "MIMALLOC_PURGE_DECOMMITS"):
        if key in os.environ:
            env[key] = os.environ[key]
    if ping_ms is None and "idle" in name:
        # The 4 ms wake pinger would dominate idle wakeup counts.
        ping_ms = 1000
    if ping_ms:
        env["GITCOMET_UI_PROBE_PING_MS"] = str(ping_ms)
    env.update(gpu_cache_environment(output, cold_gpu_cache))
    # Runtime knobs under test, e.g. allocator options; recorded in the capture.
    env.update(extra_env or {})
    capture = {"version": 1, "run_id": run_id, "scenario": name, "binary": str(binary),
               "binary_sha256": perf_metadata.sha256_file(binary), "repository": str(repository),
               "repository_head": git(repository, "rev-parse", "HEAD").stdout.decode().strip(),
               "window_size": WINDOW_SIZE, "display": display,
               "refresh_hz": REFRESH_HZ if display == "headless" else None,
               "ping_ms": ping_ms, "wrap": wrap, "gpu_cache": "cold" if cold_gpu_cache else "warm",
               "load_before": load_average(), "outcome": "failed",
               "cycles": cycles if name == "lifecycle" else None, "extra_env": extra_env or {}}
    if metadata:
        (output / "environment.json").write_text(json.dumps(perf_metadata.collect(
            binaries=[("gitcomet", binary)], fixtures=[("repository", repository)],
            command=" ".join(sys.argv)), indent=2) + "\n", encoding="utf-8")
    samples = []
    compositor = HeadlessCompositor(output) if display == "headless" else None
    if compositor:
        env = compositor.environment(env)
    started = time.time()
    capture["spawn_unix_ms"] = started * 1000
    with open(output / "stderr.log", "wb") as stderr:
        # A wrapper (perf, heaptrack) makes the run a diagnostic capture: its
        # timings are not latency evidence. The sampled pid is then the
        # wrapper's, so process samples follow the app via its children.
        prefix = [part.replace("{output}", str(output)) for part in shlex.split(wrap)] if wrap else []
        process = subprocess.Popen([*prefix, str(binary)], env=env, cwd=output, stdin=subprocess.DEVNULL,
                                   stdout=subprocess.DEVNULL, stderr=stderr, start_new_session=True)
        try:
            app_pid = process.pid
            while process.poll() is None:
                if time.time() - started > timeout:
                    raise TimeoutError(f"scenario {name} did not finish within {timeout} s")
                if prefix and app_pid == process.pid:
                    app_pid = find_app_pid(process.pid, binary) or app_pid
                sample = read_proc(app_pid)
                if sample:
                    # A mapping breakdown every ~5 s; smaps is costlier than status.
                    if len(samples) % 20 == 0:
                        sample["pss_breakdown_kib"] = smaps_breakdown(app_pid)
                    samples.append(sample)
                time.sleep(0.25)
        except BaseException:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
            raise
        finally:
            if compositor:
                compositor.close()
            capture["exit_code"] = process.returncode
            capture["wall_s"] = time.time() - started
            capture["load_after"] = load_average()
            (output / "process.jsonl").write_text("".join(json.dumps(s) + "\n" for s in samples), encoding="utf-8")
            (output / "capture.json").write_text(json.dumps(capture, indent=2) + "\n", encoding="utf-8")
    crash_dir = output / "profile/state/gitcomet/crashes"
    capture["crash_reports"] = sorted(p.name for p in crash_dir.glob("*")) if crash_dir.exists() else []
    capture["outcome"] = "passed" if process.returncode == 0 else "failed"
    (output / "capture.json").write_text(json.dumps(capture, indent=2) + "\n", encoding="utf-8")
    return summarize(output)


# ---------------------------------------------------------------- analysis

def distribution(values):
    values = sorted(v for v in values if v is not None)
    if not values:
        return {"count": 0, "mean": None, "p50": None, "p95": None, "p99": None, "max": None}
    rank = lambda q: values[max(0, math.ceil(len(values) * q) - 1)]  # noqa: E731
    return {"count": len(values), "mean": statistics.fmean(values), "p50": rank(0.5),
            "p95": rank(0.95), "p99": rank(0.99), "max": values[-1]}


def first_at_or_after(times, t):
    lo, hi = 0, len(times)
    while lo < hi:
        mid = (lo + hi) // 2
        if times[mid][0] < t:
            lo = mid + 1
        else:
            hi = mid
    return times[lo] if lo < len(times) else None


def load_records(directory):
    records = []
    with open(directory / "frames.jsonl", encoding="utf-8") as stream:
        for number, line in enumerate(stream, 1):
            try:
                records.append(json.loads(line))
            except ValueError as error:
                raise ValueError(f"corrupt record {directory}/frames.jsonl:{number}: {error}") from None
    return records


def summarize(directory):
    """Per-phase distributions and validity checks for one run."""
    directory = Path(directory)
    capture = json.loads((directory / "capture.json").read_text(encoding="utf-8"))
    problems = []
    if capture["outcome"] != "passed":
        problems.append(f"application exited {capture.get('exit_code')}")
    if capture.get("crash_reports"):
        problems.append(f"crash reports: {capture['crash_reports']}")
    records = load_records(directory)
    starts = [r for r in records if r["event"] == "start"]
    if len(starts) != 1:
        problems.append(f"expected one probe start record, saw {len(starts)}")
    start = starts[0] if starts else {"unix_ms": 0}
    if start.get("run_id") != capture["run_id"]:
        problems.append("probe records belong to another run")
    ends = [r for r in records if r["event"] == "scenario_end"]
    if not ends:
        problems.append("scenario did not finish")
    elif ends[-1]["detail"]["outcome"] != "passed":
        problems.append(f"scenario failed: {ends[-1]['detail']['errors']}")
    if any(r.get("records_dropped") or r.get("stage_records_dropped") for r in records if r["event"] == "interval"):
        problems.append("probe dropped records")
    # A frame left dirty for over a second means the compositor stopped
    # pacing the window (hidden or occluded): latencies are then meaningless.
    ready_at = next((r["at_ms"] for r in records if r["event"] == "scenario_ready"), None)
    stalled = [r for r in records if r["event"] == "draw" and r.get("dirty_ms") is not None
               and ready_at is not None and r["dirty_ms"] >= ready_at and r["at_ms"] - r["dirty_ms"] > 1000]
    if stalled:
        problems.append(f"{len(stalled)} frame(s) waited over 1 s to draw; is the window visible?")

    draws = sorted((r["start_ms"], r) for r in records if r["event"] == "draw")
    submits = sorted((r["start_ms"], r) for r in records if r["event"] == "submit")
    stages = [r for r in records if r["event"] == "stage"]
    by_op = {}
    for record in stages:
        by_op.setdefault(record["op"], []).append(record)
    applied = sorted((r["at_ms"], r) for r in stages if r["stage"] == "applied")
    threads = [r for r in records if r["event"] == "threads"]
    process = [json.loads(line) for line in (directory / "process.jsonl").read_text().splitlines()]

    phases = {}
    begins = {}
    for record in (r for r in records if r["event"] == "scenario_phase"):
        name = record["detail"]["name"]
        if record["detail"]["state"] == "begin":
            begins[name] = record
        elif name in begins:
            phases[name] = analyse_phase(begins.pop(name), record, draws, submits, by_op, applied,
                                         records, threads, process, start)
    if not phases and capture["scenario"] != "startup":
        problems.append("no complete phase")
    for name, phase in phases.items():
        inputs = phase["inputs"]
        if inputs["witnessed"] + inputs["superseded"] != inputs["expected_witnesses"]:
            problems.append(f"{name}: {inputs['expected_witnesses']} inputs expected a witness but "
                            f"{inputs['witnessed']} were witnessed and {inputs['superseded']} superseded")
    startup = None
    if starts and capture.get("spawn_unix_ms"):
        anchor = start["unix_ms"]
        first_draw = next((r for r in records if r["event"] == "draw"), None)
        ready = next((r for r in records if r["event"] == "scenario_ready"), None)
        startup = {"spawn_to_probe_ms": anchor - capture["spawn_unix_ms"],
                   "spawn_to_first_draw_ms": anchor + first_draw["at_ms"] - capture["spawn_unix_ms"] if first_draw else None,
                   "spawn_to_ready_ms": ready["unix_ms"] - capture["spawn_unix_ms"] if ready else None}
    retention = None
    if capture["scenario"] == "lifecycle" and all(name in phases for name in
                                                  ("warmup_cycles", "cycles", "after_cycles")):
        # Resources must return to a plateau: the measured cycles may not
        # keep what the 10 warm-up cycles did not.
        warm, cycles, after = (phases[name]["end_sample"] or {} for name in
                               ("warmup_cycles", "cycles", "after_cycles"))
        count = capture["cycles"]
        retention = {key: {"after_warmup": warm.get(key), "after_cycles": cycles.get(key),
                           "settled": after.get(key),
                           "growth_per_cycle": (after.get(key) - warm.get(key)) / count
                           if after.get(key) is not None and warm.get(key) is not None else None}
                     for key in ("pss_kib", "rss_kib", "threads", "fds")}
    summary = {"run_id": capture["run_id"], "scenario": capture["scenario"], "startup": startup,
               "retention": retention,
               "binary_sha256": capture["binary_sha256"], "repository_head": capture["repository_head"],
               "valid": not problems, "problems": problems, "load_before": capture.get("load_before"),
               "load_after": capture.get("load_after"), "phases": phases,
               "note": "Submission is CPU/platform work, not display completion; see README."}
    (directory / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    return summary


def analyse_phase(begin, end, draws, submits, by_op, applied, records, threads, process, start):
    lo, hi = begin["at_ms"], end["at_ms"]
    seconds = (hi - lo) / 1000
    in_phase = lambda t: lo <= t <= hi  # noqa: E731
    phase_draws = [r for t, r in draws if in_phase(t)]
    phase_submits = [r for t, r in submits if in_phase(t)]
    inputs = []
    for op, items in by_op.items():
        stage = {}
        for item in items:
            stage.setdefault(item["stage"], []).append(item)
        if "input" not in stage or not in_phase(stage["input"][0]["at_ms"]):
            continue
        entry = stage["input"][0]
        scheduled = entry["a"] / 1e6
        witness = stage.get("witness", [None])[0]
        row = {"op": op, "dispatch_delay_ms": entry["at_ms"] - scheduled,
               "handler_ms": stage["input_handled"][0]["a"] / 1e6 if "input_handled" in stage else None,
               "expects_witness": entry["b"] == 1,
               "complete": bool(witness and witness["a"] == 1), "superseded": bool(witness and witness["a"] == 0),
               "queue_ms": [r["a"] / 1e6 for r in stage.get("received", [])],
               "reduce_ms": [r["a"] / 1e6 for r in stage.get("reduced", [])],
               "task_queue_ms": [r["a"] / 1e6 for r in stage.get("task_started", [])],
               "task_ms": [r["a"] / 1e6 for r in stage.get("task_finished", [])]}
        if row["complete"]:
            at = witness["at_ms"]
            row["witness_ms"] = at - scheduled
            drawn = first_at_or_after(draws, at)
            if drawn:
                row["drawn_ms"] = drawn[1]["at_ms"] - scheduled
                submitted = first_at_or_after(submits, drawn[1]["at_ms"])
                if submitted:
                    row["submitted_ms"] = submitted[1]["at_ms"] - scheduled
            publications = [r["b"] for r in stage.get("reduced", [])]
            if publications:
                shown = first_at_or_after([(r["a"], r) for _, r in applied], max(publications))
                if shown:
                    row["published_to_applied_ms"] = shown[1]["at_ms"] - max(
                        r["at_ms"] for r in stage["reduced"])
                    row["apply_ms"] = shown[1]["b"] / 1e6
        inputs.append(row)
    # Work no scripted input caused (watcher refreshes, timers), by stage.
    background = collections.Counter(
        f"{item['stage']}:{item['label']}" for item in by_op.get(0, [])
        if in_phase(item["at_ms"]) and item["stage"] in ("received", "task_started", "applied"))
    intervals = [r for r in records if r["event"] == "interval" and lo <= r["at_ms"] - r["wall_ms"] and r["at_ms"] <= hi]
    main_cpu = [r["main_cpu_percent"] for r in intervals if r.get("main_cpu_percent") is not None]
    phase_process = [s for s in process
                     if start["unix_ms"] + lo <= s["unix_ms"] <= start["unix_ms"] + hi]

    def delta(key):
        if len(phase_process) < 2:
            return None
        return phase_process[-1][key] - phase_process[0][key]

    # /proc truncates names to 15 bytes; traced threads report full names.
    full_names = {r["tid"]: r["name"] for r in records if r["event"] == "thread" and r.get("tid")}
    main_tid = start.get("main_tid")
    # Per thread (pool): CPU ms, run-queue wait ms (CPU contention), and
    # timeslices (wakeups plus preemptions) across the phase.
    thread_cpu, thread_wait, thread_wakeups = {}, {}, {}
    inside = [r for r in threads if lo <= r["at_ms"] <= hi]
    if len(inside) >= 2:
        first = {row[0]: row for row in inside[0]["threads"]}
        for row in inside[-1]["threads"]:
            tid, name = row[0], row[1]
            name = "main" if tid == main_tid else full_names.get(tid, name)
            key = re.sub(r"-\d+$", "", name)
            before = first.get(tid, [tid, name, 0, 0, 0])
            thread_cpu[key] = thread_cpu.get(key, 0) + (row[2] - before[2]) / 1e6
            thread_wait[key] = thread_wait.get(key, 0) + (row[3] - before[3]) / 1e6
            thread_wakeups[key] = thread_wakeups.get(key, 0) + row[4] - before[4]
    return {
        "seconds": seconds,
        "frames": len(phase_draws), "frames_per_second": len(phase_draws) / seconds if seconds else None,
        "invalidations": sum(r.get("invalidations", 0) for r in phase_draws),
        "draw_ms": distribution(r["duration_ms"] for r in phase_draws),
        "submit_ms": distribution(r["duration_ms"] for r in phase_submits),
        "dirty_to_draw_ms": distribution(r["at_ms"] - r["dirty_ms"] for r in phase_draws if r.get("dirty_ms") is not None),
        "slow_frames_16ms": sum(r["duration_ms"] > 1000 / 60 for r in phase_draws),
        "wake_ms": distribution(v for r in intervals for v in r["wake_ms"]),
        "main_cpu_percent": statistics.fmean(main_cpu) if main_cpu else None,
        "process_cpu_cores": delta("cpu_s") / seconds if delta("cpu_s") is not None and seconds else None,
        "children_cpu_s": delta("children_cpu_s"),
        "wakeups_per_second": delta("voluntary_switches") / seconds if delta("voluntary_switches") is not None and seconds else None,
        "rss_kib": distribution(s["rss_kib"] for s in phase_process),
        "pss_kib": distribution(s["pss_kib"] for s in phase_process),
        "pss_breakdown_kib": next((s["pss_breakdown_kib"] for s in reversed(phase_process)
                                   if s.get("pss_breakdown_kib")), None),
        # Where the phase ended, for retention across repeated cycles.
        "end_sample": {key: phase_process[-1].get(key) for key in ("rss_kib", "pss_kib", "threads", "fds")}
        if phase_process else None,
        "threads": max((s["threads"] for s in phase_process), default=None),
        "fds": max((s["fds"] for s in phase_process), default=None),
        "background_work": dict(background.most_common()),
        "thread_cpu_ms": dict(sorted(thread_cpu.items(), key=lambda item: -item[1])),
        "thread_runqueue_wait_ms": dict(sorted(thread_wait.items(), key=lambda item: -item[1])),
        "thread_timeslices_per_second": {key: value / seconds for key, value in
                                         sorted(thread_wakeups.items(), key=lambda item: -item[1])}
        if seconds else {},
        "inputs": {
            "count": len(inputs), "expected_witnesses": sum(r["expects_witness"] for r in inputs),
            "witnessed": sum(r["complete"] for r in inputs),
            "superseded": sum(r["superseded"] for r in inputs),
            # Worker time spent for inputs whose result never showed.
            "superseded_task_ms": sum(sum(r["task_ms"]) for r in inputs if r["superseded"]),
            "superseded_tasks": sum(len(r["task_ms"]) for r in inputs if r["superseded"]),
            "dispatch_delay_ms": distribution(r["dispatch_delay_ms"] for r in inputs),
            "handler_ms": distribution(r["handler_ms"] for r in inputs),
            "input_to_witness_ms": distribution(r.get("witness_ms") for r in inputs),
            "input_to_draw_ms": distribution(r.get("drawn_ms") for r in inputs),
            "input_to_submit_ms": distribution(r.get("submitted_ms") for r in inputs),
            "store_queue_ms": distribution(v for r in inputs for v in r["queue_ms"]),
            "reduce_ms": distribution(v for r in inputs for v in r["reduce_ms"]),
            "task_queue_ms": distribution(v for r in inputs for v in r["task_queue_ms"]),
            "task_ms": distribution(v for r in inputs for v in r["task_ms"]),
            "published_to_applied_ms": distribution(r.get("published_to_applied_ms") for r in inputs),
            "apply_ms": distribution(r.get("apply_ms") for r in inputs),
        },
    }


# ---------------------------------------------------------------- paired sessions

METRICS = [
    ("inputs.input_to_submit_ms.p50", "lower"), ("inputs.input_to_submit_ms.p95", "lower"),
    ("inputs.input_to_witness_ms.p50", "lower"), ("inputs.input_to_witness_ms.p95", "lower"),
    ("inputs.dispatch_delay_ms.p95", "lower"), ("inputs.apply_ms.p95", "lower"),
    ("draw_ms.p50", "lower"), ("draw_ms.p95", "lower"), ("draw_ms.p99", "lower"),
    ("dirty_to_draw_ms.p95", "lower"), ("wake_ms.p99", "lower"),
    ("main_cpu_percent", "lower"), ("process_cpu_cores", "lower"), ("wakeups_per_second", "lower"),
    ("frames_per_second", "info"), ("pss_kib.max", "lower"), ("rss_kib.max", "lower"),
    ("slow_frames_16ms", "lower"), ("inputs.witnessed", "higher"), ("inputs.superseded", "lower"),
    ("inputs.superseded_task_ms", "lower"),
]
# Percentiles over completed inputs only compare like with like when both
# variants completed the same inputs; a variant that let most inputs be
# superseded reports only its cheap survivors.
COMPLETION_SENSITIVE = ("inputs.input_to_", "inputs.apply_ms", "inputs.dispatch_delay_ms")
# Per run, not per phase: compared under the pseudo-phase "launch".
LAUNCH_METRICS = [("spawn_to_first_draw_ms", "lower"), ("spawn_to_ready_ms", "lower")]


def lookup(data, dotted):
    for key in dotted.split("."):
        data = data.get(key) if isinstance(data, dict) else None
    return data


def bootstrap_ratio(pairs, rounds=4000, seed=7):
    """Median candidate/baseline ratio across runs with a 95% interval,
    resampling whole pairs: runs, not frames, are the independent units."""
    ratios = [c / b for b, c in pairs if b and c is not None]
    if len(ratios) < 2:
        return None
    rng = random.Random(seed)
    estimates = sorted(statistics.median(rng.choices(ratios, k=len(ratios))) for _ in range(rounds))
    return {"median_ratio": statistics.median(ratios), "ci95": [estimates[int(rounds * 0.025)],
            estimates[int(rounds * 0.975) - 1]], "pairs": len(ratios)}


def compare(samples, scenarios):
    by_pair = {}
    for sample in samples:
        by_pair.setdefault((sample["session"], sample["pair"], sample["scenario"]), {})[sample["variant"]] = sample
    result = {}
    for name in scenarios:
        phases = {}
        for key, pair in by_pair.items():
            if key[2] != name or len(pair) != 2:
                continue
            for phase in pair["baseline"]["summary"]["phases"]:
                phases.setdefault(phase, []).append(pair)
        result[name] = {}
        for phase, pairs in phases.items():
            result[name][phase] = {}
            for metric, direction in METRICS:
                values = [(lookup(p["baseline"]["summary"]["phases"][phase], metric),
                           lookup(p["candidate"]["summary"]["phases"].get(phase, {}), metric)) for p in pairs]
                values = [(b, c) for b, c in values if b is not None and c is not None]
                if not values:
                    continue
                entry = {
                    "direction": direction,
                    "baseline_median": statistics.median(b for b, _ in values),
                    "candidate_median": statistics.median(c for _, c in values),
                    "ratio": bootstrap_ratio(values)}
                if metric.startswith(COMPLETION_SENSITIVE):
                    completed = [(lookup(p["baseline"]["summary"]["phases"][phase], "inputs.witnessed"),
                                  lookup(p["candidate"]["summary"]["phases"].get(phase, {}), "inputs.witnessed"))
                                 for p in pairs]
                    if any(b is None or c is None or abs(b - c) > 0.05 * max(b, c, 1) for b, c in completed):
                        entry["not_comparable"] = "variants completed different inputs; compare inputs.witnessed"
                result[name][phase][metric] = entry
        pairs = [pair for key, pair in by_pair.items() if key[2] == name and len(pair) == 2]
        launch = {}
        for metric, direction in LAUNCH_METRICS:
            values = [((p["baseline"]["summary"].get("startup") or {}).get(metric),
                       (p["candidate"]["summary"].get("startup") or {}).get(metric)) for p in pairs]
            values = [(b, c) for b, c in values if b is not None and c is not None]
            if values:
                launch[metric] = {"direction": direction,
                                  "baseline_median": statistics.median(b for b, _ in values),
                                  "candidate_median": statistics.median(c for _, c in values),
                                  "ratio": bootstrap_ratio(values)}
        if launch:
            result[name]["launch"] = launch
    return result


def measure(args):
    if sys.platform != "linux":
        raise ValueError("live-ui.py drives the Linux application; use ui-responsiveness.py on Windows")
    if args.pairs < 1:
        raise ValueError("--pairs must be positive")
    repository = args.repository.resolve()
    head = git(repository, "rev-parse", "HEAD").stdout.decode().strip()

    def verify_repository():
        current = git(repository, "rev-parse", "HEAD").stdout.decode().strip()
        dirty = git(repository, "status", "--porcelain").stdout
        if current != head or dirty != baseline_status:
            raise ValueError("the fixture changed between runs; restore it before measuring")

    baseline_status = git(repository, "status", "--porcelain").stdout
    args.output.mkdir(parents=True, exist_ok=False)
    binaries = {"baseline": args.baseline.resolve(), "candidate": args.candidate.resolve()}
    hashes = {name: perf_metadata.sha256_file(path) for name, path in binaries.items()}
    # A runtime-only candidate (same binary, other settings) differs here instead.
    candidate = {"wrap": args.candidate_wrap,
                 "env": dict(item.split("=", 1) for item in args.candidate_env)}
    session = {"measurement_id": str(uuid.uuid4()), "session": args.session, "pairs": args.pairs,
               "candidate_runtime": candidate,
               "display": args.display,
               "scenarios": args.scenarios, "repository": str(repository), "repository_head": head,
               "binaries": {k: str(v) for k, v in binaries.items()}, "hashes": hashes,
               "environment": perf_metadata.collect(binaries=list(binaries.items()),
                                                    fixtures=[("repository", repository)]),
               "max_load": args.max_load, "samples": [], "complete": False}
    try:
        for pair in range(args.pairs):
            order = ["baseline", "candidate"] if (pair + args.reverse) % 2 == 0 else ["candidate", "baseline"]
            for name in args.scenarios:
                for variant in order:
                    verify_repository()
                    if args.max_load:
                        wait_for_quiet(args.max_load)
                    output = args.output / f"pair-{pair + 1}-{name}-{variant}"
                    runtime = candidate if variant == "candidate" else {"wrap": None, "env": {}}
                    summary = run_once(binaries[variant], repository, name, output, args.timeout,
                                       metadata=False, display=args.display, save_file=args.save_file,
                                       secondary=args.secondary_repository, wrap=runtime["wrap"],
                                       extra_env=runtime["env"])
                    if summary["binary_sha256"] != hashes[variant]:
                        raise ValueError(f"{variant} binary changed during the session")
                    if not summary["valid"]:
                        raise ValueError(f"invalid run {output}: {summary['problems']}")
                    session["samples"].append({"session": args.session, "pair": pair + 1, "scenario": name,
                                               "variant": variant, "path": str(output), "summary": summary})
                    print(f"pair {pair + 1} {name} {variant}: ok", flush=True)
        session["complete"] = True
        session["comparisons"] = compare(session["samples"], args.scenarios)
    finally:
        (args.output / "session.json").write_text(json.dumps(session, indent=2) + "\n", encoding="utf-8")


def report(directories):
    sessions = [json.loads((Path(d) / "session.json").read_text(encoding="utf-8")) for d in directories]
    if not sessions or not all(s["complete"] for s in sessions):
        raise ValueError("only complete sessions can be compared")
    if len({s["measurement_id"] for s in sessions}) != len(sessions):
        raise ValueError("a copied session is not an independent measurement")
    reference = sessions[0]
    for other in sessions[1:]:
        for key in ("hashes", "scenarios", "repository_head", "display", "candidate_runtime"):
            if other.get(key) != reference.get(key):
                raise ValueError(f"sessions disagree on {key}")
        invalid = perf_metadata.compare(reference["environment"], other["environment"])["invalidating"]
        if invalid:
            raise ValueError(f"sessions ran under different conditions: {invalid}")
    samples = [sample for s in sessions for sample in s["samples"]]
    pairs = sum(s["pairs"] for s in sessions)
    return {"sessions": len(sessions), "pairs": pairs,
            "enough_samples": pairs >= 6 and len({s["session"] for s in sessions}) >= 2,
            "hashes": reference["hashes"], "comparisons": compare(samples, reference["scenarios"]),
            "note": ("Ratios are candidate/baseline medians over runs with bootstrap 95% intervals. "
                     "Accept a timing claim only when the interval excludes 1 by more than the "
                     "calibrated noise; investigate guarded regressions above 5%.")}


def positive_int(text):
    value = int(text)
    if value < 1:
        raise argparse.ArgumentTypeError("must be at least 1")
    return value


def main():
    # Turn SIGTERM into SystemExit so cleanup kills the app and compositor.
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    fixture = commands.add_parser("fixture")
    fixture.add_argument("directory", type=Path)
    fixture.add_argument("--commits", type=int, default=20_000)
    fixture.add_argument("--files", type=int, default=2_000)
    clone = commands.add_parser("clone")
    clone.add_argument("source", type=Path)
    clone.add_argument("directory", type=Path)
    clone.add_argument("--revision")
    single = commands.add_parser("run")
    single.add_argument("--binary", type=Path, required=True)
    single.add_argument("--repository", type=Path, required=True)
    single.add_argument("--scenario", choices=SCENARIOS, required=True)
    single.add_argument("--output", type=Path, required=True)
    single.add_argument("--timeout", type=int, default=600)
    single.add_argument("--display", choices=("headless", "desktop"), default="headless")
    single.add_argument("--ping-ms", type=int)
    single.add_argument("--save-file", default=SAVE_FILE, help="tracked file status-save and status-touch rewrite")
    single.add_argument("--wrap", help="diagnostic wrapper command, e.g. 'perf record -o {output}/cpu.data --'")
    single.add_argument("--secondary-repository", type=Path, help="repository lifecycle opens and closes")
    single.add_argument("--cycles", type=positive_int, default=100,
                        help="measured lifecycle cycles; comparing two counts isolates per-cycle retention")
    single.add_argument("--env", action="append", default=[], metavar="KEY=VALUE",
                        help="extra environment for the app, e.g. MIMALLOC_ALLOW_THP=0 (repeatable)")
    single.add_argument("--cold-gpu-cache", action="store_true",
                        help="a private, empty GPU shader cache: measures a first launch")
    paired = commands.add_parser("measure")
    for name in ("baseline", "candidate", "repository", "output"):
        paired.add_argument("--" + name, type=Path, required=True)
    paired.add_argument("--scenarios", nargs="+", choices=SCENARIOS, required=True)
    paired.add_argument("--session", required=True)
    paired.add_argument("--pairs", type=int, default=3)
    paired.add_argument("--reverse", action="store_true")
    paired.add_argument("--timeout", type=int, default=600)
    paired.add_argument("--display", choices=("headless", "desktop"), default="headless")
    paired.add_argument("--save-file", default=SAVE_FILE, help="tracked file status-save and status-touch rewrite")
    paired.add_argument("--secondary-repository", type=Path, help="repository lifecycle opens and closes")
    paired.add_argument("--max-load", type=float,
                        help="wait before each run until the 1-minute load average is below this")
    paired.add_argument("--candidate-wrap", help="launch prefix for candidate runs only (runtime-only candidates)")
    paired.add_argument("--candidate-env", action="append", default=[], metavar="KEY=VALUE",
                        help="extra environment for candidate runs only (repeatable)")
    summary = commands.add_parser("summarize")
    summary.add_argument("directory", type=Path)
    combined = commands.add_parser("report")
    combined.add_argument("directories", type=Path, nargs="+")
    args = parser.parse_args()
    if args.command == "fixture":
        print(create_fixture(args.directory, args.commits, args.files))
    elif args.command == "clone":
        print(clone_fixture(args.source, args.directory, args.revision))
    elif args.command == "run":
        result = run_once(args.binary, args.repository, args.scenario, args.output, args.timeout,
                          display=args.display, ping_ms=args.ping_ms, save_file=args.save_file,
                          wrap=args.wrap, cold_gpu_cache=args.cold_gpu_cache,
                          secondary=args.secondary_repository, cycles=args.cycles,
                          extra_env=dict(item.split("=", 1) for item in args.env))
        print(json.dumps({"valid": result["valid"], "problems": result["problems"]}, indent=2))
        if not result["valid"]:
            sys.exit(1)
    elif args.command == "summarize":
        print(json.dumps(summarize(args.directory), indent=2))
    elif args.command == "report":
        print(json.dumps(report(args.directories), indent=2))
    else:
        measure(args)


if __name__ == "__main__":
    main()
