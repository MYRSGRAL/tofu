"""Ровно один approved -i units-only read; exit0 никогда не заменяет per-file proof."""

from __future__ import annotations

import fcntl
import json
import os
import posixpath
import re
import shlex
import shutil
import subprocess
import time
from pathlib import Path

import a4_offline_bootstrap_extract as offline
import a4_offline_bootstrap_units as selector

DECISION = Path(
    "/home/myrs/projects/yonik/local/orchestration/proposals/a4-offline-units-ignore-errors-root.json"
)
TOOL_SHA = "8e4bbea7bd5a3a0519070d4e41f5b68ee48df0adf034747e9674b9a26e82f17d"
REQUIRED = {
    "sshd.service",
    "cloud-init-network.service",
    "cloud-init-main.service",
    "cloud-config.service",
    "cloud-final.service",
    "network.target",
    "network-online.target",
    "time-sync.target",
    "systemd-time-wait-sync.service",
}
RELATIONS = (
    "After",
    "Before",
    "Requires",
    "Wants",
    "Requisite",
    "BindsTo",
    "PartOf",
    "Upholds",
    "Conflicts",
    "OnFailure",
    "OnSuccess",
    "PropagatesReloadTo",
    "ReloadPropagatedFrom",
    "JoinsNamespaceOf",
    "PropagatesStopTo",
    "StopPropagatedFrom",
)


def assert_source_hash(path: Path, expected: str) -> None:
    if offline.sha(path) != expected:
        raise ValueError("Independent input SHA mismatch")


def validate_decision(value: dict) -> None:
    if (
        value.get("decision") != "APPROVE one units-only existing btrfs restore -i read"
        or value.get("authority") != "Root bounded read-only investigation scope"
    ):
        raise ValueError("No exact bounded root authorization")
    expected = {
        "repair/replay/mount/boot/newdecoder/download/host changes",
        "treat exit0 with -i as complete proof",
        "use zero/partial failed unit as actual definition",
        "package transactions/product activation/userdata reads",
    }
    if set(
        value.get("forbidden", [])
    ) != expected or "after one bounded read" not in value.get("stop", ""):
        raise ValueError("Changed authorization scope")


def parse_unit(text: str) -> dict:
    if not text or not text.endswith("\n") or "\0" in text:
        raise ValueError("Zero/truncated/NUL unit text")
    section = ""
    assignments = []
    service = {}
    continuation = ""
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith(("#", ";")):
            continue
        line = continuation + line
        if line.endswith("\\"):
            continuation = line[:-1] + " "
            continue
        continuation = ""
        if line.startswith("[") and line.endswith("]"):
            section = line[1:-1]
            continue
        if "=" not in line or not section:
            raise ValueError("Invalid/unresolved unit syntax")
        key, value = line.split("=", 1)
        key = key.strip()
        value = value.strip()
        if section == "Unit":
            assignments.append([key, value])
        elif section == "Service" and key in (
            "Type",
            "ExecStart",
            "ExecStartPre",
            "ExecStartPost",
            "RemainAfterExit",
        ):
            service.setdefault(key, []).append(value)
    if continuation or not assignments:
        raise ValueError("Truncated/no actual Unit section")
    return {"assignments": assignments, "service": service}


def source_failures(
    destination: Path, text: str
) -> tuple[dict[str, list[str]], list[str]]:
    failures = {}
    unattributed = []
    pending = []
    prefix = str(destination)
    for line in text.splitlines():
        if not re.search(r"\bERROR\b|truncat|corrupt", line, re.I):
            continue
        found = re.search(re.escape(prefix) + r"(/\S+)", line)
        if found:
            path = "/" + found[1].lstrip("/")
            if ".." in Path(path).parts:
                raise ValueError("Unsafe failure path")
            failures.setdefault(path, []).extend([*pending, line])
            pending = []
        else:
            pending.append(line)
    unattributed.extend(pending)
    return failures, unattributed


