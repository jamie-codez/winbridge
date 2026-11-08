#!/usr/bin/env bash
# WinBridge setup: installs everything needed to play Windows games on this Linux system.
#
#   * graphics: Vulkan loader + drivers, including the 32-bit versions older games need
#   * the game engine launcher (umu-launcher) that runs Proton
#   * controller access rules, GameMode, MangoHud, Gamescope (extras are best effort)
#
# Used by developers (`./scripts/install-deps.sh`) and by the app, which embeds this exact
# file and runs it from its first-run screen. Safe to re-run: it only installs what's missing.
#
# Usage: install-deps.sh [--check] [--list] [--dry-run] [--yes] [--extras]
#   --check    print what is missing; exit 0 if nothing is, 10 if setup is needed, 2 if this
#              distribution isn't supported by the automatic installer
#   --list     show the full plan (also what is already installed)
#   --dry-run  like a real run, but change nothing
#   --yes      don't ask for confirmation (the app passes this after the user clicks Install)
#   --extras   also install only the optional extras (GameMode, MangoHud, Gamescope…)
#
# Supported: Debian/Ubuntu family (apt, tested), Fedora (dnf) and Arch (pacman). umu-launcher comes
# from the distribution when it carries it, otherwise from the official release; if neither exists
# the rest is still installed and a warning explains what's missing. The dnf and pacman paths
# have only been tested in simulation so far.
set -euo pipefail

UMU_VERSION="${UMU_VERSION:-1.4.4}"
REPO="Open-Wine-Components/umu-launcher"
CHECK=0 LIST=0 DRY=0 YES=0 EXTRAS_ONLY=0
for a in "$@"; do
  case "$a" in
    --check) CHECK=1 ;;
    --list) LIST=1 ;;
    --dry-run) DRY=1 ;;
    --yes|-y) YES=1 ;;
    --extras) EXTRAS_ONLY=1 ;;
    -h|--help) sed -n '2,19p' "$0"; exit 0 ;;
    *) echo "unknown option: $a" >&2; exit 64 ;;
  esac
done

say()  { echo "[winbridge-setup] $*"; }
fail() { echo "[winbridge-setup] ERROR: $*" >&2; exit 1; }

# ---- 1. Which package manager / distro family? ---------------------------------------
OS_RELEASE="${WINBRIDGE_OS_RELEASE:-/etc/os-release}"   # overridable for testing
[ -r "$OS_RELEASE" ] || fail "cannot read $OS_RELEASE"
# shellcheck disable=SC1091
. "$OS_RELEASE"
like=" ${ID:-} ${ID_LIKE:-} "
PM=""
if command -v apt-get >/dev/null 2>&1 && command -v dpkg >/dev/null 2>&1; then PM=apt
elif command -v dnf >/dev/null 2>&1; then PM=dnf
elif command -v pacman >/dev/null 2>&1; then PM=pacman
fi
if [ -z "$PM" ]; then
  say "${PRETTY_NAME:-This system} isn't supported by the automatic installer yet."
  say "You need: Vulkan drivers (32-bit too), umu-launcher ($REPO), GameMode, MangoHud."
  exit 2
fi

# ---- 2. Package lists per distro ------------------------------------------------------
# CORE   must be installed for games to run.   EXTRA   is nice to have; failures are ignored.
gpu_vendors() {
  for d in /sys/class/drm/card*/device/vendor; do [ -r "$d" ] && cat "$d"; done 2>/dev/null | sort -u
}
vendors="$(gpu_vendors || true)"

CORE=(); EXTRA=(); NEED_UMU_DEB=0; MULTIARCH_I386=0
case "$PM" in
  apt)
    CORE=(libvulkan1 mesa-vulkan-drivers libgl1-mesa-dri python3 curl xdg-desktop-portal
          libvulkan1:i386 mesa-vulkan-drivers:i386 libgl1-mesa-dri:i386 libglx-mesa0:i386)
    EXTRA=(steam-devices gamemode mangohud gamescope vulkan-tools)
    NEED_UMU_DEB=1; MULTIARCH_I386=1 ;;
  dnf)
    CORE=(vulkan-loader vulkan-loader.i686 mesa-vulkan-drivers mesa-vulkan-drivers.i686
          mesa-dri-drivers.i686 python3 curl xdg-desktop-portal)
    EXTRA=(steam-devices gamemode mangohud gamescope vulkan-tools) ;;
  pacman)
    CORE=(vulkan-icd-loader lib32-vulkan-icd-loader python curl xdg-desktop-portal)
    case "$vendors" in *0x1002*) CORE+=(vulkan-radeon lib32-vulkan-radeon) ;; esac
    case "$vendors" in *0x8086*) CORE+=(vulkan-intel lib32-vulkan-intel) ;; esac
    case "$vendors" in *0x10de*) CORE+=(nvidia-utils lib32-nvidia-utils) ;; esac
    EXTRA=(game-devices-udev gamemode lib32-gamemode mangohud gamescope vulkan-tools) ;;
