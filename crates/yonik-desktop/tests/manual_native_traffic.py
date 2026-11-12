"""Реальный Qt submit → production compiler → pinned core → controlled peer."""

from __future__ import annotations

import json
import logging
import os
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tools"))
from engine_compile import probe, stop, wait_listener  # noqa: E402
from engine_smoke import links, run_sandbox  # noqa: E402
from traffic_echo import echo_servers  # noqa: E402

UUID = "11111111-2222-4333-8444-555555555555"
PASSWORD = "synthetic-manual-secret"


def peer_config(source: dict, root: Path, bad: bool = False) -> dict:
    """Независимый oracle: endpoint и credentials заданы константами."""
    protocol = source["type"]
    inbound = {"type": protocol, "listen": "127.0.0.1", "listen_port": 24443}
    password = "wrong-secret" if bad else PASSWORD
    uuid = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee" if bad else UUID
    if protocol in ("vless", "vmess"):
        inbound["users"] = [{"uuid": uuid}]
    elif protocol == "shadowsocks":
        inbound.update(method="aes-128-gcm", password=password)
    else:
        inbound["users"] = [{"password": password}]
        if protocol in ("socks", "http"):
            inbound["users"][0]["username"] = "alice"
    if "tls" in source:
        inbound["tls"] = {
            "enabled": True,
            "certificate_path": str(root / "certificate.pem"),
            "key_path": str(root / "key.pem"),
        }
    transport = source.get("transport", {}).get("type")
    if transport == "ws":
        inbound["transport"] = {"type": "ws", "path": "/manual-echo"}
    elif transport == "grpc":
        inbound["transport"] = {"type": "grpc", "service_name": "manual-service"}
    return {
        "log": {"disabled": True},
        "inbounds": [inbound],
        "outbounds": [{"type": "direct"}],
    }


def oracle(config: dict, source: dict, engine: str) -> None:
    outbound = next(o for o in config["outbounds"] if o.get("tag") == "proxy")
    protocol = source["type"]
    if engine == "sing-box":
        assert outbound["server"] == "127.0.0.1" and outbound["server_port"] == 24443
        assert outbound["type"] == protocol
        if protocol in ("vless", "vmess"):
            assert outbound["uuid"] == UUID
        else:
            assert outbound["password"] == PASSWORD
        if protocol in ("http", "socks"):
            assert outbound["username"] == "alice"
        if protocol == "shadowsocks":
            assert outbound["method"] == "aes-128-gcm"
        if "transport" in source:
            expected = {"type": "grpc", "service_name": "manual-service"}
            if source["transport"]["type"] == "ws":
                expected = {
                    "type": "ws",
                    "path": "/manual-echo",
                    "headers": {"Host": "yonik.test"},
                }
            assert outbound["transport"] == expected
        if "tls" in source:
            assert outbound["tls"]["server_name"] == "yonik.test"
            assert not outbound["tls"].get("insecure", False)
    else:
        assert outbound["protocol"] == protocol
        settings = outbound["settings"]
        endpoint = settings["vnext" if protocol in ("vless", "vmess") else "servers"][0]
        assert endpoint["address"] == "127.0.0.1" and endpoint["port"] == 24443
        if protocol in ("vless", "vmess"):
            assert endpoint["users"][0]["id"] == UUID
        elif protocol in ("http", "socks"):
            assert endpoint["users"] == [{"user": "alice", "pass": PASSWORD}]
        else:
            assert endpoint["password"] == PASSWORD
        stream = outbound.get("streamSettings", {})
        transport = source.get("transport", {}).get("type", "tcp")
        assert stream.get("network", "tcp") == transport
        if transport == "ws":
            assert stream["wsSettings"]["path"] == "/manual-echo"
            assert stream["wsSettings"]["host"] == "yonik.test"
        if transport == "grpc":
            assert stream["grpcSettings"]["serviceName"] == "manual-service"
        if "tls" in source:
            assert stream["security"] == "tls"
            assert stream["tlsSettings"]["serverName"] == "yonik.test"
            assert not stream["tlsSettings"].get("allowInsecure", False)


