"""Native exclusions A/AAAA, реальный TTL и reset в отдельном netns."""

from __future__ import annotations

import argparse
import hashlib
import ipaddress
import json
import logging
import os
import subprocess
import sys
import time
from pathlib import Path

import fakeip_epoch_gap as probe
from dns_fixture import DnsFixture, Reply, dns_query, engine_process, verify_namespace
from dns_pinned_lease_blocker import PINS, checked_markers
from engine_compile import wait_listener
from engine_smoke import install_signal_handlers, run_sandbox


def configuration(engine: str, ipv6: bool) -> dict:
    config = probe.config(engine)
    if engine == "xray":
        config["fakedns"] = [
            {"ipPool": "fc00::/18" if ipv6 else "198.18.0.0/15", "poolSize": 65535}
        ]
        config["dns"] = {
            "servers": [
                {
                    "address": "127.0.0.1",
                    "port": 1055,
                    "domains": [
                        "full:exact.excluded.test",
                        "domain:suffix.excluded.test",
                    ],
                    "skipFallback": True,
                },
                {"address": "fakedns", "domains": ["regexp:.*"]},
            ],
            "queryStrategy": "UseIPv6" if ipv6 else "UseIPv4",
            "disableCache": True,
            "disableFallback": True,
            "tag": "dns-upstream",
        }
        config["outbounds"].append({"tag": "real-dns", "protocol": "freedom"})
        config["routing"]["rules"].insert(
            0,
            {
                "type": "field",
                "inboundTag": ["dns-upstream"],
                "outboundTag": "real-dns",
            },
        )
    else:
        config["dns"] = {
            "servers": [
                {
                    "type": "udp",
                    "tag": "real",
                    "server": "127.0.0.1",
                    "server_port": 1055,
                },
                {
                    "type": "fakeip",
                    "tag": "fake",
                    "inet6_range" if ipv6 else "inet4_range": "fc00::/18"
                    if ipv6
                    else "198.18.0.0/15",
                },
            ],
            "rules": [
                {
                    "domain": ["exact.excluded.test"],
                    "action": "route",
                    "server": "real",
                },
                {
                    "domain_suffix": ["suffix.excluded.test"],
                    "action": "route",
                    "server": "real",
                },
                {
                    "query_type": ["AAAA" if ipv6 else "A"],
                    "action": "route",
                    "server": "fake",
                },
            ],
            "final": "real",
            "disable_cache": True,
            "strategy": "ipv6_only" if ipv6 else "ipv4_only",
        }
        config["route"]["default_domain_resolver"] = "real"
    return config


def exclusions(binary: Path, engine: str, ipv6: bool, output: Path) -> dict:
    qtype = 28 if ipv6 else 1
    address = "2001:db8::10" if ipv6 else "192.0.2.10"
    expected_names = [
        "exact.excluded.test",
        "suffix.excluded.test",
        "child.suffix.excluded.test",
    ]
    with DnsFixture(
        lambda name, kind, count: Reply(addresses=(address,), ttl=2), port=1055
    ) as upstream:
        with engine_process(
            binary, engine, configuration(engine, ipv6), output
        ) as process:
            wait_listener(process, 10808)
            replies = {}
            for name in expected_names:
                replies[name] = dns_query(name, qtype=qtype, port=1053)
                if replies[name] != [address]:
                    raise RuntimeError(f"Exclusion не получил real answer: {name}")
            for name in [
                "child.exact.excluded.test",
                "notsuffix.excluded.test",
                "name-0.fakeip.test",
            ]:
                values = dns_query(name, qtype=qtype, port=1053)
                pool = ipaddress.ip_network("fc00::/18" if ipv6 else "198.18.0.0/15")
                if len(values) != 1 or ipaddress.ip_address(values[0]) not in pool:
                    raise RuntimeError("Положительный FakeIP control не прошёл")
                replies[name] = values
            if [(r["name"], r["qtype"]) for r in upstream.records] != [
                (name, qtype) for name in expected_names
            ]:
                raise RuntimeError("Upstream witness не совпал с exclusions")
            if process.poll() is not None:
                raise RuntimeError("Native process завершился")
    return {
        "engine": engine,
        "qtype": qtype,
        "answers": replies,
        "actual_upstream_records": upstream.records,
    }


