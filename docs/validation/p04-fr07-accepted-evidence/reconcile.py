#!/usr/bin/env python3
"""Data-only сверка пяти synthetic evidence bundles; ничего не исполняет."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import re
import struct
import subprocess
from pathlib import Path

ACCEPTED = "8ea826a04a745c8916e14a4329588c75f66b683e"
BASE = Path("/home/myrs/projects/yonik/local/evidence")
BUNDLES = (
    "p04-robust-9214",
    "fr07-desktop-import",
    "fr07-manual",
    "fr07-integrated",
    "pr127-oracle",
)
MANIFESTS = {
    "p04-robust-9214": "sources.sha256",
    "fr07-desktop-import": "sources.sha256",
    "fr07-manual": "sources.sha256",
    "fr07-integrated": "source-binding.txt",
}
REQUIRED_HASH_MANIFESTS = {
    "p04-robust-9214": {},
    "fr07-desktop-import": {"logs.sha256": 84},
    "fr07-manual": {"binaries.sha256": 4},
    "fr07-integrated": {"binaries.sha256": 1},
    "pr127-oracle": {},
}

HEADS = {
    "p04-robust-9214": "37638032f96c1730edefbb185e9380e370c458ed",
    "fr07-desktop-import": "048611be4cc9669276aba955337de7a3adaba128",
    "fr07-manual": "7ff75c5405ac669f1bd56fa9ac5e657bc486e154",
    "fr07-integrated": "bf97e32e639ab3b01ca2feb119c7d103441af3f4",
    "pr127-oracle": "75c0c9fb8db201e9cdcf617f20edd39e789a1df2",
}


def sha(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def entries(path: Path, *, header: tuple[str, ...] = ()) -> list[tuple[str, str]]:
    lines = path.read_text().splitlines()
    if lines[: len(header)] != list(header):
        raise ValueError(f"Некорректный заголовок манифеста: {path}")
    result = []
    seen = set()
    for number, line in enumerate(lines[len(header) :], len(header) + 1):
        match = re.fullmatch(r"([0-9a-f]{64})\s+\*?(\S(?:.*\S)?)", line)
        if match is None:
            raise ValueError(f"Некорректная декларация: {path}:{number}")
        digest, target = match.groups()
        if target in seen or any(ord(char) < 32 for char in target):
            raise ValueError(f"Некорректная или повторная цель: {path}:{number}")
        seen.add(target)
        result.append((digest, target))
    if not result:
        raise ValueError(f"Пустой hash-манифест: {path}")
    return result


def hash_manifest_entries(root: Path, name: str) -> list[tuple[str, str, str]]:
    result = []
    for file, count in REQUIRED_HASH_MANIFESTS[name].items():
        declared = entries(root / file)
        if len(declared) != count:
            raise ValueError(f"Неверное число деклараций: {root / file}")
        result.extend((file, digest, path) for digest, path in declared)
    return result


def git(*args: str) -> bytes:
    return subprocess.check_output(["git", *args])


def accepted_blobs(ref: str = ACCEPTED) -> dict[str, str]:
    tree = git("ls-tree", "-r", ref).decode().splitlines()
    paths = {
        line.split("\t", 1)[1]: line.split()[2]
        for line in tree
        if line.split()[1] == "blob"
    }
    objects = list(dict.fromkeys(paths.values()))
    raw = subprocess.run(
        ["git", "cat-file", "--batch"],
        input=("\n".join(objects) + "\n").encode(),
        capture_output=True,
        check=True,
    ).stdout
    offset = 0
    digests = {}
    for object_id in objects:
        end = raw.index(b"\n", offset)
        header = raw[offset:end].split()
        size = int(header[2])
        start = end + 1
        digests[object_id] = hashlib.sha256(raw[start : start + size]).hexdigest()
        offset = start + size + 1
    return {path: digests[object_id] for path, object_id in paths.items()}


def corpus_frames(path: Path) -> dict:
    count = 0
    sample_hashes = []
    with path.open("rb") as stream:
        while header := stream.read(4):
            if len(header) != 4:
                raise ValueError("truncated corpus length")
            size = struct.unpack("<I", header)[0]
            if size > 8 * 1024 * 1024:
                raise ValueError("oversized corpus frame")
            payload = stream.read(size)
            if len(payload) != size:
                raise ValueError("truncated corpus payload")
            if count in (0, 1, 2, 4095, 8191, 12287):
                sample_hashes.append(
                    dict(
                        index=count,
                        bytes=size,
                        sha256=hashlib.sha256(payload).hexdigest(),
                    )
                )
            count += 1
    return dict(frames=count, samples=sample_hashes, sha256=sha(path))


def regenerated_corpus() -> dict:
    # Только генерация synthetic mutation bytes из уже принятого теста, не parser replay.
    source = git("show", ACCEPTED + ":crates/yonik-import/tests/robustness.rs").decode()
    canary = re.search(r'const CANARY: &str = "([^"]+)";', source).group(1)
    state = 0x921420261112
    digest = hashlib.sha256()
    count = 0
    for name in ("uri.txt", "xray.json", "sing-box.json"):
        raw = (
            git(
                "show",
                ACCEPTED + ":crates/yonik-import/tests/corpus/p04-seeded/" + name,
            )
            .decode()
            .replace("CANARY", canary)
            .encode()
        )
        for index in range(2048):
            state = (state ^ (state << 13)) & ((1 << 64) - 1)
            state ^= state >> 7
            state = (state ^ (state << 17)) & ((1 << 64) - 1)
            data = bytearray(raw)
            at = state % len(data)
            if index % 4 == 0:
                del data[at:]
            elif index % 4 == 1:
                data[at] = (state >> 32) & 255
            elif index % 4 == 2:
                del data[at]
            else:
                data.insert(at, b'{}[]%"\\\0='[(index // 4) % 9])
            for payload in (bytes(data), base64.b64encode(data)):
                digest.update(struct.pack("<I", len(payload)))
                digest.update(payload)
                count += 1
    return dict(frames=count, sha256=digest.hexdigest())


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    blobs = accepted_blobs()
    by_hash = {}
    for path, digest in blobs.items():
        by_hash.setdefault(digest, []).append(path)
    ledger = dict(
        accepted=ACCEPTED,
        mode="historical data reconciliation; no new binary/runtime/GUI execution",
        bundles={},
    )
    for name in BUNDLES:
        root = BASE / name
        item = dict(
            root=str(root),
            exists=root.is_dir(),
            source_reference=HEADS[name],
            files=[],
            source=[],
            checks=[],
        )
        ledger["bundles"][name] = item
        if not root.is_dir():
            continue
        for path in sorted(root.rglob("*")):
            if path.is_file():
                if path.is_symlink():
                    raise ValueError(f"symlink not allowed: {path}")
                item["files"].append(
                    dict(
                        path=str(path.relative_to(root)),
                        bytes=path.stat().st_size,
                        sha256=sha(path),
                    )
                )
        item["ancestor_of_accepted"] = (
            subprocess.run(
                ["git", "merge-base", "--is-ancestor", HEADS[name], ACCEPTED],
                capture_output=True,
            ).returncode
            == 0
        )
        historical = accepted_blobs(HEADS[name])
        item["source_reference_role"] = (
            "report_base_not_full_binary_build_head"
            if name == "p04-robust-9214"
            else "fixture build base with separately hashed Python harness overlay"
            if name == "fr07-integrated"
            else "recorded_head"
        )
        source_entries = (
            entries(
                root / MANIFESTS[name],
                header=(
                    HEADS[name],
                    "Fresh fixture build from this integrated main; no retained installed binary used.",
                )
                if name == "fr07-integrated"
                else (),
            )
            if name in MANIFESTS
            else [
                (
                    json.loads((root / "manifest.json").read_text())["source_sha256"],
                    json.loads((root / "manifest.json").read_text())["source"],
                )
            ]
        )
        for digest, path in source_entries:
            current = blobs.get(path)
            equivalent = by_hash.get(digest, [])
            item["source"].append(
                dict(
                    path=path,
                    recorded_sha256=digest,
                    accepted_sha256=current,
                    reference_head_sha256=historical.get(path),
                    matches_reference_head=historical.get(path) == digest,
                    status="same_path_exact"
                    if current == digest
                    else "mapped_exact"
                    if equivalent
                    else "changed_or_missing",
                    equivalent_accepted_paths=equivalent if current != digest else [],
                )
            )
        for file, expected, path in hash_manifest_entries(root, name):
            target = Path(path) if Path(path).is_absolute() else root / path
            # Доступ только к canonical bundle и двум pinned synthetic core binaries.
            if not target.is_relative_to(root) and target not in [
                BASE.parent / "engines/xray-26.10.31/xray",
                BASE.parent / "engines/sing-box-1.14.4/sing-box",
            ]:
                item["checks"].append(
                    dict(kind=file, path=path, status="outside_allowlist")
                )
                continue
            item["checks"].append(
                dict(
                    kind=file,
                    path=path,
                    expected_sha256=expected,
                    status="missing"
                    if not target.is_file()
                    else "match"
                    if sha(target) == expected
                    else "mismatch",
                )
            )
        counts = {
            status: sum(x["status"] == status for x in item["source"])
            for status in ("same_path_exact", "mapped_exact", "changed_or_missing")
        }
        item["source_counts"] = counts
    robust = ledger["bundles"]["p04-robust-9214"]
    summary = json.loads((BASE / "p04-robust-9214/summary.json").read_text())
    robust["corpus"] = corpus_frames(BASE / "p04-robust-9214/corpus.bin")
    robust["regenerated_corpus"] = regenerated_corpus()
    robust["corpus"]["matches_regeneration"] = (
        robust["regenerated_corpus"]["sha256"] == robust["corpus"]["sha256"]
    )
    robust["corpus"]["matches_recorded"] = (
        robust["corpus"]["sha256"]
        == summary["sha256"][str(BASE / "p04-robust-9214/corpus.bin")]
    )
    robust["retained_binary_match"] = (
        sha(BASE / "p04-robust-9214/robustness")
        == summary["sha256"][str(BASE / "p04-robust-9214/robustness")]
    )
    desktop = json.loads((BASE / "fr07-desktop-import/manifest.json").read_text())
    ledger["bundles"]["fr07-desktop-import"]["manifest_checks"] = dict(
        binary=sha(BASE / "fr07-desktop-import/yonik-desktop")
        == desktop["binary_sha256"],
        source_manifest=sha(BASE / "fr07-desktop-import/sources.sha256")
        == desktop["source_manifest_sha256"],
    )
    oracle = json.loads((BASE / "pr127-oracle/manifest.json").read_text())
    ledger["bundles"]["pr127-oracle"]["manifest_checks"] = dict(
        binary=sha(BASE / "pr127-oracle/acceptance") == oracle["binary_sha256"],
        accept_log=sha(BASE / "pr127-oracle/accept.log") == oracle["logs_sha256"],
        synthetic_config_count=len(list((BASE / "pr127-oracle").glob("*-xray.json")))
        + len(list((BASE / "pr127-oracle").glob("*-sing-box.json"))),
    )
    integrated = json.loads((BASE / "fr07-integrated/result.json").read_text())
    known = json.loads(Path("docs/evidence/fr07-integrated.json").read_text())
    ledger["bundles"]["fr07-integrated"]["manifest_checks"] = dict(
        binary=sha(BASE / "fr07-integrated/tray_cleanup_negative")
        == integrated["binary_sha256"],
        remote_denial_log=sha(BASE / "fr07-integrated/remote-denial-headless.log")
        == known["remote_operation_denial"]["log_sha256"],
    )
    ledger["bundles"]["fr07-integrated"]["trace_checks"] = [
        dict(
            case=case,
            match=sha(BASE / "fr07-integrated" / f"lifecycle/{case}/trace.jsonl")
            == known["lifecycle"][case]["trace_sha256"],
        )
        for case in ("first", "reopened")
    ]
    ledger["parser_diff_bf97_to_accepted_empty"] = not git(
        "diff",
        "bf97e32e639ab3b01ca2feb119c7d103441af3f4",
        ACCEPTED,
        "--",
        "crates/yonik-import",
    )
    robust_root = BASE / "p04-robust-9214"
    robust["runner_path_mapping"] = dict(
        recorded_path=next(
            path for path in summary["sha256"] if path.endswith("run_robustness.py")
        ),
        retained_path=str(robust_root / "run_robustness.py"),
        accepted_path="crates/yonik-import/tests/evidence/run_robustness.py",
        exact=sha(robust_root / "run_robustness.py")
        == blobs["crates/yonik-import/tests/evidence/run_robustness.py"]
        == next(
            h
            for path, h in summary["sha256"].items()
            if path.endswith("run_robustness.py")
        ),
    )
    robust["negative_control"] = dict(
        kind="historical assertion/test log, not new execution",
        redaction_test_pass="test отрицательный_контроль_редакции ... ok"
        in (robust_root / "campaign.log").read_text(),
        five_tests_pass="5 passed; 0 failed"
        in (robust_root / "campaign.log").read_text(),
    )
    pr = ledger["bundles"]["pr127-oracle"]
    pr["negative_controls"] = dict(
        kind="historical four identity mutations per completed case; not separately logged 48 lines",
        completed_cases=len(
            re.findall(r"^PASS ", (BASE / "pr127-oracle/accept.log").read_text(), re.M)
        ),
        controls_per_case=4,
        total_asserted_controls=48,
        source_path=oracle["source"],
        source_sha256=oracle["source_sha256"],
    )
    for name, filename, key in [
        ("fr07-desktop-import", "manifest.json", "cores_sha256"),
        ("pr127-oracle", "manifest.json", "core_sha256"),
    ]:
        values = json.loads((BASE / name / filename).read_text())[key]
        for path, expected in values.items():
            target = (
                Path(path)
                if Path(path).is_absolute()
                else BASE.parent / "engines" / path
            )
            allowed = [
                BASE.parent / "engines/xray-26.10.31/xray",
                BASE.parent / "engines/sing-box-1.14.4/sing-box",
            ]
            if target not in allowed:
                raise ValueError("core outside allowlist")
            ledger["bundles"][name]["checks"].append(
                dict(
                    kind="pinned_core",
                    path=str(target),
                    expected_sha256=expected,
                    status="missing"
                    if not target.exists()
                    else "match"
                    if sha(target) == expected
                    else "mismatch",
                )
            )
    inter = ledger["bundles"]["fr07-integrated"]
    inter["historical_negative_peer"] = dict(
        cases=len(integrated["results"]),
        positive=sum(r["peer_credentials_valid"] for r in integrated["results"]),
        negative=sum(not r["peer_credentials_valid"] for r in integrated["results"]),
        all_closed_and_reaped=all(
            r["closed"] and r["reaped"] for r in integrated["results"]
        ),
        all_direct_endpoint_available=all(
            r["direct_endpoint_available"] for r in integrated["results"]
        ),
        all_oracles_passed=all(r["oracle"] for r in integrated["results"]),
    )
    inter["recorded_only_digests"] = [
        dict(
            kind="actual write bytes not persisted in canonical bundle",
            original_path_template="<removed task_root>/observed.jsonl",
            canonical_lookup=str(BASE / "fr07-integrated/observed.jsonl"),
            canonical_path_exists=(BASE / "fr07-integrated/observed.jsonl").exists(),
            expected_sha256=r["actual_writes_sha256"],
            case=r["case"],
        )
        for r in integrated["results"]
    ]
    manual = ledger["bundles"]["fr07-manual"]
    manual["historical_controls"] = dict(
        qml="Totals: 8 passed, 0 failed" in (BASE / "fr07-manual/qml.log").read_text(),
        backend_denials="test local_text_partial_approval_reopen_and_network_controls_fail_closed ... ok"
        in (BASE / "fr07-manual/backend.log").read_text(),
        native_matrix="1 passed; 0 failed"
        in (BASE / "fr07-manual/native-exact.log").read_text(),
    )
    desktop_control = json.loads(
        (BASE / "fr07-desktop-import/old-reader-control/result.json").read_text()
    )
    ledger["bundles"]["fr07-desktop-import"]["historical_old_reader_control"] = dict(
        metadata_consistent=desktop_control == desktop["old_reader"],
        format4_refused=desktop_control["old_open_connect_delete_write_refused"],
        backup3_opened=desktop_control["verified_backup3_opened_by_old"],
        database_bytes="recorded digest only; no database/config content read",
    )
    ledger["parser_source_comparison"] = dict(
        reference=HEADS["pr127-oracle"],
        paths=[
            dict(path=path, sha256=digest, same=historical.get(path) == digest)
            for path, digest in blobs.items()
            if path.startswith("crates/yonik-import/src/")
        ],
    )
    ledger["totals"] = dict(
        bundles=len(BUNDLES),
        files=sum(len(b["files"]) for b in ledger["bundles"].values()),
        bytes=sum(f["bytes"] for b in ledger["bundles"].values() for f in b["files"]),
        source_entries=sum(len(b["source"]) for b in ledger["bundles"].values()),
        source_exact=sum(
            b["source_counts"]["same_path_exact"] for b in ledger["bundles"].values()
        ),
        source_changed=sum(
            b["source_counts"]["changed_or_missing"] for b in ledger["bundles"].values()
        ),
        declared_data_checks=sum(len(b["checks"]) for b in ledger["bundles"].values()),
        declared_data_mismatches=[
            check
            for b in ledger["bundles"].values()
            for check in b["checks"]
            if check["status"] != "match"
        ],
    )
    manual_result = json.loads((BASE / "fr07-manual/result.json").read_text())
    ledger["metadata_consistency"] = dict(
        desktop_head=desktop["head"] == HEADS["fr07-desktop-import"],
        desktop_tree=desktop["source_tree"]
        == git("rev-parse", HEADS["fr07-desktop-import"] + "^{tree}").decode().strip(),
        manual_head=manual_result["head"]
        == (BASE / "fr07-manual/HEAD.txt").read_text().strip()
        == HEADS["fr07-manual"],
        manual_parent=manual_result["parent"] == HEADS["fr07-desktop-import"],
        integrated_base=(BASE / "fr07-integrated/source-binding.txt")
        .read_text()
        .splitlines()[0]
        == known["source_base"]
        == HEADS["fr07-integrated"],
        oracle_head=oracle["head"] == HEADS["pr127-oracle"],
        robust_count=summary["mutations"] == robust["corpus"]["frames"] == 12288,
    )
    ledger["data_integrity_pass"] = (
        all(ledger["metadata_consistency"].values())
        and all(b["exists"] for b in ledger["bundles"].values())
        and not ledger["totals"]["declared_data_mismatches"]
        and robust["corpus"]["matches_regeneration"]
        and robust["corpus"]["matches_recorded"]
        and robust["corpus"]["frames"] == 12288
        and robust["retained_binary_match"]
        and robust["runner_path_mapping"]["exact"]
        and all(ledger["bundles"]["fr07-desktop-import"]["manifest_checks"].values())
        and all(inter["manifest_checks"].values())
        and all(x["match"] for x in inter["trace_checks"])
        and pr["manifest_checks"]["binary"]
        and pr["manifest_checks"]["accept_log"]
        and pr["manifest_checks"]["synthetic_config_count"] == 12
    )
    args.output.write_text(json.dumps(ledger, ensure_ascii=False, indent=2) + "\n")
    for name, item in ledger["bundles"].items():
        print(
            name,
            len(item["files"]),
            item.get("source_counts"),
            item.get("manifest_checks"),
        )
    return 0 if ledger["data_integrity_pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
