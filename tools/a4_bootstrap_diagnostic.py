"""Один root-approved early-SSH boot: только readonly graph, package-ready=false."""

from __future__ import annotations

import json
import shlex
import sys
from pathlib import Path

import a4_package_hook_vm as common

DIAGNOSTIC_DECISION = Path(
    "/home/myrs/projects/yonik/local/orchestration/proposals/a4-bootstrap-diagnostic-root-decision.json"
)
REVIEWED = "b9db90a12318cb5bf8f572afd9725c2b82dcec89"
IDENTITY_COMMANDS = [
    ["cat", "/sys/class/dmi/id/sys_vendor"],
    ["readlink", "-f", "/proc/1/exe"],
    ["stat", "-fc", "%T", "/sys/fs/cgroup"],
    ["cat", "/proc/sys/kernel/random/boot_id"],
    ["cat", "/proc/1/stat"],
    ["id", "-u"],
]
PROPERTIES = [
    "Id",
    "Names",
    "LoadState",
    "ActiveState",
    "SubState",
    "Result",
    "Wants",
    "Requires",
    "After",
    "Before",
    "Job",
    "MainPID",
    "InvocationID",
]
GRAPH_UNITS = [
    "systemd-time-wait-sync.service",
    "time-sync.target",
    "systemd-timesyncd.service",
    "cloud-final.service",
    "cloud-config.service",
    "cloud-init.service",
    "cloud-init-main.service",
    "sshd.service",
]
MASK_PROPERTIES = [
    "LoadState",
    "ActiveState",
    "SubState",
    "MainPID",
    "Job",
    "InvocationID",
]


def properties_argv(units: list[str], properties: list[str]) -> list[str]:
    return [
        "systemctl",
        "show",
        *units,
        *[value for name in properties for value in ("-p", name)],
    ]


def diagnostic_commands() -> list[list[str]]:
    return [
        properties_argv(GRAPH_UNITS, PROPERTIES),
        ["systemctl", "list-jobs", "--no-legend", "--no-pager"],
        [
            "systemctl",
            "cat",
            "systemd-time-wait-sync.service",
            "time-sync.target",
            "cloud-final.service",
            "cloud-config.service",
        ],
        [
            "journalctl",
            "-b",
            "--no-pager",
            "-o",
            "json",
            "-u",
            "systemd-time-wait-sync.service",
            "-u",
            "systemd-timesyncd.service",
            "-u",
            "cloud-final.service",
            "-u",
            "sshd.service",
        ],
        ["journalctl", "-b", "--no-pager", "-o", "json", "_PID=1"],
        ["systemctl", "show", "-p", "Version", "-p", "SystemState"],
        ["ps", "-eo", "pid,uid,comm,args"],
    ]


def validate_diagnostic_decision(value: dict) -> None:
    if (
        value.get("source") != REVIEWED
        or value.get("scope") != "disposable guest bootstrap diagnostic only"
        or value.get("decision")
        != "APPROVED bounded implementation and one diagnostic boot"
        or value.get("limits")
        != {"total_seconds": 900, "ssh_seconds": 240, "attempts": 1}
        or "existing #147" not in value.get("basis", "")
        or "runcmd duplicates removed" not in value.get("delta", "")
    ):
        raise ValueError("Нет exact bounded diagnostic root authorization")
    forbidden = {
        "package transactions",
        "product services activation",
        "mask/stop time sync",
        "clock changes",
        "outbound/NTP",
        "new inputs/download/build/caps",
        "Runtime16/N5/callers",
        "userdata copy",
    }
    if set(value.get("forbidden", [])) != forbidden:
        raise ValueError("Diagnostic scope differs; STOP")


def parse_properties(text: str) -> dict[str, str]:
    return dict(line.split("=", 1) for line in text.splitlines() if "=" in line)


