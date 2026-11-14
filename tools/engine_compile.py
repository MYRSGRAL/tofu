"""Синтетические config-check и SOCKS→VLESS→echo в отдельном network namespace."""

from __future__ import annotations

import argparse
import json
import logging
import os
import socket
import struct
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from engine_smoke import install_signal_handlers, links, run_sandbox
from traffic_echo import PAYLOAD_SIZE, RESPONSE_PREFIX, echo_servers, receive_exact

LOG = logging.getLogger("yonik.compile")
UUID = "00112233-4455-6677-8899-aabbccddeeff"


def generated(compiler: Path, engine: str, uri: str) -> bytes:
    return subprocess.run(
        [str(compiler), engine, "--emit-secret-config"],
        input=uri.encode(),
        check=True,
        capture_output=True,
        timeout=5,
    ).stdout


def wait_listener(process: subprocess.Popen[bytes], port: int) -> None:
    deadline = time.monotonic() + 5
    while process.poll() is None:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.1):
                return
        except OSError:
            if time.monotonic() >= deadline:
                break
            time.sleep(0.05)
    raise RuntimeError("Ядро не открыло тестовый listener")


def socks_reply(control: socket.socket) -> tuple[str, int]:
    version, status, reserved, atyp = receive_exact(control, 4)
    if (version, status, reserved) != (5, 0, 0):
        raise ConnectionError("SOCKS отклонил запрос")
    if atyp == 1:
        address = socket.inet_ntop(socket.AF_INET, receive_exact(control, 4))
    elif atyp == 4:
        address = socket.inet_ntop(socket.AF_INET6, receive_exact(control, 16))
    else:
        raise RuntimeError("Неожиданный тип адреса SOCKS relay")
    return address, struct.unpack("!H", receive_exact(control, 2))[0]


def probe(address: str, udp: bool, timeout: float = 1) -> None:
    family = socket.AF_INET6 if ":" in address else socket.AF_INET
    destination = bytes([4 if family == socket.AF_INET6 else 1])
    destination += socket.inet_pton(family, address) + struct.pack("!H", 24081)
    payload = os.urandom(PAYLOAD_SIZE)
    expected = RESPONSE_PREFIX + payload
    with socket.create_connection(("127.0.0.1", 10808), timeout=1) as control:
        control.settimeout(timeout)
        control.sendall(b"\x05\x01\x00")
        if receive_exact(control, 2) != b"\x05\x00":
            raise RuntimeError("SOCKS greeting отклонён")
        command = 3 if udp else 1
        target = b"\x01\x00\x00\x00\x00\x00\x00" if udp else destination
        control.sendall(bytes([5, command, 0]) + target)
        relay = socks_reply(control)
        if udp:
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as datagram:
                datagram.settimeout(1)
                datagram.sendto(b"\x00\x00\x00" + destination + payload, relay)
                reply, _ = datagram.recvfrom(4096)
                if reply[:3] != b"\x00\x00\x00" or reply[3:4] not in (b"\x01", b"\x04"):
                    raise RuntimeError("Некорректный SOCKS UDP ответ")
                offset = 10 if reply[3] == 1 else 22
                response = reply[offset:]
        else:
            control.sendall(payload)
            response = receive_exact(control, len(expected))
        if response != expected:
            raise RuntimeError("Echo не совпадает с нагрузкой")


def stop(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=3)


def check_matrix(compiler: Path, binary: Path, engine: str, workspace: Path) -> None:
    import base64

    vmess = {
        "v": "2",
        "add": "127.0.0.1",
        "port": "24443",
        "id": UUID,
        "aid": "0",
        "scy": "auto",
        "net": "tcp",
        "tls": "",
        "host": "",
        "path": "",
    }
    sources = [
        f"vless://{UUID}@127.0.0.1:24443",
        "vmess://" + base64.b64encode(json.dumps(vmess).encode()).decode(),
        "trojan://synthetic-password@127.0.0.1:24443?sni=test.example",
        "ss://aes-128-gcm:synthetic-password@127.0.0.1:24443",
        "https://user:synthetic-password@127.0.0.1:24443?sni=test.example",
        "socks5://user:synthetic-password@127.0.0.1:24443",
        f"vless://{UUID}@127.0.0.1:24443?security=tls&type=ws&host=test.example&path=%2Fws&fp=chrome&alpn=http%2F1.1",
        f"vless://{UUID}@127.0.0.1:24443?security=tls&type=grpc&serviceName=test&fp=chrome",
        f"vless://{UUID}@127.0.0.1:24443?security=reality&sni=test.example&fp=chrome&pbk=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA&sid=abcd&flow=xtls-rprx-vision",
    ]
    if engine == "xray":
        sources.append(
            f"vless://{UUID}@127.0.0.1:24443?security=tls&type=xhttp&path=%2Ftest&mode=auto"
        )
    for index, source in enumerate(sources):
        config = workspace / f"{engine}-check.json"
        config.write_bytes(generated(compiler, engine, source))
        config.chmod(0o600)
        command = (
            [str(binary), "run", "-test", "-c", str(config)]
            if engine == "xray"
            else [str(binary), "check", "-c", str(config)]
        )
        result = subprocess.run(command, capture_output=True, timeout=5)
        if result.returncode:
            # Только синтетические данные; пользовательские профили инструмент не принимает.
            LOG.error(
                "config-check fixture %d: %s",
                index,
                result.stderr.decode(errors="replace"),
            )
            raise RuntimeError("config-check отклонил синтетическую конфигурацию")
    LOG.info(
        "%s: %d сгенерированных конфигураций прошли config-check", engine, len(sources)
    )


