#!/usr/bin/env python3
"""Generate concise third-party license notices from Cargo metadata."""

from __future__ import annotations

import json
import re
import subprocess
from collections import defaultdict
from pathlib import Path
from typing import Any, Callable, Iterable

LICENSE_FILE_RE = re.compile(
    r"^(license|licence|copying|notice|copyright)([._-].*)?$",
    re.IGNORECASE,
)

MIT_LICENSE_TEXT = """Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE."""

EMBEDDED_RESOURCES = [
    {
        "name": "Google Fonts Material Symbols / Material Icons",
        "version": "Bundled format_list_bulleted icon resource",
        "license": "Apache-2.0",
        "copyright": "Google Material Symbols/Icons by Google; upstream Apache-2.0 notice.",
        "local_files": "icon.svg, icon.ico",
        "license_text": "See the License Texts section in THIRD_PARTY_NOTICES.txt.",
        "source": "https://fonts.google.com/icons and https://github.com/google/material-design-icons",
        "modified": "Yes; icon color was set to #3a86ff and converted into repository icon resources.",
        "distribution": "Bundled as icon.svg and icon.ico resources.",
    }
]

EMBEDDED_NOTICE_TEXT = """Google Fonts Material Symbols / Material Icons

Local files:
- icon.svg
- icon.ico

Source:
- https://fonts.google.com/icons
- https://github.com/google/material-design-icons

License:
- Apache License Version 2.0

Copyright:
- Google Material Symbols/Icons by Google.

Local changes:
- The format_list_bulleted icon was recolored to #3a86ff.
- The SVG was converted into icon.ico for the j3TreeText application icon.

Notice:
- The upstream project says attribution in an app about screen is appreciated
  but not required. This project keeps attribution here.
"""

NON_DISTRIBUTED_BUILD_TOOLS = [
    (
        "actions/checkout",
        "v4",
        "GitHub Actions checkout helper; used only by CI and not copied into release artifacts.",
    ),
    (
        "Rust toolchain via rustup",
        "stable",
        "Build toolchain for rustc, cargo, rustfmt, and clippy; not shipped with this project.",
    ),
    (
        "GitHub-hosted runner images",
        "ubuntu-latest, windows-latest",
        "CI execution environments; not redistributed by this project.",
    ),
    (
        "Ubuntu CI apt packages",
        "dbus-x11, libglib2.0-bin, libgtk-4-dev, pkg-config, x11-utils, xdotool, xvfb",
        "CI build/test packages. If native GTK/X11 libraries are bundled in a release, handle their upstream licenses separately.",
    ),
    (
        "PowerShell",
        "GitHub Windows runner provided",
        "Shell used by the Windows smoke test; not copied into release artifacts.",
    ),
]


def project_root() -> Path:
    return Path(__file__).resolve().parents[1]


