"""Нативный G-APP spike. Только новый user/net/pid/mount namespace; без Rust UI."""

from __future__ import annotations

import concurrent.futures
import hashlib
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))
from engine_compile import UUID, stop  # noqa: E402
from engine_smoke import links, run_sandbox  # noqa: E402
from policy_traffic import core, marker_peer, markers, write_json  # noqa: E402

CLIENT = """import socket,sys,json,time
s=socket.socket(int(sys.argv[1]),int(sys.argv[2]));s.settimeout(1)
time.sleep(max(0,float(sys.argv[4])-time.monotonic()))
t=time.monotonic_ns()
try:
 s.connect((sys.argv[3],24081));s.sendall(b'x'*32);r=s.recv(33);v=r[:1].decode()
except OSError:v='B'
print(json.dumps({'marker':v,'elapsed_us':(time.monotonic_ns()-t)//1000}))
"""


def ip(*args: str) -> None:
    subprocess.run(["ip", *args], check=True, capture_output=True, timeout=5)


def config(engine: str, executable: str) -> dict:
    value = json.loads((ROOT / "tests/engines" / f"{engine}-traffic.json").read_text())
    destinations = ["198.18.0.2/32", "2001:db8:746f::2/128"]
    if engine == "xray":
        value["log"] = {"loglevel": "debug"}
        value["outbounds"].append(
            {
                "tag": "proxy",
                "protocol": "vless",
                "settings": {
                    "vnext": [
                        {
                            "address": "127.0.0.1",
                            "port": 24443,
                            "users": [{"id": UUID, "encryption": "none"}],
                        }
                    ]
                },
            }
        )
        value["routing"]["rules"] = [
            {
                "type": "field",
                "process": ["/yonik-fixture-absent", executable],
                "ip": destinations,
                "outboundTag": "proxy",
            },
            {
                "type": "field",
                "process": ["/yonik-fixture-absent", executable],
                "ip": ["198.18.0.4/32", "2001:db8:746f::4/128"],
                "outboundTag": "block",
            },
            {"type": "field", "network": "tcp,udp", "outboundTag": "echo4"},
        ]
    else:
        value["log"] = {"level": "debug"}
        value["outbounds"].append(
            {
                "type": "vless",
                "tag": "proxy",
                "server": "127.0.0.1",
                "server_port": 24443,
                "uuid": UUID,
            }
        )
        value["route"]["rules"] = [
            {
                "process_path": ["/yonik-fixture-absent", executable],
                "ip_cidr": destinations,
                "action": "route",
                "outbound": "proxy",
            },
            {
                "process_path": ["/yonik-fixture-absent", executable],
                "ip_cidr": ["198.18.0.4/32", "2001:db8:746f::4/128"],
                "action": "reject",
            },
            {
                "action": "route",
                "outbound": "echo",
                "override_address": "127.0.0.1",
                "override_port": 24081,
            },
        ]
    return value


def probe(exe: Path, family: int, kind: int, address: str, start: float) -> dict:
    result = subprocess.run(
        [str(exe), "-c", CLIENT, str(family), str(kind), address, str(start)],
        capture_output=True,
        text=True,
        check=True,
        timeout=4,
    )
    return json.loads(result.stdout)