def check_ss2022(compiler: Path, binary: Path, engine: str, workspace: Path) -> None:
    """Focused SS2022: только синтетический config-check без запуска трафика."""
    import base64

    method = "2022-blake3-aes-128-gcm"
    key = base64.b64encode(b"0123456789abcdef").decode()
    source = f"ss://{method}:{key}@127.0.0.1:24443"
    data = generated(compiler, engine, source)
    outbound = json.loads(data)["outbounds"][0]
    credentials = outbound["settings"]["servers"][0] if engine == "xray" else outbound
    if credentials["method"] != method or credentials["password"] != key:
        raise RuntimeError("Компилятор изменил SS2022 method/password")
    config = workspace / f"{engine}-ss2022.json"
    config.write_bytes(data)
    config.chmod(0o600)
    command = (
        [str(binary), "run", "-test", "-c", str(config)]
        if engine == "xray"
        else [str(binary), "check", "-c", str(config)]
    )
    result = subprocess.run(command, capture_output=True, timeout=10)
    (workspace / f"{engine}-checker.log").write_bytes(result.stdout + result.stderr)
    (workspace / f"{engine}-exit.json").write_text(
        json.dumps({"argv": command, "exit": result.returncode}) + "\n"
    )
    result.check_returncode()
    negatives = [
        f"ss://{method}:{base64.b64encode(bytes(15)).decode()}@127.0.0.1:24443"
    ]
    if engine == "xray":
        negatives.append(
            "ss://2022-blake3-chacha20-poly1305:"
            + base64.b64encode(bytes(32)).decode()
            + "@127.0.0.1:24443"
        )
    negative_results = []
    for source in negatives:
        result = subprocess.run(
            [str(compiler), engine, "--emit-secret-config"],
            input=source.encode(),
            capture_output=True,
            timeout=5,
        )
        negative_results.append(
            {
                "uri": source,
                "exit": result.returncode,
                "stderr": result.stderr.decode(errors="replace"),
            }
        )
        if result.returncode == 0 or result.stdout:
            raise RuntimeError("Негативный SS2022 gate пропустил конфигурацию")
    (workspace / f"{engine}-negative.json").write_text(
        json.dumps(negative_results, indent=2) + "\n"
    )
    LOG.info(
        "%s: SS2022 exact method/key, checker exit=0, negative gates=%d",
        engine,
        len(negatives),
    )


