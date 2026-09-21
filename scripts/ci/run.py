#!/usr/bin/env python3
"""Build once, inventory every test, then run nextest + the GPUI libtest harness."""

import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
from functools import partial
from contextlib import ExitStack
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import threading
import time
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]
REPORTS = ROOT / "target" / "ci-reports"
REPORT_LOCK = threading.RLock()
CONSOLE_LOCK = threading.RLock()
UI = "gitcomet-ui-gpui"
NEXTEST_PROFILES = ("ci", "ci-git-limited")
CONTEXTS = {
    "workspace": ["--workspace", "--no-default-features", "--features", "gix,gitcomet-ui-gpui/default"],
    "core": ["-p", "gitcomet-core"],
    "state": ["-p", "gitcomet-state"],
    "backend": ["-p", "gitcomet-git-gix"],
    "app": ["-p", "gitcomet", "--no-default-features", "--features", "gix"],
    "ui": ["-p", UI],
}
DISPLAY_PROFILES = {
    "x11-gnome": (":99", "", "x11", "GNOME"),
    "wayland-gnome": ("", "wayland-1", "wayland", "GNOME"),
    "wayland-kde": ("", "wayland-1", "wayland", "KDE"),
}
GIT_PREREQUISITE_SKIP = re.compile(
    r"\bskipping\b[^\n]*(?:Git-for-Windows|(?:git|posix|sh).*shell|shell.*(?:unavailable|startup))",
    re.IGNORECASE,
)


def uses_libtest(package):
    # Run the GPUI harness in one process on every platform.
    return package == UI


def record(name, duration, returncode, **details):
    REPORTS.mkdir(parents=True, exist_ok=True)
    entry = dict(name=name, seconds=round(duration, 3), returncode=returncode,
                 recorded_at=datetime.now(timezone.utc).isoformat(), **details)
    with REPORT_LOCK:
        with (REPORTS / "timings.jsonl").open("a", encoding="utf-8") as report:
            report.write(json.dumps(entry) + "\n")
        summary = os.environ.get("GITHUB_STEP_SUMMARY")
        if summary:
            with open(summary, "a", encoding="utf-8") as out:
                out.write(f"- `{name}`: {duration:.1f}s, exit {returncode}\n")


def stop_process_tree(process):
    if os.name == "nt":
        # Kill descendants before the parent disappears from the process tree.
        try:
            subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=10, check=False)
        except (OSError, subprocess.TimeoutExpired) as error:
            print(f"::error::process-tree cleanup failed: {error}", file=sys.stderr, flush=True)
        finally:
            if process.poll() is None:
                process.kill()
    else:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            # The process group may have already exited; this race is safe to ignore.
            pass
    process.wait(timeout=10)


def reject_prerequisite_skips(name, text):
    if match := GIT_PREREQUISITE_SKIP.search(text):
        raise RuntimeError(f"{name}: required Git test did not run: {match.group()}")


def configure_output():
    # Redirected Windows streams can default to cp1252, while Cargo/nextest
    # produce UTF-8. Imported callers (runtime/probes) need the CLI policy too.
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure") and (stream.encoding != "utf-8" or stream.errors != "backslashreplace"):
            stream.reconfigure(encoding="utf-8", errors="backslashreplace")


