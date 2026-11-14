"""Отдельный future one-boot serial driver; без SSH/package API и fallback."""

from __future__ import annotations

import base64
import fcntl
import hashlib
import json
import os
import re
import secrets
import shutil
import socket
import stat
import subprocess
import time
from pathlib import Path

import a4_package_hook_vm as common
import a4_serial_protocol as protocol

BASE_SHA = "794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003"
SOURCE142 = {
    "commit": "14229843f9a11bf98ec4f5992df00f3247102ea2",
    "tree": "7daa4ebb7cc25d3ceafab76809afc78753dc5ec3",
}
DECISION = Path(
    "/home/myrs/projects/yonik/local/orchestration/proposals/a4-serial-only-boot-root-decision.json"
)
NAMES = (
    "a4_serial_driver.py",
    "a4_serial_collector.py",
    "a4_serial_protocol.py",
    "a4_serial_projection.py",
    "a4_package_hook_vm.py",
)
LIMITS = {
    "boots": 1,
    "ssh_attempts": 0,
    "package_transactions": 0,
    "serial_seconds": 90,
    "total_seconds": 900,
}
IMAGE = common.CACHE / "Arch-Linux-x86_64-cloudimg-20261101.620452.qcow2"
TRANSIENT = Path("/var/tmp/codex-myrs/transient")


def sources() -> dict[str, str]:
    return {name: common.sha(Path(__file__).with_name(name)) for name in NAMES}


def launcher_pins() -> dict[str, str]:
    return {"/usr/bin/python": common.sha(Path("/usr/bin/python"))}


def validate_authority(value: dict) -> None:
    protocol.exact(
        value,
        {
            "decision",
            "basis",
            "source",
            "sources",
            "input_manifest_sha256",
            "qemu_contract_sha256",
            "launcher_sha256",
            "limits",
            "forbidden",
        },
    )
    if (
        value["decision"] != "APPROVED one bounded serial-only diagnostic boot"
        or value["basis"]
        != "59748be719b01cb74f8b3f4d210230e6093b3902 serial proposal; existing147 standing authorization"
        or value["source"] != SOURCE142
        or value["sources"] != sources()
        or value["limits"] != LIMITS
        or value["launcher_sha256"] != launcher_pins()
    ):
        raise ValueError("Missing exact future root one-boot authority/source binding")
    if set(value["forbidden"]) != {
        "SSH",
        "package transactions",
        "product activation",
        "host changes",
        "download/build",
        "mount/repair",
        "clock/network changes",
        "userdata",
    }:
        raise ValueError("Authority scope mismatch")
    for key, name in [
        ("input_manifest_sha256", "readonly-inputs.json"),
        ("qemu_contract_sha256", "execution-contract.json"),
    ]:
        if value[key] != common.sha(common.PROPOSAL / name):
            raise ValueError("Reviewed input/argv contract drift")


def cloud_config(token: str, public_key: str) -> dict:
    if not protocol.TOKEN.fullmatch(token):
        raise ValueError("Token format")
    # Только чистое переиспользование неизменного bootstrap; не Driver.__init__.
    old = common.Driver.__new__(common.Driver)
    old.nonce = token
    config = old.cloud_config(public_key)
    config["bootcmd"].extend(config.pop("runcmd"))
    expected = [
        ["mkdir", "-p", "/etc/systemd/system"],
        *[["ln", "-s", "/dev/null", "/etc/systemd/system/" + u] for u in common.UNITS],
        ["systemctl", "enable", "--now", "sshd.service"],
        ["sh", "-c", "printf %s " + token + " > /run/a4-package-hooks-ready"],
    ]
    if (
        config["bootcmd"] != expected
        or config.get("packages")
        or config["package_update"]
        or config["package_upgrade"]
    ):
        raise ValueError("Frozen bootstrap subsequence changed")
    config["bootcmd"].insert(6, ["/usr/bin/python", "-I", "-c", loader_source(token)])
    return config


