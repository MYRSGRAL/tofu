"""Native JSON grammar ручной формы → VLESS TCP/TLS controlled peers обоих cores."""

from __future__ import annotations

import json
import logging
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tools"))
import engine_compile  # noqa: E402
from engine_smoke import links, run_sandbox  # noqa: E402
from traffic_echo import echo_servers  # noqa: E402


def generated(compiler: Path, engine: str, uri: str) -> bytes:
    endpoint = urlsplit(uri)
    query = parse_qs(endpoint.query)
    outbound: dict[str, object] = {
        "type": "vless",
        "tag": "manual-peer",
        "server": endpoint.hostname,
        "server_port": endpoint.port,
        "uuid": endpoint.username,
    }
    if query.get("security") == ["tls"]:
        outbound["tls"] = {"enabled": True, "server_name": query["sni"][0]}
    return subprocess.run(
        [str(compiler), engine, "--emit-secret-config"],
        input=json.dumps({"outbounds": [outbound]}).encode(),
        capture_output=True,
        check=True,
        timeout=5,
    ).stdout


def main() -> None:
    compiler, engines = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve()
    if len(sys.argv) == 3:
        with tempfile.TemporaryDirectory(prefix="yonik-manual-peer-") as directory:
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
                    directory,
                    str(os.stat("/proc/self/ns/net").st_ino),
                ],
                timeout=60,
            )
        return
    workspace = Path(sys.argv[3])
    if os.stat("/proc/self/ns/net").st_ino == int(sys.argv[4]) or links() != {"lo"}:
        raise RuntimeError("Требуется отдельный namespace")
    subprocess.run(["ip", "link", "set", "lo", "up"], check=True, capture_output=True)
    (workspace / "empty-roots").mkdir()
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
            str(workspace / "key.pem"),
            "-out",
            str(workspace / "certificate.pem"),
        ],
        check=True,
        capture_output=True,
        timeout=10,
    )
    engine_compile.generated = generated
    with echo_servers():
        for engine, relative in [
            ("xray", "xray-26.10.31/xray"),
            ("sing-box", "sing-box-1.14.4/sing-box"),
        ]:
            engine_compile.traffic(compiler, engines / relative, engine, workspace)
            engine_compile.tls_traffic(compiler, engines / relative, engine, workspace)


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
