# Linux packaging and support

## Supported Linux distributions

The `.deb` package dependencies explicitly target the **64-bit `time_t` (`t64`) transition** packages:
- `libglib2.0-0t64`
- `libgtk-3-0t64`
- `libssl3t64 (>= 3.0.0)`
- `libatspi2.0-0t64`
- `libasound2t64`

### Verified & Tested Distributions:
- **Ubuntu 24.04 LTS (Noble Numbat)**: Primary CI verification baseline (`ubuntu-24.04` runner).
- **Debian 13 (Trixie)** / Debian Testing & Sid: Local development and live test baseline.

### Expected Compatible Distributions (预计兼容):
- **Linux Mint 22**: Compatible derivative based on Ubuntu 24.04.

### Incompatible Legacy Distributions:
The package is **not compatible with Ubuntu 22.04 LTS (Jammy) or Debian 12 (Bookworm)**:
their runtime repositories provide legacy non-t64 package names (`libglib2.0-0`, `libgtk-3-0`, `libssl3`, etc.) rather than the `t64` package names required by the debian control dependencies.

The Linux graphical application requires an active desktop session (X11 or Wayland); WSL2 requires WSLg or an X11 display server.

## Build

```bash
sudo apt-get update
sudo apt-get install -y build-essential curl git pkg-config libssl-dev \
  libsqlite3-dev libdbus-1-dev libglib2.0-dev libgtk-3-dev \
  libayatana-appindicator3-dev libjavascriptcoregtk-4.1-dev \
  libwebkit2gtk-4.1-dev libsoup-3.0-dev libz3-dev
cd apps/codex-plus-manager
npm ci
npm run vite:build
cd ../..
cargo build --release -p codex-plus-manager
bash scripts/installer/linux/build-deb.sh --package-only
```

The script reads the default version from
`apps/codex-plus-manager/package.json` using its absolute path, independent
of the calling directory. To supply a release version explicitly, pass it
after `--package-only`.

```bash
sudo apt install ./dist/linux/codex-plus-plus_*.deb
dpkg -V codex-plus-plus
ldd /usr/bin/codex-plus-plus
```

The package contains exactly one executable, `/usr/bin/codex-plus-plus`.
The PR build workflow verifies Linux release compilation and `.deb` package generation via the `linux-deb` job in `.github/workflows/pr-build.yml`.

The CI build and package installation check is not a substitute for a manual GUI launch
test in a real desktop session.
