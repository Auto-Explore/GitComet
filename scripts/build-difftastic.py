#!/usr/bin/env python3

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
PIN = ROOT / "crates/gitcomet-difftastic/upstream.json"


def build(output: Path, target: str | None, profile: str) -> None:
    pin = json.loads(PIN.read_text(encoding="utf-8"))
    version = pin["version"]
    cache = ROOT / "target/difftastic" / version
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / "source.tar.gz"
    if not archive.exists() or hashlib.sha256(archive.read_bytes()).hexdigest() != pin["sha256"]:
        url = f"https://codeload.github.com/Wilfred/difftastic/tar.gz/refs/tags/{version}"
        with tempfile.TemporaryDirectory(dir=cache) as staging:
            downloaded = Path(staging) / "source.tar.gz"
            with urllib.request.urlopen(url, timeout=60) as response, downloaded.open("wb") as destination:
                shutil.copyfileobj(response, destination)
            if hashlib.sha256(downloaded.read_bytes()).hexdigest() != pin["sha256"]:
                raise RuntimeError(f"Difftastic {version} source checksum mismatch")
            downloaded.replace(archive)
    source = cache / f"difftastic-{version}"
    if not source.exists():
        with tempfile.TemporaryDirectory(dir=cache) as staging:
            with tarfile.open(archive) as bundle:
                bundle.extractall(staging, filter="data")
            extracted = Path(staging) / source.name
            manifest = extracted / "Cargo.toml"
            with manifest.open("a", encoding="utf-8") as destination:
                destination.write("\n[workspace]\n")
            extracted.rename(source)
    build_target = cache / "target"
    environment = dict(os.environ, CARGO_TARGET_DIR=str(build_target))
    command = ["cargo", "build", "--locked", "--manifest-path", str(source / "Cargo.toml"), "--bin", "difft"]
    if profile == "release":
        command.append("--release")
    if target:
        command.extend(["--target", target])
    subprocess.run(command, cwd=ROOT, env=environment, check=True)
    target_directory = build_target / target if target else build_target
    name = "difft.exe" if ("windows" in target if target else os.name == "nt") else "difft"
    executable = target_directory / profile / name
    output.mkdir(parents=True, exist_ok=True)
    shutil.copy2(executable, output / name)
    notices = output / "difftastic-licenses"
    notices.mkdir(exist_ok=True)
    for path in source.rglob("*"):
        if path.is_file() and path.name.lower().startswith(("license", "copying", "notice")):
            destination = notices / "difftastic" / path.relative_to(source)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, destination)
    metadata_command = ["cargo", "metadata", "--locked", "--format-version=1", "--manifest-path", str(source / "Cargo.toml")]
    if target:
        metadata_command.extend(["--filter-platform", target])
    metadata = json.loads(subprocess.check_output(metadata_command, cwd=ROOT, env=environment))
    inventory = []
    for package in metadata["packages"]:
        inventory.append({key: package[key] for key in ("name", "version", "license", "license_file", "source")})
        directory = Path(package["manifest_path"]).parent
        paths = list(directory.glob("*"))
        if package.get("license_file"):
            paths.append(directory / package["license_file"])
        for path in paths:
            if path.is_file() and (path.name.lower().startswith(("license", "copying", "notice")) or str(path) == str(directory / (package.get("license_file") or ""))):
                destination = notices / f"{package['name']}-{package['version']}" / path.name
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(path, destination)
    (notices / "dependencies.json").write_text(json.dumps(inventory, indent=2) + "\n", encoding="utf-8")
    print(f"Built Difftastic {version}: {output / name}")


def main() -> None:
    parser = argparse.ArgumentParser(description="Build the pinned, isolated Difftastic helper and collect its license notices.")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--target")
    parser.add_argument("--profile", choices=["debug", "release"], default="release")
    args = parser.parse_args()
    build(args.output.resolve(), args.target, args.profile)


if __name__ == "__main__":
    main()
