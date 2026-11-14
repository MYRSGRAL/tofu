"""Reviewed #147-base package-hook VM; никакого build/download/product activation."""

from __future__ import annotations

import fcntl
import hashlib
import json
import os
import re
import secrets
import shlex
import shutil
import socket
import subprocess
import sys
import time
from pathlib import Path

BASE = Path(__file__).resolve().parents[1]
PROPOSAL = BASE / "docs/design/a4-package-hook-proposal-142"
DECISION = Path(
    "/home/myrs/projects/yonik/local/orchestration/proposals/a4-package-hook-vm-root-decision.json"
)
CACHE = Path("/home/myrs/projects/yonik/local/validation/vm-base")
REVIEWED = "6a02d91e14a28b1161f928425b4452d3acfe0aa7"
FINGERPRINT = "1B9A16984A4E8CB448712D2AE0B78BF4326C6F8F"
UNITS = (
    "yonik-helper.service",
    "yonik-worker.service",
    "yonik-worker.socket",
    "yonik-worker-check.service",
    "yonik-worker-check.socket",
)
PHASES = (
    "install-old",
    "upgrade-candidate",
    "rollback-old",
    "return-candidate",
    "uninstall-candidate",
)


class ApprovalRejected(Exception):
    """Отдельный terminal отказ, не transient SSH RuntimeError."""


def reject_approval(argv: list[str], stdout: str, stderr: str) -> None:
    text = stdout + "\n" + stderr
    lowered = text.lower()
    if "approval" in lowered and any(
        word in lowered for word in ("rejected", "denied")
    ):
        raise ApprovalRejected(
            "Automatic approval rejection; STOP: " + repr(argv) + "\n" + text
        )