def run(name, command, *, output=None, cwd=None, env=None, check=True, timeout=None, live=True, cancel=None):
    """Keep complete logs, surface runtime skips, and never mask subprocess failures."""
    configure_output()
    REPORTS.mkdir(parents=True, exist_ok=True)
    if cancel is not None and cancel.is_set():
        raise RuntimeError("test scheduling cancelled")
    command_text = "$ " + subprocess.list2cmdline([str(arg) for arg in command])
    if live:
        print(f"::group::{name}", flush=True)
        print(command_text, flush=True)
    start = time.monotonic()
    log_name = re.sub(r"[^a-zA-Z0-9_.-]", "-", name)
    log_path = REPORTS / f"{log_name}.log"
    timed_out = False
    with ExitStack() as stack:
        log = stack.enter_context(log_path.open("w", encoding="utf-8"))
        stdout = stack.enter_context(Path(output).open("w", encoding="utf-8")) if output else log
        # Tail a file instead of blocking on a pipe that a leaked descendant
        # could hold open after its parent exits. Deadlines cover silent hangs.
        reader = stack.enter_context(log_path.open(encoding="utf-8", errors="replace"))
        process = subprocess.Popen(command, cwd=ROOT if cwd is None else cwd, env=env, stdout=stdout,
                                   stderr=log, start_new_session=os.name != "nt")
        try:
            while process.poll() is None:
                if live:
                    print(reader.read(), end="", flush=True)
                if cancel is not None and cancel.is_set():
                    raise RuntimeError("test scheduling cancelled")
                if timeout is not None and time.monotonic() - start >= timeout:
                    timed_out = True
                    stop_process_tree(process)
                    break
                time.sleep(0.05)
            code = 124 if timed_out else process.wait()
            if live:
                print(reader.read(), end="", flush=True)
        except BaseException:
            stop_process_tree(process)
            record(name, time.monotonic() - start, 130, cancelled=True, timed_out=False,
                   command=[str(arg) for arg in command])
            with CONSOLE_LOCK:
                if not live:
                    print(f"::group::{name}", flush=True)
                    print(command_text, flush=True)
                    print(log_path.read_text(encoding="utf-8", errors="replace"), end="", flush=True)
                print("::endgroup::", flush=True)
            raise
    if timed_out:
        print(f"::error::{name}: exceeded {timeout}s; terminated process tree", flush=True)
    with REPORT_LOCK:
        with (REPORTS / "runtime-exclusions.log").open("a", encoding="utf-8") as excluded:
            for line in log_path.read_text(encoding="utf-8", errors="replace").splitlines():
                if re.search(r"\bskipping\b", line, re.IGNORECASE):
                    excluded.write(f"{name}: {line}\n")
    record(name, time.monotonic() - start, code, timed_out=timed_out,
           command=[str(arg) for arg in command])
    with CONSOLE_LOCK:
        if not live:
            print(f"::group::{name}", flush=True)
            print(command_text, flush=True)
            print(log_path.read_text(encoding="utf-8", errors="replace"), end="", flush=True)
        print("::endgroup::", flush=True)
    if check and code:
        raise subprocess.CalledProcessError(code, command)
    return code


def paths(context):
    directory = REPORTS / context
    directory.mkdir(parents=True, exist_ok=True)
    return directory


def reuse_args(context):
    directory = paths(context)
    return ["--binaries-metadata", str(directory / "binaries.json"),
            "--cargo-metadata", str(directory / "cargo.json")]


def inventory(context):
    return json.loads((paths(context) / "tests.json").read_text(encoding="utf-8"))


def package_names(context):
    metadata = json.loads((paths(context) / "cargo.json").read_text(encoding="utf-8"))
    return {package["id"]: package["name"] for package in metadata["packages"]}


def windows_linker_environment():
    """Discover the MSVC/SDK/Rust linker once, then share it with Cargo's links.

    This environment belongs only to the current Cargo invocation. Do not save
    it in a disk cache: another toolchain, SDK or target needs fresh discovery.
    """
    if os.name != "nt":
        return None
    started = time.monotonic()
    env = {name: value for name, value in os.environ.items()
           if not name.startswith("GITCOMET_LINKER_")}
    result = subprocess.run(["cmd.exe", "/d", "/u", "/c", "scripts\\windows\\msvc-linker.cmd",
                             "--gitcomet-print-env"], cwd=ROOT, env=dict(env),
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30, check=True)
    values = dict(line.split("=", 1) for line in result.stdout.decode("utf-16-le").splitlines() if "=" in line)
    for source, target in (("LINK_EXE", "EXE"), ("LIB", "LIB"), ("LIBPATH", "LIBPATH"),
                           ("INCLUDE", "INCLUDE"), ("GITCOMET_TARGET_ARCH", "ARCH")):
        if not values.get(source):
            raise RuntimeError(f"Linker bootstrap did not return {source}")
        env["GITCOMET_LINKER_" + target] = values[source]
    record("windows-linker-discovery", time.monotonic() - started, 0)
    return env


def prepare_runtime_binaries(context):
    """Keep static link directories out of Windows' runtime DLL search PATH.

    Cargo metadata includes native static-library directories too. Large
    workspaces can exceed CMD's inherited PATH limit, breaking nested tools.
    Keep DLL/executable directories and any directory we cannot inspect.
    """
    if os.name != "nt":
        return
    directory = paths(context)
    path = directory / "binaries.json"
    original = path.read_bytes()
    metadata = json.loads(original)
    build = metadata["rust-build-meta"]
    linked = build.get("linked-paths", [])
    target = Path(build["target-directory"])
    removed = []
    for name in linked:
        try:
            runtime_files = any(item.suffix.lower() in (".dll", ".exe", ".com", ".cmd", ".bat")
                                for item in (target / name).iterdir())
        except OSError:
            continue
        if not runtime_files:
            removed.append(name)
    if not removed:
        return
    (directory / "binaries-original.json").write_bytes(original)
    build["linked-paths"] = [name for name in linked if name not in removed]
    path.write_text(json.dumps(metadata) + "\n", encoding="utf-8")
    (directory / "runtime-link-paths.json").write_text(json.dumps({
        "removed_static_directories": removed,
        "retained_directories": list(build["linked-paths"]),
    }, indent=2) + "\n", encoding="utf-8")