def wait_ttl(ttl: int, started: float, process: subprocess.Popen) -> None:
    if not 0 < ttl <= 600:
        raise RuntimeError("TTL вне bounded бюджета")
    deadline = started + ttl + 0.2
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("Ядро завершилось во время TTL wait")
        remaining = deadline - time.monotonic()
        logging.info("TTL wait: осталось %.1f с", remaining)
        time.sleep(min(30, max(0, remaining)))


def lifecycle(binary: Path, engine: str, output: Path) -> dict:
    config = configuration(engine, False)
    with checked_markers() as hits:
        with engine_process(binary, engine, config, output) as process:
            wait_listener(process, 10808)
            ttls: list[int] = []
            old = dns_query("name-0.fakeip.test", qtype=1, port=1053, ttl_capture=ttls)
            if len(old) != 1 or len(ttls) != 1:
                raise RuntimeError("Нет единственного wire address/TTL")
            started = time.monotonic()
            if probe.connect(old[0]) != "M00":
                raise RuntimeError("Нет initial marker")
            # Реальное событие link reset, только внутри собственного netns.
            for command in [
                ["ip", "link", "add", "dns-reset", "type", "dummy"],
                ["ip", "link", "set", "dns-reset", "up"],
                ["ip", "link", "set", "dns-reset", "down"],
            ]:
                subprocess.run(command, check=True, timeout=3)
            after_reset = probe.connect(old[0])
            if after_reset != "M00":
                raise RuntimeError("Ожидаемый native reset witness не воспроизведён")
            fresh = dns_query("name-1.fakeip.test", qtype=1, port=1053)
            if len(fresh) != 1 or probe.connect(fresh[0]) != "M01":
                raise RuntimeError("Нет положительного post-reset control")
            wait_ttl(ttls[0], started, process)
            elapsed = time.monotonic() - started
            expired = probe.connect(old[0])
            if elapsed <= ttls[0] or expired != "M00":
                raise RuntimeError("Native TTL witness не воспроизведён")
            # Повторный DNS-запрос — положительный control живого resolver после ожидания.
            renewed = dns_query("name-0.fakeip.test", qtype=1, port=1053)
            if len(renewed) != 1 or probe.connect(renewed[0]) != "M00":
                raise RuntimeError("Нет fresh lookup control после TTL")
            if process.poll() is not None or hits != [
                "M00",
                "M00",
                "M01",
                "M00",
                "M00",
            ]:
                raise RuntimeError("Неожиданный process/marker transcript")
            result = {
                "engine": engine,
                "pid": process.pid,
                "netns_inode": os.stat("/proc/self/ns/net").st_ino,
                "wire_ttl": ttls[0],
                "elapsed": elapsed,
                "old_ip": old[0],
                "after_link_reset_marker": after_reset,
                "after_ttl_marker": expired,
                "fresh_after_reset": fresh,
                "renewed_after_ttl": renewed,
                "markers": list(hits),
                "production_stamp": None,
                "custody_context": "native fixture, same PID/config/netns; production fence не проверен",
            }
        subprocess.run(["ip", "link", "del", "dns-reset"], check=True, timeout=3)
    return result


