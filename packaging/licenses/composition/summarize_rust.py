"""Связь exact-build artifacts/normal-build roles/link map; не карта всех retained symbols."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import re
import sys
import tomllib
from pathlib import Path


def sha(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main() -> None:
    evidence, output = map(Path, sys.argv[1:])
    root = Path(__file__).resolve().parents[3]
    spec = importlib.util.spec_from_file_location(
        "composition", root / "tools/check_linux_composition.py"
    )
    assert spec and spec.loader
    scanner = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(scanner)
    metadata = json.loads((evidence / "metadata.json").read_text())
    inventory = json.loads(
        (root / "packaging/licenses/linux-inventory.json").read_text()
    )
    lock = tomllib.loads((evidence / "Cargo.lock").read_text())
    checksums = {
        (p["name"], p["version"], p.get("source")): p.get("checksum")
        for p in lock["package"]
    }
    artifacts = [
        json.loads(line)
        for line in (evidence / "cargo-artifacts.jsonl").read_text().splitlines()
    ]
    invocations = [
        json.loads(line)
        for line in (evidence / "rustc-invocations.jsonl").read_text().splitlines()
    ]
    library_inputs = {}
    for record in artifacts:
        if record.get("reason") != "compiler-artifact":
            continue
        for filename in record["filenames"]:
            if filename.endswith(".rlib"):
                library_inputs[Path(filename).name] = record["package_id"]
    inventory_identities = {
        (row["name"], row["version"], row["source"]) for row in inventory["packages"]
    }
    features_by_id = {
        node["id"]: node["features"] for node in metadata["resolve"]["nodes"]
    }
    binaries = []
    for binary, package in [
        ("yonik-desktop", "yonik-desktop"),
        ("yonik-helper", "yonik-linux"),
        ("yonik-worker", "yonik-linux"),
    ]:
        selected = [p for p in metadata["packages"] if p["name"] == package]
        if len(selected) != 1:
            raise ValueError("неоднозначный root package")
        roles = scanner.dependency_roles(metadata, selected[0]["id"])
        map_path = evidence / (binary.replace("-", "_") + ".map")
        linkmap = map_path.read_text()
        retained = scanner.retained_archives(linkmap)
        inputs = sorted(set(re.findall(r"lib[A-Za-z0-9_]+-[0-9a-f]+\.rlib", linkmap)))
        linked = [
            {
                "archive": name,
                "package_id": library_inputs.get(name),
                "retained_input_sections": retained.get(name),
                "observation": "упоминание archive в map; возможны discarded sections",
            }
            for name in inputs
        ]
        calls = [
            r
            for r in invocations
            if r["crate"] == binary.replace("-", "_") and r["crate_type"] == "bin"
        ]
        if not calls:
            raise ValueError("нет exact rustc invocation")
        executable = evidence / "new-binaries" / binary
        rows = []
        for p in metadata["packages"]:
            if p["id"] not in roles:
                continue
            rows.append(
                {
                    "id": p["id"],
                    "name": p["name"],
                    "version": p["version"],
                    "source": p["source"],
                    "inventory_identity_matches": (p["name"], p["version"], p["source"])
                    in inventory_identities,
                    "lock_checksum": checksums.get(
                        (p["name"], p["version"], p["source"])
                    ),
                    "roles": roles[p["id"]],
                    "metadata_features": features_by_id[p["id"]],
                    "limits": "features metadata union; exact --cfg сохранены в rustc invocation log",
                }
            )
        binaries.append(
            {
                "binary": binary,
                "sha256": sha(executable),
                "elf": scanner.elf(executable.read_bytes()),
                "map_sha256": sha(map_path),
                "source_dependency_roles": rows,
                "archive_mentions": linked,
                "native_retained_archives": {
                    name: value
                    for name, value in retained.items()
                    if name.endswith(".a")
                },
                "final_rustc": calls[-1],
            }
        )
    report = {
        "schema": 1,
        "status": "observed_new_build",
        "source_commit": "eec7a56b6b03f2b022a87682f9097ce78b61fa0b",
        "source_archive_sha256": "71e9c347ffc363da062b42b0c5ffc6ccb0a97b0988202ce21391dd1c2553a1d3",
        "rust_inventory_sha256": sha(root / "packaging/licenses/linux-inventory.json"),
        "lock_sha256": sha(evidence / "Cargo.lock"),
        "inventory_lock_matches": sha(evidence / "Cargo.lock")
        == inventory["provenance"]["Cargo.lock"],
        "evidence_sha256": {
            path.name: sha(path) for path in evidence.iterdir() if path.is_file()
        },
        "binaries": binaries,
        "limits": [
            "Новые ELF не равны preserved ELF; map не переносится на package78 как доказательство байтов",
            "Normal/build roles — source graph; macro expansion и static retained/dead-code требуют отдельной атрибуции",
            "Archive mentions включают discarded/LOAD, не означают все retained symbols",
            "Rust std/compiler_builtins не входят в 225 registry/workspace inventory; нужны toolchain notices",
            "Additive wrapper пишет map; четыре --undefined flags из принятой package78 recipe сохранены; target задан явно",
        ],
    }
    output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")


if __name__ == "__main__":
    main()
