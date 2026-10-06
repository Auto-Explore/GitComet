#!/usr/bin/env python3
"""Install the checksum-pinned Linux mold binary and its compiler-driver alias."""

import argparse
import hashlib
import io
import os
from pathlib import Path
import platform
import subprocess
import tarfile
import tempfile
import urllib.request

VERSION = "3.0.0"
SHA256 = {
    "x86_64": "6c90d4a474c7c0409dfb575be03a5345878ac14fdba18de8b40fa58c60121189",
    "aarch64": "52c759d3689babaea4c42af2f31062a74ab83e17ffb5a8090232534c67b70577",
}


def install(bin_dir):
    architecture = platform.machine()
    if platform.system() != "Linux" or architecture not in SHA256:
        raise RuntimeError("Prebuilt mold installation supports x86_64 and aarch64 Linux")
    asset = f"mold-{VERSION}-{architecture}-linux.tar.gz"
    url = f"https://github.com/rui314/mold/releases/download/v{VERSION}/{asset}"
    with urllib.request.urlopen(url, timeout=60) as response:
        data = response.read()
    if hashlib.sha256(data).hexdigest() != SHA256[architecture]:
        raise RuntimeError(f"Checksum mismatch for {asset}")
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
        members = [item for item in archive if item.isfile() and item.name == f"mold-{VERSION}-{architecture}-linux/bin/mold"]
        if len(members) != 1:
            raise RuntimeError("Archive must contain exactly one mold executable")
        executable = archive.extractfile(members[0]).read()

    bin_dir = bin_dir.resolve()
    bin_dir.mkdir(parents=True, exist_ok=True)
    # Verify the executable before replacing an existing installation.
    with tempfile.TemporaryDirectory(prefix=".mold-", dir=bin_dir) as temporary:
        candidate = Path(temporary) / "mold"
        candidate.write_bytes(executable)
        candidate.chmod(0o755)
        version = subprocess.check_output([str(candidate), "--version"], text=True).strip()
        if not version.startswith(f"mold {VERSION} "):
            raise RuntimeError(f"Unexpected mold version: {version}")
        candidate.replace(bin_dir / "mold")
    alias = bin_dir / "ld.mold"
    alias.unlink(missing_ok=True)
    alias.symlink_to("mold")
    print(version)
    print(f"Installed mold and ld.mold in {bin_dir}")
    if os.environ.get("GITHUB_PATH"):
        with open(os.environ["GITHUB_PATH"], "a", encoding="utf-8") as path_file:
            path_file.write(f"{bin_dir}\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    args = parser.parse_args()
    install(args.bin_dir)


if __name__ == "__main__":
    main()
