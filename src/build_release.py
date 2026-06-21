#!/usr/bin/env python3
"""Build the Rust release binaries and open the output directory."""

from __future__ import annotations

import os
import fnmatch
import platform
import shutil
import subprocess
import sys
import tomllib
import zipfile
from pathlib import Path
from typing import List, Optional

RELEASE_PACKAGE_NAME = "j3TreeText"
RELEASE_BINARIES = ["j3TreeText", "j3TreeTextCli"]
RELEASE_NOTICE_FILES = ["LICENSE", "about.txt", "THIRD_PARTY_NOTICES.txt"]
RELEASE_OBSOLETE_NOTICE_FILES = ["THIRD_PARTY_NOTICES.md"]
SOURCE_EXCLUDED_DIRS = {
    ".git",
    ".idea",
    ".my",
    ".vscode",
    "__pycache__",
    "coverage",
    "criterion",
    "dist",
    "target",
}
SOURCE_EXCLUDED_FILE_NAMES = {
    ".DS_Store",
    "Desktop.ini",
    "Thumbs.db",
    "cargo-tarpaulin-report.xml",
    "flamegraph.svg",
    "tarpaulin-report.html",
}
SOURCE_EXCLUDED_PATTERNS = [
    "*.bak",
    "*.ilk",
    "*.log",
    "*.pdb",
    "*.profdata",
    "*.profraw",
    "*.pyc",
    "*.rlib",
    "*.rmeta",
    "*.swo",
    "*.swp",
    "*.tmp",
    "*~",
]


def project_root() -> Path:
    return Path(__file__).resolve().parent


def should_open_output(args: List[str]) -> bool:
    if not args:
        return True
    if args == ["--no-open"]:
        return False
    raise ValueError("usage: python build_release.py [--no-open]")


def package_version(root: Path) -> str:
    with (root / "Cargo.toml").open("rb") as cargo_toml:
        manifest = tomllib.load(cargo_toml)
    return str(manifest["package"]["version"])


def target_platform_tag() -> str:
    os_name = "windows" if sys.platform.startswith("win") else sys.platform
    machine = platform.machine().lower()
    if machine in {"amd64", "x64"}:
        machine = "x86_64"
    return f"{os_name}-{machine}"


def run_release_build(root: Path) -> int:
    command = ["cargo", "build", "--release"]
    for binary in RELEASE_BINARIES:
        command.extend(["--bin", binary])

    print(f"Running: {' '.join(command)}")
    try:
        completed = subprocess.run(command, cwd=root, check=False)
    except FileNotFoundError:
        print("error: cargo was not found in PATH.", file=sys.stderr)
        return 127

    return completed.returncode


def release_binary_path(binary_dir: Path, binary: str) -> Path:
    suffix = ".exe" if sys.platform.startswith("win") else ""
    return binary_dir / f"{binary}{suffix}"


def linux_open_command() -> Optional[List[str]]:
    xdg_open = shutil.which("xdg-open")
    if xdg_open:
        return [xdg_open]

    gio = shutil.which("gio")
    if gio:
        return [gio, "open"]

    return None