esac

installed() {
  case "$PM" in
    apt)    dpkg-query -W -f='${Status}' "$1" 2>/dev/null | grep -q 'install ok installed' ;;
    dnf)    rpm -q "$1" >/dev/null 2>&1 ;;
    pacman) pacman -Qi "$1" >/dev/null 2>&1 ;;
  esac
}

MISSING_CORE=(); MISSING_EXTRA=()
for p in "${CORE[@]}";  do installed "$p" || MISSING_CORE+=("$p"); done
for p in "${EXTRA[@]}"; do installed "$p" || MISSING_EXTRA+=("$p"); done
UMU_MISSING=0
if ! command -v umu-run >/dev/null 2>&1; then UMU_MISSING=1; fi

NVIDIA_NOTE=0
case "$vendors" in *0x10de*)
  if ! ls /usr/share/vulkan/icd.d/nvidia_icd.json /etc/vulkan/icd.d/nvidia_icd.json >/dev/null 2>&1; then NVIDIA_NOTE=1; fi ;;
esac

print_plan() {
  [ "$UMU_MISSING" -eq 1 ] && echo "download: umu-launcher (the game engine launcher, from your distribution or the official release)"
  for p in "${MISSING_CORE[@]}";  do echo "install: $p"; done
  for p in "${MISSING_EXTRA[@]}"; do echo "optional: $p"; done
  [ "$MULTIARCH_I386" -eq 1 ] && ! dpkg --print-foreign-architectures | grep -qx i386 && echo "enable: 32-bit package support (i386)"
  [ "$NVIDIA_NOTE" -eq 1 ] && echo "note: NVIDIA graphics need the proprietary NVIDIA driver - install it from your system's driver tool"
  return 0
}

