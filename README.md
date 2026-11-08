# tofu

[English](README.md) · [Русский](README.ru.md)

A native, open source proxy client in development. Linux first (CachyOS/Arch,
KDE Plasma, Wayland), Android later. Planned features include Xray-core and
sing-box, TUN, Fake DNS, subscriptions, per-application routing and a system tray.

**Early development:** this repository contains specifications, an initial portable
Rust libraries, an offline import CLI and isolated engine tests. There is no usable desktop
application, installer or production network adapter yet.

### Implemented

- Canonical IPv4/IPv6 CIDR validation and matching.
- Ordered routing decisions: protected private VPN networks, discovered LAN,
  user rules, presets, default action.
- Executable and destination conditions: AND between groups, OR within each group.
- Private-network blocking when its VPN route becomes unavailable.
- Synthetic Xray/sing-box TUN lifecycle and TCP/UDP IPv4/IPv6 echo tests in isolated Linux namespaces.
- Native executable-path routing checks, including two clients with the same basename.
- Portable session reducer with stale-event rejection and explicit cleanup confirmation.
- Single-use PreparedPlan guard bound to environment generations and the checked config digest.
- Bounded URI/Base64 import for VLESS, VMess, Trojan, Shadowsocks, HTTP CONNECT and SOCKS5.

The Rust library calculates policy; the isolated suites provide the specific
network evidence described below. Domain
matching, DNS/Fake-IP, network subscription updates, storage, engine configuration compilers, helper,
GUI and tray remain to be implemented. GUI toolkit selection is still open.

`tofu-application` currently implements the initial connect/check/stop loop.
Its host-provided controller ID must be unique while callbacks from an older
controller can still arrive. It does not perform I/O, recovery, reconfiguration
or persistence, and its state is not independent proof of network health.
Preparation now goes through `prepare/apply`; direct `Event::Validated` is removed.
The trusted host supplies fresh generations and a SHA-256 digest of the immutable
checked artifact. This does not replace OS authorization or atomic host-side Apply.

### Development

Rust 1.85+ (edition 2024). Domain/application remain std-only; the importer uses
Base64, URL/IDNA and JSON libraries. Cargo.lock pins MSRV-compatible dependencies.
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
private VPN integration and broader process attribution still require verification.

Executable attribution has a separate baseline suite:

```sh
tofu_apps=/var/tmp/codex-myrs/transient/tofu-apps
mkdir -p "$tofu_apps"
flock -s "$tofu_apps" python tools/engine_apps.py xray /absolute/path/to/xray --work-dir "$tofu_apps"
flock -s "$tofu_apps" python tools/engine_apps.py sing-box /absolute/path/to/sing-box --work-dir "$tofu_apps"
```

This verifies exact executable paths for sequential same-UID TCP/UDP IPv4/IPv6
flows, not desktop application coverage or the complete G-APP gate.
[Process routing evidence](docs/validation/2026-11-09-process-routing.md).

Never commit subscription links, credentials or real server configurations.
`local/` is ignored for private development files.

### Offline import diagnostic

```sh
flock -s "$tofu_build" env CARGO_TARGET_DIR="$tofu_build/target" cargo run --quiet --locked -p tofu-import < local/subscription.txt
```

Input is a subscription body, not a download URL. The CLI only prints counts,
protocols and static issue codes/line numbers; exit 0 means an analysis report
was produced, including partial/rejected records. It never activates a profile.
Limits: 8 MiB input, 32 MiB decoded, 20,000 URI records, 64 KiB per URI, JSON depth 64.
Native JSON is retained opaque after syntax validation; managed outbound extraction
and engine capability/config-check remain pending. Fatal errors preserve input
bytes in the library API; an oversized/broken stream is explicitly marked incomplete.
See [import evidence and limitations](docs/validation/2026-11-09-subscription-import.md).

### Documentation and license

- [Requirements, architecture and roadmap (Russian)](docs/specs/README.md)
- [Initial implementation plan (Russian)](docs/superpowers/plans/2026-11-09-linux-foundation.md)
- [GNU GPL v3.0](LICENSE); SPDX variant and dependency notices remain a release prerequisite.