def open_folder(path: Path) -> bool:
    if sys.platform.startswith("win"):
        os.startfile(str(path))  # type: ignore[attr-defined]
        return True

    if sys.platform.startswith("linux"):
        opener = linux_open_command()
        if opener is None:
            return False

        subprocess.Popen(
            [*opener, str(path)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        return True

    return False


def copy_release_notices(root: Path, binary_dir: Path) -> None:
    for obsolete_file in RELEASE_OBSOLETE_NOTICE_FILES:
        target = binary_dir / obsolete_file
        if target.exists():
            target.unlink()

    for notice_file in RELEASE_NOTICE_FILES:
        source = root / notice_file
        if not source.exists():
            print(f"warning: release notice file is missing: {source}", file=sys.stderr)
            continue

        shutil.copy2(source, binary_dir / notice_file)


def should_skip_source_path(root: Path, path: Path) -> bool:
    relative = path.relative_to(root)
    if any(part in SOURCE_EXCLUDED_DIRS for part in relative.parts):
        return True
    if path.name in SOURCE_EXCLUDED_FILE_NAMES:
        return True
    return any(fnmatch.fnmatch(path.name, pattern) for pattern in SOURCE_EXCLUDED_PATTERNS)


def create_source_archive(root: Path, output_zip: Path) -> None:
    output_zip.parent.mkdir(parents=True, exist_ok=True)
    if output_zip.exists():
        output_zip.unlink()

    with zipfile.ZipFile(
        output_zip,
        "w",
        compression=zipfile.ZIP_DEFLATED,
        strict_timestamps=False,
    ) as archive:
        for path in sorted(root.rglob("*"), key=lambda value: value.relative_to(root).as_posix()):
            if not path.is_file() or should_skip_source_path(root, path):
                continue
            archive.write(path, path.relative_to(root).as_posix())


def create_binary_archive(binary_dir: Path, output_zip: Path) -> None:
    output_zip.parent.mkdir(parents=True, exist_ok=True)
    if output_zip.exists():
        output_zip.unlink()

    with zipfile.ZipFile(
        output_zip,
        "w",
        compression=zipfile.ZIP_DEFLATED,
        strict_timestamps=False,
    ) as archive:
        for binary in RELEASE_BINARIES:
            binary_path = release_binary_path(binary_dir, binary)
            archive.write(binary_path, binary_path.name)

        for notice_file in RELEASE_NOTICE_FILES:
            source = binary_dir / notice_file
            archive.write(source, notice_file)


def verify_zip_entries(zip_path: Path, required_entries: list[str]) -> None:
    with zipfile.ZipFile(zip_path) as archive:
        names = set(archive.namelist())

    missing = [entry for entry in required_entries if entry not in names]
    if missing:
        raise RuntimeError(
            f"{zip_path} is missing required release entries: {', '.join(missing)}"
        )


def create_release_archives(root: Path, binary_dir: Path) -> tuple[Path, Path]:
    version = package_version(root)
    dist_dir = root / "target" / "dist"
    source_zip = dist_dir / f"{RELEASE_PACKAGE_NAME}-{version}-source.zip"
    binary_zip = dist_dir / f"{RELEASE_PACKAGE_NAME}-{version}-{target_platform_tag()}.zip"

    create_source_archive(root, source_zip)
    create_binary_archive(binary_dir, binary_zip)

    verify_zip_entries(source_zip, RELEASE_NOTICE_FILES)
    verify_zip_entries(
        binary_zip,
        [
            *[release_binary_path(binary_dir, binary).name for binary in RELEASE_BINARIES],
            *RELEASE_NOTICE_FILES,
        ],
    )

    return source_zip, binary_zip


def main(argv: Optional[List[str]] = None) -> int:
    args = sys.argv[1:] if argv is None else argv
    try:
        open_output = should_open_output(args)
    except ValueError as error:
        print(error, file=sys.stderr)
        return 2

    root = project_root()
    binary_dir = root / "target" / "release"

    build_code = run_release_build(root)
    if build_code != 0:
        return build_code

    copy_release_notices(root, binary_dir)
    try:
        source_zip, binary_zip = create_release_archives(root, binary_dir)
    except (OSError, RuntimeError, KeyError, ValueError) as error:
        print(f"error: release archive verification failed: {error}", file=sys.stderr)
        return 1

    print(f"Release build complete: {binary_dir}")
    for binary in RELEASE_BINARIES:
        print(f"  {release_binary_path(binary_dir, binary)}")
    for notice_file in RELEASE_NOTICE_FILES:
        print(f"  {binary_dir / notice_file}")
    print("Release archives:")
    print(f"  {source_zip}")
    print(f"  {binary_zip}")
    print("Verified archive entries:")
    print("  source: LICENSE, about.txt, THIRD_PARTY_NOTICES.txt")
    print("  binary: release binaries, LICENSE, about.txt, THIRD_PARTY_NOTICES.txt")

    if open_output and not open_folder(binary_dir):
        print(f"warning: could not open release directory automatically: {binary_dir}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
