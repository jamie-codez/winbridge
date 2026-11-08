# Architecture

WinBridge is a small Rust workspace. The goal is that **everything that matters lives in `core`
and is testable without hardware**, while the CLI and the window are thin front-ends.

```
winbridge/
├─ core/      library (the logic, almost all tests)
├─ cli/       `winbridge` command
├─ gui/       `winbridge-gui` window (egui)
├─ scripts/   install-deps.sh (setup), build-deb.sh, build-tarball.sh, version + commit helpers
├─ packaging/ desktop file, icon, RPM spec, Arch PKGBUILD
└─ .github/   CI, packaging and semantic-release workflows
```

## Foundations (we don't reinvent these)

| Layer | Project |
|---|---|
| Windows API compatibility | Wine, via **Proton** |
| DirectX → Vulkan | DXVK (9/10/11), VKD3D-Proton (12) |
| Runs Proton outside Steam, in a fixed container | **umu-launcher** + Steam Linux Runtime |
| Controllers inside games | SDL (reads our mappings through `SDL_GAMECONTROLLERCONFIG`) |

WinBridge's job is to make that stack reliable and approachable.

## `core` modules

| Module | Responsibility |
|---|---|
| `config` | the game library: one TOML file per game, atomic writes, a broken file never hides the rest |
| `launch` | builds the launch command (`gamescope … -- mangohud umu-run game.exe`), env vars, logs, installed-game scan |
| `doctor` | the Health Check; every message is plain language with the exact fix |
| `setup` | runs the embedded `install-deps.sh` (check → plan → install) |
| `controllers` | finds gamepads from `/proc/bus/input/devices`, battery, hints, raw input events |
| `layout` | where each button sits (standard Xbox/PlayStation tables, else saved mapping, else SDL's database) |
| `mapping` | SDL GUID/numbering, the "Set up buttons" wizard, saved mappings |
| `sdl` | optional runtime lookup of SDL's controller database (`dlopen`) |

### Design rules

1. **One private Wine prefix per game**, never shared.
2. Games run through `umu-run`, so **distro updates can't break them**; the runner can be pinned per game.
3. **Nothing updates behind the user's back.**
4. Every launch writes a log; the last 10 per game are kept.
5. Setup is **idempotent** and installs only what's missing, with one password prompt.
6. GameMode is requested from the **host** (`gamemoded -r`), because the `gamemoderun` wrapper can't work inside Proton's container.
7. We verify against reality: SDL numbering and GUIDs are checked against a live SDL, and controller logic is tested with real device data.

## Setup script

`scripts/install-deps.sh` is the single source of truth for "what does a system need". The app
embeds it (`include_str!`), so a packaged app needs no extra files. It supports `apt`, `dnf` and
`pacman`; each gets package lists (`CORE` required, `EXTRA` best effort), and umu-launcher comes
from the distribution when it carries it, else from the official release (checksum verified).
Exit codes of `--check`: `0` nothing to do, `10` setup needed, `2` unsupported distribution.

## Release flow

Conventional Commits on `main` → *Release* workflow → semantic-release decides the version,
updates `CHANGELOG.md` and `Cargo.toml`, tags `vX.Y.Z` and creates the GitHub release → the
*Package* workflow builds `.deb`, `.rpm`, Arch and tarball packages for that tag → they're attached
with `SHA256SUMS.txt`. The same *Package* workflow runs on every push as part of CI.
