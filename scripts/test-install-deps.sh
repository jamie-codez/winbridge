#!/usr/bin/env bash
# Tests scripts/install-deps.sh against FAKE systems (fake package managers, fake network, fake
# privilege escalation). Nothing is installed or downloaded. Run: ./scripts/test-install-deps.sh
set -uo pipefail
cd "$(dirname "$0")/.."
SCRIPT="$PWD/scripts/install-deps.sh"
fails=0
ok()   { echo "  ok   - $1"; }
bad()  { echo "  FAIL - $1"; fails=$((fails + 1)); }
expect() { # $1 = description, $2 = output, $3 = fixed string that must appear
  if grep -qF -- "$3" <<< "$2"; then ok "$1"; else bad "$1 (missing: $3)"; echo "$2" | sed 's/^/       | /' | head -25; fi
}
expect_re() { # $1 = description, $2 = output, $3 = extended regex that must match a whole line
  if grep -qE -- "$3" <<< "$2"; then ok "$1"; else bad "$1 (no line matches: $3)"; echo "$2" | sed 's/^/       | /' | head -25; fi
}

# A sandbox PATH holding only the tools the script needs, plus fakes we control.
mk_sandbox() { # $1 = dir
  mkdir -p "$1"
  for t in bash sh cat grep sed awk sort id mktemp rm chmod tr cut ls head uname env sha256sum python3 mkdir dirname date wc printf cp basename; do
    [ -x "/usr/bin/$t" ] && ln -sf "/usr/bin/$t" "$1/$t"
  done
  # pkexec fake: show what WOULD run as root, then do nothing
  printf '#!/bin/sh\necho "--- privileged helper ($2):"; cat "$2"\n' > "$1/pkexec"
  chmod +x "$1/pkexec"
}
run() { # $1 = sandbox, $2 = os-release file ; extra args passed to the script
  local sb="$1" osr="$2"; shift 2
  env -i PATH="$sb" HOME="$HOME" FAKE_SERVE="${FAKE_SERVE:-}" WINBRIDGE_OS_RELEASE="$osr" /usr/bin/bash "$SCRIPT" "$@" < /dev/null 2>&1
}
fake_release_json() { # $1 = directory to serve, $2... = "name=content"
  python3 - "$@" <<'PY'
import hashlib, json, sys
out = sys.argv[1]; assets = []
for spec in sys.argv[2:]:
    name, content = spec.split("=", 1)
    open(f"{out}/{name}", "w").write(content)
    assets.append({"name": name, "browser_download_url": f"https://fake.invalid/{name}", "size": len(content),
                   "digest": "sha256:" + hashlib.sha256(content.encode()).hexdigest()})
json.dump({"assets": assets}, open(f"{out}/release.json", "w"))
PY
}
# curl fake: the API call returns release.json; a download copies the named file from $FAKE_SERVE
install_fake_curl() { # $1 = sandbox
  cat > "$1/curl" <<'CURL'
#!/bin/sh
out=""; url=""
while [ $# -gt 0 ]; do
  case "$1" in -o) out="$2"; shift 2 ;; http*) url="$1"; shift ;; *) shift ;; esac
done
case "$url" in
  *api.github.com*) cat "$FAKE_SERVE/release.json" ;;
  *) cp "$FAKE_SERVE/$(basename "$url")" "$out" ;;
esac
CURL
  chmod +x "$1/curl"
}

T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT

echo "apt: a verified umu download must reach the install command (regression: pipeline subshell lost it)"
SB="$T/apt"; mk_sandbox "$SB"; install_fake_curl "$SB"
printf '#!/bin/sh\nexit 1\n' > "$SB/dpkg-query"; chmod +x "$SB/dpkg-query"          # nothing installed
for t in dpkg apt-get; do printf '#!/bin/sh\n[ "$1" = "--print-architecture" ] && echo amd64; [ "$1" = "--print-foreign-architectures" ] && echo i386; exit 0\n' > "$SB/$t"; chmod +x "$SB/$t"; done
printf 'ID=ubuntu\nVERSION_ID="24.04"\nUBUNTU_CODENAME=noble\nID_LIKE=debian\nPRETTY_NAME="Ubuntu 24.04"\n' > "$T/ubuntu"
mkdir -p "$T/serve1"
fake_release_json "$T/serve1" "python3-umu-launcher_1.4.4-1_amd64_ubuntu-noble.deb=PYDEB" "umu-launcher_1.4.4-1_all_ubuntu-noble.deb=ALLDEB"
OUT="$(FAKE_SERVE="$T/serve1" run "$SB" "$T/ubuntu" --yes)"
expect "downloads were verified"                    "$OUT" "Checksums verified."
# These look at the actual privileged "apt-get install" line, not at the "Downloading ..." log lines.
expect_re "python3-umu-launcher .deb is on the apt-get install line" "$OUT" '^apt-get install -y .*/python3-umu-launcher_1\.4\.4-1_amd64_ubuntu-noble\.deb'
expect_re "umu-launcher .deb is on the apt-get install line"         "$OUT" '^apt-get install -y .*/umu-launcher_1\.4\.4-1_all_ubuntu-noble\.deb'

