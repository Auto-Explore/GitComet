#!/usr/bin/env python3
"""Install the checksum-pinned native nextest binary without compiling tooling."""

import argparse
import hashlib
import io
import os
from pathlib import Path
import subprocess
import tarfile
import urllib.request

VERSION = "0.9.152"
SHA256 = {
    "x86_64-unknown-linux-gnu": "d039b43c3e0958441b6ad0a39c8884d018b3d2e8fb9928f209074b1100b5e17a",
    "aarch64-unknown-linux-gnu": "ca1f81a5b86533eb45ab97e0064aa88e925e53aab9a698ca6331a7cad1f85f07",
    "universal-apple-darwin": "2fadf57df4243e691f088ee3bff4f4ab9af16da0c95a1db84b5cb1685e098a76",
    "x86_64-pc-windows-msvc": "0384fc89a5a547d4fd5e183fcc101d31dba2837feafd8e64d5ea003a0ddb1941",
    "aarch64-pc-windows-msvc": "dfdfc60a235866dbd5330102194e8f9f4fe06a2c34f6810b79c3c7e768c718e3",
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    args = parser.parse_args()
    rustc = subprocess.check_output(["rustc", "-vV"], text=True)
    host = next(line.removeprefix("host: ") for line in rustc.splitlines() if line.startswith("host: "))
    triple = "universal-apple-darwin" if host.endswith("apple-darwin") else host
    digest = SHA256[triple]  # Unsupported hosts fail instead of installing an emulated runner.
    asset = f"cargo-nextest-{VERSION}-{triple}.tar.gz"
    url = f"https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-{VERSION}/{asset}"
    with urllib.request.urlopen(url, timeout=120) as response:
        data = response.read()
    if hashlib.sha256(data).hexdigest() != digest:
        raise RuntimeError(f"Checksum mismatch for {asset}")
    executable = "cargo-nextest.exe" if "windows" in triple else "cargo-nextest"
    args.bin_dir.mkdir(parents=True, exist_ok=True)
    destination = args.bin_dir / executable
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
        members = [item for item in archive if item.isfile() and Path(item.name).name == executable]
        if len(members) != 1:
            raise RuntimeError("Archive must contain exactly one nextest executable")
        destination.write_bytes(archive.extractfile(members[0]).read())
    destination.chmod(0o755)
    subprocess.run([str(destination.resolve()), "nextest", "--version"], check=True)
    if os.environ.get("GITHUB_PATH"):
        with open(os.environ["GITHUB_PATH"], "a", encoding="utf-8") as path_file:
            path_file.write(f"{args.bin_dir.resolve()}\n")


if __name__ == "__main__":
    main()
