"""Проверка TUN lifecycle в отдельном user/network namespace, без sudo."""

from __future__ import annotations

import argparse
import json
import logging
import os
import signal
import subprocess
import sys
import time
from pathlib import Path

LOG = logging.getLogger("tofu.smoke")
FIXTURES = Path(__file__).resolve().parents[1] / "tests" / "engines"


def links() -> set[str]:
    result = subprocess.run(
        ["ip", "-j", "link", "show"], check=True, capture_output=True, timeout=5
    )
    return {item["ifname"] for item in json.loads(result.stdout)}


def worker(binary: Path, engine: str, host_namespace: int) -> None:
    if os.stat("/proc/self/ns/net").st_ino == host_namespace:
        raise RuntimeError("Изоляция сетевого namespace не выполнена")
    baseline = links()
    if baseline != {"lo"}:
        raise RuntimeError("Ожидается новый пустой network namespace")
    config = FIXTURES / f"{engine}-tun.json"
    command = [str(binary), "run", "-c", str(config)]
    # Логи только синтетического fixture; пользовательские профили не принимаются.
    for crash in (False, True):
        with subprocess.Popen(
            command, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE
        ) as process:
            try:
                deadline = time.monotonic() + 5
                while "tofu-test" not in links():
                    if process.poll() is not None:
                        raise RuntimeError("Ядро завершилось до создания TUN")
                    if time.monotonic() >= deadline:
                        raise TimeoutError("Ядро не создало TUN за 5 секунд")
                    time.sleep(0.05)
                if crash:
                    process.kill()
                else:
                    process.terminate()
                process.wait(timeout=5)
                if links() != baseline:
                    raise RuntimeError("После завершения остался интерфейс")
                LOG.info(
                    "%s: TUN создан и удалён после %s",
                    engine,
                    "SIGKILL" if crash else "SIGTERM",
                )
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=5)
                if process.stderr is not None:
                    diagnostics = process.stderr.read(65536).decode(errors="replace")
                    if diagnostics:
                        LOG.debug("%s", diagnostics)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("engine", choices=("xray", "sing-box"))
    parser.add_argument("binary", type=Path)
    parser.add_argument("--worker-namespace", type=int, help=argparse.SUPPRESS)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    if args.worker_namespace is not None:
        worker(binary, args.engine, args.worker_namespace)
        return 0
    config = FIXTURES / f"{args.engine}-tun.json"
    check = (
        [str(binary), "run", "-test", "-c", str(config)]
        if args.engine == "xray"
        else [str(binary), "check", "-c", str(config)]
    )
    subprocess.run(check, check=True, timeout=15, capture_output=True)
    LOG.info("%s: config-check пройден", args.engine)
    command = [
        "unshare",
        "--user",
        "--map-root-user",
        "--net",
        sys.executable,
        str(Path(__file__).resolve()),
        args.engine,
        str(binary),
        "--worker-namespace",
        str(os.stat("/proc/self/ns/net").st_ino),
    ]
    # Таймаут должен завершить и worker, и ядро, удерживающее namespace.
    with subprocess.Popen(command, start_new_session=True) as sandbox:
        try:
            code = sandbox.wait(timeout=30)
        except subprocess.TimeoutExpired:
            os.killpg(sandbox.pid, signal.SIGKILL)
            sandbox.wait(timeout=5)
            raise
        if code:
            raise subprocess.CalledProcessError(code, command)
    return 0


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    try:
        raise SystemExit(main())
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        LOG.error("Проверка не пройдена: %s", error)
        raise SystemExit(1) from None