class DiagnosticDriver(common.Driver):
    def verify_inputs(self) -> dict:
        decision = json.loads(DIAGNOSTIC_DECISION.read_text())
        validate_diagnostic_decision(decision)
        self.run(["git", "merge-base", "--is-ancestor", REVIEWED, "HEAD"])
        manifest = super().verify_inputs()
        (self.out / "diagnostic-authorization.json").write_text(
            json.dumps(
                {
                    "decision": decision,
                    "sha256": common.sha(DIAGNOSTIC_DECISION),
                    "mode": "DIAGNOSTIC_ONLY",
                    "package_ready": False,
                },
                indent=2,
            )
            + "\n"
        )
        for source, name in [
            (Path(__file__), "executed-diagnostic-source.py"),
            (Path(common.__file__), "executed-common-source.py"),
        ]:
            (self.out / name).write_bytes(source.read_bytes())
        return manifest

    def cloud_config(self, public_key: str) -> dict:
        config = super().cloud_config(public_key)
        config["bootcmd"].extend(config.pop("runcmd"))
        return config

    def allowed_commands(self) -> set[str]:
        allowed = {
            shlex.join(argv) for argv in [*IDENTITY_COMMANDS, *diagnostic_commands()]
        }
        allowed.update({"cat /run/a4-package-hooks-ready", "poweroff"})
        for unit in common.UNITS:
            allowed.add(shlex.join(["readlink", "/etc/systemd/system/" + unit]))
            allowed.add(shlex.join(properties_argv([unit], MASK_PROPERTIES)))
        return allowed

    def guest(self, command: str, *, timeout: int = 45) -> str:
        if command not in self.allowed_commands():
            raise ValueError("Diagnostic-only guest command запрещена: " + command)
        return super().guest(command, timeout=timeout)

    def scp(self, source: Path, target: str) -> None:
        raise ValueError("SCP/package/userdata transfer запрещён diagnostic-only mode")

    def prepare_guest(self, manifest: dict) -> None:
        identity = {}
        for argv in IDENTITY_COMMANDS:
            identity[argv[-1]] = self.guest(shlex.join(argv)).strip()
        if (
            identity["/sys/class/dmi/id/sys_vendor"] != "QEMU"
            or identity["/proc/1/exe"] != "/usr/lib/systemd/systemd"
            or identity["/sys/fs/cgroup"] != "cgroup2fs"
            or identity["-u"] != "0"
        ):
            raise ValueError("Diagnostic guest identity guard отказал")
        masks = {}
        for unit in common.UNITS:
            target = self.guest(
                shlex.join(["readlink", "/etc/systemd/system/" + unit])
            ).strip()
            props = parse_properties(
                self.guest(shlex.join(properties_argv([unit], MASK_PROPERTIES)))
            )
            invocation = props.get("InvocationID", "").strip("0")
            if (
                target != "/dev/null"
                or props.get("LoadState") != "masked"
                or props.get("ActiveState") != "inactive"
                or props.get("SubState") != "dead"
                or props.get("MainPID", "0") != "0"
                or invocation
                or props.get("Job", "").split(" ")[0] not in ("", "0")
            ):
                raise ValueError(
                    "Diagnostic five-mask guard отказал: "
                    + unit
                    + " "
                    + json.dumps(props)
                )
            masks[unit] = {"target": target, "properties": props}
        (self.out / "identity-and-masks.json").write_text(
            json.dumps(
                {"identity": identity, "masks": masks, "package_ready": False}, indent=2
            )
            + "\n"
        )
        results = []
        for index, argv in enumerate(diagnostic_commands()):
            text = self.guest(shlex.join(argv), timeout=45)
            results.append({"argv": argv, "stdout": text})
            (self.out / ("readonly-" + str(index) + ".txt")).write_text(text)
        processes = results[-1]["stdout"]
        for line in processes.splitlines()[1:]:
            fields = line.split(None, 3)
            if len(fields) >= 3 and fields[2] in (
                "yonik-helper",
                "yonik-worker",
                "yonik-desktop",
                "xray",
                "sing-box",
            ):
                raise ValueError("Product/core process detected; diagnostic FAIL")
        if (
            self.guest("cat /proc/sys/kernel/random/boot_id").strip()
            != identity["/proc/sys/kernel/random/boot_id"]
        ):
            raise ValueError("Diagnostic boot changed")
        (self.out / "readonly-diagnostic.json").write_text(
            json.dumps(
                {
                    "identity": identity,
                    "masks": masks,
                    "commands": results,
                    "package_ready": False,
                    "package_transactions": 0,
                    "live_hooks": False,
                },
                indent=2,
            )
            + "\n"
        )
        self.diagnostic_captured = True

    def workload(self, manifest: dict) -> None:
        if not getattr(self, "diagnostic_captured", False):
            raise ValueError("No diagnostic capture; package phases prohibited")

    def success_result(self) -> dict:
        return {
            "diagnostic_capture": "PASS",
            "package_ready": False,
            "package_transactions": 0,
            "live_package_hooks": False,
            "full_a4": False,
            "final_linux_acceptance": False,
            "factory": "CLOSED",
        }

    def collect_guest_evidence(self) -> None:
        # Все readonly результаты уже потоково сохранены на host вне TTL.
        pass

    def keep_overlay(self) -> bool:
        # Один actual diagnostic overlay сохраняется даже при успешном capture.
        return True


def main() -> int:
    driver = DiagnosticDriver()
    driver.execute()
    return 1 if driver.failed else 0


if __name__ == "__main__":
    sys.exit(main())
