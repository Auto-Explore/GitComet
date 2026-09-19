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

VERSION = "0.9.145"
SHA256 = {
    "x86_64-unknown-linux-gnu": "32aa82416099eb12fffae9cf1a279ad201fecbd3f74826c613e32e9006b29867",
    "aarch64-unknown-linux-gnu": "0ad2815fd91a7ecec3a7e25c749b584f66729ac688fd26b2fd494f7ff94b7fd0",
    "universal-apple-darwin": "52ecaedb4f5af9267ef7ed02bc937d2a15a94ff96cb663080e81311f798c9905",
    "x86_64-pc-windows-msvc": "bfecd545af057adb83be7ddfe135fd067fb9c990544da1ea111e675f45d628a9",
    "aarch64-pc-windows-msvc": "29614accc4b232b0fabdda1805d449bae3130a8de6cb343a5b2584e0e886cf0a",
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
