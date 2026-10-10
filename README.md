# reclaim

`reclaim` finds large development artifacts that can be regenerated, explains why they are or are not safe to remove, and deletes only items you explicitly approve.

## Install

Linux x64 and Windows x64 users can install the native CLI through npm:

```sh
npm install --global @alifnrhman/reclaim
reclaim
```

## Build and run

```sh
cargo build --release
./target/release/reclaim
```

The default scan also checks development caches owned by the current user and Docker build cache. Disable either source when needed:

```sh
reclaim --no-global --no-docker ~/Projects/project-a ~/Projects/project-b
```

When no path is given, `reclaim` scans the current directory. It checks npm for a newer version in the background at most once every 24 hours. Update the global installation with:

```sh
reclaim update
```

## Install on Windows

Download `reclaim-windows-x86_64.zip` from the latest GitHub Release, then run in PowerShell from the download directory:

```powershell
New-Item -ItemType Directory -Force "$HOME\bin\reclaim"
Expand-Archive ".\reclaim-windows-x86_64.zip" -DestinationPath "$HOME\bin\reclaim" -Force
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
[Environment]::SetEnvironmentVariable("Path", "$userPath;$HOME\bin\reclaim", "User")
```

Open a new terminal and scan an explicit project directory:

```powershell
reclaim "$HOME\Projects"
```

The release also includes `reclaim-windows-x86_64.sha256`. Verify the download with `Get-FileHash .\reclaim-windows-x86_64.zip -Algorithm SHA256` before extracting it.

## Creating a release

Pushing a version tag builds native Linux and Windows binaries, runs the tests on each platform, packages them, generates SHA-256 checksums, publishes a GitHub Release, and publishes `@alifnrhman/reclaim` to npm:

```sh
git tag v0.1.6
git push origin v0.1.6
```

The UI starts with nothing selected and sorts by allocated size. Use the arrow keys or `j`/`k` to move, Space to select, `a` to select every `Safe` item, `n` to clear selection, `f` to cycle filters, and `s` to change sorting. Press `d`, then `d` again or Enter to confirm permanent deletion; Esc cancels. Windows consoles use the same compact UI with commands submitted using Enter.

## Safety model

- **Safe**: a recognized cache, or generated output backed by the relevant project manifest/lockfile.
- **Review**: a conventional artifact name whose regeneration evidence is incomplete or whose contents may be a deliverable.
- **Protected**: a symlink, reparse point, filesystem boundary, or otherwise forbidden target. It cannot be selected.

The scanner never follows symlinks or Windows reparse points, stays on the root filesystem, ignores VCS metadata, rejects filesystem roots, deduplicates hardlinks, and revalidates every path immediately before deletion. Docker cleanup is limited to `docker builder prune --force`; images, containers, networks, and volumes are never removed.
