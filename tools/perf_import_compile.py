"""Ограниченный Linux P-04 runner: отдельный child и wait4 для каждого замера."""

import argparse
import hashlib
import json
import os
import platform
import signal
import subprocess
import tempfile
import time
from pathlib import Path

SEED = 407


def fixture(count: int) -> bytes:
    """Детерминированные offline URI без настоящих учётных данных."""
    if count not in (1000, 10000):
        raise ValueError("Нагрузка вне границ")
    return "".join(
        f"trojan://fixture-{SEED}-{i}@server-{i}.example.invalid:443\n"
        for i in range(count)
    ).encode()


def run_child(command: list[str], body: bytes, timeout: float) -> dict:
    """wait4 возвращает CPU/RSS именно этого child; вывод ограничен RLIMIT_FSIZE."""
    import resource

    if len(body) > 8 * 1024 * 1024 or not 0 < timeout <= 60:
        raise ValueError("Вход или timeout вне границ")

    def limits() -> None:
        resource.setrlimit(resource.RLIMIT_FSIZE, (65536, 65536))
        resource.setrlimit(resource.RLIMIT_CPU, (60, 60))

    with tempfile.TemporaryFile() as source, tempfile.TemporaryFile() as output:
        source.write(body)
        source.seek(0)
        start = time.monotonic()
        child = subprocess.Popen(
            command,
            stdin=source,
            stdout=output,
            stderr=output,
            start_new_session=True,
            preexec_fn=limits,
        )
        deadline = start + timeout
        cancelled = False
        while True:
            pid, status, usage = os.wait4(child.pid, os.WNOHANG)
            if pid:
                child.returncode = os.waitstatus_to_exitcode(status)
                break
            if time.monotonic() >= deadline and not cancelled:
                os.killpg(child.pid, signal.SIGKILL)
                cancelled = True
            time.sleep(0.005)
        elapsed = time.monotonic() - start
        output.seek(0)
        raw = output.read(65537)
    result = {
        "wall_s": elapsed,
        "cpu_s": usage.ru_utime + usage.ru_stime,
        "peak_rss_kib": usage.ru_maxrss,
        "cancelled": cancelled,
        "exit": child.returncode,
    }
    if not cancelled:
        if child.returncode or len(raw) > 65536:
            raise RuntimeError(raw.decode(errors="replace"))
        result["result"] = json.loads(raw)
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    sources = [
        Path(p)
        for p in subprocess.check_output(
            [
                "git",
                "ls-files",
                "crates/yonik-engine",
                "crates/yonik-import",
                "crates/yonik-domain",
                "Cargo.lock",
            ]
        )
        .decode()
        .splitlines()
    ]
    sources += [
        Path(__file__),
        Path("crates/yonik-engine/examples/import_compile_resource.rs"),
    ]
    manifest = {
        "seed": SEED,
        "head": subprocess.check_output(["git", "rev-parse", "HEAD"]).decode().strip(),
        "platform": platform.platform(),
        "loadavg": os.getloadavg(),
        "process_names": subprocess.check_output(["ps", "-eo", "comm"])
        .decode()
        .splitlines(),
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "sources": {
            str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in sources
        },
        "fixtures": {
            str(n): hashlib.sha256(fixture(n)).hexdigest() for n in (1000, 10000)
        },
    }
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    rows = []
    for count in (1000, 10000):
        for rules in (10, 100, 1000):
            for engine in ("xray", "sing-box"):
                command = [str(binary), str(count), str(rules), engine]
                for repeat in range(4):
                    row = {
                        "count": count,
                        "rules": rules,
                        "engine": engine,
                        "repeat": repeat,
                        "warmup": repeat == 0,
                        **run_child(command, fixture(count), 60),
                    }
                    rows.append(row)
                    (args.output / "runs.json").write_text(
                        json.dumps(rows, indent=2) + "\n"
                    )
    cancel = run_child([str(binary), "10000", "1000", "xray"], fixture(10000), 0.001)
    (args.output / "cancel.json").write_text(json.dumps(cancel, indent=2) + "\n")
    if not cancel["cancelled"]:
        raise RuntimeError("Нагрузка завершилась до отмены: доказательства отмены нет")


if __name__ == "__main__":
    main()
