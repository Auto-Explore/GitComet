"""Measure Rust lines in the shipped GitComet binaries hit by the native test suites."""

from contextlib import ExitStack
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
TARGET = ROOT / "target"
PROFILE = "ci-test"
EXE = ".exe" if os.name == "nt" else ""
TEST_CONTEXTS = ("workspace", "app-gui", "example")


def trace_lines(root, trace):
    source = None
    with trace.open(encoding="utf-8") as records:
        for record in records:
            if record.startswith("SF:"):
                path = (root / record[3:].rstrip("\r\n")).resolve()
                source = None
                if path.is_relative_to(root):
                    relative = path.relative_to(root)
                    if (len(relative.parts) >= 4 and relative.parts[0] == "crates"
                            and relative.parts[2] == "src" and relative.suffix == ".rs"):
                        source = relative.as_posix()
            elif source is not None and record.startswith("DA:"):
                number, count, *_ = record[3:].split(",")
                yield source, int(number), int(count)
            elif record.rstrip() == "end_of_record":
                source = None


def product_lines(root, baselines, observations):
    files = {}
    if not baselines:
        raise ValueError("no product coverage maps")
    for trace in baselines:
        found_lines = False
        for source, line, _ in trace_lines(root, trace):
            found_lines = True
            files.setdefault(source, {})[line] = False
        if not found_lines:
            raise ValueError(f"{trace.name} contains no measurable first-party Rust lines")
    for trace in observations:
        for source, line, count in trace_lines(root, trace):
            if count > 0 and source in files and line in files[source]:
                files[source][line] = True
    return files


def write_reports(directory, files):
    totals = {}
    with (directory / "lcov.info").open("w", encoding="utf-8", newline="\n") as out:
        for source, lines in sorted(files.items()):
            covered, measured = sum(lines.values()), len(lines)
            crate = source.split("/")[1]
            hits, count = totals.get(crate, (0, 0))
            totals[crate] = hits + covered, count + measured
            out.write(f"SF:{source}\n")
            for line, hit in sorted(lines.items()):
                out.write(f"DA:{line},{int(hit)}\n")
            out.write(f"LF:{measured}\nLH:{covered}\nend_of_record\n")
    covered, measured = map(sum, zip(*totals.values()))
    with (directory / "summary.md").open("w", encoding="utf-8") as out:
        out.write("# GitComet product line coverage\n\n")
        out.write("GUI and headless Rust source lines exercised by the native tests on "
                  f"{platform.system()} ({platform.machine()}).\n\n")
        out.write("| Crate | Covered lines | Measurable lines | Coverage |\n| --- | ---: | ---: | ---: |\n")
        out.write(f"| **Total** | **{covered}** | **{measured}** | **{covered / measured:.2%}** |\n")
        for crate, (hits, count) in sorted(totals.items()):
            out.write(f"| {crate} | {hits} | {count} | {hits / count:.2%} |\n")


def export_lines(llvm, executable, destination, profile=None):
    command = [llvm / ("llvm-cov" + EXE), "export", "-format=lcov", "-skip-functions",
               r"-include-filename-regex=[/\\]crates[/\\][^/\\]+[/\\]src[/\\]",
               f"-instr-profile={profile}" if profile else "-empty-profile", executable]
    with destination.open("wb") as out:
        subprocess.run(command, stdout=out, check=True)


def cargo_executables(messages):
    with messages.open(encoding="utf-8") as records:
        artifacts = map(json.loads, records)
        return sorted({item["executable"] for item in artifacts if item.get("executable")})


def test_traces(llvm, raw, executables):
    """Separate exports keep LLVM from dropping covered functions shared by multiple binaries."""
    by_build_id = {}
    linux = sys.platform.startswith("linux")
    for path in raw.glob("*.profraw"):
        build_id = path.name.partition("-")[0] if linux else None
        by_build_id.setdefault(build_id, []).append(path)
    if not by_build_id:
        raise ValueError(f"no test profiles recorded for {raw.name}")

    def merge(paths, destination):
        subprocess.run([llvm / ("llvm-profdata" + EXE), "merge", "-sparse", "-failure-mode=any",
                        *paths, "-o", destination], check=True)

    profile = raw.with_suffix(".profdata")
    if not linux:
        merge([raw], profile)
    traces = []
    flags = ["--sections", "--notes"] if linux else ["--sections"]
    for index, executable in enumerate(executables):
        sections = subprocess.check_output([llvm / ("llvm-readobj" + EXE), *flags, executable], text=True)
        if linux:
            build_id = re.search(r"Build ID: ([0-9a-fA-F]+)", sections)
            profiles = by_build_id.pop(build_id[1].lower(), []) if build_id else []
        if "__llvm_covmap" not in sections and ".lcovmap" not in sections:
            continue
        if linux:
            if not build_id:
                raise ValueError(f"instrumented executable {executable} has no ELF build ID")
            if not profiles:
                continue
            merge(profiles, profile)
        trace = raw.parent / f"{raw.name}-{index}.info"
        export_lines(llvm, executable, trace, profile)
        traces.append(trace)
    if linux and by_build_id:
        raise ValueError(f"{raw.name} recorded profiles outside its Cargo executable inventory: "
                         f"{', '.join(sorted(by_build_id))}")
    if not traces:
        raise ValueError(f"no instrumented test executables for {raw.name}")
    return traces


