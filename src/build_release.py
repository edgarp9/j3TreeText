#!/usr/bin/env python3
"""Build the Rust release binaries and open the output directory."""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import List, Optional

RELEASE_BINARIES = ["j3TreeText", "j3TreeTextCli"]


def project_root() -> Path:
    return Path(__file__).resolve().parent


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


def main() -> int:
    root = project_root()
    binary_dir = root / "target" / "release"

    build_code = run_release_build(root)
    if build_code != 0:
        return build_code

    print(f"Release build complete: {binary_dir}")
    for binary in RELEASE_BINARIES:
        print(f"  {release_binary_path(binary_dir, binary)}")

    if not open_folder(binary_dir):
        print(f"warning: could not open binary directory automatically: {binary_dir}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
