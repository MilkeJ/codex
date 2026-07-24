#!/usr/bin/env python3

import os
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from codex_package.archive import resolve_source_date_epoch, resolve_zstd_command
from codex_package.archive import write_tar_archive, write_zip_archive


class ResolveZstdCommandTest(unittest.TestCase):
    def test_prefers_zstd_from_path(self) -> None:
        def which(name: str) -> str | None:
            return {"zstd": "/usr/bin/zstd", "dotslash": "/usr/bin/dotslash"}.get(name)

        self.assertEqual(resolve_zstd_command(which=which), ["/usr/bin/zstd"])

    def test_falls_back_to_dotslash_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            manifest = Path(temp_dir) / "zstd"
            manifest.write_text("#!/usr/bin/env dotslash\n{}\n", encoding="utf-8")

            def which(name: str) -> str | None:
                return {"dotslash": "/usr/bin/dotslash"}.get(name)

            self.assertEqual(
                resolve_zstd_command(dotslash_manifest=manifest, which=which),
                ["/usr/bin/dotslash", str(manifest)],
            )

    def test_errors_when_no_zstd_or_dotslash_manifest_is_available(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            missing_manifest = Path(temp_dir) / "zstd"

            with self.assertRaisesRegex(RuntimeError, "zstd is required"):
                resolve_zstd_command(
                    dotslash_manifest=missing_manifest,
                    which=lambda _name: None,
                )


class WriteTarArchiveTest(unittest.TestCase):
    def test_normalizes_identity_and_source_date_epoch(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            package_dir = root / "package"
            package_dir.mkdir()
            payload = package_dir / "payload"
            payload.write_text("continuum\n", encoding="utf-8")
            payload.chmod(0o755)
            archive_path = root / "package.tar.gz"

            with patch.dict(os.environ, {"SOURCE_DATE_EPOCH": "1234567890"}):
                write_tar_archive(package_dir, archive_path, mode="w:gz")

            with tarfile.open(archive_path, "r:gz") as archive:
                members = archive.getmembers()

            self.assertEqual([member.name for member in members], ["payload"])
            member = members[0]
            self.assertEqual(member.uid, 0)
            self.assertEqual(member.gid, 0)
            self.assertEqual(member.uname, "")
            self.assertEqual(member.gname, "")
            self.assertEqual(member.mtime, 1234567890)
            self.assertEqual(member.mode, 0o755)
            self.assertNotIn("atime", member.pax_headers)
            self.assertNotIn("ctime", member.pax_headers)

            gzip_mtime = int.from_bytes(
                archive_path.read_bytes()[4:8],
                byteorder="little",
            )
            self.assertEqual(gzip_mtime, 1234567890)

    def test_source_date_epoch_makes_gzip_archive_reproducible(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            first_package_dir = root / "first-package"
            second_package_dir = root / "second-package"
            first_package_dir.mkdir(mode=0o700)
            second_package_dir.mkdir(mode=0o755)
            first_payload = first_package_dir / "payload"
            second_payload = second_package_dir / "payload"
            first_payload.write_text("continuum\n", encoding="utf-8")
            second_payload.write_text("continuum\n", encoding="utf-8")
            first_payload.chmod(0o700)
            second_payload.chmod(0o755)
            first_archive = root / "first.tar.gz"
            second_archive = root / "second.tar.gz"

            with patch.dict(os.environ, {"SOURCE_DATE_EPOCH": "1234567890"}):
                write_tar_archive(first_package_dir, first_archive, mode="w:gz")
                write_tar_archive(second_package_dir, second_archive, mode="w:gz")

            self.assertEqual(first_archive.read_bytes(), second_archive.read_bytes())

    def test_default_epoch_is_deterministic(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            self.assertEqual(resolve_source_date_epoch(), 0)

    def test_zip_metadata_is_normalized_and_reproducible(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            first_package_dir = root / "first-package"
            second_package_dir = root / "second-package"
            first_package_dir.mkdir(mode=0o700)
            second_package_dir.mkdir(mode=0o755)
            first_bin = first_package_dir / "bin"
            second_bin = second_package_dir / "bin"
            first_bin.mkdir(mode=0o700)
            second_bin.mkdir(mode=0o755)
            first_payload = first_bin / "codex.exe"
            second_payload = second_bin / "codex.exe"
            first_payload.write_text("continuum\n", encoding="utf-8")
            second_payload.write_text("continuum\n", encoding="utf-8")
            first_payload.chmod(0o600)
            second_payload.chmod(0o644)
            first_archive = root / "first.zip"
            second_archive = root / "second.zip"

            with patch.dict(os.environ, {"SOURCE_DATE_EPOCH": "1234567890"}):
                write_zip_archive(first_package_dir, first_archive)
                write_zip_archive(second_package_dir, second_archive)

            self.assertEqual(first_archive.read_bytes(), second_archive.read_bytes())
            with zipfile.ZipFile(first_archive) as archive:
                members = archive.infolist()
            self.assertEqual(
                [member.filename for member in members], ["bin/", "bin/codex.exe"]
            )
            self.assertEqual(
                [member.date_time for member in members],
                [(2009, 2, 13, 23, 31, 30)] * 2,
            )
            self.assertEqual(
                [member.external_attr >> 16 & 0o777 for member in members],
                [0o755, 0o644],
            )

    def test_rejects_invalid_source_date_epoch(self) -> None:
        for value in (
            "not-an-integer",
            "-1",
            "+1",
            " 1",
            "1 ",
            "1_000",
            "１２３",
        ):
            with self.subTest(value=value):
                with patch.dict(os.environ, {"SOURCE_DATE_EPOCH": value}):
                    with self.assertRaisesRegex(
                        RuntimeError,
                        "SOURCE_DATE_EPOCH must be an ASCII decimal integer",
                    ):
                        resolve_source_date_epoch()

    def test_rejects_source_date_epoch_above_gzip_limit(self) -> None:
        with patch.dict(os.environ, {"SOURCE_DATE_EPOCH": str(1 << 32)}):
            with self.assertRaisesRegex(
                RuntimeError,
                "SOURCE_DATE_EPOCH must not exceed 4294967295",
            ):
                resolve_source_date_epoch()


if __name__ == "__main__":
    unittest.main()