def compile_tests(context, profile, test_targets=()):
    directory = paths(context)
    selection = CONTEXTS[context]
    targets = [arg for target in test_targets for arg in ("--test", target)]
    # Metadata cannot select packages, but must use the same feature switches.
    features = selection[1:] if selection[0] == "--workspace" else selection[2:]
    run(f"{context}-metadata", ["cargo", "metadata", "--format-version", "1", "--locked", *features],
        output=directory / "cargo.json")
    run(f"{context}-features", ["cargo", "tree", "--locked", *selection,
        "--edges", "normal,build,dev", "--prefix", "none", "--format", "{p}|{f}"],
        output=directory / "features.txt")
    run(f"{context}-compile", ["cargo", "nextest", "list", *selection, *targets,
        "--locked", "--cargo-profile", profile, "--timings",
        "--list-type", "binaries-only", "--message-format", "json"],
        output=directory / "binaries.json", env=windows_linker_environment())
    prepare_runtime_binaries(context)
    run(f"{context}-inventory", ["cargo", "nextest", "list", *reuse_args(context),
        "--message-format", "json", "--ignore-default-filter"], output=directory / "tests.json")
    packages = package_names(context)
    entries = []
    for binary_id, suite in inventory(context)["rust-suites"].items():
        package = packages[suite["package-id"]]
        for name, test in suite["testcases"].items():
            entries.append(dict(package=package, binary=binary_id, test=name,
                                ignored=test["ignored"],
                                runner="libtest" if uses_libtest(package) else "nextest"))
    if not entries:
        raise RuntimeError(f"No tests discovered for {context}")
    (directory / "coverage.json").write_text(json.dumps({
        "context": context, "selection": [*selection, *targets], "profile": profile,
        "rustc": subprocess.check_output(["rustc", "-vV"], text=True),
        "tests": entries,
    }, indent=2) + "\n", encoding="utf-8")
    print(f"Inventoried {len(entries)} tests ({sum(t['ignored'] for t in entries)} ignored)")


def suite_env(context, suite):
    env = dict(os.environ)
    binary_dir = str(Path(suite["binary-path"]).parent)
    metadata = json.loads((paths(context) / "binaries.json").read_text(encoding="utf-8"))
    build = metadata["rust-build-meta"]
    target = Path(build["target-directory"])
    # Match Cargo's dynamic-library search environment for direct libtest calls.
    search = [binary_dir, str(Path(binary_dir).parent)]
    search.extend(str(target / item) for item in build.get("linked-paths", {}))
    platforms = build.get("platforms", {})
    for platform in [platforms.get("host", {}), *platforms.get("targets", [])]:
        libdir = platform.get("libdir", {})
        if libdir.get("status") == "available":
            search.append(libdir["path"])
    variable = "PATH" if os.name == "nt" else "DYLD_FALLBACK_LIBRARY_PATH" if sys.platform == "darwin" else "LD_LIBRARY_PATH"
    env[variable] = os.pathsep.join(search + [env.get(variable, "")])
    return env


def run_suite(context, binary_id, suite, *, test_filter=None, exact=False, env_overrides=None, threads=None, live=True, cancel=None):
    expected = [name for name, test in suite["testcases"].items()
                if not test["ignored"] and (test_filter is None or
                    (name == test_filter if exact else test_filter in name))]
    if test_filter and not expected:
        raise RuntimeError(f"Smoke selector {test_filter!r} matches no tests in {binary_id}")
    command = [suite["binary-path"], "--nocapture"]
    if threads is not None:
        command += ["--test-threads", str(threads)]
    if test_filter:
        command += [test_filter]
    if exact:
        command += ["--exact"]
    env = suite_env(context, suite)
    env.update(env_overrides or {})
    name = f"{context}-{binary_id}-{test_filter or 'all'}"
    if env_overrides:
        name += "-" + env_overrides["XDG_SESSION_TYPE"] + "-" + env_overrides["XDG_CURRENT_DESKTOP"]
    code = run(name, command, cwd=suite["cwd"], env=env, check=False,
               timeout=180 if test_filter else 600, live=live, cancel=cancel)
    log_name = re.sub(r"[^a-zA-Z0-9_.-]", "-", name)
    log = (REPORTS / f"{log_name}.log").read_text(encoding="utf-8", errors="replace")
    reject_prerequisite_skips(name, log)
    summaries = re.findall(r"test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed;", log)
    if not code and (not summaries or sum(map(int, summaries[-1])) != len(expected)):
        raise RuntimeError(f"{name}: libtest did not execute the inventoried test count ({len(expected)})")
    return code


