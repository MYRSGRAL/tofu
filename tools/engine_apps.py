"""Нативные executable-правила Xray/sing-box в изолированном TUN-стенде."""

from __future__ import annotations

import argparse
import json
import logging
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from engine_smoke import FIXTURES, install_signal_handlers, links, run_sandbox
from engine_traffic import TARGETS, counters, ip
from traffic_echo import echo_servers

LOG = logging.getLogger("tofu.apps")
PROBE = Path(__file__).resolve().with_name("app_probe.py")


def configuration(engine: str, allowed: Path) -> dict:
    config = json.loads((FIXTURES / f"{engine}-traffic.json").read_text())
    if engine == "xray":
        for rule in config["routing"]["rules"]:
            rule["process"] = [str(allowed)]
    else:
        for rule in config["route"]["rules"]:
            if rule.get("action") == "route":
                rule["process_path"] = [str(allowed)]
    return config


def check_client(executable: Path, family: str, transport: str, expected: bool) -> None:
    result = subprocess.run(
        [str(executable), str(PROBE), family, transport],
        capture_output=True,
        text=True,
        timeout=5,
    )
    if result.returncode not in (0, 2):
        raise RuntimeError("Тестовый клиент завершился с внутренней ошибкой")
    report = json.loads(result.stdout)
    if report.get("executable") != str(executable) or report.get("ok") is not expected:
        raise RuntimeError(
            f"Неожиданный результат executable-правила: family={family}, transport={transport}, expected={expected}, report={report}"
        )
    if (result.returncode == 0) is not expected:
        raise RuntimeError("Код завершения клиента расходится с отчётом")


def worker(binary: Path, engine: str, host_namespace: int, work_dir: Path) -> None:
    if os.stat("/proc/self/ns/net").st_ino == host_namespace or links() != {"lo"}:
        raise RuntimeError("Требуется новый изолированный network namespace")
    ip("link", "set", "lo", "up")
    with (
        tempfile.TemporaryDirectory(prefix="clients-", dir=work_dir) as directory,
        echo_servers(),
    ):
        root = Path(directory)
        clients = []
        for name in ("a", "b"):
            executable = root / name / "probe-python"
            executable.parent.mkdir()
            shutil.copyfile(Path(sys.executable).resolve(), executable)
            executable.chmod(0o700)
            clients.append(executable)
        for allowed, denied in (clients, list(reversed(clients))):
            config_path = root / "config.json"
            config_path.write_text(json.dumps(configuration(engine, allowed)))
            check = (
                [str(binary), "run", "-test", "-c", str(config_path)]
                if engine == "xray"
                else [str(binary), "check", "-c", str(config_path)]
            )
            subprocess.run(check, capture_output=True, check=True, timeout=10)
            with subprocess.Popen(
                [str(binary), "run", "-c", str(config_path)],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.PIPE,
            ) as process:
                try:
                    deadline = time.monotonic() + 5
                    while "tofu-test" not in links():
                        if process.poll() is not None:
                            raise RuntimeError("Ядро завершилось до создания TUN")
                        if time.monotonic() >= deadline:
                            raise TimeoutError("TUN не создан")
                        time.sleep(0.05)
                    ip("address", "replace", "198.18.0.1/30", "dev", "tofu-test")
                    ip(
                        "-6",
                        "address",
                        "replace",
                        "2001:db8:746f::1/126",
                        "dev",
                        "tofu-test",
                        "nodad",
                    )
                    ip("link", "set", "tofu-test", "up")
                    for index, (_, address) in enumerate(TARGETS):
                        family = "6" if index else "4"
                        route = json.loads(
                            ip("-j", "-" + family, "route", "get", address)
                        )
                        if route[0].get("dev") != "tofu-test":
                            raise RuntimeError("Маршрут обходит TUN")
                        for transport in ("tcp", "udp"):
                            before = counters()
                            check_client(allowed, family, transport, True)
                            after = counters()
                            if not all(
                                end > start
                                for start, end in zip(before, after, strict=True)
                            ):
                                raise RuntimeError("Не подтверждён обмен через TUN")
                            before = counters()
                            check_client(denied, family, transport, False)
                            if counters()[1] <= before[1]:
                                raise RuntimeError(
                                    "Заблокированный клиент не отправил поток в TUN"
                                )
                            LOG.info(
                                "%s: IPv%s/%s разрешён %s, заблокирован %s",
                                engine,
                                family,
                                transport,
                                allowed.parent.name,
                                denied.parent.name,
                            )
                    process.terminate()
                    process.wait(timeout=5)
                    if links() != {"lo"}:
                        raise RuntimeError("После завершения остался TUN")
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.wait(timeout=5)
                    if process.stderr is not None:
                        diagnostics = process.stderr.read(65536).decode(
                            errors="replace"
                        )
                        if diagnostics:
                            LOG.debug("%s", diagnostics)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("engine", choices=("xray", "sing-box"))
    parser.add_argument("binary", type=Path)
    parser.add_argument("--work-dir", type=Path, required=True)
    parser.add_argument("--worker-namespace", type=int, help=argparse.SUPPRESS)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    work_dir = args.work_dir.resolve(strict=True)
    if args.worker_namespace is not None:
        worker(binary, args.engine, args.worker_namespace, work_dir)
        return 0
    run_sandbox(
        [
            "unshare",
            "--user",
            "--map-root-user",
            "--net",
            sys.executable,
            str(Path(__file__).resolve()),
            args.engine,
            str(binary),
            "--work-dir",
            str(work_dir),
            "--worker-namespace",
            str(os.stat("/proc/self/ns/net").st_ino),
        ],
        timeout=60,
    )
    return 0


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    install_signal_handlers()
    try:
        raise SystemExit(main())
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        LOG.error("Проверка не пройдена: %s", error)
        raise SystemExit(1) from None
