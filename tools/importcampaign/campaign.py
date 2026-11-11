"""Ограниченная Linux campaign без внешних зависимостей и пользовательских данных."""

import argparse
import base64
import collections
import hashlib
import json
import os
import random
import re
import resource
import selectors
import signal
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def limits(memory: int, cpu: int, output: int) -> None:
    resource.setrlimit(resource.RLIMIT_AS, (memory, memory))
    resource.setrlimit(resource.RLIMIT_CPU, (cpu, cpu))
    resource.setrlimit(resource.RLIMIT_FSIZE, (output, output))
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))


def probe(
    command: list[str], data: bytes, wall: float = 3, memory: int = 256 << 20
) -> dict:
    """Nonblocking pipes ограничивают и вход, и вывод; finally всегда kill/reap группы."""
    start = time.monotonic()
    process = subprocess.Popen(
        command,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        start_new_session=True,
        preexec_fn=lambda: limits(memory, 2, 16384),
    )
    output = bytearray()
    offset = 0
    status = "exit"
    selector = selectors.DefaultSelector()
    assert process.stdin is not None and process.stdout is not None
    for stream in (process.stdin, process.stdout):
        os.set_blocking(stream.fileno(), False)
    selector.register(process.stdin, selectors.EVENT_WRITE)
    selector.register(process.stdout, selectors.EVENT_READ)
    try:
        while selector.get_map():
            if time.monotonic() - start >= wall:
                status = "timeout"
                break
            for key, _ in selector.select(0.02):
                if key.fileobj is process.stdin:
                    try:
                        offset += os.write(
                            process.stdin.fileno(), data[offset : offset + 65536]
                        )
                    except BrokenPipeError:
                        offset = len(data)
                    if offset == len(data):
                        selector.unregister(process.stdin)
                        process.stdin.close()
                else:
                    chunk = os.read(process.stdout.fileno(), 4096)
                    if not chunk:
                        selector.unregister(process.stdout)
                    output.extend(chunk)
                    if len(output) > 16384:
                        status = "output_limit"
                        break
            if status != "exit":
                break
    finally:
        os.killpg(process.pid, signal.SIGKILL)
        _, code, usage = os.wait4(process.pid, 0)
        process.returncode = os.waitstatus_to_exitcode(code)
        selector.close()
        process.stdout.close()
        if not process.stdin.closed:
            process.stdin.close()
    if status == "exit" and process.returncode:
        allocator_failure = (
            re.search(
                rb"(?:^|\n)(?:MemoryError|memory allocation of [1-9][0-9]* bytes failed)\r?\n?$",
                output,
            )
            is not None
        )
        status = "memory_failure" if allocator_failure else "crash"
    return {
        "status": status,
        "code": process.returncode,
        "seconds": time.monotonic() - start,
        "rss_kib": usage.ru_maxrss,
        "output": output[:16384].decode(errors="replace"),
    }


def cases() -> list[tuple[str, bytes, str]]:
    corpus = ROOT / "crates/yonik-import/tests/corpus/p04-seeded"
    result = []
    for name in ("uri.txt", "xray.json", "sing-box.json"):
        raw = (corpus / name).read_bytes()
        for representation, data in (
            ("raw", raw),
            ("b64", base64.b64encode(raw)),
            ("url64", base64.urlsafe_b64encode(raw).rstrip(b"=")),
        ):
            result.append(
                (
                    name + representation,
                    data,
                    "opaque" if name == "sing-box.json" else "review",
                )
            )
    for host in ("127.0.0.1", "[2001:db8::1]", "пример.рф"):
        result.append(
            ("valid-" + host, f"trojan://CANARY@{host}:443".encode(), "accept")
        )
    for n in (65535, 65536, 65537, (8 << 20) - 1, 8 << 20, (8 << 20) + 1):
        result.append(
            (
                f"size-{n}",
                b"trojan://CANARY@example.test:443#"
                + b"x" * (n - len(b"trojan://CANARY@example.test:443#")),
                "any",
            )
        )
    for n in (63, 64, 65, 128):
        result.append(
            (f"depth-{n}", b'{"opaque":' + b"[" * n + b"0" + b"]" * n + b"}", "any")
        )
    for n in (19999, 20000, 20001):
        result.append((f"records-{n}", b"socks5://127.0.0.1:1080\n" * n, "any"))
    for host in ("999.1.1.1", "[2001:::1]", "a" * 64 + ".test", "\u200d.test"):
        result.append(
            (
                "host-" + host,
                f"trojan://CANARY@{host}:443?unknown=CANARY".encode(),
                "any",
            )
        )
    return result


