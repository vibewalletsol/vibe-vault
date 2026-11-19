# vibe-vault

vibe-vault is the native signer of [VIBE Wallet](https://vibewallet.net), a
self-custody Solana wallet for Linux. It holds your keys on your own machine,
decodes every transaction itself (including v0 address lookup tables), runs the
drainer checks and shows the confirmation screen before anything is signed.

This repository contains the complete source of the three GPL binaries that
ship in VIBE Wallet packages:

| Binary       | Crate / path                           | What it does                                  |
|--------------|----------------------------------------|-----------------------------------------------|
| `vibe-vault` | `crates/vibe-ui` (package `vibe-vault`) | GTK 4 / libadwaita vault and signing UI       |
| `vibe-keys`  | `crates/vibe-core/src/bin/vibe-keys.rs` | headless key export (JSON over stdin/stdout)  |
| `vibe-mcp`   | `crates/vibe-core/src/bin/vibe-mcp.rs`  | stdio MCP server; write tools open the vault Confirm screen |

Also included: the vault translations (`po/vault`; installed builds load compiled catalogs
from the system locale dirs, a source checkout reads `po/vault` with
`VIBE_DEV=1`) and
the Anchor IDL files the vault uses to label program instructions
(`crates/vibe-core/idl`, compiled in).

The VIBE Wallet desk (the Python app, browser extension and GNOME extension) is
not part of this repository and is distributed separately under its own terms.

## Build

Tested on Ubuntu 24.04 with Rust 1.90.0. The 1.9.1-1 release binaries were built on Ubuntu 26.04 with Rust 1.95.0.

```sh
# Build dependencies (Debian/Ubuntu 24.04 or newer)
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev libudev-dev

# Rust toolchain (if you don't have one)
curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain 1.90.0

# Build all three binaries with the pinned dependency versions
cargo build --release --locked --workspace --bins

# Run the tests
cargo test --release --locked -p vibe-core
```

Outputs: `target/release/vibe-vault`, `target/release/vibe-keys`,
`target/release/vibe-mcp`. GTK 4.14+ and libadwaita 1.5+ are required
(`gtk4` feature `v4_14`, `libadwaita` feature `v1_5`).

Fedora: `sudo dnf install gcc pkgconf-pkg-config gtk4-devel libadwaita-devel systemd-devel`.
Arch: `sudo pacman -S base-devel gtk4 libadwaita`.

At runtime vibe-vault looks for its stylesheet and fonts in
`/usr/share/vibe-wallet` (or `$VIBE_DATA_DIR`); without them it still runs with
the default Adwaita look.

## Verifying a release binary

Each VIBE Wallet release names the tag of this repository it was built from
(for 1.9.1: `v1.9.1-1`). Release checksums (`SHA256SUMS`) are signed with the
Old Growth Crypto release signing key, fingerprint
`58B0 78BE 5C1D B227 8043  67A7 28EB 45CC 198C 2D3D`, which is attached to each
[VIBE Wallet release](https://github.com/vibewalletsol/VibeWallet/releases) as
`vibewallet-release-signing-key.asc`. Verify with
`gpg --verify SHA256SUMS.asc SHA256SUMS` and then `sha256sum -c SHA256SUMS`.

## License

Copyright (c) 2026 Old Growth Crypto.

vibe-vault is free software: you can redistribute it and/or modify it under the
terms of the GNU General Public License, version 3 only (`GPL-3.0-only`). See
[COPYING](COPYING).

vibe-vault is derived from [Cinder](https://github.com/gregnb/cinder),
Copyright (c) 2026 Gregory Nowakowski, licensed under the MIT License. The
unmodified Cinder notice is in [LICENSES/MIT-Cinder.txt](LICENSES/MIT-Cinder.txt).
Licenses of the Rust crates compiled into the binaries are in
[THIRD_PARTY_NOTICES](THIRD_PARTY_NOTICES).

## Security

Please report vulnerabilities privately to vibewalletsol@gmail.com (see
[SECURITY.md](SECURITY.md)).
Never send seed phrases or private keys to anyone, including us.