def cargo_metadata(root: Path) -> dict[str, Any]:
    completed = subprocess.run(
        ["cargo", "metadata", "--locked", "--format-version", "1"],
        cwd=root,
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    return json.loads(completed.stdout)


def external_packages(metadata: dict[str, Any]) -> list[dict[str, Any]]:
    workspace_members = set(metadata["workspace_members"])
    packages = [
        package
        for package in metadata["packages"]
        if package["id"] not in workspace_members
    ]
    return sorted(packages, key=lambda package: (package["name"], package["version"]))


def license_files(package: dict[str, Any]) -> list[Path]:
    manifest_dir = Path(package["manifest_path"]).parent
    files = [
        path
        for path in manifest_dir.iterdir()
        if path.is_file() and LICENSE_FILE_RE.match(path.name)
    ]
    return sorted(files, key=lambda path: path.name.lower())


def canonical_apache_license_text(packages: list[dict[str, Any]]) -> str:
    candidates = sorted(
        [
            path
            for package in packages
            for path in license_files(package)
            if path.name.lower() in {"license-apache", "license-apache-2.0"}
        ],
        key=lambda path: path.as_posix().lower(),
    )
    if not candidates:
        raise RuntimeError("no Apache-2.0 license text found in Cargo package license files")
    return candidates[0].read_text(encoding="utf-8", errors="replace").rstrip()


def package_repository(package: dict[str, Any]) -> str:
    return package.get("repository") or package.get("homepage") or "확인 필요"


def plain_text(value: str) -> str:
    return value.replace("<br>", "; ").replace("`", "")


def append_entry(lines: list[str], fields: Iterable[tuple[str, str]]) -> None:
    for key, value in fields:
        lines.append(f"{key}: {plain_text(value)}")
    lines.append("")


def explicit_notice_text(paths: list[Path]) -> str:
    lines: list[str] = []
    for path in paths:
        text = path.read_text(encoding="utf-8", errors="ignore")
        for line in text.splitlines():
            stripped = line.strip()
            if not stripped:
                continue
            lower = stripped.lower()
            looks_like_notice = (
                lower.startswith("copyright ")
                or lower.startswith("copyrights ")
                or " is copyright " in lower
            )
            generic_license_text = any(
                phrase in lower
                for phrase in [
                    "copyright owner",
                    "copyright notice",
                    "copyright license",
                    "copyright holders",
                    "copyright assignment",
                ]
            )
            if looks_like_notice and not generic_license_text:
                if stripped not in lines:
                    lines.append(stripped)
            if len(lines) >= 3:
                return "; ".join(lines)

    if lines:
        return "; ".join(lines)

    for path in paths:
        if path.name.lower().startswith(("copyright", "notice", "copying")):
            text = path.read_text(encoding="utf-8", errors="ignore")
            paragraph = []
            for line in text.splitlines():
                stripped = line.strip()
                if not stripped and paragraph:
                    break
                if stripped:
                    paragraph.append(stripped)
                if len(paragraph) >= 3:
                    break
            if paragraph:
                return "; ".join(paragraph)

    return "확인 필요: upstream license files do not contain an explicit copyright line."


def cargo_distribution_note(package: dict[str, Any]) -> str:
    name = package["name"]
    if name == "embed-resource":
        return "Build-time resource compiler helper; not a runtime library."
    if name in {"cc", "find-msvc-tools", "pkg-config", "system-deps", "vcpkg"}:
        return "Build-time helper selected by Cargo build scripts."
    if name in {"gtk4", "glib", "gio", "gdk-pixbuf", "pango", "cairo-rs"}:
        return "Conditional Linux GUI dependency; native GTK stack libraries are OS-supplied unless separately bundled."
    if name == "libsqlite3-sys":
        return "Runtime dependency of rusqlite; bundled SQLite source may be compiled into release binaries."
    if name in {"windows-sys", "windows-targets", "windows-link"} or name.startswith(
        "windows_"
    ):
        return "Conditional Windows dependency selected by target and feature resolution."
    return "Cargo dependency in the locked graph; linked when selected by Cargo target and feature resolution."


def append_license_summary(lines: list[str], packages: list[dict[str, Any]]) -> None:
    by_license: dict[str, list[str]] = defaultdict(list)
    for package in packages:
        by_license[package.get("license") or "(no license metadata)"].append(
            f"{package['name']} {package['version']}"
        )

    lines.extend(["License Summary", "---------------", ""])
    for license_expr, items in sorted(by_license.items()):
        append_entry(
            lines,
            [
                ("License expression", license_expr),
                ("Count", str(len(items))),
                ("Packages", ", ".join(sorted(items))),
            ],
        )


def append_embedded_resource_inventory(lines: list[str]) -> None:
    lines.extend(
        [
            "Third-Party Component Summary",
            "-----------------------------",
            "",
            "The following entries summarize bundled resources and Cargo packages used by this release.",
            "",
            "Embedded Resource Inventory",
            "---------------------------",
            "",
        ]
    )
    for resource in EMBEDDED_RESOURCES:
        append_entry(
            lines,
            [
                ("Component", resource["name"]),
                ("Version", resource["version"]),
                ("License", resource["license"]),
                ("Copyright / Notice", resource["copyright"]),
                ("Source URL", resource["source"]),
                ("Modified", resource["modified"]),
                ("Distribution", resource["distribution"]),
                ("License text", resource["license_text"]),
            ],
        )


def append_embedded_resource_notices(lines: list[str]) -> None:
    lines.extend(
        [
            "Embedded Resource Notices",
            "-------------------------",
            "",
            EMBEDDED_NOTICE_TEXT.rstrip(),
            "",
        ]
    )


def append_package_inventory(
    lines: list[str], packages: list[dict[str, Any]]
) -> None:
    lines.extend(["Package Inventory", "-----------------", ""])
    lines.append(
        "The following Cargo packages are from the locked dependency graph."
    )
    lines.append("")

    for package in packages:
        package_license_files = license_files(package)
        append_entry(
            lines,
            [
                ("Component", package["name"]),
                ("Version", package["version"]),
                ("License", package.get("license") or "(no license metadata)"),
                ("Copyright / Notice", explicit_notice_text(package_license_files)),
                ("Source URL", package_repository(package)),
                ("Modified", "No"),
                ("Distribution", cargo_distribution_note(package)),
                (
                    "License text",
                    "See the License Texts section in THIRD_PARTY_NOTICES.txt.",
                ),
            ],
        )


def package_label(package: dict[str, Any]) -> str:
    return f"{package['name']} {package['version']}"


def matching_packages(
    packages: list[dict[str, Any]], predicate: Callable[[str], bool]
) -> list[str]:
    return [
        package_label(package)
        for package in packages
        if predicate(package.get("license") or "")
    ]


def append_component_list(
    lines: list[str], title: str, intro: str, components: list[str]
) -> None:
    lines.extend([title, "-" * len(title), "", intro, ""])
    if components:
        for component in sorted(components):
            lines.append(f"- {component}")
    else:
        lines.append("- None")
    lines.append("")


def append_license_component_sections(
    lines: list[str], packages: list[dict[str, Any]]
) -> None:
    append_component_list(
        lines,
        "MIT-Licensed Components",
        "The following components are distributed under MIT, either solely or as one option in a dual license.",
        matching_packages(packages, lambda license_expr: "MIT" in license_expr),
    )
    append_component_list(
        lines,
        "Apache-2.0-Licensed Components",
        "The following components are distributed under Apache-2.0, either solely, with an exception, or as one option in a dual license.",
        [
            *matching_packages(packages, lambda license_expr: "Apache-2.0" in license_expr),
            "Google Fonts Material Symbols / Material Icons",
        ],
    )
    append_component_list(
        lines,
        "BSD-3-Clause-Licensed Components",
        "The following components include BSD-3-Clause terms.",
        matching_packages(packages, lambda license_expr: "BSD-3-Clause" in license_expr),
    )
    append_component_list(
        lines,
        "Unicode-3.0-Licensed Components",
        "The following components include Unicode-3.0 terms.",
        matching_packages(packages, lambda license_expr: "Unicode-3.0" in license_expr),
    )
    append_component_list(
        lines,
        "Apache-2.0 WITH LLVM-exception Components",
        "The following components use Apache-2.0 together with the LLVM exception.",
        matching_packages(packages, lambda license_expr: "LLVM-exception" in license_expr),
    )
    append_component_list(
        lines,
        "Unlicense OR MIT Components",
        "The following components can be used under either Unlicense or MIT terms.",
        matching_packages(packages, lambda license_expr: "Unlicense" in license_expr),
    )
    append_component_list(
        lines,
        "Zlib-Licensed Components",
        "The following components are distributed under Zlib terms.",
        matching_packages(packages, lambda license_expr: "Zlib" in license_expr),
    )
    append_component_list(
        lines,
        "GPL-Compatible / Copyleft Components",
        "No third-party GPL, LGPL, AGPL, or other copyleft component was identified in the locked Cargo dependency graph. The j3TreeText project itself is GPL-3.0-or-later.",
        [],
    )
    append_component_list(
        lines,
        "SIL Open Font License Components",
        "No SIL Open Font License font was identified in the checked repository resources. Bundled Google Material Symbols/Icons resources are Apache-2.0 resources, not OFL fonts.",
        [],
    )


def package_license_text(
    packages: list[dict[str, Any]], package_name: str, file_names: Iterable[str]
) -> str:
    names = list(file_names)
    wanted = {file_name.lower() for file_name in names}
    for package in packages:
        if package["name"] != package_name:
            continue
        for path in license_files(package):
            if path.name.lower() in wanted:
                return path.read_text(encoding="utf-8", errors="replace").rstrip()
        return (
            f"확인 필요: no upstream license file matched {', '.join(sorted(names))} "
            f"for package {package_name}"
        )
    return f"확인 필요: package {package_name} was not found in Cargo metadata"


def llvm_exception_text(packages: list[dict[str, Any]]) -> str:
    text = package_license_text(packages, "target-lexicon", ["LICENSE"])
    marker = "--- LLVM Exceptions to the Apache 2.0 License ----"
    if marker not in text:
        return "확인 필요: LLVM exception text was not found in target-lexicon license file."
    return text[text.index(marker) :].rstrip()


def append_license_text(lines: list[str], title: str, text: str) -> None:
    lines.extend([title, "-" * len(title), "", text.rstrip(), ""])


def append_license_texts(lines: list[str], packages: list[dict[str, Any]]) -> None:
    lines.extend(
        [
            "License Texts",
            "=============",
            "",
            "Full license texts are included at the end of THIRD_PARTY_NOTICES.txt for the license families used by this release.",
            "",
        ]
    )

    append_license_text(lines, "MIT License", MIT_LICENSE_TEXT)
    append_license_text(
        lines,
        "Apache License 2.0",
        canonical_apache_license_text(packages),
    )
    append_license_text(
        lines,
        "BSD-3-Clause License",
        package_license_text(packages, "encoding_rs", ["LICENSE-WHATWG"]),
    )
    append_license_text(
        lines,
        "Unicode License V3",
        package_license_text(packages, "unicode-ident", ["LICENSE-UNICODE"]),
    )
    append_license_text(lines, "LLVM Exception To Apache-2.0", llvm_exception_text(packages))
    append_license_text(
        lines,
        "Unlicense OR MIT Notice",
        package_license_text(packages, "memchr", ["COPYING"]),
    )
    append_license_text(
        lines,
        "Zlib License",
        package_license_text(packages, "foldhash", ["LICENSE"]),
    )


def append_non_distributed_tools(lines: list[str]) -> None:
    lines.extend(["Non-Distributed Build and CI Tooling", "------------------------------------", ""])
    lines.append(
        "These tools are used to build or test the project but are not copied into release artifacts."
    )
    lines.append("")
    for name, version, notes in NON_DISTRIBUTED_BUILD_TOOLS:
        append_entry(
            lines,
            [
                ("Tool", name),
                ("Version / selector", version),
                ("Distributed in project artifacts?", "No"),
                ("Notes", notes),
            ],
        )


def generate_notice(root: Path, packages: list[dict[str, Any]]) -> None:
    notice_path = root / "THIRD_PARTY_NOTICES.txt"
    lines: list[str] = [
        "THIRD PARTY NOTICES",
        "===================",
        "",
        "Project: j3TreeText",
        "Project License: GNU General Public License v3.0 or later (GPL-3.0-or-later)",
        "Project Repository: https://github.com/edgarp9",
        "",
        "",
        "Corresponding Source",
        "--------------------",
        "",
        "This binary release is distributed under GPL-3.0-or-later.",
        "",
        "Source code for this release:",
        "https://github.com/edgarp9",
        "",
        "Full project license text:",
        "LICENSE",
        "",
        "",
        "Release Files",
        "-------------",
        "",
        "The source and binary distributions for this release should include:",
        "",
        "- LICENSE",
        "- about.txt",
        "- THIRD_PARTY_NOTICES.txt",
        "",
        "",
        "Distribution Notes",
        "------------------",
        "",
        "- Source inventory command: cargo metadata --locked --format-version 1",
        "- This inventory covers Rust crates in Cargo.lock, including target-specific and build dependencies.",
        "- Third-party full license texts are included at the end of THIRD_PARTY_NOTICES.txt for the license families used by this release.",
        "- The project license is GPL-3.0-or-later. Binary distributors must provide the corresponding source for the distributed build and keep GPL notices intact.",
        "- Apache-2.0 terms are GPLv3-compatible, not GPLv2-compatible.",
        "- rusqlite enables libsqlite3-sys with the bundled feature, so release builds may contain SQLite source compiled into the binary. SQLite's public-domain status and any required attribution should be confirmed for the release jurisdiction and channel.",
        "- Linux GUI builds link to native GTK stack libraries discovered by pkg-config. If those libraries are bundled instead of supplied by the OS/package manager, handle their upstream license texts and redistribution obligations separately.",
        "",
    ]

    append_license_summary(lines, packages)
    append_embedded_resource_inventory(lines)
    append_embedded_resource_notices(lines)
    append_non_distributed_tools(lines)
    append_package_inventory(lines, packages)
    append_license_component_sections(lines, packages)
    append_license_texts(lines, packages)

    notice_path.write_text("\n".join(lines), encoding="utf-8")


def main() -> int:
    root = project_root()
    metadata = cargo_metadata(root)
    packages = external_packages(metadata)

    generate_notice(root, packages)

    print(f"wrote {root / 'THIRD_PARTY_NOTICES.txt'}")
    print(f"embedded third-party license texts for {len(packages)} packages")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
