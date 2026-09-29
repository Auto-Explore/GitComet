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

By default (--display headless) each run gets its own headless mutter (the
GNOME compositor) with one virtual monitor: the app window is the only,
focused, unoccluded window, so the compositor paces frames steadily and the
desktop is untouched. On the desktop GNOME denies a background launch focus,
and an occluded window gets no frame callbacks: it then draws only when the
next key event forces a draw, and every latency is wrong. --display desktop
keeps the old behaviour for checking native presentation, with the window
brought to front by hand. Input is always dispatched inside the app.
"""

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import random
import re
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
WINDOW_SIZE = (1400, 900)
REFRESH_HZ = 60


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
    git(path, "add", SEARCH_FILE, SAVE_FILE, env=env)
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
    # One branch keeps the ref sidebar comparable between clones.
    return path


# ---------------------------------------------------------------- scenarios

def scenario(name, repository):
    """Scenario files for the in-app driver (view/scenario_driver.rs)."""
    ready = [{"do": "wait_ready", "timeout_ms": 180_000}, {"do": "settle", "ms": 3000}]
    steps = {
        "idle": ready + [{"do": "phase", "name": "idle"}, {"do": "settle", "ms": 60_000}],
        "idle-minimized": ready + [{"do": "minimize"}, {"do": "settle", "ms": 2000},
                                   {"do": "phase", "name": "idle_minimized"}, {"do": "settle", "ms": 60_000}],
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
            {"do": "write_file", "path": SAVE_FILE, "contents": "edited by the scenario\n",
             "repeat": 40, "interval_ms": 1500}],
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
    return {"name": name, "steps": steps[name], "quit": True}


SCENARIOS = ("idle", "idle-minimized", "history-select", "history-scroll", "status-save", "diff-search")


# ---------------------------------------------------------------- one run

def survey_id():
    match = re.search(r'SURVEY_ID: &str = "([^"]+)"', SURVEY_SOURCE.read_text(encoding="utf-8"))
    return match.group(1) if match else "unknown"


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
        except FileNotFoundError:
            continue
        for line in task_status.splitlines():
            if line.startswith("voluntary_ctxt_switches:"):
                switches += int(line.split(":")[1])
    return {"unix_ms": time.time() * 1000, "rss_kib": kib(fields.get("VmRSS")),
            "hwm_kib": kib(fields.get("VmHWM")), "pss_kib": pss, "threads": len(tasks), "fds": fds,
            "cpu_s": (int(after[11]) + int(after[12])) / ticks,
            "children_cpu_s": (int(after[13]) + int(after[14])) / ticks,
            "voluntary_switches": switches}


def load_average():
    return Path("/proc/loadavg").read_text().split()[:3]


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
             ping_ms=None):
    binary, repository = binary.resolve(), repository.resolve()
    output.mkdir(parents=True, exist_ok=False)
    run_id = str(uuid.uuid4())
    env = seed_profile(output, repository)
    scenario_file = output / "scenario.json"
    scenario_file.write_text(json.dumps(scenario(name, repository), indent=2), encoding="utf-8")
    frames = output / "frames.jsonl"
    env.update(GITCOMET_UI_PROBE="1", GITCOMET_UI_PROBE_JSONL=str(frames),
               GITCOMET_UI_PROBE_LOG=str(output / "ui.log"), GITCOMET_UI_SCENARIO=str(scenario_file),
               GITCOMET_PERF_RUN_ID=run_id)
    for key in ("MIMALLOC_PURGE_DELAY", "MIMALLOC_PURGE_DECOMMITS"):
        if key in os.environ:
            env[key] = os.environ[key]
    if ping_ms is None and name.startswith("idle"):
        # The 4 ms wake pinger would dominate idle wakeup counts.
        ping_ms = 1000
    if ping_ms:
        env["GITCOMET_UI_PROBE_PING_MS"] = str(ping_ms)
    capture = {"version": 1, "run_id": run_id, "scenario": name, "binary": str(binary),
               "binary_sha256": perf_metadata.sha256_file(binary), "repository": str(repository),
               "repository_head": git(repository, "rev-parse", "HEAD").stdout.decode().strip(),
               "window_size": WINDOW_SIZE, "display": display,
               "refresh_hz": REFRESH_HZ if display == "headless" else None,
               "ping_ms": ping_ms, "load_before": load_average(), "outcome": "failed"}
    if metadata:
        (output / "environment.json").write_text(json.dumps(perf_metadata.collect(
            binaries=[("gitcomet", binary)], fixtures=[("repository", repository)],
            command=" ".join(sys.argv)), indent=2) + "\n", encoding="utf-8")
    samples = []
    compositor = HeadlessCompositor(output) if display == "headless" else None
    if compositor:
        env = compositor.environment(env)
    started = time.time()
    with open(output / "stderr.log", "wb") as stderr:
        process = subprocess.Popen([str(binary)], env=env, cwd=output, stdin=subprocess.DEVNULL,
                                   stdout=subprocess.DEVNULL, stderr=stderr, start_new_session=True)
        try:
            while process.poll() is None:
                if time.time() - started > timeout:
                    raise TimeoutError(f"scenario {name} did not finish within {timeout} s")
                sample = read_proc(process.pid)
                if sample:
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
    if not phases:
        problems.append("no complete phase")
    for name, phase in phases.items():
        inputs = phase["inputs"]
        if inputs["witnessed"] + inputs["superseded"] != inputs["expected_witnesses"]:
            problems.append(f"{name}: {inputs['expected_witnesses']} inputs expected a witness but "
                            f"{inputs['witnessed']} were witnessed and {inputs['superseded']} superseded")
    summary = {"run_id": capture["run_id"], "scenario": capture["scenario"],
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
    thread_cpu = {}
    inside = [r for r in threads if lo <= r["at_ms"] <= hi]
    if len(inside) >= 2:
        first = {tid: cpu for tid, _, cpu in inside[0]["threads"]}
        for tid, name, cpu in inside[-1]["threads"]:
            name = "main" if tid == main_tid else full_names.get(tid, name)
            key = re.sub(r"-\d+$", "", name)
            thread_cpu[key] = thread_cpu.get(key, 0) + (cpu - first.get(tid, 0)) / 1e6
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
        "threads": max((s["threads"] for s in phase_process), default=None),
        "fds": max((s["fds"] for s in phase_process), default=None),
        "thread_cpu_ms": dict(sorted(thread_cpu.items(), key=lambda item: -item[1])),
        "inputs": {
            "count": len(inputs), "expected_witnesses": sum(r["expects_witness"] for r in inputs),
            "witnessed": sum(r["complete"] for r in inputs),
            "superseded": sum(r["superseded"] for r in inputs),
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
    ("slow_frames_16ms", "lower"),
]


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
                result[name][phase][metric] = {
                    "direction": direction,
                    "baseline_median": statistics.median(b for b, _ in values),
                    "candidate_median": statistics.median(c for _, c in values),
                    "ratio": bootstrap_ratio(values)}
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
    session = {"measurement_id": str(uuid.uuid4()), "session": args.session, "pairs": args.pairs,
               "display": args.display,
               "scenarios": args.scenarios, "repository": str(repository), "repository_head": head,
               "binaries": {k: str(v) for k, v in binaries.items()}, "hashes": hashes,
               "environment": perf_metadata.collect(binaries=list(binaries.items()),
                                                    fixtures=[("repository", repository)]),
               "samples": [], "complete": False}
    try:
        for pair in range(args.pairs):
            order = ["baseline", "candidate"] if (pair + args.reverse) % 2 == 0 else ["candidate", "baseline"]
            for name in args.scenarios:
                for variant in order:
                    verify_repository()
                    output = args.output / f"pair-{pair + 1}-{name}-{variant}"
                    summary = run_once(binaries[variant], repository, name, output, args.timeout,
                                       metadata=False, display=args.display)
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
        for key in ("hashes", "scenarios", "repository_head", "display"):
            if other[key] != reference[key]:
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


def main():
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
    paired = commands.add_parser("measure")
    for name in ("baseline", "candidate", "repository", "output"):
        paired.add_argument("--" + name, type=Path, required=True)
    paired.add_argument("--scenarios", nargs="+", choices=SCENARIOS, required=True)
    paired.add_argument("--session", required=True)
    paired.add_argument("--pairs", type=int, default=3)
    paired.add_argument("--reverse", action="store_true")
    paired.add_argument("--timeout", type=int, default=600)
    paired.add_argument("--display", choices=("headless", "desktop"), default="headless")
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
                          display=args.display, ping_ms=args.ping_ms)
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
