#!/usr/bin/env bash
# Builds dist/winbridge_<version>_<arch>.deb (release binaries, launcher entry, icon).
# Needs: cargo, dpkg-deb, fakeroot.  Usage: ./scripts/build-deb.sh
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

VERSION="$(sed -n '/^\[workspace.package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p}' Cargo.toml | head -1)"
ARCH="$(dpkg --print-architecture)"
TILDE="~"   # kept in a variable: a literal ~ in the replacement would expand to $HOME
DEB_VERSION="${VERSION/-/$TILDE}"   # 0.2.0-dev.1 -> 0.2.0~dev.1 so a pre-release sorts before the stable release
NAME="winbridge_${VERSION}_${ARCH}"
PKG="dist/$NAME"

echo "==> Building release binaries"
cargo build --release --workspace

echo "==> Staging $PKG"
rm -rf "$PKG"
install -Dm755 target/release/winbridge       "$PKG/usr/bin/winbridge"
install -Dm755 target/release/winbridge-gui   "$PKG/usr/bin/winbridge-gui"
install -Dm644 packaging/winbridge.desktop    "$PKG/usr/share/applications/winbridge.desktop"
install -Dm644 packaging/winbridge.svg        "$PKG/usr/share/icons/hicolor/scalable/apps/winbridge.svg"
install -Dm644 README.md                      "$PKG/usr/share/doc/winbridge/README.md"
cat > "$PKG/usr/share/doc/winbridge/copyright" <<'COPYRIGHT'
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: WinBridge

Files: *
Copyright: the WinBridge authors
License: MIT
COPYRIGHT

SIZE_KB="$(du -sk "$PKG/usr" | cut -f1)"
mkdir -p "$PKG/DEBIAN"
cat > "$PKG/DEBIAN/control" <<CONTROL
Package: winbridge
Version: ${DEB_VERSION}
Section: games
Priority: optional
Architecture: ${ARCH}
Installed-Size: ${SIZE_KB}
Maintainer: WinBridge <winbridge@localhost>
Depends: libc6, libgcc-s1, libxkbcommon0, libwayland-client0 | libx11-6, libegl1 | libgl1, python3, curl, libvulkan1, mesa-vulkan-drivers, libgl1-mesa-dri, xdg-desktop-portal
Recommends: gamemode, steam-devices | game-devices-udev, mangohud, vulkan-tools
Suggests: gamescope
Description: Run Windows games on Linux, the easy way
 WinBridge is a simple launcher for Windows games built on umu-launcher and
 Proton. It checks your system, installs the game engine on first start, keeps
 one private Windows folder per game, and includes a controller test with
 Xbox and PlayStation artwork plus a button-mapping wizard.
 .
 Installing this package pulls in the graphics (Vulkan) and system pieces. On
 first start WinBridge offers one click to install the rest: 32-bit graphics
 libraries and the umu-launcher game engine.
CONTROL

echo "==> Building package"
fakeroot dpkg-deb --build --root-owner-group "$PKG" "dist/$NAME.deb" >/dev/null
echo "Built dist/$NAME.deb ($(du -h "dist/$NAME.deb" | cut -f1))"
