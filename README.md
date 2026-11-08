<div align="center">

<img src="packaging/winbridge.svg" width="96" alt="WinBridge logo">

# WinBridge

**Make gaming fun on Linux.** Run Windows games with a few clicks: no terminal, no Wine tinkering.

[![CI](https://github.com/jamie-codez/winbridge/actions/workflows/ci.yml/badge.svg)](https://github.com/jamie-codez/winbridge/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)
[![PRs welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)](CONTRIBUTING.md)

</div>

WinBridge is a friendly launcher for Windows games on Linux. It sits on top of the most
reliable foundation there is, [umu-launcher](https://github.com/Open-Wine-Components/umu-launcher)
and Valve's [Proton](https://github.com/ValveSoftware/Proton), and takes care of the parts that
usually go wrong: missing graphics libraries, broken Wine setups, controllers that "don't work".

> **Status: early (0.1).** It works and is tested on Ubuntu 24.04, but it is young. Please read
> [what is and isn't supported](#what-works-and-what-doesnt) before you buy a game for it.

## What you get

- **One-click setup.** On first start WinBridge checks your system and offers to install
  everything needed: Vulkan graphics (including the 32-bit libraries older games need), the
  umu-launcher game engine, GameMode, controller access rules and more, using your
  distribution's package manager. You enter your password once.
- **Add a game by dragging its `.exe` onto the window.** Installers work too: run the installer,
  then press *Find installed game*.
- **Games in your normal applications menu.** Each game can get a launcher entry (and a desktop
  shortcut), so you start it like any other app, without opening WinBridge first. Games you add, and
  games found after an installer, are added to the menu automatically; you can switch it off per game.
- **One private Windows folder per game**, so one broken game can't break another, and
  nothing updates behind your back.
- **A Health Check page** that explains in plain language what is missing and gives the exact
  fix for it.
- **Controllers that just work**: Xbox, PlayStation (DualShock/DualSense), Switch Pro and most
  generic pads, wired or Bluetooth. A live test draws your controller (Xbox or PlayStation
  artwork, picked automatically) and lights up each button as you press it. For odd pads,
  a *Set up buttons* wizard builds the mapping games need.
- **Performance, handled.** GameMode boost, optional frame limiter and FPS overlay, and
  fsync/ntsync where your kernel supports them.
- **Logs for every launch**, so when something fails you can see why.

## Install

Download the package for your distribution from the
[latest release](https://github.com/jamie-codez/winbridge/releases/latest). Want to try new things early?
Pre-releases (`-dev.N`, from the `dev` branch) are on the [releases page](https://github.com/jamie-codez/winbridge/releases) too.

| Platform | Package | Install |
|---|---|---|
| Ubuntu, Debian, Linux Mint, Pop!_OS… | `winbridge_*_amd64.deb` | `sudo apt install ./winbridge_*_amd64.deb` |
| Fedora and compatible | `winbridge-*.rpm` | `sudo dnf install ./winbridge-*.rpm` |
| Arch, Manjaro, EndeavourOS… | `winbridge-*.pkg.tar.zst` | `sudo pacman -U winbridge-*.pkg.tar.zst` |
| Any other Linux | `winbridge-*-linux-x86_64.tar.gz` | extract, then run `./install.sh` (installs to `~/.local`) |

Then open **WinBridge** from your applications menu. The first screen offers to install the
rest. Installing the `.deb`/`.rpm`/`.pkg` already pulls in the core system packages through your
package manager; the first-run setup completes the pieces a package can't (32-bit graphics
libraries, the game engine launcher, controller rules and optional extras).

### Which platforms are real?

| Platform | Setup coverage | Tested? |
|---|---|---|
| Ubuntu 24.04 (apt) | full: Vulkan + 32-bit, umu-launcher (verified download), controller rules, extras | **yes**, on real hardware |
| Debian family (apt) | same as Ubuntu | not yet |
| Fedora (dnf) | Vulkan + 32-bit, umu-launcher from the distro or the release RPM, extras | simulated only |
| Arch (pacman) | Vulkan (per GPU vendor) + 32-bit, umu-launcher from the repos, extras | simulated only |
| openSUSE and others | not supported by the automatic setup yet | no |
| macOS, Windows | **not possible** (see below) | n/a |

Help us test the untested rows: see [CONTRIBUTING.md](CONTRIBUTING.md).

**Why no macOS `.app`?** umu-launcher and Proton exist only for Linux, and WinBridge relies on
Linux features (the kernel's controller interface, `/proc`, the Steam Linux Runtime container). A
macOS build could not run a single game. On macOS, look at CrossOver, Whisky or Game Porting Toolkit.

## What works, and what doesn't

Be realistic about Linux gaming; we would rather tell you now:

- **Kernel-level anti-cheat does not work.** Online play in *Call of Duty* (Ricochet) and
  *EA Sports FC* (EA anticheat) is blocked by the publishers on Linux. No launcher can fix that,
  and trying to bypass anti-cheat will get you banned. Check
  [ProtonDB](https://www.protondb.com) and [Are We Anti-Cheat Yet?](https://areweanticheatyet.com)
  **before** you buy a game.
- **Your graphics card sets the ceiling.** Integrated graphics (many laptops) run lighter and
  older games well, but demanding new titles will be slow whatever the software.
- **NVIDIA** users need the proprietary NVIDIA driver; WinBridge tells you if it's missing but
  doesn't install it for you.
- Single-player and non-anti-cheat games (the large majority) are in good shape.

## Using WinBridge

1. Open WinBridge and press **Install everything** if it offers to.
2. Drag a game's `.exe` (or its installer) onto the window.
3. Press **▶ Play**. The very first launch downloads Proton (about 1.4 GB) and is slow; later
   launches are quick.

Prefer a terminal? Everything is available there too:

```bash
winbridge doctor            # check your system
winbridge setup             # install what's missing
winbridge add ~/Games/Hades/Hades.exe --steam-id 1145360
winbridge list
winbridge run hades
winbridge shortcut hades --desktop   # applications menu (+ desktop); --remove to undo
winbridge controllers --watch --seconds 30   # live button test
```

Setting `--steam-id` (the number in the game's Steam store URL) lets umu apply the known fixes
for that game automatically.

### Controllers

Open **Controllers** and press **Test buttons**. Press a button and the matching part of the
on-screen controller lights up. If a game puts buttons in the wrong places, or ignores your pad,
press **Set up buttons** and follow the prompts (about a minute). Your layout is saved and given
to every game you start.

Wireless Xbox controllers via the official USB dongle need the separate, third-party
[`xone`](https://github.com/dlundqvist/xone) driver; Bluetooth and wired Xbox pads work with the
kernel. PlayStation and Switch Pro pads work out of the box on current kernels.

## How it works

WinBridge is a Rust workspace:

| Crate | Purpose |
|---|---|
| `core` | game library, launch builder, health check, controller detection, layouts, button mapping, setup |
| `cli` | the `winbridge` command |
| `gui` | the `winbridge-gui` window ([egui](https://github.com/emilk/egui)) |

A game launches as `gamescope … -- mangohud umu-run game.exe` inside its own private prefix, with
GameMode requested from the host. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Build from source

You need [Rust](https://rustup.rs) (stable). No system development libraries are required.

```bash
git clone https://github.com/jamie-codez/winbridge
cd winbridge
cargo run -p winbridge-gui          # the window
cargo test --workspace              # tests
./scripts/build-deb.sh              # a .deb in dist/   (also build-tarball.sh)
```

`./scripts/install-deps.sh` is the same installer the app runs; `--dry-run` previews it.

## Contributing

We use [Conventional Commits](https://www.conventionalcommits.org) and automatic
[semantic releases](CONTRIBUTING.md#releases-automatic), so every release is tracked in the
[changelog](CHANGELOG.md).

WinBridge exists to make gaming on Linux fun for everyone, so all contributions are welcome:
code, testing on your distribution, controller reports, documentation, translations, ideas. Start
with [CONTRIBUTING.md](CONTRIBUTING.md) and please follow our
[Code of Conduct](CODE_OF_CONDUCT.md). Security issues: see [SECURITY.md](SECURITY.md).

## Support the project

WinBridge is free and open source. If it makes your gaming nicer, see
[DONATIONS.md](DONATIONS.md) for ways to help, including non-financial ones.

## Thanks

WinBridge stands on the shoulders of [umu-launcher](https://github.com/Open-Wine-Components/umu-launcher),
[Proton](https://github.com/ValveSoftware/Proton), [Wine](https://www.winehq.org),
[DXVK](https://github.com/doitsujin/dxvk), [VKD3D-Proton](https://github.com/HansKristian-Work/vkd3d-proton),
[SDL](https://libsdl.org), [GameMode](https://github.com/FeralInteractive/gamemode),
[MangoHud](https://github.com/flightlessmango/MangoHud) and [egui](https://github.com/emilk/egui).

## License

[MIT](LICENSE). WinBridge is not affiliated with or endorsed by Valve, Microsoft, Sony or any
game publisher; product names are used only to describe compatibility.
