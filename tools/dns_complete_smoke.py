"""Native opt-in complete A/AAAA: закреплённый sing-box и небольшой Go patch."""

from __future__ import annotations

import argparse
import hashlib
import json
import logging
import os
import subprocess
import sys
from collections import Counter
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from dns_dual_stack_gap import IPV4, IPV6, connect_marker, marker_servers, native_config
from dns_fixture import (
    DnsFixture,
    Reply,
    engine_process,
    validated_binary,
    verify_namespace,
)
from engine_compile import wait_listener
from engine_smoke import install_signal_handlers, run_sandbox

SOURCE_COMMIT = "c005f35300ce76427dc285481ec5695f5b179cd2"
PATCH = (
    Path(__file__).resolve().parents[1]
    / "adapters/sing-box/v1.14.4/strict-complete-dns.patch"
)
LOG = logging.getLogger("tofu.complete-dns")
_WORKER_BOOTSTRAP = """
import logging, runpy, sys
from pathlib import Path
sys.path.insert(0, str(Path(sys.argv[1]).parent))
m = runpy.run_path(sys.argv[1], run_name="tofu_complete_dns_worker")
logging.basicConfig(level=logging.INFO, format="%(message)s")
m["install_signal_handlers"]()
m["worker"](Path(sys.argv[2]), Path(sys.argv[3]), int(sys.argv[4]), Path(sys.argv[5]))
"""


@dataclass(frozen=True)
class Case:
    name: str
    a: str
    aaaa: str
    strategy: str = "prefer_ipv4"
    higher_block: bool = False


CASES = (
    Case("both_valid", "valid", "valid"),
    Case("both_servfail", "servfail", "servfail"),
    Case("a_servfail", "servfail", "valid"),
    Case("aaaa_servfail", "valid", "servfail"),
    Case("aaaa_nxdomain", "valid", "nxdomain"),
    Case("a_nodata", "nodata", "valid"),
    Case("aaaa_nodata", "valid", "nodata"),
    Case("both_nodata", "nodata", "nodata"),
    Case("ipv4_only", "valid", "servfail", "ipv4_only"),
    Case("ipv6_only", "servfail", "valid", "ipv6_only"),
    Case("higher_aaaa_block", "valid", "valid", higher_block=True),
)


def expected_marker(case: Case, strict: bool) -> bool:
    outcomes = [case.a, case.aaaa]
    if case.strategy == "ipv4_only":
        outcomes = [case.a]
    elif case.strategy == "ipv6_only":
        outcomes = [case.aaaa]
    return (
        not case.higher_block
        and "valid" in outcomes
        and (not strict or all(value in {"valid", "nodata"} for value in outcomes))
    )


def configuration(
    case: Case, strict: bool | None, explicit: bool, cache: bool
) -> dict[str, Any]:
    config = native_config()
    config["dns"]["disable_cache"] = not cache
    resolve = config["route"]["rules"][0]
    resolve["strategy"] = case.strategy
    if not explicit:
        del resolve["server"]
    if strict is not None:
        resolve["strict_complete"] = strict
    return config