def check_nextest_results(context, suites, packages, junit):
    expected = {(binary_id, name) for binary_id, suite in suites.items()
                if not uses_libtest(packages[suite["package-id"]])
                for name, test in suite["testcases"].items() if not test["ignored"]}
    xml = ET.parse(junit)
    for case in xml.iter("testcase"):
        reject_prerequisite_skips(f"{context}:{case.attrib['name']}",
                                  case.findtext("system-out", "") + "\n" + case.findtext("system-err", ""))
    actual = {(suite.attrib["name"], case.attrib["name"])
              for suite in xml.getroot().findall("testsuite") for case in suite.findall("testcase")
              if case.find("skipped") is None}
    if expected != actual:
        raise RuntimeError(f"{context}: nextest coverage mismatch: {len(expected - actual)} missing, {len(actual - expected)} unexpected")
    with REPORT_LOCK, (REPORTS / "runtime-exclusions.log").open("a", encoding="utf-8") as excluded:
        for case in xml.iter("testcase"):
            for stream in (case.findtext("system-out", ""), case.findtext("system-err", "")):
                for line in stream.splitlines():
                    if re.search(r"\bskipping\b", line, re.IGNORECASE):
                        excluded.write(f"{context}:{case.attrib['name']}: {line}\n")


def run_parallel(tasks):
    """Bound concurrency and join/clean every child on orchestration failure."""
    cancel = threading.Event()
    executor = ThreadPoolExecutor(max_workers=2)
    futures = []
    try:
        futures = [executor.submit(task, cancel=cancel, live=False) for task in tasks]
        return [future.result() for future in as_completed(futures)]
    except BaseException:
        cancel.set()
        for future in futures:
            future.cancel()
        raise
    finally:
        executor.shutdown(wait=True, cancel_futures=True)


