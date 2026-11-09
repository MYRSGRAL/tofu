"""Нативные доказательства policy routing; только синтетика в user/net/mount namespace.

Запускать под flock корня transient-каталога, задав TMPDIR внутри этого корня.
Аргументы: yonik-compile yonik-local xray sing-box. Rust этот инструмент не собирает.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import logging
import os
import socket
import socketserver
import struct
import subprocess
import sys
import tempfile
import threading
from collections.abc import Iterator
from contextlib import ExitStack, contextmanager
from pathlib import Path
from typing import Any

from engine_compile import UUID, socks_reply, stop, wait_listener
from engine_host import Events, arguments, environment, https_server, once, peer
from engine_smoke import install_signal_handlers, links, run_sandbox
from traffic_echo import receive_exact

LOG = logging.getLogger("yonik.policy")
PROFILE = f"vless://{UUID}@127.0.0.1:24443".encode()
DIGESTS = {
    "xray": "cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1",
    "sing-box": "1264361b5f8868c8768f7555d3e77e20118bf503c41ac08bb290a0d99056aa3a",
}
Policy = dict[str, Any]


def write_json(path: Path, value: Policy) -> None:
    # umask установлен до создания: секрет не бывает доступен между write/chmod.
    path.write_text(json.dumps(value, ensure_ascii=False), encoding="utf-8")
    path.chmod(0o600)


def rule(number: int, action: str, **conditions: list[str]) -> Policy:
    return {"id": number, "action": action, "conditions": conditions}


def names_policy() -> Policy:
    return {
        "version": 1,
        "default_action": "block",
        "user_rules": [
            rule(1, "direct", domain=["direct.test", "first.test"]),
            rule(2, "block", domain=["first.test", "block.test"]),
            rule(3, "proxy", domain=["proxy.test", "bücher.test", "private.test"]),
            rule(4, "proxy", domain_suffix=["suffix.test"]),
            {**rule(5, "proxy", domain=["disabled.test"]), "enabled": False},
        ],
        "preset_rules": [
            rule(1, "block", domain=["first.test"]),
            rule(2, "proxy", domain=["preset.test"]),
        ],
        "private_domains": [{"suffix": "private.test", "vpn": 1, "available": False}],
    }


def ips_policy() -> Policy:
    def cidrs(last: int) -> list[str]:
        return [f"198.51.100.{last}/32", f"2001:db8::{last}/128"]

    return {
        "version": 1,
        "default_action": "proxy",
        "private_networks": [
            {"cidr": value, "vpn": 1, "available": False} for value in cidrs(10)
        ],
        "lan": cidrs(10) + cidrs(15),
        "user_rules": [
            rule(1, "direct", ip_cidr=cidrs(11)),
            rule(2, "block", ip_cidr=cidrs(12)),
            rule(3, "proxy", ip_cidr=cidrs(13)),
            rule(4, "block", ip_cidr=cidrs(13) + cidrs(15)),
            rule(5, "direct", ip_cidr=cidrs(14)),
            {**rule(6, "block", ip_cidr=cidrs(16)), "enabled": False},
        ],
        "preset_rules": [rule(1, "block", ip_cidr=cidrs(14))],
    }


class MarkerTcp(socketserver.BaseRequestHandler):
    def handle(self) -> None:
        self.request.settimeout(2)
        try:
            payload = receive_exact(self.request, 32)
            self.request.sendall(self.server.marker + payload)
        except (OSError, ConnectionError):
            LOG.debug("Тестовое соединение закрыто до echo")


class MarkerUdp(socketserver.BaseRequestHandler):
    def handle(self) -> None:
        payload, connection = self.request
        if len(payload) == 32:
            connection.sendto(self.server.marker + payload, self.client_address)


class Tcp6(socketserver.TCPServer):
    address_family = socket.AF_INET6

    def server_bind(self) -> None:
        self.socket.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 1)
        super().server_bind()


class Udp6(socketserver.UDPServer):
    address_family = socket.AF_INET6

    def server_bind(self) -> None:
        self.socket.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 1)
        super().server_bind()


@contextmanager
def markers() -> Iterator[None]:
    with ExitStack() as stack:
        for address, port, marker, tcp, udp in [
            ("0.0.0.0", 24081, b"D", socketserver.TCPServer, socketserver.UDPServer),
            ("::", 24081, b"D", Tcp6, Udp6),
            ("127.0.0.1", 24082, b"P", socketserver.TCPServer, socketserver.UDPServer),
        ]:
            for server_type, handler in [(tcp, MarkerTcp), (udp, MarkerUdp)]:
                server = stack.enter_context(server_type((address, port), handler))
                server.marker = marker
                thread = threading.Thread(
                    target=server.serve_forever, kwargs={"poll_interval": 0.05}
                )
                thread.start()
                stack.callback(thread.join, 3)
                stack.callback(server.shutdown)
        yield


def destination(address: str) -> bytes:
    try:
        packed = socket.inet_pton(socket.AF_INET, address)
        return b"\x01" + packed + struct.pack("!H", 24081)
    except OSError:
        pass
    try:
        packed = socket.inet_pton(socket.AF_INET6, address)
        return b"\x04" + packed + struct.pack("!H", 24081)
    except OSError:
        encoded = address.encode("utf-8")
        if not 0 < len(encoded) <= 255:
            raise ValueError("Недопустимая длина тестового имени") from None
        return b"\x03" + bytes([len(encoded)]) + encoded + struct.pack("!H", 24081)


def probe(
    address: str,
    udp: bool = False,
    port: int = 10808,
    credentials: tuple[str, str] | None = None,
) -> bytes:
    target = destination(address)
    payload = os.urandom(32)
    with socket.create_connection(("127.0.0.1", port), timeout=1) as control:
        control.settimeout(0.6)
        control.sendall(b"\x05\x01" + (b"\x02" if credentials else b"\x00"))
        if receive_exact(control, 2) != (b"\x05\x02" if credentials else b"\x05\x00"):
            raise ConnectionError("SOCKS отклонил метод аутентификации")
        if credentials:
            username, password = [value.encode() for value in credentials]
            control.sendall(
                b"\x01"
                + bytes([len(username)])
                + username
                + bytes([len(password)])
                + password
            )
            if receive_exact(control, 2) != b"\x01\x00":
                raise ConnectionError("SOCKS отклонил аутентификацию")
        control.sendall(
            bytes([5, 3 if udp else 1, 0])
            + (b"\x01\x00\x00\x00\x00\x00\x00" if udp else target)
        )
        relay = socks_reply(control)
        if udp:
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as datagram:
                datagram.settimeout(0.6)
                datagram.sendto(b"\x00\x00\x00" + target + payload, relay)
                reply, _ = datagram.recvfrom(4096)
                if reply[:3] != b"\x00\x00\x00":
                    raise RuntimeError("Неверный заголовок SOCKS UDP")
                if reply[3] in (1, 4):
                    offset = 10 if reply[3] == 1 else 22
                elif reply[3] == 3:
                    offset = 7 + reply[4]
                else:
                    raise RuntimeError("Неверный тип адреса SOCKS UDP")
                response = reply[offset:]
        else:
            control.sendall(payload)
            response = receive_exact(control, 33)
    if (
        len(response) != 33
        or response[1:] != payload
        or response[:1] not in (b"D", b"P")
    ):
        raise RuntimeError("Echo не подтвердил одноразовую нагрузку и маршрут")
    return response[:1]


class Checks:
    def __init__(self, engine: str) -> None:
        self.engine = engine
        self.count = 0

    def route(
        self, label: str, address: str, expected: bytes | None, **kwargs: Any
    ) -> None:
        try:
            actual = probe(address, **kwargs)
        except (OSError, ConnectionError):
            actual = None
        if actual != expected:
            raise RuntimeError(
                f"{self.engine}: {label}: ожидался {expected!r}, получен {actual!r}"
            )
        self.count += 1
        LOG.info("%s PASS %s", self.engine, label)


@contextmanager
def core(binary: Path, config: Path, port: int) -> Iterator[subprocess.Popen[bytes]]:
    with subprocess.Popen(
        [str(binary), "run", "-c", str(config)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    ) as process:
        try:
            wait_listener(process, port)
            yield process
        finally:
            stop(process)


def marker_peer(engine: str) -> Policy:
    if engine == "xray":
        return {
            "log": {"loglevel": "none"},
            "inbounds": [
                {
                    "listen": "127.0.0.1",
                    "port": 24443,
                    "protocol": "vless",
                    "settings": {"decryption": "none", "clients": [{"id": UUID}]},
                }
            ],
            "outbounds": [
                {
                    "protocol": "freedom",
                    "settings": {
                        "redirect": "127.0.0.1:24082",
                        "finalRules": [
                            {"action": "allow", "ip": ["127.0.0.1/32"], "port": "24082"}
                        ],
                    },
                }
            ],
        }
    return {
        "log": {"disabled": True},
        "inbounds": [
            {
                "type": "vless",
                "listen": "127.0.0.1",
                "listen_port": 24443,
                "users": [{"uuid": UUID}],
            }
        ],
        "outbounds": [{"type": "direct", "tag": "echo"}],
        "route": {
            "rules": [
                {
                    "action": "route",
                    "outbound": "echo",
                    "override_address": "127.0.0.1",
                    "override_port": 24082,
                }
            ]
        },
    }


def policy_flags(path: Path, mode: str) -> list[str]:
    return ["--policy", str(path), "--policy-mode", mode, "--probe-port", "10809"]


def compile_config(
    compiler: Path, engine: str, workspace: Path, policy: Policy, mode: str
) -> tuple[Path, tuple[str, str]]:
    policy_path = workspace / f"{engine}-policy.json"
    write_json(policy_path, policy)
    result = subprocess.run(
        [
            str(compiler),
            engine,
            *policy_flags(policy_path, mode),
            "--emit-secret-config",
        ],
        input=PROFILE,
        capture_output=True,
        timeout=5,
    )
    if result.returncode:
        raise RuntimeError(
            f"Компилятор отклонил синтетическую политику: {result.stderr.decode(errors='replace')}"
        )
    config = json.loads(result.stdout)
    inbound = next(value for value in config["inbounds"] if value.get("tag") == "probe")
    if engine == "xray":
        user = inbound["settings"]["accounts"][0]
        credentials = user["user"], user["pass"]
    else:
        user = inbound["users"][0]
        credentials = user["username"], user["password"]
    if credentials[0] != "yonik-health" or len(credentials[1]) != 64:
        raise RuntimeError("Нет отдельной случайной авторизации health SOCKS")
    config_path = workspace / f"{engine}-client.json"
    write_json(config_path, config)
    return config_path, credentials


def capability_checks(compiler: Path, engine: str, workspace: Path) -> int:
    fixtures = [
        (
            "names-cidr",
            "names",
            {"user_rules": [rule(1, "block", ip_cidr=["198.51.100.10/32"])]},
        ),
        ("names-lan", "names", {"lan": ["198.51.100.10/32"]}),
        (
            "names-private-ip",
            "names",
            {
                "private_networks": [
                    {"cidr": "198.51.100.10/32", "vpn": 1, "available": False}
                ]
            },
        ),
        (
            "mixed-resolution",
            "names",
            {
                "user_rules": [
                    rule(1, "block", ip_cidr=["198.51.100.10/32"]),
                    rule(2, "direct", domain=["synthetic-private.test"]),
                ]
            },
        ),
        (
            "source-path",
            "names",
            {
                "user_rules": [
                    rule(
                        1, "direct", executable_paths=["/synthetic-private/application"]
                    )
                ]
            },
        ),
        (
            "available-vpn",
            "ip-only",
            {
                "private_networks": [
                    {"cidr": "198.51.100.10/32", "vpn": 1, "available": True}
                ]
            },
        ),
        (
            "available-domain",
            "names",
            {
                "private_domains": [
                    {"suffix": "synthetic-private.test", "vpn": 1, "available": True}
                ]
            },
        ),
        (
            "ip-only-domain",
            "ip-only",
            {"user_rules": [rule(1, "direct", domain=["synthetic-private.test"])]},
        ),
    ]
    for label, mode, policy in fixtures:
        path = workspace / "unsupported.json"
        write_json(path, {"version": 1, **policy})
        result = subprocess.run(
            [str(compiler), engine, *policy_flags(path, mode), "--emit-secret-config"],
            input=PROFILE,
            capture_output=True,
            timeout=5,
        )
        if (
            result.returncode == 0
            or result.stdout
            or b"synthetic-private" in result.stderr
        ):
            raise RuntimeError(
                f"{engine}: capability-error {label} не подтверждён или раскрывает ввод"
            )
        LOG.info("%s PASS capability-%s", engine, label)
    return len(fixtures)


def routing_checks(args: argparse.Namespace, engine: str, binary: Path) -> int:
    checks = Checks(engine)
    workspace = args.workspace
    peer_path = workspace / f"{engine}-marker-peer.json"
    write_json(peer_path, marker_peer(engine))
    with core(binary, peer_path, 24443) as server:
        config, auth = compile_config(
            args.compiler, engine, workspace, names_policy(), "names"
        )
        with core(binary, config, 10808):
            for address, expected in [
                ("direct.test", b"D"),
                ("first.test", b"D"),
                ("proxy.test", b"P"),
                ("PROXY.TEST", b"P"),
                ("xn--bcher-kva.test", b"P"),
                ("suffix.test", b"P"),
                ("child.suffix.test", b"P"),
                ("notsuffix.test", None),
                ("child.proxy.test", None),
                ("block.test", None),
                ("disabled.test", None),
                ("preset.test", b"P"),
                ("private.test", None),
                ("child.private.test", None),
                ("default.test", None),
            ]:
                for udp in (False, True):
                    checks.route(
                        f"names-{address}-{'udp' if udp else 'tcp'}",
                        address,
                        expected,
                        udp=udp,
                    )
            checks.route(
                "health-bypass-default-block",
                "198.51.100.12",
                b"P",
                port=10809,
                credentials=auth,
            )
            checks.route("health-reject-anonymous", "198.51.100.12", None, port=10809)
            checks.route(
                "health-reject-wrong-password",
                "198.51.100.12",
                None,
                port=10809,
                credentials=(auth[0], "wrong"),
            )
        config, next_auth = compile_config(
            args.compiler,
            engine,
            workspace,
            {"version": 1, "default_action": "proxy"},
            "names",
        )
        if next_auth == auth:
            raise RuntimeError("Авторизация health повторилась между компиляциями")
        with core(binary, config, 10808):
            checks.route("guard-control-valid-proxy", "valid.test", b"P")
            for address in [
                "bücher.test",
                "proxy.test.",
                "-bad.test",
                "a..test",
                "bad_name.test",
            ]:
                for udp in (False, True):
                    checks.route(
                        f"invalid-wire-{address}-{'udp' if udp else 'tcp'}",
                        address,
                        None,
                        udp=udp,
                    )
        config, _ = compile_config(
            args.compiler, engine, workspace, ips_policy(), "ip-only"
        )
        with core(binary, config, 10808):
            for family in ("198.51.100.", "2001:db8::"):
                for suffix, expected in [
                    (10, None),
                    (11, b"D"),
                    (12, None),
                    (13, b"P"),
                    (14, b"D"),
                    (15, b"D"),
                    (16, b"P"),
                ]:
                    address = f"{family}{suffix}"
                    for udp in (False, True):
                        checks.route(
                            f"ip-{address}-{'udp' if udp else 'tcp'}",
                            address,
                            expected,
                            udp=udp,
                        )
            checks.route("ip-default", "203.0.113.17", b"P")
            for udp in (False, True):
                checks.route(
                    f"mapped-protected-{'udp' if udp else 'tcp'}",
                    "::ffff:198.51.100.10",
                    None,
                    udp=udp,
                )
                checks.route(
                    f"ip-only-denies-name-{'udp' if udp else 'tcp'}",
                    "valid.test",
                    None,
                    udp=udp,
                )
            stop(server)
            for udp in (False, True):
                checks.route(
                    f"dead-proxy-no-fallback-{'udp' if udp else 'tcp'}",
                    "198.51.100.13",
                    None,
                    udp=udp,
                )
                checks.route(
                    f"dead-proxy-direct-alive-{'udp' if udp else 'tcp'}",
                    "198.51.100.11",
                    b"D",
                    udp=udp,
                )
    return checks.count + capability_checks(args.compiler, engine, workspace)


def host_checks(args: argparse.Namespace, engine: str, binary: Path) -> int:
    path = args.workspace / f"{engine}-host-policy.json"
    env = environment(args.workspace)
    write_json(path, {"version": 1, "default_action": "block"})
    command = arguments(args.host, engine, binary) + policy_flags(path, "ip-only")
    with peer(binary, engine, args.workspace):
        once(command, env, "Connected", 0, 204)
        live_command = arguments(args.host, engine, binary, once=False) + policy_flags(
            path, "ip-only"
        )
        with subprocess.Popen(
            live_command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
        ) as host:
            try:
                if host.stdin is None:
                    raise RuntimeError("Нет stdin host")
                host.stdin.write(PROFILE)
                host.stdin.close()
                host.stdin = None
                reader = Events(host)
                checking = reader.until("Checking")
                event = reader.until("Connected")
                if event.get("probe", {}).get("status") != 204:
                    raise RuntimeError("Живой host не подтвердил HTTPS204")
                checks = Checks(engine)
                checks.route("host-default-block-user-data", "127.0.0.1", None)
                host.terminate()
                stdout, _ = host.communicate(timeout=5)
                states = [json.loads(line)["state"] for line in stdout.splitlines()]
                if host.returncode or "Disconnected" not in states:
                    raise RuntimeError("Живой host не подтвердил cleanup")
                if checking.get("pid") and Path(f"/proc/{checking['pid']}").exists():
                    raise RuntimeError("После host остался процесс ядра")
            finally:
                stop(host)
    LOG.info("%s PASS host-default-block-health-connected-204", engine)
    write_json(path, {"version": 1, "default_action": "direct"})
    args.https.requested.clear()
    once(command, env, "Limited", 2)
    if args.https.requested.is_set():
        raise RuntimeError("Health выполнил direct fallback после остановки peer")
    LOG.info("%s PASS host-default-direct-dead-proxy-limited-no-direct-probe", engine)
    write_json(
        path,
        {
            "version": 1,
            "private_networks": [{"cidr": "127.0.0.0/8", "vpn": 1, "available": False}],
        },
    )
    args.https.requested.clear()
    result = subprocess.run(
        command, input=PROFILE, env=env, capture_output=True, timeout=10
    )
    events = [json.loads(line) for line in result.stdout.splitlines()]
    if (
        result.returncode != 1
        or any(event.get("state") == "Connected" for event in events)
        or args.https.requested.is_set()
    ):
        raise RuntimeError("Health обошёл известную защищённую сеть")
    for event in events:
        if event.get("pid") and Path(f"/proc/{event['pid']}").exists():
            raise RuntimeError("После запрета защищённого probe остался процесс")
    LOG.info("%s PASS protected-health-endpoint-rejected", engine)
    return 4


def worker(args: argparse.Namespace) -> None:
    if os.stat("/proc/self/ns/net").st_ino == args.worker or links() != {"lo"}:
        raise RuntimeError("Требуется новый пустой network namespace")
    subprocess.run(
        ["ip", "link", "set", "lo", "up"], check=True, capture_output=True, timeout=5
    )
    for address in [
        *[f"198.51.100.{n}/32" for n in range(10, 17)],
        "203.0.113.17/32",
        *[f"2001:db8::{n}/128" for n in range(10, 17)],
    ]:
        subprocess.run(
            [
                "ip",
                "addr",
                "add",
                address,
                "dev",
                "lo",
                *(["nodad"] if ":" in address else []),
            ],
            check=True,
            capture_output=True,
            timeout=5,
        )
    hosts = args.workspace / "hosts"
    hosts.write_text(
        "127.0.0.1 localhost direct.test first.test valid.test\n::1 localhost\n"
    )
    subprocess.run(
        ["mount", "--bind", str(hosts), "/etc/hosts"],
        check=True,
        capture_output=True,
        timeout=5,
    )
    (args.workspace / "empty-roots").mkdir()
    with markers(), https_server(args.workspace, "good", 24444, "127.0.0.1") as https:
        args.https = https
        (args.workspace / "roots.pem").write_bytes(
            (args.workspace / "good.ca.pem").read_bytes()
        )
        for engine, binary in [("xray", args.xray), ("sing-box", args.sing_box)]:
            total = routing_checks(args, engine, binary) + host_checks(
                args, engine, binary
            )
            LOG.info("%s TOTAL %d проверок пройдено", engine, total)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("compiler", "host", "xray", "sing_box"):
        parser.add_argument(name, type=Path)
    parser.add_argument("--worker", type=int, help=argparse.SUPPRESS)
    parser.add_argument("--workspace", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.worker is not None:
        worker(args)
        return 0
    os.umask(0o077)
    for engine, binary in [("xray", args.xray), ("sing-box", args.sing_box)]:
        if hashlib.sha256(binary.read_bytes()).hexdigest() != DIGESTS[engine]:
            raise RuntimeError(f"Не совпадает SHA256 тестового ядра {engine}")
    with tempfile.TemporaryDirectory(prefix="yonik-policy-") as directory:
        run_sandbox(
            [
                "unshare",
                "--user",
                "--map-root-user",
                "--net",
                "--pid",
                "--fork",
                "--mount-proc",
                "--kill-child",
                "--mount",
                sys.executable,
                str(Path(__file__).resolve()),
                *[
                    str(getattr(args, name).resolve(strict=True))
                    for name in ("compiler", "host", "xray", "sing_box")
                ],
                "--worker",
                str(os.stat("/proc/self/ns/net").st_ino),
                "--workspace",
                directory,
            ],
            timeout=180,
        )
    return 0


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    install_signal_handlers()
    try:
        raise SystemExit(main())
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        LOG.error("Проверка policy не пройдена: %s", error)
        raise SystemExit(1) from None