def loader_source(token: str) -> str:
    """Закрытая доставка fixture, не зависит от порядка cloud-init write_files."""
    if not protocol.TOKEN.fullmatch(token):
        raise ValueError("Token format")
    files = {name: Path(__file__).with_name(name).read_text() for name in NAMES[1:4]}
    payload = {
        "files": files,
        "contract": {
            "token": token,
            "sources": {
                name: hashlib.sha256(content.encode()).hexdigest()
                for name, content in files.items()
            },
        },
    }
    encoded = base64.b64encode(json.dumps(payload).encode()).decode()
    program = """import base64,hashlib,json,os,runpy,sys
assert os.getuid()==0 and os.geteuid()==0
assert os.readlink('/proc/1/exe')=='/usr/lib/systemd/systemd'
with open('/sys/class/dmi/id/sys_vendor') as stream: assert stream.read(64).strip()=='QEMU'
assert os.path.isfile('/sys/fs/cgroup/cgroup.controllers')
assert all(os.readlink('/etc/systemd/system/'+unit)=='/dev/null' for unit in MASK_UNITS)
payload=json.loads(base64.b64decode(ENCODED))
expected={'a4_serial_collector.py','a4_serial_projection.py','a4_serial_protocol.py'}
assert set(payload)=={'files','contract'} and set(payload['files'])==expected
assert set(payload['contract']['sources'])==expected
for name,content in payload['files'].items():
    assert hashlib.sha256(content.encode()).hexdigest()==payload['contract']['sources'][name]
os.umask(0o077)
os.mkdir('/run/a4-serial',0o700)
root=os.open('/run/a4-serial',os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
for name,content in {**payload['files'],'contract.json':json.dumps(payload['contract'])}.items():
    fd=os.open(name,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600,dir_fd=root)
    with os.fdopen(fd,'w') as stream: stream.write(content)
os.close(root)
sys.path.insert(0,'/run/a4-serial')
runpy.run_path('/run/a4-serial/a4_serial_collector.py',run_name='__main__')
""".replace("ENCODED", repr(encoded)).replace("MASK_UNITS", repr(common.UNITS))
    if len(program.encode()) > 98304:
        raise ValueError("Bounded inline loader argv limit")
    return program


def qemu_argv(task: Path, token: str, port: int) -> list[str]:
    if (
        task.parent != TRANSIENT
        or task.name != "a4-serial-only-" + token
        or not protocol.TOKEN.fullmatch(token)
        or not 1024 <= port <= 65535
    ):
        raise ValueError("Own QEMU substitution guard")
    contract = json.loads((common.PROPOSAL / "execution-contract.json").read_text())
    result = []
    for arg in contract["qemu_argv_template"]:
        arg = (
            arg.replace("{task_root}", str(task))
            .replace("{nonce}", token)
            .replace("{ssh_port}", str(port))
        )
        if re.search(r"\{[^}]+\}", arg):
            raise ValueError("Unexpanded QEMU argv")
        result.append(arg)
    return result


