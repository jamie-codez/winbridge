# Contributing to WinBridge

Thank you for helping make gaming on Linux more fun! WinBridge is built by and for people who
just want their games to run. You don't need to be a Linux expert, or even a programmer, to help.

By taking part you agree to follow our [Code of Conduct](CODE_OF_CONDUCT.md).

## Ways to help

| You are… | You can… |
|---|---|
| **A gamer** | Try WinBridge with your games and report what works or breaks (use the *Game report* issue template). Tell us which controller you used. |
| **On Fedora, Arch, Debian, openSUSE…** | Test the installer on your distribution. The Fedora and Arch paths were only tested in simulation, so real reports are gold. |
| **Owner of a controller** | Run `winbridge controllers --watch` and the in-app *Test buttons*, and tell us whether the right parts light up. Odd adapters help us improve detection. |
| **A writer or translator** | Improve the README and docs, or help us prepare for translations. |
| **A designer** | Improve the window, the controller artwork, the icon. |
| **A Rust developer** | Pick an issue labelled `good first issue` or `help wanted`. |

## Reporting bugs

Open an [issue](https://github.com/jamie-codez/winbridge/issues/new/choose) and include:

- your distribution and version, GPU (and driver), and WinBridge version;
- what you did and what you expected;
- the **launch log** for the game: `~/.local/share/winbridge/logs/` (newest file), and the output of
  `winbridge doctor`.

Please do not attach anything private. Security problems go through [SECURITY.md](SECURITY.md), not public issues.

## Development setup

You need [Rust](https://rustup.rs) (stable) and, for packaging, `fakeroot`/`dpkg-deb`. No system
development libraries are required.

```bash
git clone https://github.com/jamie-codez/winbridge && cd winbridge
cargo run -p winbridge-gui            # run the window
cargo run -p winbridge-cli -- doctor  # run the CLI
cargo test --workspace
```

Useful while developing:

- `WINBRIDGE_HOME=/tmp/wb cargo run -p winbridge-gui` keeps all of WinBridge's data in a throwaway
  folder, so you don't touch your real library.
- `cargo run -p winbridge-gui -- --preview xbox|ps|generic [--step N]` shows the controller
  artwork with parts lighting up, no controller needed.
- `./scripts/install-deps.sh --dry-run` shows what the installer would do on your system.

### Project layout

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). In short: `core/` has the logic and is where most
tests live, `cli/` and `gui/` are thin front-ends over it.

## Making a change

1. **Open an issue first** for anything bigger than a small fix, so we can agree on the approach.
2. Fork, then branch from `main`, named after the change type: `git switch -c fix/short-description`.
3. Keep changes focused. Add or update **tests**, especially in `core/` (we test with realistic
   device data rather than needing hardware: see `core/src/controllers.rs` for examples).
4. Before pushing, run:
   ```bash
   cargo fmt --all
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```
   CI runs the same checks.
5. Open a pull request **into `main`** using the template. Explain *what* and *why*, and say how you tested it,
   including what you could **not** test.

### Style

- Rust: `cargo fmt` (see `rustfmt.toml`) and no clippy warnings.
- Messages shown to users are **plain language**, say what's wrong and how to fix it, and never blame the user.
- Don't add a dependency lightly. We want WinBridge to stay small and quick to build.
- Shell scripts must pass `bash -n` and work with `set -euo pipefail`.

### Commit messages: Conventional Commits

We use [Conventional Commits](https://www.conventionalcommits.org). The commit (or PR) title
decides the next version automatically, so it matters:

```
<type>(<optional scope>): <short summary in the imperative, max 72 characters>
```

| Type | Use for | Release |
|---|---|---|
| `feat` | a new feature | **minor** (0.1.0 → 0.2.0) |
| `fix` | a bug fix | **patch** (0.1.0 → 0.1.1) |
| `perf` | a performance improvement | patch |
| `revert` | reverting a change | patch |
| `docs`, `test`, `build`, `ci`, `refactor`, `style`, `chore` | everything else | no release |

Add `!` after the type (or a `BREAKING CHANGE:` footer) for a breaking change, for example
`feat(core)!: change the library file format`. While we are on 0.x a breaking change bumps the minor
version; from 1.0 it bumps the major.

Scopes are optional: `core`, `cli`, `gui`, `setup`, `packaging`, `ci`, `docs`.

Examples: `feat(gui): add switch pro artwork` · `fix(core): centre sticks before the first event` ·
`docs: explain the controller wizard`.

Enable the local check once per clone (it also loads a commit template):

```bash
git config core.hooksPath .githooks
git config commit.template .gitmessage
```

CI checks every commit subject and the pull request title with the same rule, and the PR
title is what ends up in the history when we squash-merge.

## Adding or improving a distribution

Setup lives in [`scripts/install-deps.sh`](scripts/install-deps.sh). To support a new package
manager add its package lists (`CORE` = required, `EXTRA` = best effort), an `installed()` check,
and the privileged commands in the helper section. The script has a `WINBRIDGE_OS_RELEASE`
override so you can test the logic against a fake `/etc/os-release`. CI runs
`install-deps.sh --list` inside Debian, Ubuntu, Fedora and Arch containers.

## Branches and releases

Changes go **feature branch → `main` → `dev` (pre-release) → `prod` (stable)**. Open pull requests
against `main` only; maintainers promote it. The full process, with the exact commands, is in
[docs/RELEASING.md](docs/RELEASING.md).

You never tag or version anything by hand: when `dev` or `prod` is updated, the *Release* workflow runs
[semantic-release](https://semantic-release.gitbook.io), picks the version from the Conventional Commits
(`feat` → minor, `fix`/`perf` → patch), creates the tag and GitHub release with notes, then builds and attaches
the `.deb`, `.rpm`, Arch and tarball packages with checksums.

## Questions

Open a [discussion](https://github.com/jamie-codez/winbridge/discussions) or an issue. There are no silly questions.
