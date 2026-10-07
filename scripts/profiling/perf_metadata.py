#!/usr/bin/env python3
"""Record what a performance run depends on, so two runs can be compared honestly.

Importable (`collect`) and usable as a command:

  python3 scripts/profiling/perf_metadata.py --output run/environment.json \
      --binary gitcomet=target/release/gitcomet --cargo-profile release --command "..."

Everything is best effort: a missing tool is recorded as unavailable rather than
failing the run. Compare two captures with `--compare A B`, which lists the
fields that differ and flags the ones that invalidate a paired comparison.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
# A baseline/candidate pair may differ only in source and binaries; any change
# in these fields means the comparison measures the machine, not the code.
PAIR_INVARIANT_FIELDS = (
    "toolchain.rustc", "toolchain.cargo_lock_sha256", "build",
    "machine.cpu_model", "machine.kernel", "machine.cpu_governor", "machine.cpu_boost",
    "gpu.cards", "gpu.vulkan", "gpu.nvidia", "display.session_type", "allocator", "git.version",
    "gpu.native", "git.lfs", "git.annex",
)


def run(command, **kwargs):
    try:
        completed = subprocess.run(command, capture_output=True, text=True, timeout=30, **kwargs)
    except (OSError, subprocess.TimeoutExpired) as error:
        return {"unavailable": str(error)}
    if completed.returncode != 0:
        return {"unavailable": completed.stderr.strip()[:500] or f"exit {completed.returncode}"}
    return completed.stdout.strip()


def read(path):
    try:
        return Path(path).read_text(encoding="utf-8", errors="replace").strip()
    except OSError:
        return None


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as file:
        for block in iter(lambda: file.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def source_info():
    diff = subprocess.run(["git", "-C", str(ROOT), "diff", "HEAD", "--binary"], capture_output=True)
    untracked = run(["git", "-C", str(ROOT), "ls-files", "--others", "--exclude-standard"])
    return {
        "revision": run(["git", "-C", str(ROOT), "rev-parse", "HEAD"]),
        "branch": run(["git", "-C", str(ROOT), "symbolic-ref", "--short", "-q", "HEAD"]),
        "patch_sha256": hashlib.sha256(diff.stdout).hexdigest(),
        "patch_bytes": len(diff.stdout),
        "untracked_files": untracked.splitlines() if isinstance(untracked, str) and untracked else [],
    }


def toolchain_info():
    lock = ROOT / "Cargo.lock"
    return {
        "rustc": run(["rustc", "-vV"], cwd=ROOT),
        "cargo": run(["cargo", "-V"], cwd=ROOT),
        "rust_toolchain_toml": read(ROOT / "rust-toolchain.toml"),
        "cargo_lock_sha256": sha256_file(lock) if lock.exists() else None,
    }


def fixture_info(path):
    path = Path(path).resolve()
    info = {"path": str(path)}
    if (path / ".git").exists():
        status = subprocess.run(["git", "-C", str(path), "status", "--porcelain=v1", "-z"],
                                capture_output=True)
        info.update(
            head=run(["git", "-C", str(path), "rev-parse", "HEAD"]),
            tree=run(["git", "-C", str(path), "rev-parse", "HEAD^{tree}"]),
            status_sha256=hashlib.sha256(status.stdout).hexdigest(),
            clean=status.returncode == 0 and not status.stdout,
            commits=run(["git", "-C", str(path), "rev-list", "--count", "--all"]),
            objects=run(["git", "-C", str(path), "count-objects", "-v"]),
        )
    elif path.is_file():
        info["sha256"] = sha256_file(path)
    storage = run(["findmnt", "-n", "-o", "SOURCE,FSTYPE,TARGET", "--target", str(path)])
    info["storage"] = storage
    if isinstance(storage, str) and storage:
        device = storage.split()[0].split("[")[0]
        info["storage_device"] = run(["lsblk", "-ndo", "MODEL,ROTA,TRAN", device])
    return info


def parse_json(output):
    if not isinstance(output, str):
        return output
    try:
        return json.loads(output)
    except ValueError:
        return output


def machine_info():
    cpu = read("/proc/cpuinfo") or ""
    model = next((line.split(":", 1)[1].strip() for line in cpu.splitlines()
                  if line.startswith("model name")), platform.processor())
    meminfo = {line.split(":")[0]: line.split(":")[1].strip()
               for line in (read("/proc/meminfo") or "").splitlines() if ":" in line}
    loadavg = read("/proc/loadavg")
    if platform.system() == "Darwin":
        model = run(["sysctl", "-n", "machdep.cpu.brand_string"])
        meminfo["MemTotal"] = run(["sysctl", "-n", "hw.memsize"])
    elif platform.system() == "Windows":
        model = run(["powershell", "-NoProfile", "-Command", "(Get-CimInstance Win32_Processor).Name"])
    return {
        "hostname": platform.node(),
        "cpu_model": model,
        "logical_cpus": os.cpu_count(),
        "kernel": platform.release(),
        "os": read("/etc/os-release") or platform.platform(),
        "cpu_governor": read("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
        "cpu_driver": read("/sys/devices/system/cpu/cpu0/cpufreq/scaling_driver"),
        "cpu_boost": read("/sys/devices/system/cpu/cpufreq/boost"),
        "energy_performance_preference": read(
            "/sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference"),
        "memory_total": meminfo.get("MemTotal"),
        "memory_available": meminfo.get("MemAvailable"),
        "swap_free": meminfo.get("SwapFree"),
        "swap_total": meminfo.get("SwapTotal"),
        "loadavg": loadavg,
        "thermal": parse_json(run(["sensors", "-j"])),
    }


def busy_processes(limit=8):
    """Competing work at capture time: a paired run on a busy machine is suspect."""
    output = run(["ps", "-eo", "pid,pcpu,rss,comm", "--sort=-pcpu", "--no-headers"])
    if not isinstance(output, str):
        return output
    rows = []
    for line in output.splitlines()[:limit]:
        pid, pcpu, rss, comm = line.split(None, 3)
        rows.append({"pid": int(pid), "cpu_percent": float(pcpu), "rss_kib": int(rss), "command": comm})
    return rows


def gpu_info():
    cards = []
    for card in sorted(Path("/sys/class/drm").glob("card[0-9]")):
        device = card / "device"
        driver = device / "driver"
        cards.append({"card": card.name, "vendor": read(device / "vendor"), "device": read(device / "device"),
                      "driver": driver.resolve().name if driver.exists() else None})
    info = {"cards": cards}
    if platform.system() == "Darwin":
        info["native"] = parse_json(run(["system_profiler", "SPDisplaysDataType", "-json"]))
    elif platform.system() == "Windows":
        info["native"] = parse_json(run(["powershell", "-NoProfile", "-Command",
            "Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion | ConvertTo-Json"]))
    if shutil.which("nvidia-smi"):
        # Model and driver identify the setup; clocks and temperature are
        # state at capture time and differ between any two captures.
        info["nvidia"] = run(["nvidia-smi", "--query-gpu=name,driver_version", "--format=csv,noheader"])
        info["nvidia_state"] = run(["nvidia-smi", "--query-gpu=pstate,clocks.gr,temperature.gpu",
                                    "--format=csv,noheader"])
    if shutil.which("vulkaninfo"):
        summary = run(["vulkaninfo", "--summary"])
        if isinstance(summary, str):
            info["vulkan"] = [line.strip() for line in summary.splitlines()
                              if any(key in line for key in ("deviceName", "driverName", "driverInfo", "apiVersion"))]
    return info


def display_info():
    info = {key.lower(): os.environ.get(key) for key in
            ("XDG_SESSION_TYPE", "WAYLAND_DISPLAY", "DISPLAY", "XDG_CURRENT_DESKTOP")}
    info["session_type"] = info.pop("xdg_session_type")
    if shutil.which("xrandr") and os.environ.get("DISPLAY"):
        modes = run(["xrandr", "--current"])
        if isinstance(modes, str):
            # Connected outputs and their active mode (marked with '*').
            info["outputs"] = [line.strip() for line in modes.splitlines()
                               if " connected" in line or "*" in line]
    return info


def git_info():
    return {
        "version": run(["git", "--version"]),
        "config": run(["git", "config", "--list", "--show-origin"]),
        "lfs": run(["git", "lfs", "version"]),
        "annex": run(["git", "annex", "version", "--raw"]),
    }


def collect(binaries=(), fixtures=(), cargo_profile=None, features=None, command=None):
    return {
        "version": 1,
        "captured_unix_ms": int(time.time() * 1000),
        "command": command if command is not None else " ".join(sys.argv),
        "source": source_info(),
        "toolchain": toolchain_info(),
        "build": {
            "cargo_profile": cargo_profile, "features": features,
            "env": {key: value for key, value in os.environ.items()
                    if key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_RUSTFLAGS")
                    or key.startswith("CARGO_PROFILE_")},
        },
        "binaries": {name: {"path": str(Path(path).resolve()), "sha256": sha256_file(path),
                            "bytes": Path(path).stat().st_size} for name, path in binaries},
        "fixtures": {name: fixture_info(path) for name, path in fixtures},
        "machine": machine_info(),
        "busy_processes": busy_processes(),
        "gpu": gpu_info(),
        "display": display_info(),
        "git": git_info(),
        "allocator": {key: value for key, value in os.environ.items() if key.startswith("MIMALLOC_")},
    }


def lookup(data, dotted):
    for key in dotted.split("."):
        data = data.get(key) if isinstance(data, dict) else None
    return data


def compare(first, second):
    """Fields that differ; `invalidating` lists the pair-invariant ones."""
    invalidating = [field for field in PAIR_INVARIANT_FIELDS if lookup(first, field) != lookup(second, field)]
    return {"invalidating": invalidating,
            "source_differs": lookup(first, "source.revision") != lookup(second, "source.revision")
            or lookup(first, "source.patch_sha256") != lookup(second, "source.patch_sha256")}


def pairs(values, flag):
    result = []
    for value in values or ():
        name, separator, path = value.partition("=")
        if not separator or not name or not path:
            raise SystemExit(f"{flag} expects NAME=PATH, got {value!r}")
        result.append((name, path))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--binary", action="append", help="NAME=PATH of a measured executable")
    parser.add_argument("--cargo-profile")
    parser.add_argument("--features")
    parser.add_argument("--command")
    parser.add_argument("--compare", nargs=2, type=Path, metavar=("A", "B"))
    args = parser.parse_args()
    if args.compare:
        first, second = (json.loads(path.read_text(encoding="utf-8")) for path in args.compare)
        print(json.dumps(compare(first, second), indent=2))
        return
    if args.output is None:
        parser.error("--output is required unless --compare is used")
    data = collect(pairs(args.binary, "--binary"), (), args.cargo_profile, args.features, args.command)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
