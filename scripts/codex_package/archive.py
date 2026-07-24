"""Archive writers for canonical Codex package directories."""

import gzip
import os
import re
import shutil
import stat
import subprocess
import tarfile
import tempfile
import time
import zipfile
from collections.abc import Callable
from pathlib import Path

from .targets import REPO_ROOT


ZSTD_DOTSLASH = REPO_ROOT / ".github" / "workflows" / "zstd"
DEFAULT_ARCHIVE_EPOCH = 0
GZIP_MAX_EPOCH = (1 << 32) - 1
ZIP_MIN_EPOCH = 315532800


def write_archive(package_dir: Path, archive_path: Path, *, force: bool) -> None:
    if is_relative_to(archive_path, package_dir):
        raise RuntimeError(
            f"Archive output must be outside the package directory: {archive_path}"
        )

    archive_path.parent.mkdir(parents=True, exist_ok=True)
    if archive_path.exists():
        if not force:
            raise RuntimeError(f"Archive output already exists: {archive_path}")
        archive_path.unlink()

    archive_format = archive_format_for_path(archive_path)
    if archive_format == "tar.gz":
        write_tar_archive(package_dir, archive_path, mode="w:gz")
    elif archive_format == "tar.zst":
        write_tar_zst_archive(package_dir, archive_path)
    elif archive_format == "zip":
        write_zip_archive(package_dir, archive_path)
    else:
        raise AssertionError(f"unexpected archive format: {archive_format}")


def is_relative_to(path: Path, parent: Path) -> bool:
    try:
        path.relative_to(parent)
        return True
    except ValueError:
        return False


def archive_format_for_path(path: Path) -> str:
    suffixes = path.suffixes
    if suffixes[-2:] == [".tar", ".gz"] or path.suffix == ".tgz":
        return "tar.gz"
    if suffixes[-2:] == [".tar", ".zst"]:
        return "tar.zst"
    if path.suffix == ".zip":
        return "zip"
    raise RuntimeError(
        f"Unsupported archive suffix for {path}. Use .tar.gz, .tgz, .tar.zst, or .zip."
    )


def write_tar_archive(package_dir: Path, archive_path: Path, *, mode: str) -> None:
    source_date_epoch = resolve_source_date_epoch()
    if mode == "w:gz":
        with archive_path.open("wb") as archive_file:
            with gzip.GzipFile(
                filename="",
                mode="wb",
                fileobj=archive_file,
                mtime=source_date_epoch,
            ) as compressed_file:
                write_tar_stream(
                    package_dir,
                    compressed_file,
                    source_date_epoch=source_date_epoch,
                )
    elif mode == "w":
        with archive_path.open("wb") as archive_file:
            write_tar_stream(
                package_dir,
                archive_file,
                source_date_epoch=source_date_epoch,
            )
    else:
        raise ValueError(f"unsupported tar mode: {mode}")


def write_tar_stream(
    package_dir: Path,
    archive_file,
    *,
    source_date_epoch: int,
) -> None:
    def canonicalize_tar_info(info: tarfile.TarInfo) -> tarfile.TarInfo:
        info.uid = 0
        info.gid = 0
        info.uname = ""
        info.gname = ""
        if info.isdir():
            info.mode = 0o755
        elif info.isreg():
            info.mode = 0o755 if info.mode & 0o111 else 0o644
        elif info.issym() or info.islnk():
            info.mode = 0o777
        info.pax_headers.pop("atime", None)
        info.pax_headers.pop("ctime", None)
        info.pax_headers.pop("mtime", None)
        info.mtime = source_date_epoch
        return info

    with tarfile.open(fileobj=archive_file, mode="w") as archive:
        for path in package_entries(package_dir):
            archive.add(
                path,
                arcname=path.relative_to(package_dir),
                recursive=False,
                filter=canonicalize_tar_info,
            )


def resolve_source_date_epoch() -> int:
    raw_value = os.environ.get("SOURCE_DATE_EPOCH")
    if raw_value is None:
        return DEFAULT_ARCHIVE_EPOCH

    if re.fullmatch(r"[0-9]+", raw_value, flags=re.ASCII) is None:
        raise RuntimeError("SOURCE_DATE_EPOCH must be an ASCII decimal integer")
    value = int(raw_value)
    if value > GZIP_MAX_EPOCH:
        raise RuntimeError(f"SOURCE_DATE_EPOCH must not exceed {GZIP_MAX_EPOCH}")
    return value


def write_tar_zst_archive(package_dir: Path, archive_path: Path) -> None:
    zstd_command = resolve_zstd_command()

    with tempfile.TemporaryDirectory(prefix="codex-package-archive-") as temp_dir_str:
        tar_path = Path(temp_dir_str) / "package.tar"
        write_tar_archive(package_dir, tar_path, mode="w")
        subprocess.check_call(
            [*zstd_command, "-T0", "-19", "-f", str(tar_path), "-o", str(archive_path)]
        )


def resolve_zstd_command(
    *,
    dotslash_manifest: Path = ZSTD_DOTSLASH,
    which: Callable[[str], str | None] = shutil.which,
) -> list[str]:
    zstd = which("zstd")
    if zstd is not None:
        return [zstd]

    dotslash = which("dotslash")
    if dotslash is not None and dotslash_manifest.is_file():
        return [dotslash, str(dotslash_manifest)]

    raise RuntimeError(
        "zstd is required to write .tar.zst archives. Install zstd, or install "
        f"DotSlash so the repository wrapper can run: {dotslash_manifest}"
    )


def write_zip_archive(package_dir: Path, archive_path: Path) -> None:
    source_date_epoch = resolve_source_date_epoch()
    zip_date_time = time.gmtime(max(source_date_epoch, ZIP_MIN_EPOCH))[:6]

    with zipfile.ZipFile(
        archive_path, "w", compression=zipfile.ZIP_DEFLATED
    ) as archive:
        for path in package_entries(package_dir):
            relative_path = path.relative_to(package_dir)
            archive_name = relative_path.as_posix()
            info = zipfile.ZipInfo(
                f"{archive_name}/" if path.is_dir() else archive_name,
                date_time=zip_date_time,
            )
            info.create_system = 3
            info.compress_type = zipfile.ZIP_DEFLATED
            if path.is_dir():
                info.external_attr = (stat.S_IFDIR | 0o755) << 16
                info.external_attr |= 0x10
                archive.writestr(info, b"")
            elif path.is_file():
                mode = 0o755 if path.stat().st_mode & 0o111 else 0o644
                info.external_attr = (stat.S_IFREG | mode) << 16
                archive.writestr(info, path.read_bytes())
            else:
                raise RuntimeError(f"Unsupported package entry for ZIP archive: {path}")


def package_entries(package_dir: Path) -> list[Path]:
    return sorted(
        package_dir.rglob("*"),
        key=lambda path: path.relative_to(package_dir).as_posix(),
    )