def run_case(
    root: Path, peer_binary: Path, binary: Path, config: dict, source: dict
) -> None:
    client_file, peer_file = root / "client.json", root / "peer.json"
    client_file.write_text(json.dumps(config))
    environment = dict(
        os.environ,
        SSL_CERT_FILE=str(root / "certificate.pem"),
        SSL_CERT_DIR=str(root / "empty-roots"),
    )
    modes = ["valid", "credentials"]
    if "tls" in source:
        modes += ["ca", "sni"]
    for mode in modes:
        bad = mode != "valid"
        environment["SSL_CERT_FILE"] = (
            "/dev/null" if mode == "ca" else str(root / "certificate.pem")
        )
        current = json.loads(json.dumps(config))
        if mode == "sni":
            outbound = next(o for o in current["outbounds"] if o.get("tag") == "proxy")
            if "tls" in outbound:
                outbound["tls"]["server_name"] = "wrong.test"
            else:
                outbound["streamSettings"]["tlsSettings"]["serverName"] = "wrong.test"
        client_file.write_text(json.dumps(current))
        peer_file.write_text(
            json.dumps(peer_config(source, root, mode == "credentials"))
        )
        with subprocess.Popen(
            [str(peer_binary), "run", "-c", str(peer_file)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        ) as peer:
            try:
                wait_listener(peer, 24443)
                with subprocess.Popen(
                    [str(binary), "run", "-c", str(client_file)],
                    env=environment,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                ) as client:
                    try:
                        wait_listener(client, 10808)
                        try:
                            probe("127.0.0.1", False, timeout=2)
                        except (OSError, ConnectionError):
                            if not bad:
                                raise
                        else:
                            if bad:
                                raise RuntimeError(
                                    "Неверные credentials пропустили трафик"
                                )
                    finally:
                        stop(client)
            finally:
                stop(peer)


def main() -> None:
    compiler, engines, root = (Path(p).resolve() for p in sys.argv[1:4])
    if len(sys.argv) == 4:
        run_sandbox(
            [
                "unshare",
                "--user",
                "--map-root-user",
                "--net",
                sys.executable,
                str(Path(__file__).resolve()),
                str(compiler),
                str(engines),
                str(root),
                str(os.stat("/proc/self/ns/net").st_ino),
            ],
            timeout=180,
        )
        return
    assert os.stat("/proc/self/ns/net").st_ino != int(sys.argv[4]) and links() == {"lo"}
    subprocess.run(["ip", "link", "set", "lo", "up"], check=True)
    (root / "empty-roots").mkdir(exist_ok=True)
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
            str(root / "key.pem"),
            "-out",
            str(root / "certificate.pem"),
        ],
        check=True,
        capture_output=True,
    )
    result = subprocess.run(
        [
            "/usr/lib/qt6/bin/qmltestrunner",
            "-input",
            str(Path(__file__).with_name("manual_traffic_qml")),
        ],
        env=dict(os.environ, QT_QPA_PLATFORM="offscreen"),
        capture_output=True,
        text=True,
        check=True,
        timeout=20,
    )
    (root / "qt-fixtures.log").write_text(result.stdout + result.stderr)
    fixtures = [
        json.loads(line.split("MANUAL_FIXTURE:", 1)[1])
        for line in result.stdout.splitlines()
        if "MANUAL_FIXTURE:" in line
    ]
    assert len(fixtures) == 12
    peer_binary = engines / "sing-box-1.14.4/sing-box"
    with echo_servers():
        for engine, relative in [
            ("xray", "xray-26.10.31/xray"),
            ("sing-box", "sing-box-1.14.4/sing-box"),
        ]:
            for fixture in fixtures:
                source = fixture["outbounds"][0]
                if source["type"] == "vless" and "transport" not in source:
                    continue
                started = time.monotonic()
                compiled = subprocess.run(
                    [str(compiler), engine, "--emit-secret-config"],
                    input=json.dumps(fixture),
                    capture_output=True,
                    text=True,
                    check=True,
                    timeout=5,
                )
                config = json.loads(compiled.stdout)
                oracle(config, source, engine)
                run_case(root, peer_binary, engines / relative, config, source)
                logging.info(
                    "PASS %s %s echo + credentials/TLS controls %.3fs",
                    engine,
                    source["tag"],
                    time.monotonic() - started,
                )


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