def execute(context, schedule="serial", nextest_threads=None, nextest_profile="ci", ui_threads=None):
    for option, threads in (("nextest", nextest_threads), ("ui", ui_threads)):
        if threads is not None and (threads < 1 or schedule != "serial"):
            raise ValueError(f"--{option}-threads must be positive and requires --schedule serial")
    if nextest_profile not in NEXTEST_PROFILES:
        raise ValueError(f"Unsupported nextest profile: {nextest_profile}")
    prepare_runtime_binaries(context)
    packages = package_names(context)
    suites = inventory(context)["rust-suites"]
    cpus = os.cpu_count() or 1
    balanced = schedule == "balanced" and cpus > 1
    start = time.monotonic()
    codes = []

    def nextest(*, cancel=None, live=True, threads=None):
        if not any(not uses_libtest(packages[suite["package-id"]]) for suite in suites.values()):
            return 0
        build = json.loads((paths(context) / "binaries.json").read_text(encoding="utf-8"))
        junit = Path(build["rust-build-meta"]["target-directory"]) / "nextest" / nextest_profile / "junit.xml"
        junit.unlink(missing_ok=True)
        command = ["cargo", "nextest", "run", *reuse_args(context), "--profile", nextest_profile,
                   "--ignore-default-filter", "-E", f"not package(={UI})", "--no-fail-fast"]
        if threads is not None:
            command += ["--test-threads", str(threads)]
        code = run(f"{context}-nextest", command, check=False, live=live, cancel=cancel)
        if junit.exists():
            shutil.copyfile(junit, paths(context) / "junit.xml")
            check_nextest_results(context, suites, packages, junit)
        elif not code:
            raise RuntimeError(f"{context}: nextest produced no results")
        return code

    libtest = [(binary_id, suite) for binary_id, suite in suites.items()
               if uses_libtest(packages[suite["package-id"]])]
    if not libtest or len(libtest) == len(suites):
        # Package-only contexts have nothing to overlap. Keep their full budget.
        balanced = False
    effective_ui_threads = max(1, cpus // 2) if balanced else ui_threads
    default_ui_threads = os.environ.get("RUST_TEST_THREADS", str(cpus))
    # Invalid environment overrides will fail libtest, but must not prevent the
    # finally block from recording that failure.
    default_ui_threads = int(default_ui_threads) if default_ui_threads.isdecimal() else None

    def ui(*, cancel=None, live=True):
        results = [run_suite(context, binary_id, suite, threads=effective_ui_threads, live=live, cancel=cancel)
                   for binary_id, suite in libtest]
        return int(any(results))

    succeeded = False
    try:
        if balanced:
            codes.extend(run_parallel([partial(nextest, threads=cpus - effective_ui_threads), ui]))
        else:
            codes.append(nextest(threads=nextest_threads))
            for binary_id, suite in libtest:
                codes.append(run_suite(context, binary_id, suite, threads=ui_threads))
        if any(codes):
            raise RuntimeError(f"{context}: test execution failed")
        succeeded = True
    finally:
        (paths(context) / "execution.json").write_text(json.dumps({
            "schedule": schedule, "effective_schedule": "balanced" if balanced else "serial",
            "nextest_profile": nextest_profile,
            "ui_threads": ui_threads,
            "effective_ui_threads": effective_ui_threads if effective_ui_threads is not None else
                                    default_ui_threads,
            "effective_nextest_threads": cpus - effective_ui_threads if balanced else nextest_threads or cpus,
            "nextest_threads": nextest_threads, "cpus": cpus, "seconds": round(time.monotonic() - start, 3), "success": succeeded,
        }, indent=2) + "\n", encoding="utf-8")


def smoke(context, target, selector, *, exact=False, env=None):
    matches = [(binary_id, suite) for binary_id, suite in inventory(context)["rust-suites"].items()
               if suite["binary-name"] == target]
    if len(matches) != 1:
        raise RuntimeError(f"Expected exactly one {target} binary, found {len(matches)}")
    binary_id, suite = matches[0]
    if run_suite(context, binary_id, suite, test_filter=selector, exact=exact, env_overrides=env):
        raise RuntimeError(f"Smoke test failed: {selector}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=["compile", "test", "doc", "display", "cmd-smoke", "command"])
    parser.add_argument("--context", choices=CONTEXTS, default="workspace")
    parser.add_argument("--cargo-profile", default="ci-test")
    parser.add_argument("--test-target", action="append", default=[],
                        help="Compile only this integration target; repeat for multiple smoke targets")
    parser.add_argument("--name", default="command")
    parser.add_argument("--schedule", choices=["serial", "balanced"], default="serial")
    parser.add_argument("--nextest-threads", type=int, help="Opt-in concurrency experiment (serial schedule only)")
    parser.add_argument("--ui-threads", type=int, help="Opt-in libtest concurrency experiment (serial schedule only)")
    parser.add_argument("--nextest-profile", choices=NEXTEST_PROFILES, default="ci")
    args, extra = parser.parse_known_args()
    if args.test_target and args.phase != "compile":
        parser.error("--test-target requires compile")
    for option in ("nextest", "ui"):
        threads = getattr(args, option + "_threads")
        if threads is not None and (args.phase != "test" or threads < 1 or args.schedule != "serial"):
            parser.error(f"--{option}-threads must be positive and requires test --schedule serial")
    os.chdir(ROOT)
    if args.phase == "compile":
        compile_tests(args.context, args.cargo_profile, args.test_target)
    elif args.phase == "test":
        execute(args.context, args.schedule, args.nextest_threads, args.nextest_profile, args.ui_threads)
    elif args.phase == "doc":
        # The app contains only binaries, so it has no doctest targets.
        if args.context != "app":
            run(f"{args.context}-doctests", ["cargo", "test", *CONTEXTS[args.context],
                "--doc", "--locked", "--profile", args.cargo_profile, "--no-fail-fast"])
    elif args.phase == "display":
        for name, values in DISPLAY_PROFILES.items():
            env = dict(zip(["DISPLAY", "WAYLAND_DISPLAY", "XDG_SESSION_TYPE", "XDG_CURRENT_DESKTOP"], values))
            print(f"Display profile: {name}: {env}", flush=True)
            # Keep the headless app context; the workspace already includes the
            # UI's default features and its complete test suite.
            for target in ("mergetool_git_integration", "difftool_git_integration"):
                smoke("app", target, "gui_default", env=env)
            smoke("workspace", "gitcomet_ui_gpui", "smoke_tests::smoke_view_renders_without_panicking", exact=True, env=env)
    elif args.phase == "cmd-smoke":
        smoke("app", "standalone_tool_mode_integration", "help_flag_exits_zero", exact=True)
    else:
        command = extra[1:] if extra[:1] == ["--"] else extra
        if not command:
            parser.error("command requires an executable after --")
        run(args.name, command)


if __name__ == "__main__":
    configure_output()
    try:
        main()
    except (RuntimeError, subprocess.CalledProcessError) as error:
        print(f"::error::{error}", file=sys.stderr)
        sys.exit(1)