def digest(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def validate_artifact(binary: Path, manifest_path: Path) -> Path:
    if manifest_path.stat().st_size > 65536:
        raise ValueError("Слишком большой adapter manifest")
    manifest = json.loads(manifest_path.read_bytes())
    if not isinstance(manifest, dict) or not isinstance(
        manifest.get("adapter_features"), dict
    ):
        raise ValueError("Некорректный adapter manifest")
    if (
        manifest.get("engine") != "sing-box"
        or manifest.get("source_commit") != SOURCE_COMMIT
        or manifest.get("binary_sha256") != digest(binary)
        or manifest.get("patch_sha256") != digest(PATCH)
        or manifest.get("adapter_features", {}).get("strict_complete_dns") is not True
        or manifest.get("strict_dns_gate") is not False
    ):
        raise ValueError("Не подтверждён бинарник ограниченного DNS adapter")
    return binary.resolve(strict=True)


def worker(binary: Path, baseline: Path, host_inode: int, output: Path) -> None:
    verify_namespace(host_inode)
    for address in [f"{IPV4}/32", f"{IPV6}/128"]:
        verify_namespace(host_inode)
        subprocess.run(
            ["ip", "addr", "add", address, "dev", "lo"], check=True, timeout=3
        )
    results = []
    controls = {"both_valid", "a_servfail", "aaaa_servfail", "higher_aaaa_block"}
    variants = [
        ("upstream", baseline, None),
        ("patched_default", binary, None),
        ("patched_false", binary, False),
        ("strict", binary, True),
    ]
    for variant, engine, strict in variants:
        for explicit in (True, False):
            for cache in (False, True):
                for case in CASES:
                    if variant != "strict" and case.name not in controls:
                        continue
                    directory = (
                        output
                        / variant
                        / ("explicit" if explicit else "rules")
                        / ("cache" if cache else "no_cache")
                        / case.name
                    )

                    def answer(name: str, qtype: int, _index: int) -> Reply:
                        if name != "partial.test" or qtype not in (1, 28):
                            raise ValueError("Неожиданный запрос синтетического DNS")
                        outcome = case.a if qtype == 1 else case.aaaa
                        if outcome == "valid":
                            address = (
                                IPV4
                                if qtype == 1
                                else (
                                    "2001:db8:ffff::10" if case.higher_block else IPV6
                                )
                            )
                            return Reply((address,), ttl=60)
                        return Reply(
                            rcode={"nodata": 0, "servfail": 2, "nxdomain": 3}[outcome],
                            ttl=60,
                        )

                    with DnsFixture(answer) as fixture, marker_servers() as received:
                        with engine_process(
                            engine,
                            "sing-box",
                            configuration(case, strict, explicit, cache),
                            directory,
                        ) as process:
                            wait_listener(process, 10808)
                            observations = [
                                connect_marker("partial.test")[0] for _ in range(2)
                            ]
                            alive = process.poll() is None
                        reaped = process.returncode is not None
                        counts = Counter(record["qtype"] for record in fixture.records)
                        hits = len(received)
                    expected = expected_marker(case, strict is True)
                    families = (
                        [1]
                        if case.strategy == "ipv4_only"
                        else [28]
                        if case.strategy == "ipv6_only"
                        else [1, 28]
                    )
                    passed = (
                        observations == [expected, expected]
                        and hits == (2 if expected else 0)
                        and alive
                        and reaped
                        and all(counts[kind] > 0 for kind in families)
                        and all(counts[kind] == 0 for kind in {1, 28} - set(families))
                    )
                    if case.name == "both_valid":
                        passed &= counts[1] == counts[28] == (1 if cache else 2)
                    result = {
                        "variant": variant,
                        "explicit_transport": explicit,
                        "cache": cache,
                        "case": case.name,
                        "markers": observations,
                        "hits": hits,
                        "dns_counts": {"A": counts[1], "AAAA": counts[28]},
                        "alive": alive,
                        "reaped": reaped,
                        "passed": passed,
                    }
                    results.append(result)
                    (directory / "observation.json").write_text(
                        json.dumps(result, indent=2) + "\n"
                    )
                    if not passed:
                        raise RuntimeError(
                            f"Native complete DNS mismatch: {variant}/{explicit}/{cache}/{case.name}"
                        )
    report = {
        "source_commit": SOURCE_COMMIT,
        "binary_sha256": digest(binary),
        "baseline_sha256": digest(baseline),
        "strict_complete_dns_passed": True,
        "strict_dns_gate": False,
        "isolated": True,
        "cases": results,
    }
    (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    LOG.info("Native complete DNS: %d сценариев; полный G-DNS ещё закрыт", len(results))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("local/validation/strict-dns-adapter/native"),
    )
    args = parser.parse_args()
    binary = validate_artifact(args.binary, args.manifest)
    baseline = validated_binary(args.baseline, "sing-box")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    run_sandbox(
        [
            "unshare",
            "--user",
            "--map-root-user",
            "--keep-caps",
            "--net",
            "--pid",
            "--fork",
            "--mount",
            "--mount-proc",
            sys.executable,
            "-c",
            _WORKER_BOOTSTRAP,
            str(Path(__file__).resolve()),
            str(binary),
            str(baseline),
            str(os.stat("/proc/self/ns/net").st_ino),
            str(output),
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
        LOG.error("Native DNS проверка не пройдена: %s", error)
        raise SystemExit(1) from None
