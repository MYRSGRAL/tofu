"""Дополнительные крупные native/Base64 controls через тот же supervisor."""

import argparse
import base64
import hashlib
import json
from pathlib import Path

from campaign import probe


def run(binary: Path, destination: Path) -> None:
    destination.mkdir(parents=True, exist_ok=False)
    paths = [Path(__file__), Path(__file__).with_name("campaign.py"), binary]
    (destination / "before.json").write_text(
        json.dumps({str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths})
    )
    results = []
    for size in (65535, 65536, 65537, 6 << 20, (8 << 20) - 1, 8 << 20, (8 << 20) + 1):
        prefix = b'{"opaque":"CANARY'
        raw = prefix + b"x" * (size - len(prefix) - 2) + b'"}'
        for representation, data in (
            ("native", raw),
            ("base64", base64.b64encode(raw)),
        ):
            result = probe([str(binary)], data)
            expected = "reject TooLarge\n" if len(data) > 8 << 20 else "0 0 0 1\n"
            result.update(
                bytes=len(data),
                representation=representation,
                sha256=hashlib.sha256(data).hexdigest(),
                passed=result["status"] == "exit" and result["output"] == expected,
            )
            results.append(result)
    for size in (63, 64, 65):
        data = f"trojan://CANARY@{'a' * size}.test:443".encode()
        result = probe([str(binary)], data)
        result.update(
            bytes=len(data),
            representation=f"label-{size}",
            sha256=hashlib.sha256(data).hexdigest(),
        )
        results.append(result)
    (destination / "results.json").write_text(json.dumps(results, indent=2))
    if not all(result.get("passed", True) for result in results):
        raise SystemExit("Нарушено ожидание крупного native/Base64 control")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    run(args.binary.resolve(), args.destination)
