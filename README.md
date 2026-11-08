# tofu

[English](README.md) · [Русский](README.ru.md)

A native, open source proxy client in development. Linux first (CachyOS/Arch,
KDE Plasma, Wayland), Android later. Planned features include Xray-core and
sing-box, TUN, Fake DNS, subscriptions, per-application routing and a system tray.

**Early development:** this repository contains specifications, an initial portable
Rust routing library and isolated engine smoke tests. There is no usable desktop
application, installer or production network adapter yet.

### Implemented

- Canonical IPv4/IPv6 CIDR validation and matching.
- Ordered routing decisions: protected private VPN networks, discovered LAN,
  user rules, presets, default action.
- Executable and destination conditions: AND between groups, OR within each group.
- Private-network blocking when its VPN route becomes unavailable.
- Synthetic Xray/sing-box TUN lifecycle and TCP/UDP IPv4/IPv6 echo tests in isolated Linux namespaces.

These are policy calculations, not evidence of actual packet routing. Domain
matching, DNS/Fake-IP, import, storage, engine configuration compilers, helper,
GUI and tray remain to be implemented. GUI toolkit selection is still open.

### Development

Rust 1.85+ (edition 2024); no third-party Rust dependencies in the initial library.
Build artifacts belong outside the checkout. On the project development host:

```sh
tofu_build=/var/tmp/codex-myrs/transient/tofu-dev
mkdir -p "$tofu_build"
cargo fmt --all -- --check
flock -s "$tofu_build" env CARGO_TARGET_DIR="$tofu_build/target" cargo test --workspace --locked
flock -s "$tofu_build" env CARGO_TARGET_DIR="$tofu_build/target" cargo clippy --workspace --all-targets --locked -- -D warnings
```

Engine smoke tests require Linux, Python 3.12+, `ip`, `unshare`, `/dev/net/tun`,
unprivileged user/network namespaces and independently verified engine binaries.
They never configure the host network or system DNS:

```sh
python tools/engine_smoke.py xray /absolute/path/to/xray
python tools/engine_smoke.py sing-box /absolute/path/to/sing-box
python tools/engine_traffic.py xray /absolute/path/to/xray
python tools/engine_traffic.py sing-box /absolute/path/to/sing-box
```

If binaries are in a transient task directory, hold `flock -s` on its root for
the entire test. See [recorded checks and pinned versions](docs/validation/2026-11-09-foundation.md).
The smoke suite covers config validation and interface lifecycle. The traffic
suite sends random payloads through TUN to redirected local echo endpoints and
checks interface counters and a negative control after shutdown.
[Traffic evidence](docs/validation/2026-11-09-tun-traffic.md).
Neither suite completes the G-LINUX/G-DNS/G-APP gates: proxy outbounds, DNS,
private VPN integration and process attribution still require verification.

Never commit subscription links, credentials or real server configurations.
`local/` is ignored for private development files.

### Documentation and license

- [Requirements, architecture and roadmap (Russian)](docs/specs/README.md)
- [Initial implementation plan (Russian)](docs/superpowers/plans/2026-11-09-linux-foundation.md)
- [GNU GPL v3.0](LICENSE); SPDX variant and dependency notices remain a release prerequisite.
