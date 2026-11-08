#!/usr/bin/env bash
# Sets the workspace version in Cargo.toml and refreshes Cargo.lock. Run by semantic-release.
# Usage: ./scripts/set-version.sh X.Y.Z
set -euo pipefail
v="${1:?usage: set-version.sh X.Y.Z}"
[[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+].+)?$ ]] || { echo "not a version: $v" >&2; exit 64; }
cd "$(dirname "$0")/.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
python3 -I - "$v" <<'PY'
import re, sys
v = sys.argv[1]
s = open("Cargo.toml").read()
head, sep, rest = s.partition("[workspace.package]")
assert sep, "no [workspace.package] section"
rest = re.sub(r'^version\s*=\s*".*"', f'version = "{v}"', rest, count=1, flags=re.M)
open("Cargo.toml", "w").write(head + sep + rest)
PY
cargo update --workspace --quiet
echo "workspace version is now $v"