echo "apt: a corrupted download is refused"
mkdir -p "$T/serve2"; fake_release_json "$T/serve2" "python3-umu-launcher_1.4.4-1_amd64_ubuntu-noble.deb=GOOD" "umu-launcher_1.4.4-1_all_ubuntu-noble.deb=GOOD"
printf 'TAMPERED' > "$T/serve2/umu-launcher_1.4.4-1_all_ubuntu-noble.deb"
OUT="$(FAKE_SERVE="$T/serve2" run "$SB" "$T/ubuntu" --yes)"
expect "checksum mismatch aborts" "$OUT" "checksum mismatch"

echo "dnf: umu not in the repos -> verified release RPM is used"
SB="$T/dnf"; mk_sandbox "$SB"; install_fake_curl "$SB"
printf '#!/bin/sh\n[ "$1 $2 $3" = "-q list --available" ] && exit 1\nexit 0\n' > "$SB/dnf"; printf '#!/bin/sh\nexit 1\n' > "$SB/rpm"; chmod +x "$SB/dnf" "$SB/rpm"
printf 'ID=fedora\nVERSION_ID=43\nPRETTY_NAME="Fedora 43"\n' > "$T/fedora"
mkdir -p "$T/serve3"; fake_release_json "$T/serve3" "umu-launcher-1.4.4.fc43.x86_64.rpm=RPMDATA"
OUT="$(FAKE_SERVE="$T/serve3" run "$SB" "$T/fedora" --yes)"
expect_re "the verified rpm is on the dnf install line" "$OUT" '^dnf install -y .*/umu-launcher-1\.4\.4\.fc43\.x86_64\.rpm'

echo "dnf: nothing available anywhere -> graceful, everything else still installs"
mkdir -p "$T/serve4"; fake_release_json "$T/serve4"
OUT="$(FAKE_SERVE="$T/serve4" run "$SB" "$T/fedora" --yes)"
expect "warns that umu is unavailable"  "$OUT" "isn't available for"
expect "still installs the graphics stack" "$OUT" "dnf install -y vulkan-loader"

echo "pacman: umu from the repos, multilib guard"
SB="$T/pac"; mk_sandbox "$SB"; install_fake_curl "$SB"
printf '#!/bin/sh\ncase "$1" in -Qi) exit 1;; *) exit 0;; esac\n' > "$SB/pacman"; chmod +x "$SB/pacman"
printf 'ID=arch\nPRETTY_NAME="Arch"\n' > "$T/arch"
OUT="$(FAKE_SERVE="$T/serve4" run "$SB" "$T/arch" --yes)"
expect "installs umu-launcher from the repos" "$OUT" "pacman -S --needed --noconfirm umu-launcher"
expect "has the multilib guard"               "$OUT" 'multilib=1'

echo "minimal system without curl/python3: they are installed first"
SB="$T/min"; mk_sandbox "$SB"; rm -f "$SB/curl" "$SB/python3"
printf '#!/bin/sh\nexit 1\n' > "$SB/dpkg-query"; chmod +x "$SB/dpkg-query"
for t in dpkg apt-get; do printf '#!/bin/sh\n[ "$1" = "--print-architecture" ] && echo amd64; [ "$1" = "--print-foreign-architectures" ] && echo i386; exit 0\n' > "$SB/$t"; chmod +x "$SB/$t"; done
OUT="$(run "$SB" "$T/ubuntu" --yes)"
expect "installs curl and python3 first" "$OUT" "Installing curl python3 first"
expect "that first helper installs them"  "$OUT" "apt-get install -y curl python3"

echo
if [ "$fails" -eq 0 ]; then echo "all installer tests passed"; else echo "$fails installer test(s) FAILED"; exit 1; fi