def sha(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def validate_decision(value: dict) -> None:
    if (
        value.get("source") != REVIEWED
        or "CLEAN" not in value.get("review", "")
        or "earlier authorized147" not in value.get("scope", "")
    ):
        raise ValueError(
            "Нет exact root decision/prior147 standing authorization attestation"
        )
    needed = {
        "fresh overlay with readonly base and exact input verification",
        "five service/socket masks before transactions; zero helper/worker/check/core activation",
        "real guest PID1/libalpm/Manager.Reload phase-correlated evidence",
        "existing normal-user QEMU/KVM access only; no host services/build/download/new caps",
        "bounded own process cleanup/reap preserving unique evidence",
    }
    if set(value.get("allowed", [])) != needed:
        raise ValueError("Root decision scope отличается")


def process_start(pid: int) -> str:
    text = Path(f"/proc/{pid}/stat").read_text()
    return text[text.rfind(")") + 2 :].split()[19]


def own_process(pid: int, start: str, argv: list[str]) -> bool:
    try:
        return (
            Path(f"/proc/{pid}").stat().st_uid == os.getuid()
            and process_start(pid) == start
            and Path(f"/proc/{pid}/cmdline").read_bytes().split(b"\0")[:-1]
            == [a.encode() for a in argv]
        )
    except OSError:
        return False


def references(task: Path) -> list[dict]:
    found = []
    for process in Path("/proc").iterdir():
        if not process.name.isdigit() or int(process.name) == os.getpid():
            continue
        try:
            if process.stat().st_uid != os.getuid():
                continue
            for kind in ("cwd", "exe"):
                try:
                    if str(task) in os.readlink(process / kind):
                        found.append({"pid": int(process.name), "kind": kind})
                except OSError:
                    pass
            for fd in (process / "fd").iterdir():
                try:
                    if str(task) in os.readlink(fd):
                        found.append({"pid": int(process.name), "kind": "fd"})
                except OSError:
                    pass
            try:
                if str(task) in (process / "maps").read_text():
                    found.append({"pid": int(process.name), "kind": "maps"})
            except OSError:
                pass
        except OSError:
            pass
    return found


class Driver:
    def __init__(self) -> None:
        self.nonce = secrets.token_hex(16)
        self.out = BASE / "local/validation/a4-package-hooks-142" / self.nonce
        self.out.mkdir(mode=0o700, parents=True)
        self.task = Path("/var/tmp/codex-myrs/transient") / (
            "a4-mtree4-pid1-hooks-" + self.nonce
        )
        self.started = time.monotonic()
        self.vm = None
        self.ssh = []
        self.argv = []
        self.vm_start = ""
        self.failed = True
        self.unknown = False
        self.cleaning = False

    def run(
        self, args: list[str], *, accepted: tuple[int, ...] = (0,), timeout: int = 45
    ) -> str:
        if not self.cleaning and time.monotonic() - self.started > 900:
            raise TimeoutError("900s total budget")
        if not args[0].startswith("/"):
            args = ["/usr/bin/" + args[0], *args[1:]]
        process = subprocess.Popen(
            args,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=dict(os.environ, PATH="/usr/bin", LC_ALL="C", LANG="C"),
        )
        began = time.monotonic_ns()
        try:
            stdout, stderr = process.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            process.kill()
            stdout, stderr = process.communicate()
            self.record(
                "commands",
                {
                    "argv": args,
                    "pid": process.pid,
                    "exit": process.returncode,
                    "timeout": True,
                    "stdout": stdout,
                    "stderr": stderr,
                },
            )
            raise
        self.record(
            "commands",
            {
                "argv": args,
                "pid": process.pid,
                "start_ns": began,
                "end_ns": time.monotonic_ns(),
                "exit": process.returncode,
                "stdout": stdout,
                "stderr": stderr,
            },
        )
        reject_approval(args, stdout, stderr)
        if process.returncode not in accepted:
            raise RuntimeError(f"Command exit {process.returncode}: {args}")
        return stdout

    def record(self, name: str, value: dict) -> None:
        with (self.out / (name + ".jsonl")).open("a") as stream:
            stream.write(json.dumps(value) + "\n")

    def check_vm(self) -> None:
        if (
            not self.vm
            or self.vm.poll() is not None
            or not own_process(self.vm.pid, self.vm_start, self.argv)
        ):
            raise ValueError("Own QEMU PID/starttime/argv guard отказал")

    def guest(self, command: str, *, timeout: int = 45) -> str:
        self.check_vm()
        return self.run([*self.ssh, command], timeout=timeout)

    def scp(self, source: Path, target: str) -> None:
        self.check_vm()
        args = [
            "scp",
            "-i",
            str(self.task / "key"),
            "-P",
            str(self.port),
            "-o",
            "StrictHostKeyChecking=accept-new",
            "-o",
            "UserKnownHostsFile=" + str(self.task / "known_hosts"),
            str(source),
            "root@127.0.0.1:" + target,
        ]
        self.run(args, timeout=60)

    def verify_inputs(self) -> dict:
        if os.getuid() != os.geteuid() or os.getuid() == 0:
            raise ValueError("Only existing normal UID")
        decision = json.loads(DECISION.read_text())
        validate_decision(decision)
        manifest = json.loads((PROPOSAL / "readonly-inputs.json").read_text())
        for row in manifest["inputs"]:
            if sha(Path(row["path"])) != row["sha256"]:
                raise ValueError("Changed input: " + row["path"])
        for row in manifest["host_tool_inputs"].values():
            if sha(Path(row["path"])) != row["sha256"]:
                raise ValueError("Changed host tool: " + row["path"])
        if not os.access("/dev/kvm", os.R_OK | os.W_OK):
            raise ValueError("Missing existing KVM access; no privilege fallback")
        self.run(["git", "merge-base", "--is-ancestor", REVIEWED, "HEAD"])
        collected = next(
            Path(r["path"])
            for r in manifest["inputs"]
            if r["path"].endswith("/collected.json")
        )
        descriptor = json.loads(collected.read_text())
        if (
            descriptor["final_linux_acceptance"] is not False
            or descriptor["source"] != manifest["source"]
        ):
            raise ValueError("Source/COLLECTED mismatch")
        frozen = json.loads(collected.with_name("frozen.json").read_text())
        for name, expected in frozen["tooling_sha256"].items():
            if sha(BASE / "tools" / name) != expected:
                raise ValueError("Controller tooling changed: " + name)
        versions = []
        for suffix in (".pkg.tar.zst", "/package.tar"):
            archive = Path(
                next(
                    row["path"]
                    for row in manifest["inputs"]
                    if row["path"].endswith(suffix)
                )
            )
            listing = self.run(["bsdtar", "-tf", str(archive)]).splitlines()
            listing = [name.removeprefix("./") for name in listing]
            unit_names = sorted(
                name.removeprefix("usr/lib/systemd/system/")
                for name in listing
                if name.startswith("usr/lib/systemd/system/") and not name.endswith("/")
            )
            if ".INSTALL" in listing or unit_names != sorted(UNITS):
                raise ValueError("Package script/unit union differs before bootstrap")
            metadata = self.run(["bsdtar", "-xOf", str(archive), ".PKGINFO"])
            versions.append(
                next(
                    line.split(" = ", 1)[1]
                    for line in metadata.splitlines()
                    if line.startswith("pkgver = ")
                )
            )
        if self.run(["vercmp", *versions]).strip() != "-1":
            raise ValueError("Two actual ordered versions required")
        bindings = {
            "source": descriptor["source"],
            "decision_sha256": sha(DECISION),
            "decision": decision,
            "inputs": manifest,
            "driver_sha256": sha(Path(__file__)),
            "guest_sha256": sha(Path(__file__).with_name("a4_package_hook_guest.py")),
            "vm_launch_authorization": "root attestation of standing147 scope, exact reviewed plan; not new user grant",
        }
        (self.out / "input-bindings.json").write_text(
            json.dumps(bindings, indent=2) + "\n"
        )
        return manifest

    def bootstrap(self, manifest: dict) -> None:
        image = Path(
            next(r["path"] for r in manifest["inputs"] if r["path"].endswith(".qcow2"))
        )
        gnupg = self.task / "gnupg"
        gnupg.mkdir(mode=0o700)
        try:
            self.run(
                [
                    "gpg",
                    "--homedir",
                    str(gnupg),
                    "--import",
                    str(CACHE / "arch-ci-public-key.asc"),
                ]
            )
            signature = self.run(
                [
                    "gpg",
                    "--homedir",
                    str(gnupg),
                    "--status-fd",
                    "1",
                    "--verify",
                    str(image) + ".sig",
                    str(image),
                ]
            )
            if not any(
                line.startswith("[GNUPG:] VALIDSIG ")
                and line.split()[-1] == FINGERPRINT
                for line in signature.splitlines()
            ):
                raise ValueError("Image signature mismatch")
        finally:
            self.run(["gpgconf", "--homedir", str(gnupg), "--kill", "gpg-agent"])
        info = json.loads(self.run(["qemu-img", "info", "--output=json", str(image)]))
        if info.get("backing-filename") or info.get("dirty-flag"):
            raise ValueError("Base image not clean standalone")
        self.run(
            [
                "qemu-img",
                "create",
                "-f",
                "qcow2",
                "-F",
                "qcow2",
                "-b",
                str(image),
                str(self.task / "disk.qcow2"),
                "12G",
            ]
        )
        overlay = json.loads(
            self.run(
                ["qemu-img", "info", "--output=json", str(self.task / "disk.qcow2")]
            )
        )
        if overlay.get("full-backing-filename") != str(image):
            raise ValueError("Overlay backing mismatch")
        self.run(
            [
                "ssh-keygen",
                "-q",
                "-t",
                "ed25519",
                "-N",
                "",
                "-f",
                str(self.task / "key"),
            ]
        )
        seed = self.task / "seed"
        seed.mkdir(mode=0o700)
        (seed / "meta-data").write_text(
            "instance-id: a4-package-hooks-" + self.nonce + "\n"
        )
        config = self.cloud_config((self.task / "key.pub").read_text().strip())
        (seed / "user-data").write_text("#cloud-config\n" + json.dumps(config) + "\n")
        self.run(
            [
                "xorriso",
                "-as",
                "mkisofs",
                "-quiet",
                "-output",
                str(self.task / "seed.iso"),
                "-volid",
                "cidata",
                "-joliet",
                "-rock",
                str(seed),
            ]
        )
        shutil.copy2("/usr/share/edk2/x64/OVMF_VARS.4m.fd", self.task / "vars.fd")
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            self.port = reservation.getsockname()[1]
        contract = json.loads((PROPOSAL / "execution-contract.json").read_text())
        substitutions = {
            "{task_root}": str(self.task),
            "{nonce}": self.nonce,
            "{ssh_port}": str(self.port),
        }

        def expand(value: str) -> str:
            for before, after in substitutions.items():
                value = value.replace(before, after)
            if re.search(r"\{[^}]+\}", value):
                raise ValueError("Unexpanded argv")
            return value

        self.argv = [expand(x) for x in contract["qemu_argv_template"]]
        self.ssh = [expand(x) for x in contract["ssh_argv_template"]]
        (self.out / "qemu-command.json").write_text(
            json.dumps(self.argv, indent=2) + "\n"
        )
        with (self.out / "qemu.log").open("w") as log:
            self.vm = subprocess.Popen(
                self.argv, stdout=log, stderr=log, start_new_session=True
            )
        self.vm_start = process_start(self.vm.pid)
        (self.out / "active.json").write_text(
            json.dumps(
                {
                    "pid": self.vm.pid,
                    "starttime": self.vm_start,
                    "uid": os.getuid(),
                    "argv": self.argv,
                    "task": str(self.task),
                    "nonce": self.nonce,
                    "ssh_port": self.port,
                },
                indent=2,
            )
            + "\n"
        )
        deadline = time.monotonic() + 240
        while time.monotonic() < deadline:
            self.check_vm()
            try:
                actual = self.guest(
                    "cat /run/a4-package-hooks-ready", timeout=8
                ).strip()
                if actual != self.nonce:
                    raise ValueError("Wrong guest nonce")
                break
            except RuntimeError:
                time.sleep(5)
        else:
            raise TimeoutError("SSH readiness deadline240s")
        self.prepare_guest(manifest)

    def prepare_guest(self, manifest: dict) -> None:
        self.guest(
            "umask 077; mkdir /root/a4-package-hooks; mkdir /root/a4-package-hooks/input"
        )
        self.scp(
            Path(__file__).with_name("a4_package_hook_guest.py"),
            "/root/a4-package-hooks/guest.py",
        )
        self.guest(
            "python /root/a4-package-hooks/guest.py preflight " + self.nonce, timeout=75
        )
        packages = {}
        elfs = {}
        for label in ("old", "candidate"):
            row = (
                next(
                    r
                    for r in manifest["inputs"]
                    if r["path"].endswith(".pkg.tar.zst")
                    if label == "old"
                )
                if label == "old"
                else next(
                    r for r in manifest["inputs"] if r["path"].endswith("/package.tar")
                )
            )
            archive = Path(row["path"])
            filename = "old.pkg.tar.zst" if label == "old" else "candidate.tar"
            listing = self.run(["bsdtar", "-tf", str(archive)]).splitlines()
            if ".INSTALL" in [x.removeprefix("./") for x in listing]:
                raise ValueError("Unexpected package install script")
            elfs[label] = {}
            for member in (
                "usr/bin/yonik-desktop",
                "usr/lib/yonik/yonik-helper",
                "usr/lib/yonik/yonik-worker",
            ):
                payload = subprocess.check_output(
                    ["/usr/bin/bsdtar", "-xOf", str(archive), member]
                )
                elfs[label][member] = hashlib.sha256(payload).hexdigest()
            packages[filename] = row["sha256"]
            self.scp(archive, "/root/a4-package-hooks/input/" + filename)
        binding = self.task / "binding.json"
        binding.write_text(
            json.dumps(
                {"source": manifest["source"], "packages": packages, "elfs": elfs}
            )
        )
        self.scp(binding, "/root/a4-package-hooks/binding.json")
        config = self.task / "pacman.conf"
        config.write_text(
            "[options]\nArchitecture = x86_64\nSigLevel = Required DatabaseOptional\nLocalFileSigLevel = Optional TrustedOnly\n"
        )
        self.scp(config, "/root/a4-package-hooks/pacman.conf")

    def workload(self, manifest: dict) -> None:
        for name in PHASES:
            print(json.dumps({"event": "phase", "name": name}), flush=True)
            try:
                self.guest(
                    "python /root/a4-package-hooks/guest.py " + name + " " + self.nonce,
                    timeout=100,
                )
            except Exception:
                self.unknown = True
                try:
                    self.guest(
                        "python /root/a4-package-hooks/guest.py reconcile "
                        + self.nonce,
                        timeout=30,
                    )
                except Exception as error:
                    self.record("reconcile-errors", {"error": str(error)})
                raise

    def success_result(self) -> dict:
        return {
            "five_phases": "PASS",
            "live_package_hooks": True,
            "final_linux_acceptance": False,
            "factory": "CLOSED",
        }

    def cloud_config(self, public_key: str) -> dict:
        bootcmd = [["mkdir", "-p", "/etc/systemd/system"]] + [
            ["ln", "-s", "/dev/null", "/etc/systemd/system/" + unit] for unit in UNITS
        ]
        return {
            "disable_root": False,
            "ssh_pwauth": False,
            "package_update": False,
            "package_upgrade": False,
            "users": [{"name": "root", "ssh_authorized_keys": [public_key]}],
            "bootcmd": bootcmd,
            "runcmd": [
                ["systemctl", "enable", "--now", "sshd.service"],
                [
                    "sh",
                    "-c",
                    "printf %s "
                    + shlex.quote(self.nonce)
                    + " > /run/a4-package-hooks-ready",
                ],
            ],
        }

    def execute(self) -> None:
        print(json.dumps({"event": "starting", "out": str(self.out)}), flush=True)
        fd = None
        base_fd = None
        try:
            base_fd = os.open(CACHE, os.O_RDONLY | os.O_DIRECTORY)
            fcntl.flock(base_fd, fcntl.LOCK_SH)
            manifest = self.verify_inputs()
            self.task.mkdir(mode=0o700)
            fd = os.open(self.task, os.O_RDONLY | os.O_DIRECTORY)
            fcntl.flock(fd, fcntl.LOCK_EX)
            (self.task / "owner.json").write_text(
                json.dumps(
                    {"nonce": self.nonce, "uid": os.getuid(), "pid": os.getpid()}
                )
            )
            self.bootstrap(manifest)
            self.workload(manifest)
            self.failed = False
            (self.out / "result.json").write_text(
                json.dumps(self.success_result()) + "\n"
            )
        except Exception as error:
            (self.out / "failure.json").write_text(
                json.dumps(
                    {
                        "type": type(error).__name__,
                        "error": str(error),
                        "failed": True,
                        "unknown_transaction_outcome": self.unknown,
                    },
                    indent=2,
                )
                + "\n"
            )
            print(
                json.dumps(
                    {"event": "BLOCKER", "error": str(error), "out": str(self.out)}
                ),
                flush=True,
            )
        finally:
            self.cleanup()
            if base_fd is not None:
                os.close(base_fd)
            if fd is not None:
                os.close(fd)

    def collect_guest_evidence(self) -> None:
        self.guest(
            "tar -C /root/a4-package-hooks -cf /root/a4-hooks-evidence.tar evidence",
            timeout=20,
        )
        args = [
            "scp",
            "-i",
            str(self.task / "key"),
            "-P",
            str(self.port),
            "-o",
            "StrictHostKeyChecking=accept-new",
            "-o",
            "UserKnownHostsFile=" + str(self.task / "known_hosts"),
            "root@127.0.0.1:/root/a4-hooks-evidence.tar",
            str(self.out / "guest-evidence.tar"),
        ]
        self.run(args, timeout=30)

    def keep_overlay(self) -> bool:
        return self.failed

    def cleanup(self) -> None:
        self.cleaning = True
        if (self.task / "serial.log").exists():
            shutil.copy2(
                self.task / "serial.log", self.out / "serial-before-shutdown.log"
            )
        if self.vm and self.vm.poll() is None:
            try:
                self.collect_guest_evidence()
            except Exception as error:
                self.record("cleanup-errors", {"collect": str(error)})
            try:
                self.guest("poweroff", timeout=10)
            except Exception as error:
                self.record("cleanup-errors", {"poweroff": str(error)})
            try:
                self.vm.wait(timeout=20)
            except subprocess.TimeoutExpired:
                if not own_process(self.vm.pid, self.vm_start, self.argv):
                    raise ValueError("Refuse signaling changed QEMU owner")
                self.vm.terminate()
                try:
                    self.vm.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    self.vm.kill()
                    self.vm.wait(timeout=10)
        if (self.task / "serial.log").exists():
            shutil.copy2(self.task / "serial.log", self.out / "serial.log")
        if (
            self.keep_overlay()
            and self.vm is not None
            and (self.task / "disk.qcow2").exists()
        ):
            shutil.copy2(self.task / "disk.qcow2", self.out / "blocked-disk.qcow2")
        refs = references(self.task)
        image = CACHE / "Arch-Linux-x86_64-cloudimg-20261101.620452.qcow2"
        base_sha = sha(image)
        if (
            base_sha
            != "794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003"
        ):
            raise ValueError("BASE IMAGE CHANGED")
        if self.task.exists() and not refs:
            marker = json.loads((self.task / "owner.json").read_text())
            if (
                marker["nonce"] != self.nonce
                or marker["uid"] != os.getuid()
                or self.task.is_symlink()
                or self.task.parent != Path("/var/tmp/codex-myrs/transient")
            ):
                raise ValueError("Cleanup path guard отказал")
            shutil.rmtree(self.task)
        (self.out / "cleanup.json").write_text(
            json.dumps(
                {
                    "task": str(self.task),
                    "task_exists": self.task.exists(),
                    "references": refs,
                    "base_sha256_after": base_sha,
                    "vm_exit": None if self.vm is None else self.vm.returncode,
                    "blocked_disk_retained": (self.out / "blocked-disk.qcow2").exists(),
                },
                indent=2,
            )
            + "\n"
        )
        print(
            json.dumps(
                {
                    "event": "finished",
                    "failed": self.failed,
                    "out": str(self.out),
                    "task_exists": self.task.exists(),
                }
            ),
            flush=True,
        )


def main() -> int:
    driver = Driver()
    driver.execute()
    return 1 if driver.failed else 0


if __name__ == "__main__":
    sys.exit(main())
