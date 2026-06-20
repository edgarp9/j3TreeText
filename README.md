# j3TreeText

j3TreeText is a Windows desktop editor and command-line tool for tree-structured plain text documents stored in a SQLite database.

<img width="409" height="353" alt="j3TreeText" src="https://github.com/user-attachments/assets/2a54e50d-31be-447f-bc55-eac132fb3cfd" />


## Features

- Stores a document tree in a local SQLite database.
- Provides a desktop editor with document tree navigation, tabs, search, trash, restore, and permanent delete flows.
- Saves document bodies as UTF-8 plain text.
- Supports text import and export with multiple encodings, including UTF-8, UTF-8 BOM, UTF-16 BOM variants, Korean EUC-KR/CP949, and Windows-1252.
- Includes a CLI for listing, showing, creating, editing, renaming, searching, deleting, restoring, purging, and moving documents.
- Persists editor preferences such as font, word wrap, theme, language, window size, splitter position, and last selection.

## Project Status

This project was created with AI assistance using an in-house tool.

Test coverage is limited, and the application has not been thoroughly validated. Review the code and test with your own data before relying on it for important documents.

## Repository Layout

```text
.
|-- LICENSE
|-- README.md
`-- src/
    |-- Cargo.toml
    |-- build_release.py
    |-- src/
    |-- docs/
    `-- resources/
```

The Rust project is located in `src/`.

## Build

Requirements:

- Rust stable toolchain
- Windows development environment for the desktop application

Build both release binaries:

```powershell
cd src
cargo build --release --bin j3TreeText --bin j3TreeTextCli
```

Or use the helper script:

```powershell
cd src
python build_release.py
```

Release binaries are created under:

```text
src/target/release/
```

## Run

Run the desktop editor with the default database path:

```powershell
cd src
cargo run --bin j3TreeText
```

Run the desktop editor with an explicit database file:

```powershell
cd src
cargo run --bin j3TreeText -- path\to\notes.db
```

Show CLI help:

```powershell
cd src
cargo run --bin j3TreeTextCli -- --help
```

List the document tree in a database:

```powershell
cd src
cargo run --bin j3TreeTextCli -- --db path\to\notes.db tree
```

## CLI Overview

```text
j3TreeTextCli [--db <path>] <command> [args]

Commands:
  tree
  show [--trash] <node_id>
  create --parent <node_id> --title <title> [--content <text> | --content-file <path> | --stdin]
  edit <node_id> (--content <text> | --content-file <path> | --stdin) [--append]
  rename <node_id> --title <title>
  search <query>
  delete <node_id>
  trash
  restore <node_id>
  purge <node_id>
  move <node_id> --parent <node_id>
  move-up <node_id>
  move-down <node_id>
```

## Data Model

The application creates and migrates a SQLite database automatically. Documents are stored as nodes in a tree. Each node has a title, body content, timestamps, ordering metadata, and optional deletion metadata for the trash workflow.

On first use, the database is initialized with a root document and one empty child document.

## License

This repository includes the GNU General Public License version 3 license text. See [LICENSE](LICENSE).

## Third-Party Notices

This project uses icons from [Google Fonts Icons](https://fonts.google.com/icons). Google Material Icons / Material Symbols are made available by Google under the [Apache License Version 2.0](https://www.apache.org/licenses/LICENSE-2.0).

Thank you to Google and the Material Icons / Material Symbols contributors for making these icons available.
