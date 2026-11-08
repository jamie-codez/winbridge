# Branches and releases

WinBridge uses a short, linear promotion model. Every change flows in one direction and every
release is a package you can download.

```
feat/* fix/* docs/* ci/* …  ──PR, squash──▶  main   default branch, source of truth
                                              │  promote: merge commit
                                              ▼
                                             dev    ──▶ pre-release   v0.2.0-dev.1, -dev.2, …
                                              │  promote: merge commit
                                              ▼
                                             prod   ──▶ stable release v0.2.0
```

| Branch | Purpose | What a push does |
|---|---|---|
| `feat/*`, `fix/*`, `docs/*`, `ci/*`, … | one change, named after its Conventional Commit type | CI runs on the pull request |
| `main` | the **source of truth** and default branch; always releasable | CI only, **no release** |
| `dev` | testers' channel | publishes a **pre-release** with all four packages |
| `prod` | what everyone should install | publishes a **stable release** with all four packages |

> **Strict rule: releases happen only from `dev` (pre-release) and `prod` (stable), never from any other
> branch.** This is enforced three ways: the Release workflow triggers only on pushes to those two
> branches and re-checks the branch name itself; semantic-release is configured for exactly those two
> branches; and CI runs `scripts/check-release-config.sh` on every change, which fails if anything
> (a new trigger, another workflow, a changed branch list) would allow a release from elsewhere.

## Day to day

1. Branch from `main`: `git switch -c fix/centre-sticks`.
2. Commit with [Conventional Commits](https://www.conventionalcommits.org) and open a PR **into `main`**.
   The PR title must also follow the convention. **Squash-merge** it, so the title becomes the commit.
3. CI (format, lint, tests, installer on five distributions, all four package builds) must pass.

## Cutting a pre-release (`main` → `dev`)

```bash
gh pr create --base dev --head main --title "chore: promote main to dev" --body "Promote main to the dev channel."
```

Merge it with **Create a merge commit**. The *Release* workflow then reads the commits since the last
release and publishes, for example, `v0.2.0-dev.1` marked as a pre-release, with the `.deb`, `.rpm`, Arch
package and tarball attached. Push more promotions and you get `-dev.2`, `-dev.3`, …

## Cutting a stable release (`dev` → `prod`)

```bash
gh pr create --base prod --head dev --title "chore: release dev to prod" --body "Promote the tested pre-release to stable."
```

Merge it with **Create a merge commit**. The workflow publishes the stable `v0.2.0` with all packages.

> **Never squash a promotion.** Promotions must keep the commit history intact: semantic-release
> decides the next version from the commits it can see, and the tags must stay reachable.

## How the version is decided

| Commit since the last release | Version change |
|---|---|
| `feat:` | minor (0.1.0 → 0.2.0) |
| `fix:`, `perf:`, `revert:` | patch (0.1.0 → 0.1.1) |
| `feat!:` or a `BREAKING CHANGE:` footer | minor while we are on 0.x, major from 1.0 |
| `docs:`, `ci:`, `build:`, `test:`, `chore:`, `refactor:`, `style:` | no release |

No `feat`/`fix`/`perf` commits since the last release means a promotion publishes nothing.

## Where the version lives

The version is **not** committed back to the repository. Several long-lived branches would otherwise
conflict every time `dev` is merged into `prod`. Instead:

- `Cargo.toml` stays at a neutral `0.0.0`;
- the release version comes from the **git tag**, and the release workflow stamps it into every package
  while building (`scripts/set-version.sh`);
- the **GitHub Releases** page is the changelog, with notes generated from the commits.

So a build you make locally says `winbridge 0.0.0`; a downloaded release says its real version.

## Package formats and versions

Each release attaches `.deb`, `.rpm`, Arch `.pkg.tar.zst` and a portable `.tar.gz`, plus `SHA256SUMS.txt`.
Pre-release versions are written so that package managers sort them **before** the stable release:
`0.2.0-dev.1` becomes `0.2.0~dev.1` for `.deb` and `.rpm`, and `0.2.0_dev.1` for Arch.

## Maintainers: first-time setup

- Default branch: `main`. Create `dev` and `prod` from it.
- Repository setting **Actions → General → Workflow permissions** must allow *Read and write*
  (the release workflow also declares the permissions it needs).
- Recommended: protect `main`, `dev` and `prod` (require a pull request and the CI checks) so
  nothing reaches a release branch without passing them.
