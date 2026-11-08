"""Один поток тестового executable: только синтетические адреса стенда."""

from __future__ import annotations

import argparse
import json
import socket
from pathlib import Path

from engine_traffic import TARGETS, probe


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("family", choices=("4", "6"))
    parser.add_argument("transport", choices=("tcp", "udp"))
    args = parser.parse_args()
    family, address = TARGETS[1 if args.family == "6" else 0]
    result: dict[str, str | bool] = {
        "executable": str(Path("/proc/self/exe").resolve())
    }
    try:
        probe(
            family,
            address,
            socket.SOCK_STREAM if args.transport == "tcp" else socket.SOCK_DGRAM,
        )
    except OSError as error:
        result.update(ok=False, error=type(error).__name__)
        # stdout — машинный протокол стенда, диагностические логи сюда не идут.
        print(json.dumps(result))
        return 2
    result["ok"] = True
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
