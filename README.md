# reclaim

[![CI](https://github.com/alifnrhman/reclaim/actions/workflows/ci.yml/badge.svg)](https://github.com/alifnrhman/reclaim/actions/workflows/ci.yml)
[![npm](https://img.shields.io/npm/v/%40alifnrhman%2Freclaim?logo=npm&label=npm)](https://www.npmjs.com/package/@alifnrhman/reclaim)
[![GitHub release](https://img.shields.io/github/v/release/alifnrhman/reclaim?logo=github&label=release)](https://github.com/alifnrhman/reclaim/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Recover disk space from development artifacts without blindly deleting directories.**

`reclaim` is an interactive native CLI that finds regenerable build output, dependency directories, tool caches, and eligible Docker data. Every candidate includes a risk level, the reason for its classification, and how to recreate it. Nothing is selected by default, and deletion only happens after explicit confirmation.

## Why reclaim?

- **Evidence-based decisions** — manifests, lockfiles, tool metadata, and Docker labels determine whether an item is safe, needs review, or must be protected.
- **One view across your toolchain** — inspect project artifacts, user-level caches, Docker build cache, and Compose volumes together.
- **Conservative by default** — no preselected items, no filesystem-root scans, and no automatic deletion.
- **Revalidated before deletion** — paths, contents, filesystem identity, and Docker references are checked again immediately before removal.
- **Native and portable** — distributed as a small Rust binary through npm and GitHub Releases.

## Installation

### npm (recommended)

Requires Node.js 18 or newer on Linux x64 or Windows x64.

```sh
npm install --global @alifnrhman/reclaim
reclaim --version
```

The npm package contains the native binary for your platform. It does not download or run a separate installer during installation.

### Standalone binary

Prebuilt archives and SHA-256 checksums are available from [GitHub Releases](https://github.com/alifnrhman/reclaim/releases/latest).

| Platform | Archive |
| --- | --- |
| Linux x64 | `reclaim-linux-x86_64.tar.gz` |
| Windows x64 | `reclaim-windows-x86_64.zip` |

Verify the checksum before extracting the archive. For example, on Windows:

```powershell
Get-FileHash .\reclaim-windows-x86_64.zip -Algorithm SHA256
```

## Quick start

Scan the current directory, user-level development caches, and Docker:

```sh
reclaim
```

Scan one or more project directories:

```sh
reclaim ~/Projects/project-a ~/Projects/project-b
```

Scan projects only:

```sh
reclaim --no-global --no-docker ~/Projects
```

Inside the UI, review the explanation for each candidate, select what you want to remove, then confirm the deletion. Protected items cannot be selected.

> [!CAUTION]
> Deletion is permanent. `reclaim` deliberately starts with nothing selected and requires a second confirmation before removing anything.

## What it finds

Project candidates are classified using nearby manifests, lockfiles, and build configuration—not directory names alone.

| Ecosystem | Recognized artifacts |
| --- | --- |
| Node.js | `node_modules`, `dist`, `.next`, `.nuxt`, `.output`, `.turbo`, `.vite`, coverage and lint caches |
| Rust | Cargo `target` directories |
| Python | `__pycache__`, virtual environments, pytest, mypy, and Ruff caches |
| JVM | Maven `target`, Gradle `build`, and `.gradle` directories |
| .NET | `bin` and `obj` directories in recognized projects |
| Generic | Common `dist`, `build`, `coverage`, and `.cache` directories, marked for review when evidence is incomplete |

The default scan also includes:

- npm, Bun, pnpm, Yarn, pip, Cargo, Go, Gradle, NuGet, and Maven user caches when present;
- reclaimable Docker build cache;
- unused, Compose-labeled Docker volumes as review-only candidates.

Docker images, containers, networks, anonymous volumes, and any volume referenced by a running or stopped container are never removed.

## Risk levels

| Level | Meaning | Selectable? |
| --- | --- | --- |
| **Safe** | Recognized cache or generated output backed by sufficient regeneration evidence | Yes |
| **Review** | Likely generated, but may contain a deliverable, local-only artifact, or persistent data | Yes, manually |
| **Protected** | Symlink, reparse point, filesystem boundary, active Docker resource, or otherwise forbidden target | No |

Classification affects selection only. Even safe items are never selected automatically when the UI opens.

## Commands

```text
reclaim [--no-global] [--no-docker] [PATH]...
reclaim update
```

| Command | Description |
| --- | --- |
| `reclaim` | Scan the current directory plus global caches and Docker |
| `reclaim [PATH]...` | Scan one or more paths plus global caches and Docker |
| `reclaim --no-global [PATH]...` | Skip user-level development caches |
| `reclaim --no-docker [PATH]...` | Skip Docker build cache and Compose volumes |
| `reclaim update` | Update the global npm installation |
| `reclaim --version` | Print the installed version |
| `reclaim --help` | Show command-line help |

Options can be combined, and the current directory is used when no path is provided.

## Interactive controls

| Key | Action |
| --- | --- |
| Arrow keys, mouse wheel, `j` / `k` | Move through candidates |
| Page Up / Page Down | Move by one page |
| Space or `x` | Select or deselect the current item |
| `a` | Select every item classified as safe |
| `n` | Clear the selection |
| `f` | Cycle through risk filters |
| `s` | Reverse the size sort order |
| `w` | Show scan warnings, when present |
| `d` or Enter | Open the deletion confirmation |
| `q` | Quit, or cancel a pending confirmation |

Press `d` again or Enter to confirm deletion. Press Esc or `q` to cancel. Windows consoles use the same compact interface with commands submitted using Enter.

## Safety guarantees

Before and during deletion, `reclaim`:

- refuses to scan a filesystem root;
- never follows symlinks or Windows reparse points;
- stays on the scan root's filesystem and ignores `.git`, `.hg`, `.svn`, `.ssh`, and `.gnupg`;
- deduplicates hard-linked files when calculating allocated size;
- verifies that a candidate still matches the rule that discovered it;
- rejects a candidate if its filesystem identity, allocated size, or file count changed after scanning;
- rechecks Docker volume labels and container references before an exact `docker volume rm`;
- removes Docker build cache only through `docker builder prune --force`.

## Updates

`reclaim` checks npm for a newer version in the background at most once every 24 hours. Update a global npm installation with:

```sh
reclaim update
```

## Build from source

A current stable Rust toolchain is required.

```sh
git clone https://github.com/alifnrhman/reclaim.git
cd reclaim
cargo build --release
./target/release/reclaim --version
```

Run the same checks used by CI before submitting a change:

```sh
cargo fmt -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

## Platform support

| Platform | npm package | GitHub Release |
| --- | :---: | :---: |
| Linux x64 | Yes | Yes |
| Windows x64 | Yes | Yes |
| macOS / ARM64 | Not yet | Not yet |

## Contributing

Issues and pull requests are welcome. Please include a focused description, keep safety-sensitive behavior conservative, and ensure formatting, tests, and Clippy all pass.

## License

Released under the [MIT License](LICENSE).
