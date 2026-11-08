<!-- The PR title becomes the squash/merge commit and drives releases: use Conventional Commits,
     e.g. "feat(gui): add switch pro artwork" or "fix(core): handle missing /proc entry". -->

## What and why

<!-- What does this change, and why is it needed? Link the issue: "Closes #123". -->

## How I tested it

<!-- Commands you ran, distributions / hardware you tried. -->

**Not tested / not possible to test here:**

## Checklist

- [ ] The PR title follows [Conventional Commits](https://www.conventionalcommits.org) (`feat`, `fix`, `docs`…)
- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass
- [ ] I added or updated tests for the behaviour I changed
- [ ] User-facing messages are plain language and tell people how to fix the problem
- [ ] I updated the docs (README / docs/) if behaviour changed