def traffic(compiler: Path, binary: Path, engine: str, workspace: Path) -> None:
    if engine == "xray":
        server = {
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
                        "finalRules": [
                            {
                                "action": "allow",
                                "ip": ["127.0.0.0/8", "::1/128"],
                                "port": "24081",
                            }
                        ]
                    },
                }
            ],
        }
    else:
        server = {
            "log": {"disabled": True},
            "inbounds": [
                {
                    "type": "vless",
                    "listen": "127.0.0.1",
                    "listen_port": 24443,
                    "users": [{"uuid": UUID}],
                }
            ],
            "outbounds": [{"type": "direct"}],
        }
    server_path = workspace / f"{engine}-server.json"
    server_path.write_text(json.dumps(server))
    client_path = workspace / f"{engine}-client.json"
    client_path.write_bytes(
        generated(compiler, engine, f"vless://{UUID}@127.0.0.1:24443")
    )
    client_path.chmod(0o600)
    with subprocess.Popen(
        [str(binary), "run", "-c", str(server_path)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    ) as peer:
        try:
            wait_listener(peer, 24443)
            with subprocess.Popen(
                [str(binary), "run", "-c", str(client_path)],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            ) as client:
                try:
                    wait_listener(client, 10808)
                    for address in ("127.0.0.1", "::1"):
                        for udp in (False, True):
                            LOG.info(
                                "%s: проверка %s %s",
                                engine,
                                address,
                                "UDP" if udp else "TCP",
                            )
                            probe(address, udp)
                    stop(peer)
                    for address in ("127.0.0.1", "::1"):
                        for udp in (False, True):
                            try:
                                probe(address, udp)
                            except (OSError, ConnectionError):
                                continue
                            raise RuntimeError(
                                "Трафик прошёл после остановки proxy server"
                            )
                    LOG.info(
                        "%s: 4 TCP/UDP IPv4/IPv6 ответа; 4 отказа без proxy server",
                        engine,
                    )
                finally:
                    stop(client)
        finally:
            stop(peer)


def tls_traffic(compiler: Path, binary: Path, engine: str, workspace: Path) -> None:
    certificate = str(workspace / "certificate.pem")
    key = str(workspace / "key.pem")
    if engine == "xray":
        inbound = {
            "listen": "127.0.0.1",
            "port": 24443,
            "protocol": "vless",
            "settings": {"decryption": "none", "clients": [{"id": UUID}]},
            "streamSettings": {
                "security": "tls",
                "tlsSettings": {
                    "certificates": [{"certificateFile": certificate, "keyFile": key}]
                },
            },
        }
        server = {
            "log": {"loglevel": "none"},
            "inbounds": [inbound],
            "outbounds": [
                {
                    "protocol": "freedom",
                    "settings": {
                        "finalRules": [
                            {"action": "allow", "ip": ["127.0.0.1/32"], "port": "24081"}
                        ]
                    },
                }
            ],
        }
    else:
        server = {
            "log": {"disabled": True},
            "inbounds": [
                {
                    "type": "vless",
                    "listen": "127.0.0.1",
                    "listen_port": 24443,
                    "users": [{"uuid": UUID}],
                    "tls": {
                        "enabled": True,
                        "certificate_path": certificate,
                        "key_path": key,
                    },
                }
            ],
            "outbounds": [{"type": "direct"}],
        }
    server_path = workspace / f"{engine}-tls-server.json"
    server_path.write_text(json.dumps(server))
    with subprocess.Popen(
        [str(binary), "run", "-c", str(server_path)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    ) as peer:
        try:
            wait_listener(peer, 24443)
            for name, trusted, allowed in [
                ("yonik.test", True, True),
                ("wrong.test", True, False),
                ("yonik.test", False, False),
            ]:
                config = workspace / f"{engine}-tls-client.json"
                config.write_bytes(
                    generated(
                        compiler,
                        engine,
                        f"vless://{UUID}@127.0.0.1:24443?security=tls&sni={name}&fp=chrome",
                    )
                )
                config.chmod(0o600)
                environment = dict(os.environ)
                environment["SSL_CERT_FILE"] = certificate if trusted else "/dev/null"
                environment["SSL_CERT_DIR"] = str(workspace / "empty-roots")
                with subprocess.Popen(
                    [str(binary), "run", "-c", str(config)],
                    env=environment,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                ) as client:
                    try:
                        wait_listener(client, 10808)
                        try:
                            probe("127.0.0.1", False, timeout=5)
                        except (OSError, ConnectionError):
                            if allowed:
                                raise
                        else:
                            if not allowed:
                                raise RuntimeError(
                                    "TLS пропустил неверное имя или недоверенный сертификат"
                                )
                    finally:
                        stop(client)
            LOG.info(
                "%s: TLS с доверенным сертификатом; отказ при неверном SNI и неизвестном CA",
                engine,
            )
        finally:
            stop(peer)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("compiler", type=Path)
    parser.add_argument("xray", type=Path)
    parser.add_argument("sing_box", type=Path)
    parser.add_argument("--worker", type=int)
    parser.add_argument("--workspace", type=Path)
    args = parser.parse_args()
    if args.worker is None:
        with tempfile.TemporaryDirectory(prefix="yonik-compile-") as directory:
            run_sandbox(
                [
                    "unshare",
                    "--user",
                    "--map-root-user",
                    "--net",
                    sys.executable,
                    str(Path(__file__).resolve()),
                    str(args.compiler.resolve()),
                    str(args.xray.resolve()),
                    str(args.sing_box.resolve()),
                    "--worker",
                    str(os.stat("/proc/self/ns/net").st_ino),
                    "--workspace",
                    directory,
                ],
                timeout=60,
            )
        return 0
    if (
        os.stat("/proc/self/ns/net").st_ino == args.worker
        or links() != {"lo"}
        or args.workspace is None
    ):
        raise RuntimeError(
            "Требуется новый изолированный network namespace и workspace"
        )
    subprocess.run(
        ["ip", "link", "set", "lo", "up"], check=True, capture_output=True, timeout=5
    )
    (args.workspace / "empty-roots").mkdir()
    subprocess.run(
        [
            "openssl",
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=yonik.test",
            "-addext",
            "subjectAltName=DNS:yonik.test",
            "-keyout",
            str(args.workspace / "key.pem"),
            "-out",
            str(args.workspace / "certificate.pem"),
        ],
        check=True,
        capture_output=True,
        timeout=10,
    )
    with echo_servers():
        for engine, binary in [("xray", args.xray), ("sing-box", args.sing_box)]:
            check_matrix(args.compiler, binary, engine, args.workspace)
            traffic(args.compiler, binary, engine, args.workspace)
            tls_traffic(args.compiler, binary, engine, args.workspace)
    return 0


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    install_signal_handlers()
    raise SystemExit(main())