class SerialDriver:
    def __init__(self) -> None:
        # Constructor не создаёт seed/overlay/out и ничего не исполняет.
        self.token = secrets.token_hex(16)
        self.task = TRANSIENT / ("a4-serial-only-" + self.token)
        self.out = common.BASE / "local/validation/a4-serial-only-142" / self.token
        self.vm: subprocess.Popen | None = None
        self.vm_start = ""
        self.argv: list[str] = []
        self.port = 0
        self.authority: dict = {}
        self.created_out = False
        self.created_task = False
        self.started = 0.0
        self.base_stat: tuple | None = None

    def guest(self, *_: object, **__: object) -> None:
        raise ValueError("SSH/guest dispatch absent in serial-only mode")

    def scp(self, *_: object, **__: object) -> None:
        raise ValueError("SCP absent in serial-only mode")

    def workload(self, *_: object) -> None:
        raise ValueError("Package workload absent in serial-only mode")

    def _record(self, name: str, value: dict) -> None:
        path = self.out / name
        with path.open("w") as stream:
            json.dump(value, stream, indent=2)
            stream.write("\n")
        path.chmod(0o600)

    def _host(self, kind: str) -> str:
        """Фиксированный набор bootstrap commands; нет метода generic run(argv)."""
        commands = {
            "gpg-import": [
                "/usr/bin/gpg",
                "--no-options",
                "--no-autostart",
                "--batch",
                "--homedir",
                str(self.task / "gnupg"),
                "--import",
                str(common.CACHE / "arch-ci-public-key.asc"),
            ],
            "gpg-verify": [
                "/usr/bin/gpg",
                "--no-options",
                "--no-autostart",
                "--batch",
                "--homedir",
                str(self.task / "gnupg"),
                "--status-fd",
                "1",
                "--verify",
                str(IMAGE) + ".sig",
                str(IMAGE),
            ],
            "base-info": ["/usr/bin/qemu-img", "info", "--output=json", str(IMAGE)],
            "overlay-create": [
                "/usr/bin/qemu-img",
                "create",
                "-f",
                "qcow2",
                "-F",
                "qcow2",
                "-b",
                str(IMAGE),
                str(self.task / "disk.qcow2"),
                "12G",
            ],
            "overlay-info": [
                "/usr/bin/qemu-img",
                "info",
                "--output=json",
                str(self.task / "disk.qcow2"),
            ],
            "keygen": [
                "/usr/bin/ssh-keygen",
                "-q",
                "-t",
                "ed25519",
                "-N",
                "",
                "-f",
                str(self.task / "key"),
            ],
            "seed-iso": [
                "/usr/bin/xorriso",
                "-as",
                "mkisofs",
                "-quiet",
                "-output",
                str(self.task / "seed.iso"),
                "-volid",
                "cidata",
                "-joliet",
                "-rock",
                str(self.task / "seed"),
            ],
        }
        if kind not in commands or time.monotonic() - self.started > 900:
            raise ValueError("Closed command/budget guard")
        argv = commands[kind]
        began = time.monotonic_ns()
        process = subprocess.Popen(
            argv,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env={"PATH": "/usr/bin", "LC_ALL": "C", "LANG": "C"},
        )
        timeout = False
        try:
            stdout, stderr = process.communicate(
                timeout=max(0.001, min(45, 900 - (time.monotonic() - self.started)))
            )
        except subprocess.TimeoutExpired:
            process.kill()
            stdout, stderr = process.communicate()
            timeout = True
        self._record(
            kind + ".json",
            {
                "argv": argv,
                "pid": process.pid,
                "start_ns": began,
                "end_ns": time.monotonic_ns(),
                "exit": process.returncode,
                "timeout": timeout,
                "stdout": stdout.decode(errors="replace"),
                "stderr": stderr.decode(errors="replace"),
            },
        )
        common.reject_approval(
            argv, stdout.decode(errors="replace"), stderr.decode(errors="replace")
        )
        if timeout or process.returncode != 0:
            raise ValueError("Host command terminal failure: " + kind)
        return stdout.decode()

    def _verify(self) -> dict:
        if (
            os.getuid() == 0
            or os.getuid() != os.geteuid()
            or not os.access("/dev/kvm", os.R_OK | os.W_OK)
        ):
            raise ValueError("Existing normal-user KVM access required")
        manifest = json.loads((common.PROPOSAL / "readonly-inputs.json").read_text())
        if manifest["source"] != SOURCE142:
            raise ValueError("Source142 lineage")
        for row in [*manifest["inputs"], *manifest["host_tool_inputs"].values()]:
            if common.sha(Path(row["path"])) != row["sha256"]:
                raise ValueError("Pinned input/tool drift")
        if common.sha(IMAGE) != BASE_SHA or common.references(common.CACHE):
            raise ValueError("Base hash/active references")
        common.validate_decision(json.loads(common.DECISION.read_text()))
        descriptor_path = next(
            Path(r["path"])
            for r in manifest["inputs"]
            if r["path"].endswith("/collected.json")
        )
        descriptor = json.loads(descriptor_path.read_text())
        if (
            descriptor["source"] != SOURCE142
            or descriptor["final_linux_acceptance"] is not False
        ):
            raise ValueError("Nonfinal source142 descriptor")
        frozen = json.loads(descriptor_path.with_name("frozen.json").read_text())
        for name, digest in frozen["tooling_sha256"].items():
            if common.sha(common.BASE / "tools" / name) != digest:
                raise ValueError("Frozen controller changed")
        self.base_stat = self._base_identity()
        return manifest

    @staticmethod
    def _base_identity() -> tuple:
        info = IMAGE.stat()
        return (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns)

    def _prepare(self) -> None:
        (self.task / "gnupg").mkdir(mode=0o700)
        self._host("gpg-import")
        signature = self._host("gpg-verify")
        if not any(
            line.startswith("[GNUPG:] VALIDSIG ")
            and line.split()[-1] == common.FINGERPRINT
            for line in signature.splitlines()
        ):
            raise ValueError("Signed base fingerprint")
        info = json.loads(self._host("base-info"))
        if info.get("backing-filename") or info.get("dirty-flag"):
            raise ValueError("Base standalone guard")
        self._host("overlay-create")
        info = json.loads(self._host("overlay-info"))
        if info.get("full-backing-filename") != str(IMAGE):
            raise ValueError("Backing guard")
        self._host("keygen")
        seed = self.task / "seed"
        seed.mkdir(mode=0o700)
        (seed / "meta-data").write_text("instance-id: a4-serial-" + self.token + "\n")
        if sources() != self.authority["sources"]:
            raise ValueError("Seed source drift")
        config = cloud_config(self.token, (self.task / "key.pub").read_text().strip())
        if sources() != self.authority["sources"]:
            raise ValueError("Seed source drift")
        (seed / "user-data").write_text("#cloud-config\n" + json.dumps(config) + "\n")
        self._host("seed-iso")
        shutil.copy2("/usr/share/edk2/x64/OVMF_VARS.4m.fd", self.task / "vars.fd")
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            self.port = reservation.getsockname()[1]
        self.argv = qemu_argv(self.task, self.token, self.port)
        self._record("qemu-command.json", {"argv": self.argv})

    def _check_vm(self) -> None:
        if (
            self.vm is None
            or self.vm.poll() is not None
            or not common.own_process(self.vm.pid, self.vm_start, self.argv)
        ):
            raise ValueError("Own QEMU UID/PID/starttime/argv guard")

    def _capture(self) -> dict:
        reader = protocol.Stream(
            self.token,
            {
                "collector_sha256": sources()["a4_serial_collector.py"],
                "projection_sha256": sources()["a4_serial_projection.py"],
            },
        )
        serial = self.task / "serial.log"
        deadline = min(time.monotonic() + 90, self.started + 870)
        offset = 0
        while time.monotonic() < deadline:
            self._check_vm()
            if serial.exists():
                if serial.stat().st_size > protocol.MAX_STREAM:
                    raise ValueError("Raw serial limit")
                with serial.open("rb") as stream:
                    stream.seek(offset)
                    data = stream.read(8192)
                offset += len(data)
                reader.feed(data)
                self._record("sanitized.json", reader.result())
                if reader.ended == {"initial", "delayed"}:
                    return reader.result()
            time.sleep(0.1)
        return reader.result()

    def _retain(self, kind: str) -> dict:
        files = {
            "serial": ("serial.log", "serial-private.log", protocol.MAX_STREAM),
            "meta": ("seed/meta-data", "seed-meta-private", 4096),
            "seed": ("seed/user-data", "seed-user-private", 128 * 1024),
            "disk": ("disk.qcow2", "blocked-disk.qcow2", 256 * 1024 * 1024),
        }
        source, target, limit = files[kind]
        path = self.task / source
        if not path.exists():
            return {"present": False}
        if (self.task / "seed").is_symlink():
            raise ValueError("Seed path symlink")
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        digest = hashlib.sha256()
        copied = 0
        with os.fdopen(descriptor, "rb") as stream:
            before = os.fstat(stream.fileno())
            if not stat.S_ISREG(before.st_mode) or (
                before.st_size > limit and kind != "serial"
            ):
                raise ValueError("Retained artifact size/type bound")
            targetfd = os.open(
                self.out / target,
                os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                0o600,
            )
            with os.fdopen(targetfd, "wb") as output:
                while copied < limit:
                    if self.started and time.monotonic() - self.started > 880:
                        raise TimeoutError("Retention deadline; reserve own stop/reap")
                    block = stream.read(min(1024 * 1024, limit - copied))
                    if not block:
                        break
                    output.write(block)
                    digest.update(block)
                    copied += len(block)
        return {
            "present": True,
            "bytes": copied,
            "sha256": digest.hexdigest(),
            "truncated": before.st_size > copied,
        }

    def cleanup(self) -> dict:
        """Partial bootstrap безопасен: нет guest calls, только exact own PID signals."""
        result: dict = {"task": str(self.task), "removed": False, "unknown": True}
        marker = self.task / "owner.json"
        if (
            not self.created_task
            or self.task.parent != TRANSIENT
            or self.task.name != "a4-serial-only-" + self.token
            or self.task.is_symlink()
            or not marker.is_file()
            or marker.is_symlink()
            or json.loads(marker.read_text())
            != {"token": self.token, "uid": os.getuid()}
        ):
            raise ValueError("Cleanup ownership guard before IO/signals")
        # До сигналов — bounded собственные seed/serial records вне TTL.
        retained = {}
        retention_error = False
        for kind in ("serial", "meta", "seed"):
            try:
                retained[kind] = self._retain(kind)
            except (OSError, ValueError, TimeoutError) as error:
                retention_error = True
                retained[kind] = {"error": str(error)}
        self._record("retention-before-stop.json", retained)
        if self.vm is not None:
            if self.vm.poll() is None:
                self._check_vm()
                self.vm.terminate()
                try:
                    self.vm.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    self._check_vm()
                    self.vm.kill()
                    self.vm.wait(timeout=5)
            else:
                self.vm.wait(timeout=1)
            result["vm_exit"] = self.vm.returncode
        refs = common.references(self.task)
        actual_base_sha = common.sha(IMAGE)
        stable = (
            self.base_stat is not None
            and self._base_identity() == self.base_stat
            and actual_base_sha == BASE_SHA
        )
        result |= {
            "references": refs,
            "base_unchanged": stable,
            "base_sha256_after": actual_base_sha,
            "retained": retained,
        }
        marker = self.task / "owner.json"
        own = (
            self.task.parent == TRANSIENT
            and self.task.name == "a4-serial-only-" + self.token
            and not self.task.is_symlink()
            and marker.is_file()
            and not marker.is_symlink()
            and json.loads(marker.read_text())
            == {"token": self.token, "uid": os.getuid()}
        )
        if own and not refs and stable and not retention_error:
            # Consistent failed overlay только после QEMU reap, до scratch delete.
            retained["disk"] = self._retain("disk")
            shutil.rmtree(self.task)
            result |= {"removed": True, "unknown": False}
        self._record("cleanup.json", result)
        for path in self.out.rglob("*"):
            if not path.is_symlink():
                path.chmod(0o700 if path.is_dir() else 0o600)
        return result

    def _claim_authority(self, digest: str) -> None:
        if not protocol.HASH.fullmatch(digest):
            raise ValueError("Authority digest format")
        marker = self.out.parent / ("authority-used-" + digest + ".json")
        descriptor = os.open(
            marker, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600
        )
        with os.fdopen(descriptor, "w") as stream:
            json.dump({"token": self.token, "attempts": 1}, stream)

    def execute(self) -> None:
        # Отсутствующий future decision блокирует ДО создания каталогов/seed/overlay.
        authority_bytes = DECISION.read_bytes()
        authority_digest = hashlib.sha256(authority_bytes).hexdigest()
        authority = protocol.decode_unique(authority_bytes.decode())
        validate_authority(authority)
        self.authority = authority
        self.started = time.monotonic()
        locks = []
        previous_umask = os.umask(0o077)
        try:
            fd = os.open(common.CACHE, os.O_RDONLY | os.O_DIRECTORY)
            fcntl.flock(fd, fcntl.LOCK_SH)
            locks.append(fd)
            manifest = self._verify()
            self.out.mkdir(mode=0o700, parents=True)
            self.created_out = True
            self.task.mkdir(mode=0o700)
            self.created_task = True
            fd = os.open(self.task, os.O_RDONLY | os.O_DIRECTORY)
            fcntl.flock(fd, fcntl.LOCK_EX)
            locks.append(fd)
            (self.task / "owner.json").write_text(
                json.dumps({"token": self.token, "uid": os.getuid()})
            )
            self._record(
                "bindings.json",
                {
                    "authority": authority,
                    "authority_sha256": authority_digest,
                    "inputs": manifest,
                    "sources": sources(),
                    "base_identity_before": self.base_stat,
                },
            )
            for name in NAMES:
                shutil.copy2(Path(__file__).with_name(name), self.out / name)
                (self.out / name).chmod(0o600)
            self._claim_authority(authority_digest)
            self._prepare()
            validate_authority(authority)
            if common.sha(DECISION) != authority_digest:
                raise ValueError("Root authority changed before boot")
            with (self.out / "qemu-private.log").open("wb") as log:
                self.vm = subprocess.Popen(
                    self.argv,
                    stdin=subprocess.DEVNULL,
                    stdout=log,
                    stderr=log,
                    start_new_session=True,
                    env={"PATH": "/usr/bin", "LC_ALL": "C", "LANG": "C"},
                )
            self.vm_start = common.process_start(self.vm.pid)
            self._record(
                "active.json",
                {
                    "pid": self.vm.pid,
                    "uid": os.getuid(),
                    "starttime": self.vm_start,
                    "argv": self.argv,
                },
            )
            result = self._capture()
            self._record("result.json", result)
            if result["capture"] != "CAPTURED":
                raise ValueError("Serial transport incomplete UNKNOWN")
        except Exception as error:
            if self.created_out:
                self._record(
                    "failure.json",
                    {
                        "type": type(error).__name__,
                        "reason": str(error),
                        "fullA4": False,
                        "finalLinux": False,
                    },
                )
            raise
        finally:
            try:
                if self.created_task and self.task.exists() and self.created_out:
                    try:
                        self.cleanup()
                    except Exception as error:
                        self._record(
                            "cleanup-blocker.json",
                            {
                                "reason": str(error),
                                "task": str(self.task),
                                "retained": True,
                            },
                        )
                        raise
            finally:
                try:
                    if self.created_out:
                        manifest = []
                        for path in self.out.rglob("*"):
                            if path.is_symlink():
                                continue
                            path.chmod(0o700 if path.is_dir() else 0o600)
                            if path.is_file():
                                manifest.append(
                                    {
                                        "path": str(path.relative_to(self.out)),
                                        "bytes": path.stat().st_size,
                                        "sha256": common.sha(path),
                                    }
                                )
                        self._record("private-manifest.json", {"files": manifest})
                finally:
                    for descriptor in reversed(locks):
                        os.close(descriptor)
                    os.umask(previous_umask)


if __name__ == "__main__":
    SerialDriver().execute()