NEEDS=$(( ${#MISSING_CORE[@]} + UMU_MISSING ))
[ "$EXTRAS_ONLY" -eq 1 ] && NEEDS=${#MISSING_EXTRA[@]}

if [ "$CHECK" -eq 1 ]; then
  print_plan
  [ "$NEEDS" -eq 0 ] && exit 0 || exit 10
fi
if [ "$LIST" -eq 1 ]; then
  say "System: ${PRETTY_NAME:-unknown} (package manager: $PM)"
  print_plan
  exit 0
fi
if [ "$NEEDS" -eq 0 ] && { [ "$EXTRAS_ONLY" -eq 1 ] || [ ${#MISSING_EXTRA[@]} -eq 0 ]; }; then
  say "Everything is already installed. Nothing to do."
  exit 0
fi

say "System: ${PRETTY_NAME:-unknown} (package manager: $PM)"
say "Plan:"
print_plan | sed 's/^/[winbridge-setup]   /'
if [ "$DRY" -eq 1 ]; then say "Dry run: nothing downloaded or installed."; exit 0; fi
if [ "$YES" -eq 0 ] && [ -t 0 ]; then
  read -r -p "[winbridge-setup] Install now? [y/N] " ans
  case "$ans" in y|Y|yes) ;; *) say "Cancelled."; exit 1 ;; esac
fi

# ---- 3. apt: fetch + verify the umu-launcher packages (as the normal user) -------------
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
chmod 755 "$tmp"   # the package manager's sandbox user must be able to read it
UMU_FILES=()
UMU_REPO=0     # 1 = install umu-launcher from the distribution's own repositories
UMU_WARN=""    # set when umu-launcher can't be installed on this platform

# Print "name<TAB>url<TAB>size<TAB>sha256" for release assets whose name starts with a prefix
# and ends with one of the given suffixes.
release_assets() { # $1 = comma-separated prefixes, rest = suffixes
  local prefixes="$1"; shift
  printf '%s' "$meta" | python3 -I -c '
import sys, json
prefixes = tuple(sys.argv[1].split(","))
suffixes = sys.argv[2:]
for a in json.load(sys.stdin)["assets"]:
    if a["name"].startswith(prefixes) and any(a["name"].endswith(x) for x in suffixes):
        d = (a.get("digest") or "")
        print(a["name"], a["browser_download_url"], a["size"], d.removeprefix("sha256:"), sep="\t")
' "$prefixes" "$@"
}

# Download the given asset lines into $tmp, verifying SHA-256, and append them to UMU_FILES.
fetch_verified() { # stdin = release_assets output
  local name url size sha got
  while IFS=$'\t' read -r name url size sha; do
    [ -n "$name" ] || continue
    [ -n "$sha" ] || fail "GitHub gave no checksum for $name; refusing to install unverified software."
    say "Downloading $name"
    curl -fL --proto '=https' --tlsv1.2 -sS -o "$tmp/$name" "$url" || fail "download failed: $name"
    got="$(sha256sum "$tmp/$name" | cut -d' ' -f1)"
    [ "$got" = "$sha" ] || fail "checksum mismatch for $name (expected $sha, got $got). Aborting."
    chmod 644 "$tmp/$name"
    UMU_FILES+=("$tmp/$name")
  done
  say "Checksums verified."
}

# Run a helper script with administrator rights (root, sudo in a terminal, or a pkexec dialog).
run_priv() { # $1 = path of a sh script
  if [ "$(id -u)" -eq 0 ]; then
    sh "$1"
  elif [ -t 0 ] && command -v sudo >/dev/null 2>&1; then
    say "Asking for your password (sudo) to install the packages."
    sudo sh "$1"
  elif command -v pkexec >/dev/null 2>&1; then
    say "Asking for your password in a system dialog to install the packages."
    pkexec sh "$1"
  else
    fail "need root to install. Run this script from a terminal with sudo available."
  fi
}

# The download step needs curl and python3. On a minimal system install just those first, so
# first-run setup can provision its own bootstrap tools (this can mean a second password prompt).
ensure_download_tools() {
  local missing=() pypkg=python3 h="$tmp/tools.sh"
  [ "$PM" = pacman ] && pypkg=python
  command -v curl >/dev/null 2>&1    || missing+=(curl)
  command -v python3 >/dev/null 2>&1 || missing+=("$pypkg")
  [ ${#missing[@]} -eq 0 ] && return 0
  say "Installing ${missing[*]} first (needed to fetch the game engine launcher)."
  {
    echo 'set -e'
    case "$PM" in
      apt) echo 'export DEBIAN_FRONTEND=noninteractive'; echo 'apt-get update || true'; printf 'apt-get install -y' ;;
      dnf) printf 'dnf install -y' ;;
      pacman) printf 'pacman -S --needed --noconfirm' ;;
    esac
    printf ' %q' "${missing[@]}"; echo
  } > "$h"
  chmod 644 "$h"
  run_priv "$h"
  command -v curl >/dev/null 2>&1 && command -v python3 >/dev/null 2>&1 || fail "could not install curl and python3"
}

if [ "$UMU_MISSING" -eq 1 ] && [ "$EXTRAS_ONLY" -eq 0 ]; then
  ensure_download_tools
  fetch_meta() { meta="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/tags/$UMU_VERSION")" || fail "could not reach GitHub. Check your internet connection."; }
  case "$PM" in
    apt)
      arch="$(dpkg --print-architecture)"
      case "$like" in
        *" ubuntu "*) suffix="ubuntu-${UBUNTU_CODENAME:-${VERSION_CODENAME:-}}" ;;
        *" debian "*) major="${VERSION_ID:-}"; suffix="debian-${major%%.*}" ;;
        *) fail "unsupported Debian-family system: ${PRETTY_NAME:-unknown}" ;;
      esac
      fetch_meta
      wanted="$(release_assets "python3-umu-launcher_,umu-launcher_" "_${arch}_${suffix}.deb" "_all_${suffix}.deb")"
      if [ "$(printf '%s\n' "$wanted" | grep -c .)" -eq 2 ]; then
        fetch_verified <<< "$wanted"   # here-string, NOT a pipeline: a pipeline would run it in a subshell and lose UMU_FILES
      else
        UMU_WARN="No umu-launcher $UMU_VERSION package exists for '$suffix'/$arch. Get it from https://github.com/$REPO/releases"
      fi ;;
    dnf)
      if dnf -q list --available umu-launcher >/dev/null 2>&1; then
        UMU_REPO=1
      else
        # Not in the repositories: use the release RPM built for this Fedora version, if there is one.
        fetch_meta
        wanted="$(release_assets "umu-launcher-" ".fc${VERSION_ID:-0}.$(uname -m).rpm")"
        if [ "$(printf '%s\n' "$wanted" | grep -c .)" -ge 1 ]; then
          fetch_verified <<< "$wanted"   # here-string, NOT a pipeline: a pipeline would run it in a subshell and lose UMU_FILES
        else
          UMU_WARN="umu-launcher isn't available for ${PRETTY_NAME:-your system}. Get it from https://github.com/$REPO/releases"
        fi
      fi ;;
    pacman) UMU_REPO=1 ;;
  esac
  [ -z "$UMU_WARN" ] || say "Warning: $UMU_WARN (continuing with everything else)"
