#!/usr/bin/env bash
# Builds dist/winbridge-<version>-linux-<arch>.tar.gz, a portable bundle for any Linux distribution.
# Needs: cargo, tar.  Usage: ./scripts/build-tarball.sh
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

VERSION="$(sed -n '/^\[workspace.package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p}' Cargo.toml | head -1)"
ARCH="$(uname -m)"
NAME="winbridge-${VERSION}-linux-${ARCH}"
STAGE="dist/$NAME"

cargo build --release --workspace
rm -rf "$STAGE"
install -Dm755 target/release/winbridge       "$STAGE/bin/winbridge"
install -Dm755 target/release/winbridge-gui   "$STAGE/bin/winbridge-gui"
install -Dm644 packaging/winbridge.desktop    "$STAGE/share/applications/winbridge.desktop"
install -Dm644 packaging/winbridge.svg        "$STAGE/share/icons/hicolor/scalable/apps/winbridge.svg"
install -Dm644 README.md LICENSE              -t "$STAGE"
cat > "$STAGE/install.sh" <<'INSTALL'
#!/usr/bin/env bash
# Installs WinBridge for the current user into ~/.local (no root needed).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
prefix="${PREFIX:-$HOME/.local}"
install -Dm755 "$here/bin/winbridge"     "$prefix/bin/winbridge"
install -Dm755 "$here/bin/winbridge-gui" "$prefix/bin/winbridge-gui"
install -Dm644 "$here/share/applications/winbridge.desktop" "$prefix/share/applications/winbridge.desktop"
install -Dm644 "$here/share/icons/hicolor/scalable/apps/winbridge.svg" "$prefix/share/icons/hicolor/scalable/apps/winbridge.svg"
echo "Installed to $prefix. Start WinBridge from your applications menu, or run: winbridge-gui"
echo "On first start it offers to install the graphics support and game engine it needs."
INSTALL
chmod 755 "$STAGE/install.sh"
tar -C dist -czf "dist/$NAME.tar.gz" "$NAME"
echo "Built dist/$NAME.tar.gz ($(du -h "dist/$NAME.tar.gz" | cut -f1))"
