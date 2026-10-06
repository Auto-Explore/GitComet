"""Check that linker installation verifies downloads before replacing tooling."""

from contextlib import redirect_stdout
import hashlib
import importlib.util
import io
import os
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("install_mold", Path(__file__).resolve().parents[1] / "install-mold.py")
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


def archive_with(path, content=b"mold executable"):
    data = io.BytesIO()
    with tarfile.open(fileobj=data, mode="w:gz") as archive:
        member = tarfile.TarInfo(path)
        member.size = len(content)
        archive.addfile(member, io.BytesIO(content))
    return data.getvalue()


@unittest.skipUnless(os.name == "posix", "mold installation needs a POSIX filesystem")
class MoldInstallerTests(unittest.TestCase):
    def test_installs_each_supported_architecture_and_exports_alias(self):
        for architecture in installer.SHA256:
            with self.subTest(architecture=architecture), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                bin_dir = root / "bin"
                data = archive_with(f"mold-{installer.VERSION}-{architecture}-linux/bin/mold")
                with patch.object(installer.platform, "system", return_value="Linux"), \
                        patch.object(installer.platform, "machine", return_value=architecture), \
                        patch.dict(installer.SHA256, {architecture: hashlib.sha256(data).hexdigest()}), \
                        patch.object(installer.urllib.request, "urlopen", return_value=io.BytesIO(data)) as download, \
                        patch.object(installer.subprocess, "check_output", return_value="mold 3.0.0 (compatible with GNU ld)\n"), \
                        patch.dict(os.environ, {"GITHUB_PATH": str(root / "github-path")}), \
                        redirect_stdout(io.StringIO()):
                    installer.install(bin_dir)
                self.assertIn(f"mold-3.0.0-{architecture}-linux.tar.gz", download.call_args.args[0])
                self.assertEqual((bin_dir / "mold").read_bytes(), b"mold executable")
                self.assertEqual((bin_dir / "mold").stat().st_mode & 0o777, 0o755)
                self.assertEqual((bin_dir / "ld.mold").resolve(), bin_dir / "mold")
                self.assertEqual((root / "github-path").read_text(), f"{bin_dir}\n")

    def test_bad_download_preserves_existing_installation(self):
        with tempfile.TemporaryDirectory() as directory:
            bin_dir = Path(directory)
            (bin_dir / "mold").write_bytes(b"existing mold")
            (bin_dir / "ld.mold").symlink_to("mold")
            with patch.object(installer.platform, "system", return_value="Linux"), \
                    patch.object(installer.platform, "machine", return_value="x86_64"), \
                    patch.object(installer.urllib.request, "urlopen", return_value=io.BytesIO(b"corrupted download")), \
                    patch.object(installer.subprocess, "check_output") as execute:
                with self.assertRaisesRegex(RuntimeError, "Checksum mismatch"):
                    installer.install(bin_dir)
                execute.assert_not_called()
            self.assertEqual((bin_dir / "mold").read_bytes(), b"existing mold")
            self.assertEqual((bin_dir / "ld.mold").readlink(), Path("mold"))

    def test_archive_cannot_write_outside_the_install_directory(self):
        data = archive_with("../mold")
        with tempfile.TemporaryDirectory() as directory, \
                patch.object(installer.platform, "system", return_value="Linux"), \
                patch.object(installer.platform, "machine", return_value="x86_64"), \
                patch.dict(installer.SHA256, {"x86_64": hashlib.sha256(data).hexdigest()}), \
                patch.object(installer.urllib.request, "urlopen", return_value=io.BytesIO(data)):
            with self.assertRaisesRegex(RuntimeError, "exactly one mold executable"):
                installer.install(Path(directory) / "bin")
            self.assertEqual(list(Path(directory).iterdir()), [])

    def test_unsupported_hosts_do_not_download(self):
        for system, architecture in (("Darwin", "aarch64"), ("Windows", "AMD64"), ("Linux", "armv7l")):
            with self.subTest(system=system, architecture=architecture), \
                    patch.object(installer.platform, "system", return_value=system), \
                    patch.object(installer.platform, "machine", return_value=architecture), \
                    patch.object(installer.urllib.request, "urlopen") as download:
                with self.assertRaisesRegex(RuntimeError, "supports x86_64 and aarch64 Linux"):
                    installer.install(Path("unused"))
                download.assert_not_called()