fi

# ---- 4. Write ONE privileged helper so the user types their password once ------------
helper="$tmp/helper.sh"
{
  echo 'set -e'
  case "$PM" in
    apt)
      echo 'export DEBIAN_FRONTEND=noninteractive'
      if [ "$MULTIARCH_I386" -eq 1 ] && ! dpkg --print-foreign-architectures | grep -qx i386; then
        echo 'dpkg --add-architecture i386'
      fi
      echo 'apt-get update || echo "[winbridge-setup] warning: apt-get update reported problems; continuing"'
      if [ "$EXTRAS_ONLY" -eq 0 ] && { [ ${#MISSING_CORE[@]} -gt 0 ] || [ ${#UMU_FILES[@]} -gt 0 ]; }; then
        printf 'apt-get install -y'; printf ' %q' "${MISSING_CORE[@]}" "${UMU_FILES[@]}"; echo
      fi
      for p in "${MISSING_EXTRA[@]}"; do
        printf 'apt-get install -y %q || echo "[winbridge-setup] optional package %s not available here, skipping"\n' "$p" "$p"
      done ;;
    dnf)
      if [ "$EXTRAS_ONLY" -eq 0 ] && [ ${#MISSING_CORE[@]} -gt 0 ]; then
        printf 'dnf install -y'; printf ' %q' "${MISSING_CORE[@]}"; echo
      fi
      if [ "$EXTRAS_ONLY" -eq 0 ] && [ "$UMU_REPO" -eq 1 ]; then
        echo 'dnf install -y umu-launcher || echo "[winbridge-setup] warning: could not install umu-launcher from the repositories"'
      elif [ "$EXTRAS_ONLY" -eq 0 ] && [ ${#UMU_FILES[@]} -gt 0 ]; then
        printf 'dnf install -y'; printf ' %q' "${UMU_FILES[@]}"; echo ' || echo "[winbridge-setup] warning: could not install the umu-launcher package"'
      fi
      for p in "${MISSING_EXTRA[@]}"; do
        printf 'dnf install -y %q || echo "[winbridge-setup] optional package %s not available here, skipping"\n' "$p" "$p"
      done ;;
    pacman)
      # lib32-* packages need the multilib repository; drop them (with a warning) if it is off.
      echo 'multilib=1; pacman -Sp lib32-vulkan-icd-loader >/dev/null 2>&1 || multilib=0'
      if [ "$EXTRAS_ONLY" -eq 0 ] && [ ${#MISSING_CORE[@]} -gt 0 ]; then
        printf 'core=('; printf ' %q' "${MISSING_CORE[@]}"; echo ' )'
        echo 'if [ "$multilib" = 0 ]; then echo "[winbridge-setup] multilib repo is off: skipping 32-bit packages"; core=( "${core[@]/lib32-*/}" ); fi'
        echo 'pacman -S --needed --noconfirm $(printf "%s " "${core[@]}")'
      fi
      if [ "$EXTRAS_ONLY" -eq 0 ] && [ "$UMU_REPO" -eq 1 ]; then
        echo 'pacman -S --needed --noconfirm umu-launcher || echo "[winbridge-setup] warning: umu-launcher is not in your repositories (try the AUR: yay -S umu-launcher)"'
      fi
      for p in "${MISSING_EXTRA[@]}"; do
        printf 'pacman -S --needed --noconfirm %q || echo "[winbridge-setup] optional package %s not available here, skipping"\n' "$p" "$p"
      done ;;
  esac
} > "$helper"
chmod 644 "$helper"

run_priv "$helper"

UMU_STILL_MISSING=0
if [ "$EXTRAS_ONLY" -eq 0 ] && ! command -v umu-run >/dev/null 2>&1; then
  if [ "$PM" = apt ] && [ -z "$UMU_WARN" ]; then fail "install finished but umu-run is still missing."; fi
  UMU_STILL_MISSING=1
  say "Warning: umu-launcher is not installed, so games can't start yet. See https://github.com/$REPO/releases"
fi
[ "$NVIDIA_NOTE" -eq 1 ] && say "Reminder: NVIDIA graphics need the proprietary NVIDIA driver (use your system's driver tool)."
if [ "$UMU_STILL_MISSING" -eq 1 ]; then
  say "Finished, but the game engine launcher (umu-launcher) is still missing - see the warning above."
else
  say "Done. Your system is ready to play."
fi
