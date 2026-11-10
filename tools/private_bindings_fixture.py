"""Настоящий WireGuard только внутри private namespace старой traffic fixture."""

from __future__ import annotations

import subprocess
import sys

from public_block_fixture import PEER, guard_namespace, run_ip, setup


def wg(arguments: list[str], data: bytes | None = None) -> bytes:
    result = subprocess.run(
        arguments, input=data, capture_output=True, check=True, timeout=5
    )
    return result.stdout


def connect() -> None:
    guard_namespace()
    for ns in ([], ["-n", PEER]):
        run_ip(*ns, "link", "add", "vpn", "type", "wireguard")
    # Приватные ключи остаются в памяти и stdin; argv и evidence их не содержат.
    keys = [wg(["wg", "genkey"]).strip() for _ in range(2)]
    pubs = [wg(["wg", "pubkey"], key + b"\n").strip() for key in keys]
    for i, ns in enumerate(([], ["netns", "exec", PEER])):
        endpoint = "198.18.1.2" if i == 0 else "198.18.1.1"
        config = (
            b"[Interface]\nPrivateKey = "
            + keys[i]
            + b"\nListenPort = 24100\nFwMark = 11\n[Peer]\nPublicKey = "
            + pubs[1 - i]
            + b"\nAllowedIPs = 10.42.1.0/24, fd42:1::/64\nEndpoint = "
            + endpoint.encode()
            + b":24100\n"
        )
        command = (["ip", *ns] if ns else []) + ["wg", "setconf", "vpn", "/dev/stdin"]
        wg(command, config)
        ip_ns = ["-n", PEER] if i else []
        run_ip(*ip_ns, "link", "set", "vpn", "up")
        for addr in (f"10.42.1.{i + 1}/24", f"fd42:1::{i + 1}/64"):
            run_ip(*ip_ns, "address", "add", addr, "dev", "vpn", "nodad")


def main() -> None:
    guard_namespace()
    action = sys.argv[1] if len(sys.argv) == 2 else "setup"
    if action in ("setup", "underlay"):
        setup()
        run_ip("link", "del", "vpn")
    if action in ("setup", "connect"):
        connect()
    elif action == "disconnect":
        run_ip("link", "del", "vpn")
        run_ip("-n", PEER, "link", "del", "vpn")
    elif action != "underlay":
        raise ValueError("Неизвестное действие fixture")


if __name__ == "__main__":
    main()
