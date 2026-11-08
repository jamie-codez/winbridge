#!/usr/bin/env bash
# Checks that a commit subject follows Conventional Commits (https://www.conventionalcommits.org).
# Used by the local commit-msg hook (.githooks/) and by CI. Usage: check-commit-msg.sh "<subject>"
set -euo pipefail
subject="${1:-}"
[ -n "$subject" ] || { echo "usage: $0 \"<commit subject>\"" >&2; exit 64; }

# Git-generated subjects are allowed.
case "$subject" in "Merge "*|'Revert "'*|"fixup! "*|"squash! "*) exit 0 ;; esac

re='^(feat|fix|perf|refactor|docs|test|build|ci|chore|style|revert)(\([a-z0-9._/-]+\))?!?: [^ ].{0,71}$'
if [[ "$subject" =~ $re ]]; then
  exit 0
fi

cat >&2 <<MSG
Commit subject does not follow Conventional Commits:

    $subject

Expected:  <type>(<optional scope>): <short summary>
Types:     feat  fix  perf  refactor  docs  test  build  ci  chore  style  revert
Examples:  feat(gui): add dualsense artwork
           fix(core): centre sticks before the first event
           feat!: change the library file format      (the ! marks a breaking change)

feat -> minor release, fix/perf -> patch release. See CONTRIBUTING.md.
MSG
exit 1
