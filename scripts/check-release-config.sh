#!/usr/bin/env bash
# Enforces the release policy: ONLY `dev` (pre-release) and `prod` (stable) may publish a release.
# Fails if semantic-release is configured for any other branch, if the Release workflow can be
# triggered from any other branch, or if any other workflow publishes releases or tags.
# Run by CI on every change.  Usage: ./scripts/check-release-config.sh
set -euo pipefail
cd "$(dirname "$0")/.."
python3 -I - <<'PY'
import glob, json, re, sys

errors = []

# 1. semantic-release is configured for exactly: prod (stable) and dev (pre-release)
rc = json.load(open(".releaserc.json"))
branches = rc.get("branches", [])
norm = [(b, None) if isinstance(b, str) else (b.get("name"), b.get("prerelease")) for b in branches]
if norm != [("prod", None), ("dev", "dev")]:
    errors.append(f".releaserc.json branches must be exactly [prod (stable), dev (prerelease)], found: {norm}")

# 2. the Release workflow only triggers on pushes to dev and prod, and re-checks the branch itself
rel = open(".github/workflows/release.yml").read()
m = re.search(r"^on:\s*\n\s*push:\s*\n\s*branches:\s*\[([^\]]*)\]", rel, re.M)
trig = sorted(x.strip() for x in m.group(1).split(",")) if m else None
if trig != ["dev", "prod"]:
    errors.append(f"release.yml must trigger only on push to [dev, prod], found: {trig}")
if re.search(r"^\s*(workflow_dispatch|pull_request|schedule|release|create):?", rel, re.M) or re.search(r"\btags:", rel):
    errors.append("release.yml must not have any trigger other than push to dev/prod (no tags, dispatch, PRs, schedule)")
if "refs/heads/dev" not in rel or "refs/heads/prod" not in rel:
    errors.append("release.yml must guard its jobs with an explicit dev/prod branch check")

# 3. no other workflow may publish releases or create tags
for f in glob.glob(".github/workflows/*.yml"):
    if f.endswith("release.yml"):
        continue
    text = open(f).read()
    for needle in ("gh release create", "gh release upload", "semantic-release", "softprops/action-gh-release", "git push --tags", "git tag "):
        if needle in text:
            errors.append(f"{f} must not publish releases or tags (found '{needle}')")

if errors:
    print("Release policy violations:")
    for e in errors:
        print("  -", e)
    sys.exit(1)
print("release policy ok: only dev (pre-release) and prod (stable) can release")
PY