def collect(work):
    tool_lib = subprocess.check_output(["rustc", "--print", "target-libdir"], text=True).strip()
    llvm = Path(tool_lib).parent / "bin"
    if not (llvm / ("llvm-cov" + EXE)).is_file():
        raise ValueError("missing Rust LLVM tools; run rustup component add llvm-tools-preview")
    inventories = work / "ci"
    setup = dict(os.environ, LLVM_PROFILE_FILE=str(work / "setup-%m.profraw"),
                 GITCOMET_CI_REPORT_DIR=str(inventories))
    setup.pop("GITHUB_STEP_SUMMARY", None)
    baselines, observations = [], []
    for variant, features in (("gui", []), ("headless", ["--no-default-features", "--features", "gix"])):
        messages = work / f"product-{variant}.jsonl"
        with messages.open("wb") as out:
            subprocess.run(["cargo", "build", "--locked", "--profile", PROFILE, "-p", "gitcomet",
                            "--bin", "gitcomet", "--message-format=json-render-diagnostics", *features],
                           stdout=out, env=setup, check=True)
        trace = work / f"product-{variant}.info"
        executable, = cargo_executables(messages)
        export_lines(llvm, executable, trace)
        baselines.append(trace)

    runner = [sys.executable, str(ROOT / "scripts/ci/run.py")]
    pattern = "%b-%4m.profraw" if sys.platform.startswith("linux") else "%4m.profraw"
    for context in TEST_CONTEXTS:
        raw = work / context
        raw.mkdir()
        subprocess.run([*runner, "compile", "--context", context, "--cargo-profile", PROFILE],
                       env=setup, check=True)
        subprocess.run([*runner, "test", "--context", context],
                       env=dict(setup, LLVM_PROFILE_FILE=str(raw / pattern)), check=True)
        executables = cargo_executables(inventories / context / "cargo-messages.jsonl")
        observations.extend(test_traces(llvm, raw, executables))
    return product_lines(ROOT, baselines, observations)


def lock_build(lock):
    try:
        if os.name == "nt":
            import msvcrt
            msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except (BlockingIOError, PermissionError) as error:
        raise RuntimeError("another coverage run owns this checkout's instrumented build") from error


def publish_reports(staged, output):
    output.mkdir(exist_ok=True)
    previous = staged.parent / "previous"
    previous.mkdir()
    with ExitStack() as rollback:
        for name in ("html", "lcov.info", "summary.md"):
            destination = output / name
            if destination.exists():
                backup = previous / name
                destination.replace(backup)
                rollback.callback(backup.replace, destination)
            (staged / name).replace(destination)
            rollback.callback(destination.replace, staged / name)
        rollback.pop_all()


def main():
    TARGET.mkdir(exist_ok=True)
    output = TARGET / "llvm-cov"
    with (TARGET / ".source-coverage.lock").open("a+b") as lock:
        lock_build(lock)
        with tempfile.TemporaryDirectory(prefix=".source-coverage-", dir=TARGET) as temp:
            work = Path(temp)
            report = work / "report"
            report.mkdir()
            write_reports(report, collect(work))
            subprocess.run(["genhtml", "--quiet", "--no-function-coverage", "--legend",
                            "--title", "GitComet source line coverage", "--output-directory",
                            report / "html", report / "lcov.info"], check=True, cwd=ROOT)
            publish_reports(report, output)
        print((output / "summary.md").read_text(encoding="utf-8"))
        print(f"LCOV: {output / 'lcov.info'}\nHTML: {output / 'html/index.html'}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        sys.exit(f"Source coverage failed: {error}")