def run(
    binary: Path, destination: Path, duration: float, seed: int, replay: int | None
) -> None:
    destination.mkdir(parents=True, exist_ok=False)
    sources = [
        Path(__file__),
        ROOT / "Cargo.lock",
        ROOT / "Cargo.toml",
        ROOT / "crates/yonik-import/Cargo.toml",
    ]
    sources += sorted((ROOT / "crates/yonik-import").rglob("*.rs"))
    sources += sorted((ROOT / "crates/yonik-import/tests/corpus").rglob("*.*"))
    manifest = {
        str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in sources
    }
    manifest["binary"] = hashlib.sha256(binary.read_bytes()).hexdigest()
    (destination / "before.json").write_text(
        json.dumps(
            {
                "sources": manifest,
                "seed": seed,
                "binary": str(binary),
                "duration": duration,
            },
            indent=2,
        )
    )
    rng = random.Random(seed)
    seeds = cases()
    counts = collections.Counter()
    started = time.monotonic()
    index = 0
    with (destination / "cases.jsonl").open("w") as log:
        while (replay is not None and index <= replay) or (
            replay is None
            and (time.monotonic() - started < duration or index < len(seeds))
        ):
            name, data, expected = seeds[index % len(seeds)]
            if index >= len(seeds):
                expected = "any"
                data = bytearray(data)
                for _ in range(rng.randrange(1, 17)):
                    if not data:
                        data.append(0)
                    pos = rng.randrange(len(data))
                    operation = rng.randrange(3)
                    alphabet = b'0123456789abcdef%\\"{}[]=@?#\x00'
                    value = rng.choice(alphabet)
                    if operation == 0:
                        data[pos] = value
                    elif operation == 1:
                        del data[pos]
                    else:
                        data.insert(pos, value)
                data = bytes(data)
            if replay is None or replay == index:
                observation = probe([str(binary)], data)
                status = observation["status"]
                if status == "exit":
                    parts = observation["output"].split()
                    if parts[0] == "reject":
                        status = "rejected"
                    else:
                        accepted, rejected, reviewed, native = map(int, parts)
                        counts.update(
                            accepted_records=accepted,
                            rejected_records=rejected,
                            reviewed_records=reviewed,
                        )
                        status = "parsed"
                        if expected == "accept" and not accepted:
                            status = "control_failure"
                        if expected == "opaque" and not native:
                            status = "control_failure"
                        if expected == "review" and not reviewed:
                            status = "control_failure"
                    if (
                        expected in ("accept", "review", "opaque")
                        and status == "rejected"
                    ):
                        status = "control_failure"
                counts[f"{status}_calls"] += 1
                observation.update(
                    index=index,
                    family=name,
                    expected=expected,
                    status=status,
                    sha256=hashlib.sha256(data).hexdigest(),
                    bytes=len(data),
                )
                log.write(json.dumps(observation) + "\n")
                log.flush()
                if status not in ("parsed", "rejected"):
                    (destination / f"failure-{index}.bin").write_bytes(data)
                if replay is not None:
                    break
            index += 1
    (destination / "summary.json").write_text(
        json.dumps(
            {
                "format_version": 2,
                "counts": counts,
                "cases": index,
                "seconds": time.monotonic() - started,
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--seconds", type=float, default=600)
    parser.add_argument("--seed", type=int, default=0x5040465230372026)
    parser.add_argument("--replay", type=int)
    args = parser.parse_args()
    if args.replay is not None and not 0 <= args.replay <= 1000000:
        parser.error("Replay index должен быть в пределах 0..1000000")
    if not 0 < args.seconds <= 1800:
        parser.error("Продолжительность должна быть в пределах (0, 1800]")
    run(args.binary.resolve(), args.destination, args.seconds, args.seed, args.replay)
