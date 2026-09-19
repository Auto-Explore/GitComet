#!/usr/bin/env python3
"""Build once, inventory every test, then run nextest + the GPUI libtest harness."""

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]
REPORTS = ROOT / "target" / "ci-reports"
UI = "gitcomet-ui-gpui"
CONTEXTS = {
    "workspace": ["--workspace", "--no-default-features", "--features", "gix"],
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
WINDOWS_LIBTEST_BINARIES = {
    "mergetool_git_integration", "difftool_git_integration",
    "standalone_tool_mode_integration", "submodules_integration",
    "remote_management_integration",
}


def uses_libtest(package, suite, platform_name=sys.platform):
    # These Windows binaries cache expensive Git-shell capability probes with
    # OnceLock. A process per test would repeat each probe dozens of times.
    return package == UI or (platform_name == "win32" and suite["binary-name"] in WINDOWS_LIBTEST_BINARIES)


def record(name, duration, returncode, **details):
    REPORTS.mkdir(parents=True, exist_ok=True)
    entry = dict(name=name, seconds=round(duration, 3), returncode=returncode,
                 recorded_at=datetime.now(timezone.utc).isoformat(), **details)
    with (REPORTS / "timings.jsonl").open("a", encoding="utf-8") as report:
        report.write(json.dumps(entry) + "\n")
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as out:
            out.write(f"- `{name}`: {duration:.1f}s, exit {returncode}\n")


def run(name, command, *, output=None, cwd=ROOT, env=None, check=True):
    """Keep complete logs, surface runtime skips, and never mask subprocess failures."""
    REPORTS.mkdir(parents=True, exist_ok=True)
    print(f"::group::{name}", flush=True)
    print("$ " + subprocess.list2cmdline([str(arg) for arg in command]), flush=True)
    start = time.monotonic()
    log_name = re.sub(r"[^a-zA-Z0-9_.-]", "-", name)
    with (REPORTS / f"{log_name}.log").open("w", encoding="utf-8") as log:
        if output:
            with Path(output).open("w", encoding="utf-8") as stream:
                process = subprocess.Popen(command, cwd=cwd, env=env, stdout=stream,
                                           stderr=subprocess.PIPE, text=True, errors="replace")
                for line in process.stderr:
                    log.write(line)
                    print(line, end="", flush=True)
        else:
            process = subprocess.Popen(command, cwd=cwd, env=env, stdout=subprocess.PIPE,
                                       stderr=subprocess.STDOUT, text=True, errors="replace")
            for line in process.stdout:
                log.write(line)
                print(line, end="", flush=True)
                if re.search(r"\bskipping\b", line, re.IGNORECASE):
                    with (REPORTS / "runtime-exclusions.log").open("a", encoding="utf-8") as excluded:
                        excluded.write(f"{name}: {line}")
        code = process.wait()
        if process.stdout:
            process.stdout.close()
        if process.stderr:
            process.stderr.close()
    record(name, time.monotonic() - start, code)
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


def compile_tests(context, profile):
    directory = paths(context)
    selection = CONTEXTS[context]
    # Metadata cannot select packages, but must use the same feature switches.
    features = selection[1:] if selection[0] == "--workspace" else selection[2:]
    run(f"{context}-metadata", ["cargo", "metadata", "--format-version", "1", "--locked", *features],
        output=directory / "cargo.json")
    run(f"{context}-features", ["cargo", "tree", "--locked", *selection,
        "--edges", "normal,build,dev", "--prefix", "none", "--format", "{p}|{f}"],
        output=directory / "features.txt")
    run(f"{context}-compile", ["cargo", "nextest", "list", *selection,
        "--locked", "--cargo-profile", profile, "--timings",
        "--list-type", "binaries-only", "--message-format", "json"],
        output=directory / "binaries.json")
    run(f"{context}-inventory", ["cargo", "nextest", "list", *reuse_args(context),
        "--message-format", "json", "--ignore-default-filter"], output=directory / "tests.json")
    packages = package_names(context)
    entries = []
    for binary_id, suite in inventory(context)["rust-suites"].items():
        package = packages[suite["package-id"]]
        for name, test in suite["testcases"].items():
            entries.append(dict(package=package, binary=binary_id, test=name,
                                ignored=test["ignored"],
                                runner="libtest" if uses_libtest(package, suite) else "nextest"))
    if not entries:
        raise RuntimeError(f"No tests discovered for {context}")
    (directory / "coverage.json").write_text(json.dumps({
        "context": context, "selection": selection, "profile": profile,
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


def run_suite(context, binary_id, suite, *, test_filter=None, exact=False, env_overrides=None):
    expected = [name for name, test in suite["testcases"].items()
                if not test["ignored"] and (test_filter is None or
                    (name == test_filter if exact else test_filter in name))]
    if test_filter and not expected:
        raise RuntimeError(f"Smoke selector {test_filter!r} matches no tests in {binary_id}")
    command = [suite["binary-path"], "--nocapture"]
    if sys.platform == "win32" and suite["binary-name"] in WINDOWS_LIBTEST_BINARIES:
        command += ["--test-threads", "2"]
    if test_filter:
        command += [test_filter]
    if exact:
        command += ["--exact"]
    env = suite_env(context, suite)
    env.update(env_overrides or {})
    name = f"{context}-{binary_id}-{test_filter or 'all'}"
    if env_overrides:
        name += "-" + env_overrides["XDG_SESSION_TYPE"] + "-" + env_overrides["XDG_CURRENT_DESKTOP"]
    code = run(name, command, cwd=suite["cwd"], env=env, check=False)
    log_name = re.sub(r"[^a-zA-Z0-9_.-]", "-", name)
    log = (REPORTS / f"{log_name}.log").read_text(encoding="utf-8")
    summaries = re.findall(r"test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed;", log)
    if not code and (not summaries or sum(map(int, summaries[-1])) != len(expected)):
        raise RuntimeError(f"{name}: libtest did not execute the inventoried test count ({len(expected)})")
    return code


def check_nextest_results(context, suites, packages, junit):
    expected = {(binary_id, name) for binary_id, suite in suites.items()
                if not uses_libtest(packages[suite["package-id"]], suite)
                for name, test in suite["testcases"].items() if not test["ignored"]}
    xml = ET.parse(junit)
    actual = {(suite.attrib["name"], case.attrib["name"])
              for suite in xml.getroot().findall("testsuite") for case in suite.findall("testcase")
              if case.find("skipped") is None}
    if expected != actual:
        raise RuntimeError(f"{context}: nextest coverage mismatch: {len(expected - actual)} missing, {len(actual - expected)} unexpected")
    with (REPORTS / "runtime-exclusions.log").open("a", encoding="utf-8") as excluded:
        for case in xml.iter("testcase"):
            for stream in (case.findtext("system-out", ""), case.findtext("system-err", "")):
                for line in stream.splitlines():
                    if re.search(r"\bskipping\b", line, re.IGNORECASE):
                        excluded.write(f"{context}:{case.attrib['name']}: {line}\n")


def execute(context):
    packages = package_names(context)
    suites = inventory(context)["rust-suites"]
    codes = []
    if any(not uses_libtest(packages[suite["package-id"]], suite) for suite in suites.values()):
        build = json.loads((paths(context) / "binaries.json").read_text(encoding="utf-8"))
        junit = Path(build["rust-build-meta"]["target-directory"]) / "nextest/ci/junit.xml"
        junit.unlink(missing_ok=True)  # Never accept an old run's successful report.
        excluded = [f"package(={UI})"]
        if sys.platform == "win32":
            excluded += [f"binary(={name})" for name in sorted(WINDOWS_LIBTEST_BINARIES)]
        codes.append(run(f"{context}-nextest", ["cargo", "nextest", "run",
            *reuse_args(context), "--profile", "ci", "--ignore-default-filter",
            "-E", "not (" + " | ".join(excluded) + ")", "--no-fail-fast"], check=False))
        if junit.exists():
            shutil.copyfile(junit, paths(context) / "junit.xml")
            check_nextest_results(context, suites, packages, junit)
        elif not codes[-1]:
            raise RuntimeError(f"{context}: nextest produced no results")
    for binary_id, suite in suites.items():
        if uses_libtest(packages[suite["package-id"]], suite):
            codes.append(run_suite(context, binary_id, suite))
    if any(codes):
        raise RuntimeError(f"{context}: test execution failed")


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
    parser.add_argument("--name", default="command")
    args, extra = parser.parse_known_args()
    os.chdir(ROOT)
    if args.phase == "compile":
        compile_tests(args.context, args.cargo_profile)
    elif args.phase == "test":
        execute(args.context)
    elif args.phase == "doc":
        # The app contains only binaries, so it has no doctest targets.
        if args.context != "app":
            run(f"{args.context}-doctests", ["cargo", "test", *CONTEXTS[args.context],
                "--doc", "--locked", "--profile", args.cargo_profile, "--no-fail-fast"])
    elif args.phase == "display":
        for name, values in DISPLAY_PROFILES.items():
            env = dict(zip(["DISPLAY", "WAYLAND_DISPLAY", "XDG_SESSION_TYPE", "XDG_CURRENT_DESKTOP"], values))
            print(f"Display profile: {name}: {env}", flush=True)
            # These package-only contexts deliberately preserve their original
            # feature graphs, including the headless app and default-feature UI.
            for target in ("mergetool_git_integration", "difftool_git_integration"):
                smoke("app", target, "gui_default", env=env)
            smoke("ui", "gitcomet_ui_gpui", "smoke_tests::smoke_view_renders_without_panicking", exact=True, env=env)
    elif args.phase == "cmd-smoke":
        smoke("app", "standalone_tool_mode_integration", "help_flag_exits_zero", exact=True)
    else:
        command = extra[1:] if extra[:1] == ["--"] else extra
        if not command:
            parser.error("command requires an executable after --")
        run(args.name, command)


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, subprocess.CalledProcessError) as error:
        print(f"::error::{error}", file=sys.stderr)
        sys.exit(1)
