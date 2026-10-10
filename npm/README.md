# @alifnrhman/reclaim

[![CI](https://github.com/alifnrhman/reclaim/actions/workflows/ci.yml/badge.svg)](https://github.com/alifnrhman/reclaim/actions/workflows/ci.yml)
[![npm](https://img.shields.io/npm/v/%40alifnrhman%2Freclaim?logo=npm&label=npm)](https://www.npmjs.com/package/@alifnrhman/reclaim)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/alifnrhman/reclaim/blob/main/LICENSE)

**Recover disk space from development artifacts without blindly deleting directories.**

`reclaim` is an interactive native CLI that finds regenerable build output, dependency directories, tool caches, and eligible Docker data. Every candidate includes a risk level, an explanation, and restoration guidance. Nothing is selected by default.

## Install

Requires Node.js 18 or newer on Linux x64 or Windows x64.

```sh
npm install --global @alifnrhman/reclaim
reclaim --version
```

The package includes the matching native binary and does not download or execute a separate installer during `npm install`.

## Quick start

```sh
# Scan the current directory, user caches, and Docker
reclaim

# Scan specific projects
reclaim ~/Projects/project-a ~/Projects/project-b

# Scan projects only
reclaim --no-global --no-docker ~/Projects
```

| Command | Description |
| --- | --- |
| `reclaim [PATH]...` | Scan the given paths, or the current directory when omitted |
| `reclaim --no-global [PATH]...` | Skip user-level development caches |
| `reclaim --no-docker [PATH]...` | Skip Docker build cache and Compose volumes |
| `reclaim update` | Update the global npm installation |
| `reclaim --version` | Print the installed version |
| `reclaim --help` | Show command-line help |

> [!CAUTION]
> Deletion is permanent. Nothing is preselected, protected items cannot be selected, and removal requires a second confirmation.

Use the arrow keys or `j`/`k` to move, Space to select, `a` to select every safe item, `n` to clear the selection, `f` to cycle filters, and `s` to change sorting. Press `d`, then `d` again or Enter to confirm deletion; `q` quits and Esc cancels confirmation.

Read the [full documentation](https://github.com/alifnrhman/reclaim#readme) for recognized artifacts, the safety model, standalone downloads, and source build instructions.

## License

[MIT](https://github.com/alifnrhman/reclaim/blob/main/LICENSE)