def account_units(
    destination: Path, tool_text: str, exit_code: int, *, required: set[str] = REQUIRED
) -> dict:
    failed, unattributed = source_failures(destination, tool_text)
    files = []
    by_path = {}
    for path in sorted(destination.rglob("*")):
        if path.is_dir() and not path.is_symlink():
            continue
        guest = "/" + str(path.relative_to(destination))
        offline.allowed_guest_path(guest)
        if not re.fullmatch(selector.units_pattern(), guest):
            raise ValueError("Output outside units-only selector")
        row = {"path": guest, "bytes": path.lstat().st_size, "status": "UNKNOWN"}
        errors = [
            error
            for failed_path, values in failed.items()
            if guest == failed_path or guest.startswith(failed_path.rstrip("/") + "/")
            for error in values
        ]
        if errors:
            row.update(status="RED_TOOL_ERROR", errors=errors)
        elif path.is_symlink():
            target = os.readlink(path)
            row["target"] = target
            row["status"] = "MASKED" if target == "/dev/null" else "ALIAS"
            if target != "/dev/null":
                row["resolved_guest_target"] = posixpath.normpath(
                    posixpath.join(posixpath.dirname(guest), target)
                )
        elif row["bytes"] == 0:
            row["status"] = "RED_ZERO"
        elif row["bytes"] > 4 * 1024 * 1024:
            row["status"] = "RED_SIZE_LIMIT"
        elif any(
            parent.is_symlink()
            for parent in path.parents
            if parent != destination.parent
        ):
            raise ValueError("Output parent symlink")
        else:
            row["sha256"] = offline.sha(path)
            try:
                parsed = parse_unit(path.read_text(encoding="utf-8", errors="strict"))
                row.update(
                    status="VALID_FILE" if not unattributed else "UNKNOWN_TOOL_CONTEXT",
                    parsed=parsed,
                )
            except (ValueError, UnicodeError) as error:
                row.update(status="RED_TRUNCATED_OR_INVALID", reason=str(error))
        files.append(row)
        by_path[guest] = row
    # ERROR мог оставить файл отсутствующим: отсутствие не скрывает source failure.
    for path, errors in failed.items():
        if path not in by_path:
            row = {
                "path": path,
                "bytes": None,
                "status": "RED_TOOL_ERROR_MISSING",
                "errors": errors,
            }
            files.append(row)
            by_path[path] = row
    unit_names = {}
    for row in files:
        match = re.fullmatch(r"/(usr/lib|etc)/systemd/system/([^/]+)", row["path"])
        if match and match[2].endswith((".service", ".socket", ".target")):
            unit_names.setdefault(match[2], []).append(row)
    effective = {}
    edges = []
    definition_files = {}
    unresolved = []
    for name, candidates in sorted(unit_names.items()):
        chosen = next(
            (r for r in candidates if r["path"].startswith("/etc/")), candidates[0]
        )
        seen = set()
        while chosen.get("status") == "ALIAS":
            target = chosen.get("resolved_guest_target")
            if target in seen or target not in by_path:
                chosen = {**chosen, "status": "ALIAS_TARGET_UNKNOWN"}
                break
            seen.add(target)
            chosen = by_path[target]
        if chosen["status"] != "VALID_FILE":
            effective[name] = {"status": chosen["status"], "source": chosen["path"]}
            continue
        dropins = {}
        for row in files:
            if re.fullmatch(
                r"/(usr/lib|etc)/systemd/system/"
                + re.escape(name)
                + r"\.d/[^/]+\.conf",
                row["path"],
            ):
                key = Path(row["path"]).name
                if key not in dropins or row["path"].startswith("/etc/"):
                    dropins[key] = row
        broken = [
            r["path"]
            for r in dropins.values()
            if r["status"] not in ("VALID_FILE", "MASKED")
        ]
        if broken:
            effective[name] = {
                "status": "DROPIN_UNKNOWN",
                "source": chosen["path"],
                "unavailable_dropins": broken,
            }
            continue
        merged = {}
        sources = [chosen["path"]]
        assignments = list(chosen["parsed"]["assignments"])
        for key, row in sorted(dropins.items()):
            if row["status"] == "MASKED":
                continue
            assignments += row["parsed"]["assignments"]
            sources.append(row["path"])
        for key, value in assignments:
            if key in RELATIONS:
                values = shlex.split(value)
                if not value:
                    merged[key] = []
                else:
                    merged.setdefault(key, []).extend(values)
            else:
                merged[key] = value
        effective[name] = {
            "status": "VALID_DEFINITION",
            "sources": sources,
            "unit": merged,
            "service": chosen["parsed"]["service"],
        }
        definition_files[name] = sources
        for relation in RELATIONS:
            for target in merged.get(relation, []):
                edges.append(
                    {
                        "unit": name,
                        "relation": relation,
                        "target": target,
                        "sources": sources,
                    }
                )
                if "%" in target or "*" in target:
                    unresolved.append(target)
    valid = {
        name for name, node in effective.items() if node["status"] == "VALID_DEFINITION"
    }
    missing_required = sorted(required - valid)
    missing_transitive = sorted(
        {edge["target"] for edge in edges if edge["target"] not in valid}
    )
    defaults = [
        name
        for name in valid
        if effective[name]["unit"].get("DefaultDependencies", "yes") != "no"
    ]
    complete = (
        not missing_required
        and not missing_transitive
        and not unattributed
        and not unresolved
        and not defaults
        and exit_code == 0
    )
    return {
        "tool_exit": exit_code,
        "files": files,
        "source_failures": failed,
        "unattributed_errors": unattributed,
        "effective_units": effective,
        "edges": edges,
        "missing_required": missing_required,
        "missing_transitive": missing_transitive,
        "implicit_defaults_unexpanded": sorted(defaults),
        "unresolved_specifiers": sorted(set(unresolved)),
        "graph_kind": "offline explicit definition graph; runtime jobs not observed",
        "graph_complete": complete,
        "graph_status": "COMPLETE_EXPLICIT_ONLY" if complete else "UNKNOWN",
        "causal_status": "NOT_CLAIMED",
        "source_hash_verification": "SEPARATE_REQUIRED",
    }