def worker(
    binary: Path, engine: str, output: Path, host: int, unknown: bool = False
) -> None:
    if os.stat("/proc/self/ns/net").st_ino == host or links() != {"lo"}:
        raise RuntimeError("Нужен новый пустой network namespace")
    label = engine + ("-unknown" if unknown else "")
    ip("link", "set", "lo", "up")
    with tempfile.TemporaryDirectory(prefix="yonik-attribution-") as temp:
        work = Path(temp)
        executables = []
        for name in ("a", "b"):
            directory = work / name
            directory.mkdir()
            exe = directory / "client"
            shutil.copy2(Path(sys.executable).resolve(), exe)
            executables.append(exe)
        path = work / "tun.json"
        value = config(engine, str(executables[0]))
        compiler = os.environ.get("YONIK_ATTRIBUTION_COMPILER")
        if compiler:
            policy = {
                "version": 1,
                "default_action": "direct",
                "user_rules": [
                    {
                        "id": 1,
                        "action": "proxy",
                        "conditions": {
                            "executable_paths": [
                                "/yonik-fixture-absent",
                                str(executables[0]),
                            ],
                            "ip_cidr": ["198.18.0.2/32", "2001:db8:746f::2/128"],
                        },
                    },
                    {
                        "id": 2,
                        "action": "block",
                        "conditions": {
                            "executable_paths": [
                                "/yonik-fixture-absent",
                                str(executables[0]),
                            ],
                            "ip_cidr": ["198.18.0.4/32", "2001:db8:746f::4/128"],
                        },
                    },
                ],
            }
            policy_path = work / "policy.json"
            write_json(policy_path, policy)
            result = subprocess.run(
                [
                    compiler,
                    engine,
                    "--policy",
                    str(policy_path),
                    "--policy-mode",
                    "ip-only",
                    "--probe-port",
                    "10809",
                    "--tun-id",
                    "1",
                    "--tun-mtu",
                    "1500",
                    "--routing-mark",
                    "99",
                    "--tun-address",
                    "198.18.0.1/29",
                    "--tun-address",
                    "2001:db8:746f::1/125",
                    "--emit-secret-config",
                ],
                input=f"vless://{UUID}@127.0.0.1:24443".encode(),
                capture_output=True,
                check=True,
                timeout=5,
            )
            compiled = json.loads(result.stdout)
            write_json(output / f"{label}-compiled.json", compiled)
            write_json(output / f"{label}-policy.json", policy)
            inbound = value["inbounds"][0]
            inbound["tag"] = "system-tun"
            compiled["inbounds"] = [inbound]
            if engine == "xray":
                for outbound in compiled["outbounds"]:
                    if outbound["tag"] == "direct":
                        outbound["settings"] = {"redirect": "127.0.0.1:24081"}
                compiled["log"] = {"loglevel": "debug"}
            else:
                for rule in compiled["route"]["rules"]:
                    if rule.get("outbound") == "direct":
                        rule.update(override_address="127.0.0.1", override_port=24081)
                compiled["log"] = {"level": "debug"}
            value = compiled
        write_json(path, value)
        write_json(output / f"{label}-config.json", value)
        peer_path = work / "peer.json"
        write_json(peer_path, marker_peer(engine))
        rows = []
        with (
            markers(),
            core(binary, peer_path, 24443),
            (output / f"{label}.log").open("w") as log,
        ):
            prefix = (
                ["unshare", "--mount", "--pid", "--fork", "--mount-proc"]
                if unknown
                else []
            )
            process = subprocess.Popen(
                [*prefix, str(binary), "run", "-c", str(path)], stdout=log, stderr=log
            )
            try:
                deadline = time.monotonic() + 5
                while "yonik-test" not in links():
                    if process.poll() is not None or time.monotonic() > deadline:
                        raise RuntimeError("Ядро не создало TUN; см. log")
                    time.sleep(0.05)
                ip("address", "replace", "198.18.0.1/29", "dev", "yonik-test")
                ip(
                    "-6",
                    "address",
                    "replace",
                    "2001:db8:746f::1/125",
                    "dev",
                    "yonik-test",
                    "nodad",
                )
                ip("link", "set", "yonik-test", "up")
                ip("route", "replace", "198.18.0.4/32", "dev", "yonik-test")
                ip(
                    "-6",
                    "route",
                    "replace",
                    "2001:db8:746f::4/128",
                    "dev",
                    "yonik-test",
                )
                with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                    for family, prefix in [
                        (socket.AF_INET, "198.18.0."),
                        (socket.AF_INET6, "2001:db8:746f::"),
                    ]:
                        for kind in (socket.SOCK_STREAM, socket.SOCK_DGRAM):
                            for suffix in (2, 4):
                                address = prefix + str(suffix)
                                route = subprocess.run(
                                    ["ip", "-j", "route", "get", address],
                                    check=True,
                                    capture_output=True,
                                    text=True,
                                ).stdout
                                if json.loads(route)[0].get("dev") != "yonik-test":
                                    raise RuntimeError("Маршрут обходит TUN")
                                start = time.monotonic() + 0.1
                                futures = [
                                    pool.submit(
                                        probe, exe, family, kind, address, start
                                    )
                                    for exe in executables
                                ]
                                for index, future in enumerate(futures):
                                    rows.append(
                                        {
                                            "client": index,
                                            "family": int(family),
                                            "kind": int(kind),
                                            "destination": address,
                                            "expected": ("P" if suffix == 2 else "B")
                                            if index == 0 and not unknown
                                            else "D",
                                            **future.result(),
                                        }
                                    )
            finally:
                stop(process)
        report = {
            "engine": engine,
            "unknown_owner_fixture": unknown,
            "binary_sha256": hashlib.file_digest(
                binary.open("rb"), "sha256"
            ).hexdigest(),
            "rows": rows,
            "passed": all(r["marker"] == r["expected"] for r in rows),
            "limits": "native spike; elapsed_us включает соединение, не отдельную стоимость owner lookup; child process Python, два разных пути одного basename; unknown owner: ядро в дочернем PID namespace не видит клиентов, общий netns",
        }
        write_json(output / f"{label}-result.json", report)
        print(json.dumps(report, ensure_ascii=False))
        if not report["passed"]:
            raise RuntimeError("Маршруты не соответствуют ожиданиям")


if __name__ == "__main__":
    if sys.argv[1] == "worker":
        worker(
            Path(sys.argv[2]),
            sys.argv[3],
            Path(sys.argv[4]),
            int(sys.argv[5]),
            len(sys.argv) > 6 and sys.argv[6] == "unknown",
        )
    else:
        binary, engine, output = (
            Path(sys.argv[1]).resolve(),
            sys.argv[2],
            Path(sys.argv[3]).resolve(),
        )
        output.mkdir(parents=True, exist_ok=True)
        run_sandbox(
            [
                "unshare",
                "--user",
                "--map-root-user",
                "--net",
                "--mount",
                "--pid",
                "--fork",
                "--mount-proc",
                sys.executable,
                str(Path(__file__).resolve()),
                "worker",
                str(binary),
                engine,
                str(output),
                str(os.stat("/proc/self/ns/net").st_ino),
                *sys.argv[4:],
            ],
            timeout=90,
        )
