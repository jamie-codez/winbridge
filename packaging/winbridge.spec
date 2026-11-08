# RPM spec for Fedora and compatible distributions.
# Build (after `cargo build --release`):
#   rpmbuild -bb packaging/winbridge.spec --define "srcdir $PWD" --define "_topdir $PWD/dist/rpmbuild"
%global debug_package %{nil}
%global __strip /bin/true

Name:           winbridge
Version:        %{?pkgver}%{!?pkgver:0.1.0}
Release:        1%{?dist}
Summary:        Run Windows games on Linux, the easy way
License:        MIT
URL:            https://github.com/jamie-codez/winbridge

Requires:       python3
Requires:       curl
Requires:       vulkan-loader
Requires:       mesa-vulkan-drivers
Requires:       mesa-dri-drivers
Requires:       xdg-desktop-portal
Recommends:     gamemode
Recommends:     mangohud
Recommends:     steam-devices
Recommends:     vulkan-tools
Suggests:       gamescope

%description
WinBridge is a simple launcher for Windows games built on umu-launcher and Proton.
It checks your system, installs the game engine on first start, keeps one private
Windows folder per game, and includes a controller test with Xbox and PlayStation
artwork plus a button-mapping wizard.

%install
install -Dm755 %{srcdir}/target/release/winbridge      %{buildroot}%{_bindir}/winbridge
install -Dm755 %{srcdir}/target/release/winbridge-gui  %{buildroot}%{_bindir}/winbridge-gui
install -Dm644 %{srcdir}/packaging/winbridge.desktop   %{buildroot}%{_datadir}/applications/winbridge.desktop
install -Dm644 %{srcdir}/packaging/winbridge.svg       %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/winbridge.svg
install -Dm644 %{srcdir}/README.md                     %{buildroot}%{_docdir}/winbridge/README.md
install -Dm644 %{srcdir}/LICENSE                       %{buildroot}%{_licensedir}/winbridge/LICENSE

%files
%{_licensedir}/winbridge/LICENSE
%{_docdir}/winbridge/README.md
%{_bindir}/winbridge
%{_bindir}/winbridge-gui
%{_datadir}/applications/winbridge.desktop
%{_datadir}/icons/hicolor/scalable/apps/winbridge.svg

%changelog
* Sun Nov 08 2026 jamie-codez <jamie-codez@users.noreply.github.com> - 0.1.0-1
- Initial package
