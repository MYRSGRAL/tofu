"""Один bounded offline image reader; никаких mount/replay/repair/VM."""

from __future__ import annotations

import fcntl
import hashlib
import json
import os
import re
import secrets
import select
import shutil
import struct
import subprocess
import sys
import time
import uuid
import zlib
from pathlib import Path

BASE = Path(__file__).resolve().parents[1]
SUMMARY = BASE / "docs/evidence/a4-bootstrap-diagnostic-142/run1/summary.json"
CACHE = Path("/home/myrs/projects/yonik/local/validation/vm-base")
IMAGE = CACHE / "Arch-Linux-x86_64-cloudimg-20261101.620452.qcow2"
BASE_SHA = "794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003"
JOURNAL_UNITS = {
    "cloud-init.service",
    "cloud-init-main.service",
    "cloud-init-local.service",
    "cloud-config.service",
    "cloud-final.service",
    "systemd-time-wait-sync.service",
    "systemd-timesyncd.service",
    "sshd.service",
}


def sha(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def check_window(start: int, length: int, size: int) -> None:
    if start < 0 or length <= 0 or start % 512 or length % 512 or start + length > size:
        raise ValueError("Image window outside checked sector bounds")


def window_args(
    source: Path, target: Path, start: int, length: int, size: int, fmt: str = "qcow2"
) -> list[str]:
    check_window(start, length, size)
    if (
        fmt not in ("qcow2", "raw")
        or any(c in str(source) for c in ",\n\r")
        or target.exists()
    ):
        raise ValueError("Unsafe window source/target")
    options = f"driver=raw,offset={start},size={length},file.driver={fmt},file.file.driver=file,file.file.filename={source}"
    return [
        "/usr/bin/qemu-img",
        "convert",
        "--image-opts",
        "-O",
        "raw",
        "-S",
        "4096",
        options,
        str(target),
    ]


def gpt_partitions(data: bytes, virtual_size: int) -> list[dict]:
    if len(data) < 1024 or virtual_size % 512 or data[510:512] != b"\x55\xaa":
        raise ValueError("Invalid protective MBR/size")
    header = data[512:1024]
    if header[:8] != b"EFI PART":
        raise ValueError("No GPT")
    length, crc = struct.unpack_from("<II", header, 12)
    if not 92 <= length <= 512:
        raise ValueError("GPT header size")
    checked = bytearray(header[:length])
    checked[16:20] = b"\0" * 4
    if zlib.crc32(checked) & 0xFFFFFFFF != crc:
        raise ValueError("GPT header CRC")
    current, backup, first, last = struct.unpack_from("<QQQQ", header, 24)
    table_lba, count, entry_size, table_crc = struct.unpack_from("<QIII", header, 72)
    if (
        current != 1
        or not 1 < backup < virtual_size // 512
        or not 2 <= first <= last < virtual_size // 512
    ):
        raise ValueError("GPT LBA bounds")
    if not 1 <= count <= 1024 or not 128 <= entry_size <= 1024 or entry_size % 8:
        raise ValueError("GPT entry limits")
    offset = table_lba * 512
    size = count * entry_size
    if table_lba < 2 or offset + size > len(data) or offset + size > first * 512:
        raise ValueError("GPT table outside bounded metadata read")
    table = data[offset : offset + size]
    if zlib.crc32(table) & 0xFFFFFFFF != table_crc:
        raise ValueError("GPT table CRC")
    rows = []
    for index in range(count):
        entry = table[index * entry_size : (index + 1) * entry_size]
        if entry[:16] == b"\0" * 16:
            continue
        start, end = struct.unpack_from("<QQ", entry, 32)
        if not first <= start <= end <= last:
            raise ValueError("Partition LBA bounds")
        check_window(start * 512, (end - start + 1) * 512, virtual_size)
        row = {
            "index": index + 1,
            "type": str(uuid.UUID(bytes_le=entry[:16])),
            "start": start * 512,
            "length": (end - start + 1) * 512,
            "name": entry[56:128].decode("utf-16-le").rstrip("\0"),
        }
        if any(
            not (
                row["start"] + row["length"] <= other["start"]
                or other["start"] + other["length"] <= row["start"]
            )
            for other in rows
        ):
            raise ValueError("Overlapping partitions")
        rows.append(row)
    return rows


def allowed_guest_path(path: str) -> None:
    if (
        not path.startswith("/")
        or any(part in (".", "..") for part in path.split("/"))
        or any(c in path for c in '\n\r\0"')
    ):
        raise ValueError("Unsafe guest path")
    if re.fullmatch(
        r"/(usr/lib|etc|run)/systemd/(system|generator(?:\.early|\.late)?)(?:/[A-Za-z0-9@_.:-]+(?:/[A-Za-z0-9@_.:-]+)?)?",
        path,
    ):
        leaf = path.rsplit("/", 1)[-1]
        if leaf in (
            "system",
            "generator",
            "generator.early",
            "generator.late",
        ) or leaf.endswith(
            (".service", ".socket", ".target", ".conf", ".d", ".wants", ".requires")
        ):
            return
    if re.fullmatch(
        r"/var/log/journal(?:/[a-f0-9]{32}(?:/[A-Za-z0-9@_.~-]+\.journal~?)?)?", path
    ):
        return
    if path in ("/var/log/cloud-init.log", "/var/log/cloud-init-output.log"):
        return
    raise ValueError("Guest path outside unit/cloud-init/journal allowlist")


def sanitize_text(text: str, nonce: str) -> str:
    if re.search(r"-----BEGIN [A-Z ]*PRIVATE KEY-----", text):
        return "[private key payload redacted]\n"
    lines = []
    for line in text.splitlines():
        if re.search(
            r"(PRIVATE KEY|ssh-(ed25519|rsa)\s|ecdsa-sha2|password|passwd|authorized_keys|ssh_host_|printf\s+%s|bootcmd.*nonce)",
            line,
            re.I,
        ):
            lines.append("[credential/bootstrap payload redacted]")
            continue
        line = line.replace(nonce, "[bootstrap-nonce-redacted]") if nonce else line
        line = re.sub(
            r"(?<![0-9a-f])[0-9a-f]{32}(?![0-9a-f])",
            "[guest-id-redacted]",
            line,
            flags=re.I,
        )
        lines.append(line)
    return "\n".join(lines) + "\n"


def sanitize_journal(value: dict, nonce: str) -> dict | None:
    if (
        str(value.get("_PID")) != "1"
        and value.get("_SYSTEMD_UNIT") not in JOURNAL_UNITS
    ):
        return None
    selected = {
        key: value[key]
        for key in (
            "__MONOTONIC_TIMESTAMP",
            "_PID",
            "_SYSTEMD_UNIT",
            "SYSLOG_IDENTIFIER",
            "PRIORITY",
        )
        if key in value
    }
    selected["MESSAGE"] = sanitize_text(str(value.get("MESSAGE", "")), nonce).strip()
    return selected


def checked_journal_directory(path: Path, root: Path) -> Path:
    canonical = path.resolve()
    if (
        path != canonical
        or not canonical.is_relative_to(root.resolve())
        or not path.is_dir()
        or path.is_symlink()
    ):
        raise ValueError("Only canonical own extracted journal directory")
    return canonical


def source_references(paths: list[Path]) -> list[dict]:
    results = []
    needles = [str(p) for p in paths]
    for process in Path("/proc").iterdir():
        if not process.name.isdigit() or int(process.name) == os.getpid():
            continue
        try:
            if process.stat().st_uid != os.getuid():
                continue
            for fd in (process / "fd").iterdir():
                try:
                    target = os.readlink(fd)
                    if any(n in target for n in needles):
                        results.append({"pid": int(process.name), "kind": "fd"})
                except OSError:
                    pass
            try:
                if any(n in (process / "maps").read_text() for n in needles):
                    results.append({"pid": int(process.name), "kind": "maps"})
            except OSError:
                pass
            for kind in ("cwd", "exe"):
                try:
                    if any(n in os.readlink(process / kind) for n in needles):
                        results.append({"pid": int(process.name), "kind": kind})
                except OSError:
                    pass
        except OSError:
            pass
    return results


class Extraction:
    def __init__(self) -> None:
        self.tag = secrets.token_hex(4)
        self.out = BASE / "local/validation/a4-offline-bootstrap" / self.tag
        self.out.mkdir(mode=0o700, parents=True)
        self.task = Path("/var/tmp/codex-myrs/transient") / (
            "a4-offline-extract-" + self.tag
        )
        self.started = time.monotonic()
        self.fs = ""
        self.root = None
        self.commands = 0
        summary = json.loads(SUMMARY.read_text())
        self.overlay = Path(summary["failed_overlay"]["path"])
        self.overlay_sha = summary["failed_overlay"]["sha256"]
        self.nonce = json.loads((self.overlay.parent / "active.json").read_text())[
            "nonce"
        ]
        self.inventory = {}
        self.original_stats = {}

    def run(self, argv: list[str], timeout: int = 60) -> str:
        if time.monotonic() - self.started > 900:
            raise TimeoutError("Offline extraction deadline900s")
        if argv[0] not in (
            "/usr/bin/qemu-img",
            "/usr/bin/debugfs",
            "/usr/bin/btrfs",
            "/usr/bin/journalctl",
        ):
            raise ValueError("Tool outside existing inventory")
        self.commands += 1
        started = time.monotonic_ns()
        process = subprocess.Popen(
            argv,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env={"PATH": "/usr/bin", "LC_ALL": "C", "LANG": "C"},
        )
        try:
            stdout, stderr = process.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            process.kill()
            stdout, stderr = process.communicate()
            raise TimeoutError("Bounded offline tool timeout") from None
        row = {
            "argv": argv,
            "pid": process.pid,
            "exit": process.returncode,
            "start_ns": started,
            "end_ns": time.monotonic_ns(),
            "stdout": stdout,
            "stderr": stderr,
        }
        with (self.out / "commands-private.jsonl").open("a") as stream:
            stream.write(json.dumps(row) + "\n")
        if process.returncode:
            raise ValueError(
                "Offline tool failed: "
                + json.dumps(
                    {
                        "argv": argv,
                        "exit": process.returncode,
                        "stderr": sanitize_text(stderr, self.nonce),
                    }
                )
            )
        return stdout

    def prepare(self) -> dict:
        if os.getuid() != os.geteuid() or os.getuid() == 0:
            raise ValueError("Normal UID required")
        for name in ("qemu-img", "debugfs", "btrfs", "e2ls", "e2cp", "journalctl"):
            path = Path("/usr/bin") / name
            self.inventory[name] = {
                "present": path.is_file(),
                "sha256": sha(path) if path.is_file() else None,
                "uid": path.stat().st_uid if path.exists() else None,
            }
        if (
            not self.inventory["qemu-img"]["present"]
            or not self.inventory["journalctl"]["present"]
        ):
            raise ValueError("Required existing tool absent")
        for path, expected in [(self.overlay, self.overlay_sha), (IMAGE, BASE_SHA)]:
            if path.is_symlink() or sha(path) != expected:
                raise ValueError("Immutable input SHA mismatch")
            st = path.stat()
            self.original_stats[str(path)] = (
                st.st_dev,
                st.st_ino,
                st.st_size,
                st.st_mtime_ns,
            )
        refs = source_references([self.overlay, IMAGE])
        if refs:
            raise ValueError("Active source/QEMU references: " + json.dumps(refs))
        info = json.loads(
            self.run(["/usr/bin/qemu-img", "info", "--output=json", str(self.overlay)])
        )
        if info.get("backing-filename") != str(IMAGE) or info.get("dirty-flag"):
            raise ValueError("Overlay backing/dirty mismatch")
        baseinfo = json.loads(
            self.run(["/usr/bin/qemu-img", "info", "--output=json", str(IMAGE)])
        )
        if baseinfo.get("backing-filename"):
            raise ValueError("Unexpected base chain")
        self.virtual = info["virtual-size"]
        metadata = self.task / "gpt.raw"
        self.run(
            [
                "/usr/bin/qemu-img",
                "dd",
                "-f",
                "qcow2",
                "-O",
                "raw",
                "bs=512",
                "count=2048",
                "if=" + str(self.overlay),
                "of=" + str(metadata),
            ]
        )
        if metadata.stat().st_size != 1048576:
            raise ValueError("Metadata dd size mismatch")
        partitions = gpt_partitions(metadata.read_bytes(), self.virtual)
        probes = []
        for row in partitions:
            if row["type"] in (
                "c12a7328-f81f-11d2-ba4b-00a0c93ec93b",
                "21686148-6449-6e6f-744e-656564454649",
            ):
                continue
            size = min(row["length"], 131072)
            size = size // 512 * 512
            if size < 66560:
                continue
            target = self.task / ("probe-" + str(row["index"]) + ".raw")
            self.run(
                window_args(self.overlay, target, row["start"], size, self.virtual)
            )
            data = target.read_bytes()
            fs = (
                "ext4"
                if data[1080:1082] == b"\x53\xef"
                else "btrfs"
                if data[65600:65608] == b"_BHRfS_M"
                else "unsupported"
            )
            probes.append({**row, "filesystem": fs})
        supported = [r for r in probes if r["filesystem"] in ("ext4", "btrfs")]
        if len(supported) != 1:
            raise ValueError(
                "Exactly one supported root partition required: " + json.dumps(probes)
            )
        row = supported[0]
        self.fs = row["filesystem"]
        self.root = self.task / "root.raw"
        if not self.inventory["debugfs" if self.fs == "ext4" else "btrfs"]["present"]:
            raise ValueError("Readonly filesystem tool absent")
        self.run(
            window_args(
                self.overlay, self.root, row["start"], row["length"], self.virtual
            ),
            timeout=120,
        )
        if self.root.stat().st_size != row["length"]:
            raise ValueError("Root window size mismatch")
        result = {
            "inventory": self.inventory,
            "overlay_sha256": self.overlay_sha,
            "base_sha256": BASE_SHA,
            "source_references": refs,
            "virtual_size": self.virtual,
            "partitions": partitions,
            "probes": probes,
            "root": row,
            "sparse_bytes": self.root.stat().st_blocks * 512,
        }
        (self.out / "prepare.json").write_text(json.dumps(result, indent=2) + "\n")
        return result

    def debugfs(self, action: str, path: str) -> dict:
        if self.fs != "ext4":
            raise ValueError("Root is not ext4")
        allowed_guest_path(path)
        if action not in ("stat", "ls", "cat", "dump"):
            raise ValueError("Readonly debugfs operation only")
        stat = self.run(["/usr/bin/debugfs", "-R", "stat " + path, str(self.root)])
        if "Inode:" not in stat:
            return {"exists": False, "stat": stat}
        if action == "stat":
            return {"exists": True, "text": sanitize_text(stat, self.nonce)}
        if action in ("cat", "dump") and "Type: symlink" in stat:
            raise ValueError("Refuse following guest symlink")
        if action == "dump":
            if not re.fullmatch(
                r"/var/log/journal/[a-f0-9]{32}/[A-Za-z0-9@_.~-]+\.journal~?", path
            ):
                raise ValueError("Only binary journals may be dumped")
            destination = (
                self.out / "journals" / Path(path).parent.name / Path(path).name
            )
            destination.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
            if destination.exists():
                raise ValueError("Fresh extraction target required")
            text = self.run(
                [
                    "/usr/bin/debugfs",
                    "-R",
                    "dump " + path + " " + str(destination),
                    str(self.root),
                ]
            )
            return {
                "extracted": str(destination),
                "bytes": destination.stat().st_size,
                "sha256": sha(destination),
                "diagnostic": text,
            }
        text = self.run(
            [
                "/usr/bin/debugfs",
                "-R",
                ("ls -p " if action == "ls" else "cat ") + path,
                str(self.root),
            ]
        )
        if action == "cat" and len(text) > 4 * 1024 * 1024:
            raise ValueError("Text extraction limit")
        safe = sanitize_text(text, self.nonce)
        dest = self.out / "texts" / (path.lstrip("/").replace("/", "__") + ".txt")
        dest.parent.mkdir(mode=0o700, exist_ok=True)
        dest.write_text(safe)
        return {"exists": True, "text": safe, "saved": str(dest)}

    def btrfs_restore(self, root_id: int | None, dry: bool = False) -> dict:
        if self.fs != "btrfs":
            raise ValueError("Root is not btrfs")
        if root_id is not None:
            listing = self.run(["/usr/bin/btrfs", "restore", "-l", str(self.root)])
            roots = {
                int(value)
                for value in re.findall(r"tree key \((\d+) ROOT_ITEM", listing)
            }
            if (
                len(roots) > 32
                or root_id not in roots
                or (root_id != 5 and root_id < 256)
            ):
                raise ValueError("Root ID outside observed filesystem roots")
        # POSIX ERE: только точные unit namespaces и binary journal namespace.
        unit = r"(sshd|sshdgenkeys|cloud-init|cloud-init-local|cloud-init-main|cloud-init-network|cloud-config|cloud-final|systemd-time-wait-sync|systemd-timesyncd|time-sync|network|network-online|yonik-helper|yonik-worker|yonik-worker-check)\.(service|socket|target)"
        # Явно раскрытые parents нужны btrfs restore path-regex traversal.
        alternatives = [
            "",
            "usr",
            "usr/lib",
            "usr/lib/systemd",
            "usr/lib/systemd/system",
            "etc",
            "etc/systemd",
            "etc/systemd/system",
            "var",
            "var/log",
            "var/log/journal",
        ]
        for parent in ("usr/lib/systemd/system", "etc/systemd/system"):
            alternatives += [
                parent + "/" + unit,
                parent + "/" + unit + r"\.d",
                parent + "/" + unit + r"\.d/[^/]+\.conf",
            ]
        alternatives += [
            r"var/log/journal/[a-f0-9]{32}",
            r"var/log/journal/[a-f0-9]{32}/[A-Za-z0-9@_.~-]+\.journal~?",
            r"var/log/cloud-init\.log",
            r"var/log/cloud-init-output\.log",
        ]
        pattern = "^/(" + "|".join(alternatives) + ")$"
        destination = self.out / (
            "btrfs-" + str(root_id or "default") + ("-dry" if dry else "")
        )
        destination.mkdir(mode=0o700)
        argv = ["/usr/bin/btrfs", "restore", "-v", "-S", "--path-regex", pattern]
        if dry:
            argv += ["-D"]
        if root_id is not None:
            argv += ["-r", str(root_id)]
        text = self.run([*argv, str(self.root), str(destination)])
        files = []
        for item in destination.rglob("*"):
            relative = "/" + str(item.relative_to(destination))
            if item.is_symlink():
                allowed_guest_path(relative)
                files.append({"path": relative, "symlink": os.readlink(item)})
            elif item.is_file():
                allowed_guest_path(relative)
                if any(
                    parent.is_symlink()
                    for parent in item.parents
                    if parent != destination.parent
                ):
                    raise ValueError("Extracted parent symlink")
                files.append(
                    {
                        "path": relative,
                        "bytes": item.stat().st_size,
                        "sha256": sha(item),
                    }
                )
                if not re.search(r"\.journal~?$", item.name):
                    if item.stat().st_size > 4 * 1024 * 1024:
                        raise ValueError("Text limit")
                    safe = sanitize_text(item.read_text(errors="replace"), self.nonce)
                    output = (
                        self.out
                        / "texts"
                        / (relative.lstrip("/").replace("/", "__") + ".txt")
                    )
                    output.parent.mkdir(mode=0o700, exist_ok=True)
                    output.write_text(safe)
        (self.out / (destination.name + "-manifest.json")).write_text(
            json.dumps(files, indent=2) + "\n"
        )
        return {
            "destination": str(destination),
            "files": files,
            "log": sanitize_text(text, self.nonce),
        }

    def journals(self, directory: str) -> dict:
        path = checked_journal_directory(Path(directory), self.out)
        for item in path.rglob("*"):
            if item.is_symlink():
                raise ValueError("Journal symlink forbidden")
        prefix = [
            "/usr/bin/journalctl",
            "--directory",
            str(path),
            "--no-pager",
            "-o",
            "json",
        ]
        text = self.run(
            [*prefix, *[arg for unit in sorted(JOURNAL_UNITS) for arg in ("-u", unit)]]
        )
        text += "\n" + self.run([*prefix, "_PID=1"])
        messages = []
        seen = set()
        for line in text.splitlines():
            if not line.startswith("{"):
                continue
            value = json.loads(line)
            key = value.get("__CURSOR", line)
            if key in seen:
                continue
            seen.add(key)
            safe = sanitize_journal(value, self.nonce)
            if safe is not None:
                messages.append(safe)
        target = self.out / ("sanitized-journal-" + str(self.commands) + ".json")
        target.write_text(json.dumps(messages, ensure_ascii=False, indent=2) + "\n")
        return {"sanitized": str(target), "records": len(messages)}

    def dispatch(self, value: dict) -> dict:
        action = value["action"]
        if action == "btrfs-restore":
            return self.btrfs_restore(value.get("root_id"), value.get("dry", False))
        if action == "journals":
            return self.journals(value["directory"])
        if action == "debugfs":
            return self.debugfs(value["operation"], value["path"])
        if action == "btrfs-list":
            if self.fs != "btrfs":
                raise ValueError("Root is not btrfs")
            return {
                "roots": self.run(["/usr/bin/btrfs", "restore", "-l", str(self.root)])
            }
        raise ValueError("Unimplemented/unsupported readonly extraction action")

    def execute(self) -> None:
        self.task.mkdir(mode=0o700)
        lock = os.open(self.task, os.O_RDONLY | os.O_DIRECTORY)
        fcntl.flock(lock, fcntl.LOCK_EX)
        base_lock = os.open(CACHE, os.O_RDONLY | os.O_DIRECTORY)
        fcntl.flock(base_lock, fcntl.LOCK_SH)
        input_lock = os.open(self.overlay.parent, os.O_RDONLY | os.O_DIRECTORY)
        fcntl.flock(input_lock, fcntl.LOCK_SH)
        (self.out / "executed-source.py").write_bytes(Path(__file__).read_bytes())
        try:
            ready = self.prepare()
            print(
                json.dumps(
                    {
                        "ready": True,
                        "out": str(self.out),
                        "filesystem": self.fs,
                        "root": ready["root"],
                    }
                ),
                flush=True,
            )
            deadline = self.started + 900
            while time.monotonic() < deadline:
                readable, _, _ = select.select(
                    [sys.stdin], [], [], min(30, max(0, deadline - time.monotonic()))
                )
                if not readable:
                    continue
                line = sys.stdin.readline()
                if not line:
                    raise ValueError("Controller EOF before finish")
                request = json.loads(line)
                if request.get("action") == "finish":
                    break
                try:
                    result = self.dispatch(request)
                except Exception as error:
                    result = {"error": str(error)}
                    (self.out / "action-failure.json").write_text(
                        json.dumps({"request": request, "result": result}, indent=2)
                        + "\n"
                    )
                with (self.out / "actions.jsonl").open("a") as stream:
                    stream.write(
                        json.dumps({"request": request, "result": result}) + "\n"
                    )
                print(json.dumps(result), flush=True)
            else:
                raise TimeoutError("Offline controller deadline900s")
        except Exception as error:
            (self.out / "failure.json").write_text(
                json.dumps(
                    {"error": str(error), "type": type(error).__name__}, indent=2
                )
                + "\n"
            )
            print(json.dumps({"BLOCKER": str(error), "out": str(self.out)}), flush=True)
        finally:
            stable = {}
            for path, expected in [(self.overlay, self.overlay_sha), (IMAGE, BASE_SHA)]:
                st = path.stat()
                stable["overlay" if path == self.overlay else "base"] = sha(
                    path
                ) == expected and self.original_stats.get(str(path)) == (
                    st.st_dev,
                    st.st_ino,
                    st.st_size,
                    st.st_mtime_ns,
                )
            refs = source_references([self.task])
            assert all(stable.values())
            if not refs:
                shutil.rmtree(self.task)
            (self.out / "cleanup.json").write_text(
                json.dumps(
                    {
                        "task": str(self.task),
                        "task_exists": self.task.exists(),
                        "references": refs,
                        "input_bytes_and_identity_unchanged": stable,
                    },
                    indent=2,
                )
                + "\n"
            )
            os.close(input_lock)
            os.close(base_lock)
            os.close(lock)
            print(
                json.dumps(
                    {
                        "finished": True,
                        "out": str(self.out),
                        "task_exists": self.task.exists(),
                    }
                ),
                flush=True,
            )


if __name__ == "__main__":
    Extraction().execute()