def reset_control(binary: Path, engine: str, output: Path) -> dict:
    """Reset используемого loopback; fixture не выдаёт production Stamp."""
    answer = ["192.0.2.10"]
    with (
        checked_markers() as hits,
        DnsFixture(
            lambda name, kind, count: Reply(addresses=(answer[0],), ttl=2), port=1055
        ) as upstream,
    ):
        with engine_process(
            binary, engine, configuration(engine, False), output
        ) as process:
            wait_listener(process, 10808)
            before_real = dns_query("exact.excluded.test", qtype=1, port=1053)
            old = dns_query("name-0.fakeip.test", qtype=1, port=1053)
            if (
                before_real != [answer[0]]
                or len(old) != 1
                or probe.connect(old[0]) != "M00"
            ):
                raise RuntimeError("Нет положительного pre-reset control")
            before_links = subprocess.check_output(
                ["ip", "-j", "link", "show", "lo"], timeout=3
            ).decode()
            subprocess.run(["ip", "link", "set", "lo", "down"], check=True, timeout=3)
            down_links = subprocess.check_output(
                ["ip", "-j", "link", "show", "lo"], timeout=3
            ).decode()
            if "UP" in json.loads(down_links)[0]["flags"]:
                raise RuntimeError("Loopback не перешёл down")
            subprocess.run(["ip", "link", "set", "lo", "up"], check=True, timeout=3)
            answer[0] = "192.0.2.11"
            fresh_real = dns_query("exact.excluded.test", qtype=1, port=1053)
            stale = probe.connect(old[0])
            fresh = dns_query("name-1.fakeip.test", qtype=1, port=1053)
            if (
                fresh_real != [answer[0]]
                or stale != "M00"
                or len(fresh) != 1
                or probe.connect(fresh[0]) != "M01"
            ):
                raise RuntimeError("Post-reset control/witness не совпал")
            if process.poll() is not None or hits != ["M00", "M00", "M01"]:
                raise RuntimeError("Неожиданный marker/process")
            if [(r["name"], r["addresses"]) for r in upstream.records] != [
                ("exact.excluded.test", ["192.0.2.10"]),
                ("exact.excluded.test", ["192.0.2.11"]),
            ]:
                raise RuntimeError("Нет реального upstream witness reset")
            result = {
                "engine": engine,
                "same_pid": process.pid,
                "netns_inode": os.stat("/proc/self/ns/net").st_ino,
                "lo_before": json.loads(before_links),
                "lo_down": json.loads(down_links),
                "real_before": before_real,
                "real_after": fresh_real,
                "actual_upstream_records": upstream.records,
                "old_fake": old,
                "old_mapping_after_reset": stale,
                "fresh_fake": fresh,
                "markers": list(hits),
                "production_stamp": None,
                "production_epoch_fence": "NOT_TESTED",
            }
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("xray", type=Path)
    parser.add_argument("sing_box", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--worker", type=int)
    parser.add_argument("--exclusions-only", action="store_true")
    parser.add_argument("--reset-only", action="store_true")
    args = parser.parse_args()
    binaries = {"xray": args.xray.resolve(), "sing-box": args.sing_box.resolve()}
    for engine, path in binaries.items():
        with path.open("rb") as source:
            if hashlib.file_digest(source, "sha256").hexdigest() != PINS[engine]:
                raise RuntimeError("Неверный pin")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if args.worker is None:
        run_sandbox(
            [
                "unshare",
                "--user",
                "--map-current-user",
                "--keep-caps",
                "--net",
                "--mount",
                "--pid",
                "--fork",
                "--kill-child",
                "--mount-proc",
                sys.executable,
                str(Path(__file__).resolve()),
                str(binaries["xray"]),
                str(binaries["sing-box"]),
                "--output",
                str(output),
                "--worker",
                str(os.stat("/proc/self/ns/net").st_ino),
            ]
            + (["--exclusions-only"] if args.exclusions_only else [])
            + (["--reset-only"] if args.reset_only else []),
            timeout=750,
        )
        return
    verify_namespace(args.worker)
    results = {
        "pins": PINS,
        "exclusions": [],
        "lifecycle": [],
        "reset": [],
        "full_gdns": "OPEN",
    }
    for engine, binary in binaries.items():
        if args.reset_only:
            results["reset"].append(
                reset_control(binary, engine, output / f"{engine}-reset")
            )
            continue
        for ipv6 in [False, True]:
            results["exclusions"].append(
                exclusions(binary, engine, ipv6, output / f"{engine}-exclusions-{ipv6}")
            )
        (output / "partial.json").write_text(
            json.dumps(results, indent=2, ensure_ascii=False) + "\n"
        )
        if not args.exclusions_only:
            results["lifecycle"].append(
                lifecycle(binary, engine, output / f"{engine}-lifecycle")
            )
    (output / "result.json").write_text(
        json.dumps(results, indent=2, ensure_ascii=False) + "\n"
    )


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(message)s")
    install_signal_handlers()
    main()
