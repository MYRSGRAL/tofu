"""Wire-контрпример D-07 на pinned cores; не является положительным G-DNS."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import socket
import sys
import threading
import time
from contextlib import contextmanager
from pathlib import Path
from typing import Iterator

import fakeip_epoch_gap as probe
from dns_fixture import dns_query, engine_process, verify_namespace
from engine_compile import wait_listener
from engine_smoke import run_sandbox

PINS = {
    "xray": "cc4b84d3069569f297a61f04b914f2bb37e9d202fd7b84cb50def1fc30fd40e1",
    "sing-box": "4c31db07cb394479b58cc9aace8c8bef5da05a73db6dacca4a019aeb8b14e16d",
}


@contextmanager
def checked_markers() -> Iterator[list[str]]:
    """Ошибки потоков и незавершённый cleanup запрещают успешный результат."""
    stop = threading.Event()
    hits: list[str] = []
    errors: list[str] = []
    listeners: list[socket.socket] = []
    threads: list[threading.Thread] = []

    def serve(listener: socket.socket, marker: str) -> None:
        try:
            while not stop.is_set():
                try:
                    connection, _ = listener.accept()
                except socket.timeout:
                    continue
                with connection:
                    connection.settimeout(1)
                    hits.append(marker)
                    connection.sendall(marker.encode("ascii"))
        except Exception as error:
            errors.append(repr(error))

    try:
        for index in range(probe.LIMIT + 1):
            listener = socket.socket()
            listeners.append(listener)
            listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            listener.bind(("127.0.0.1", 24100 + index))
            listener.listen(8)
            listener.settimeout(0.1)
            thread = threading.Thread(target=serve, args=(listener, f"M{index:02d}"))
            threads.append(thread)
            thread.start()
        yield hits
    finally:
        stop.set()
        for thread in threads:
            thread.join(timeout=2)
        for listener in listeners:
            listener.close()
        if errors or any(thread.is_alive() for thread in threads):
            raise RuntimeError(f"Ошибка marker fixture: {errors}")


def xray_expiry(binary: Path, output: Path) -> dict:
    """Реальное истечение wire TTL, без fixture clock и без переполнения pool."""
    config = probe.config("xray")
    config["fakedns"] = [{"ipPool": "198.18.0.0/15", "poolSize": 65535}]
    with checked_markers() as hits:
        with engine_process(binary, "xray", config, output) as process:
            wait_listener(process, 10808)
            ttls: list[int] = []
            addresses = dns_query(
                "name-0.fakeip.test", qtype=1, port=1053, ttl_capture=ttls
            )
            if len(addresses) != 1 or len(ttls) != 1 or not 0 < ttls[0] <= 2:
                raise RuntimeError("Неожиданный wire TTL/ответ Xray")
            started = time.monotonic()
            if probe.connect(addresses[0]) != "M00":
                raise RuntimeError("Нет положительного marker до TTL")
            time.sleep(ttls[0] + 0.1)
            elapsed = time.monotonic() - started
            after = probe.connect(addresses[0])
            if elapsed <= ttls[0] or after != "M00" or len(hits) != 2:
                raise RuntimeError("Контрпример TTL expiry не воспроизведён")
            if process.poll() is not None:
                raise RuntimeError("Ядро завершилось во время expiry probe")
    return {
        "ttl": ttls[0],
        "elapsed": elapsed,
        "after_expiry_marker": after,
        "pool": "198.18.0.0/15",
        "pool_size": 65535,
        "allocations": 1,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("xray", type=Path)
    parser.add_argument("sing_box", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--worker", type=int)
    args = parser.parse_args()
    binaries = {"xray": args.xray.resolve(), "sing-box": args.sing_box.resolve()}
    for engine, path in binaries.items():
        with path.open("rb") as source:
            if hashlib.file_digest(source, "sha256").hexdigest() != PINS[engine]:
                raise RuntimeError(f"Неверный pinned binary: {engine}")
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
            ],
            timeout=90,
        )
        return
    verify_namespace(args.worker)
    # Сохраняем старый bounded wire oracle, заменяя непроверяемые marker threads.
    probe.markers = checked_markers
    results = [
        probe.run_engine(binary, engine, output / engine)
        for engine, binary in binaries.items()
    ]
    manifest = {
        "classification": "NativePinnedCoreBlocker",
        "compiler_result": "Unsupported(DnsContract), отдельный факт",
        "pins": PINS,
        "xray_actual_expiry": xray_expiry(binaries["xray"], output / "xray-expiry"),
        "results": results,
        "limitations": [
            "Малый валидный pool, не production default pool",
            "TTL expiry проверен только для Xray; network reset не проверен",
            "Не доказывает положительные exclusions и полный G-DNS",
            "None старой connect-пробы не доказывает protocol refusal: timeout не PASS",
            "Restart/reuse без production Stamp не доказывает нарушение epoch fence",
        ],
    }
    (output / "result.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n"
    )


if __name__ == "__main__":
    main()