class IgnoreUnitsRead(offline.Extraction):
    def execute_once(self) -> None:
        decision = json.loads(DECISION.read_text())
        validate_decision(decision)
        prior = json.loads(
            (
                offline.BASE
                / "docs/evidence/a4-offline-bootstrap-142/run1/prepare.json"
            ).read_text()
        )
        for name in ("qemu-img", "btrfs"):
            assert_source_hash(
                Path("/usr/bin") / name, prior["inventory"][name]["sha256"]
            )
        if prior["inventory"]["btrfs"]["sha256"] != TOOL_SHA:
            raise ValueError("Pinned decoder mismatch")
        self.task.mkdir(mode=0o700)
        locks = []
        for directory, operation in [
            (self.task, fcntl.LOCK_SH),
            (offline.CACHE, fcntl.LOCK_SH),
            (self.overlay.parent, fcntl.LOCK_SH),
        ]:
            fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
            fcntl.flock(fd, operation)
            locks.append(fd)
        self.capture_locks = tuple(locks)
        (self.out / "root-decision-binding.json").write_text(
            json.dumps(
                {"decision": decision, "sha256": offline.sha(DECISION)}, indent=2
            )
            + "\n"
        )
        for path, name in [
            (Path(__file__), "executed-reader.py"),
            (Path(offline.__file__), "executed-preparation.py"),
            (Path(selector.__file__), "executed-selector.py"),
        ]:
            (self.out / name).write_bytes(path.read_bytes())
        try:
            self.prepare()
            if self.fs != "btrfs":
                raise ValueError(
                    "Approved decoder requires same Btrfs root; no fallback"
                )
            destination = self.out / "units-ignore-errors"
            destination.mkdir(mode=0o700)
            authority = self.out.parent / (
                "authority-used-" + offline.sha(DECISION) + ".json"
            )
            fd = os.open(authority, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(fd, "w") as stream:
                json.dump(
                    {
                        "root_decision_sha256": offline.sha(DECISION),
                        "output": str(self.out),
                        "restore_attempts": 1,
                    },
                    stream,
                )
            argv = [
                "/usr/bin/btrfs",
                "restore",
                "-i",
                "-S",
                "--path-regex",
                selector.units_pattern(),
                str(self.root),
                str(destination),
            ]
            began = time.monotonic_ns()
            process = subprocess.Popen(
                argv,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                start_new_session=True,
                pass_fds=self.capture_locks,
                env={"PATH": "/usr/bin", "LC_ALL": "C"},
            )
            stdout, stderr, capture_failure = offline.a4_offline_capture.capture(
                process, 60
            )
            command = {
                "argv": argv,
                "pid": process.pid,
                "exit": process.returncode,
                "timeout": capture_failure == "timeout",
                "capture_failure": capture_failure,
                "start_ns": began,
                "end_ns": time.monotonic_ns(),
                "stdout": stdout,
                "stderr": stderr,
                "restore_invocations": 1,
            }
            (self.out / "restore-command.json").write_text(
                json.dumps(command, indent=2) + "\n"
            )
            if capture_failure:
                raise offline.OfflineCaptureFailure(capture_failure)
            analysis = account_units(
                destination, stdout + "\n" + stderr, process.returncode
            )
            (self.out / "per-file-and-graph.json").write_text(
                json.dumps(analysis, ensure_ascii=False, indent=2) + "\n"
            )
            print(
                json.dumps(
                    {
                        "out": str(self.out),
                        "tool_exit": process.returncode,
                        "statuses": {
                            s: sum(r["status"] == s for r in analysis["files"])
                            for s in {r["status"] for r in analysis["files"]}
                        },
                        "valid_units": sum(
                            n["status"] == "VALID_DEFINITION"
                            for n in analysis["effective_units"].values()
                        ),
                        "graph_status": analysis["graph_status"],
                        "restore_invocations": 1,
                    }
                ),
                flush=True,
            )
        except Exception as error:
            (self.out / "failure.json").write_text(
                json.dumps(
                    {"type": type(error).__name__, "error": str(error)}, indent=2
                )
                + "\n"
            )
            raise
        finally:
            stable = {}
            for path, expected in [
                (self.overlay, self.overlay_sha),
                (offline.IMAGE, offline.BASE_SHA),
            ]:
                st = path.stat()
                stable["overlay" if path == self.overlay else "base"] = offline.sha(
                    path
                ) == expected and self.original_stats.get(str(path)) == (
                    st.st_dev,
                    st.st_ino,
                    st.st_size,
                    st.st_mtime_ns,
                )
            refs = offline.source_references([self.task])
            manifest = []
            for path in self.out.rglob("*"):
                if path.is_symlink():
                    continue
                path.chmod(0o700 if path.is_dir() else 0o600)
                if path.is_file():
                    manifest.append(
                        {
                            "path": str(path),
                            "sha256": offline.sha(path),
                            "bytes": path.stat().st_size,
                        }
                    )
            (self.out / "private-manifest.json").write_text(
                json.dumps(manifest, indent=2) + "\n"
            )
            (self.out / "private-manifest.json").chmod(0o600)
            if all(stable.values()) and not refs:
                shutil.rmtree(self.task)
            (self.out / "cleanup.json").write_text(
                json.dumps(
                    {
                        "task": str(self.task),
                        "task_exists": self.task.exists(),
                        "references": refs,
                        "input_hashes_and_identity_unchanged": stable,
                        "restore_limit": 1,
                    },
                    indent=2,
                )
                + "\n"
            )
            for fd in reversed(locks):
                os.close(fd)
            print(
                json.dumps(
                    {
                        "cleanup": str(self.out / "cleanup.json"),
                        "task_exists": self.task.exists(),
                    }
                ),
                flush=True,
            )


if __name__ == "__main__":
    IgnoreUnitsRead().execute_once()
