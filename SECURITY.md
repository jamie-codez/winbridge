# Security Policy

## Supported versions

WinBridge is young. Only the **latest release** receives security fixes.

## Reporting a vulnerability

**Please do not open a public issue for security problems.**

Use GitHub's private reporting: go to the repository's **Security** tab and choose
**Report a vulnerability**. Include what you found, how to reproduce it, and the impact you
expect. You will get a response as soon as a maintainer can look at it; please allow a few days.

If private reporting is unavailable, open a public issue that says only "I would like to report a
security problem" (no details) and a maintainer will contact you.

## What is in scope

WinBridge does a few things that deserve extra care:

- **The setup script** (`scripts/install-deps.sh`, also embedded in the app) installs packages with
  administrator rights. It runs a single generated helper after the user confirms, downloads
  umu-launcher only from its official GitHub release over HTTPS, and **refuses to install if the
  SHA-256 published by GitHub doesn't match**.
- **Launching games** runs programs the user chose, inside Proton's container and a private
  Wine prefix. This is **not a security sandbox**: a malicious Windows program can still harm your
  user account. Only run software you trust.
- **Reading input devices** (`/dev/input/event*`) for the controller test. WinBridge only reads
  them; it never records or transmits input.

WinBridge makes no network requests of its own except downloading umu-launcher during setup (and
umu-launcher then downloads Proton and the Steam Linux Runtime).

Reports about weaknesses in those areas, or in packaging, are very welcome. Vulnerabilities in
Wine, Proton, umu-launcher or your games belong with those projects.
