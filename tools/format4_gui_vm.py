"""Одноразовая VM для production DesktopHost и настоящего gnome-keyring.

Host не подключается к Secret Service. Все секреты остаются внутри overlay.
"""

import fcntl
import hashlib
import json
import os
import shutil
import socket
import subprocess
import time
from pathlib import Path

BASE = Path("/home/myrs/projects/yonik/local/validation/vm-base")
ROOT = Path(
    os.environ.get("YONIK_VM_ROOT", "/var/tmp/codex-myrs/transient/secret-format4-gui")
)
EVIDENCE = Path(
    os.environ.get(
        "YONIK_VM_EVIDENCE",
        "/home/myrs/.t3/worktrees/yonik/t3-af487ac0-310beaee221833d5/local/validation/secret-format4-gui",
    )
)
IMAGE = "Arch-Linux-x86_64-cloudimg-20261101.620452.qcow2"
FINGERPRINT = "1B9A16984A4E8CB448712D2AE0B78BF4326C6F8F"


def run(args: list[str], **kwargs: object) -> str:
    return subprocess.run(
        args,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=300,
        **kwargs,
    ).stdout


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def execute() -> None:
    ROOT.mkdir(mode=0o700, exist_ok=True)
    EVIDENCE.mkdir(exist_ok=True)
    root_fd = os.open(ROOT, os.O_RDONLY | os.O_DIRECTORY)
    base_fd = os.open(BASE, os.O_RDONLY | os.O_DIRECTORY)
    fcntl.flock(root_fd, fcntl.LOCK_SH)
    fcntl.flock(base_fd, fcntl.LOCK_SH)
    provenance = json.loads((BASE / "manifest.json").read_text())
    assert provenance["primary_fingerprint"] == FINGERPRINT
    for name, expected in [
        (IMAGE, provenance["sha256"]),
        ("arch-ci-public-key.asc", provenance["public_key_sha256"]),
        (IMAGE + ".sig", provenance["signature_sha256"]),
    ]:
        assert digest(BASE / name) == expected
    gnupg = ROOT / "gnupg"
    gnupg.mkdir(mode=0o700, exist_ok=True)
    run(
        [
            "gpg",
            "--homedir",
            str(gnupg),
            "--import",
            str(BASE / "arch-ci-public-key.asc"),
        ]
    )
    signature = run(
        [
            "gpg",
            "--homedir",
            str(gnupg),
            "--status-fd",
            "1",
            "--verify",
            str(BASE / (IMAGE + ".sig")),
            str(BASE / IMAGE),
        ]
    )
    assert any(
        line.startswith("[GNUPG:] VALIDSIG ") and line.split()[-1] == FINGERPRINT
        for line in signature.splitlines()
    )
    run(["gpgconf", "--homedir", str(gnupg), "--kill", "gpg-agent"])
    info = json.loads(run(["qemu-img", "info", "--output=json", str(BASE / IMAGE)]))
    assert not info.get("backing-filename")
    if not (ROOT / "key").exists():
        run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(ROOT / "key")])
    seed = ROOT / "seed"
    seed.mkdir(exist_ok=True)
    (seed / "meta-data").write_text("instance-id: format4-custody-v1\n")
    (seed / "user-data").write_text(
        "#cloud-config\ndisable_root: false\nssh_pwauth: false\nusers:\n  - name: root\n    ssh_authorized_keys:\n      - "
        + (ROOT / "key.pub").read_text().strip()
        + "\nruncmd:\n  - [systemctl, enable, --now, sshd.service]\n  - [touch, /run/credential-ready]\n"
    )
    run(
        [
            "xorriso",
            "-as",
            "mkisofs",
            "-quiet",
            "-output",
            str(ROOT / "seed.iso"),
            "-volid",
            "cidata",
            "-joliet",
            "-rock",
            str(seed),
        ]
    )
    if not (ROOT / "disk.qcow2").exists():
        run(
            [
                "qemu-img",
                "create",
                "-f",
                "qcow2",
                "-F",
                "qcow2",
                "-b",
                str(BASE / IMAGE),
                str(ROOT / "disk.qcow2"),
                "12G",
            ]
        )
    shutil.copyfile("/usr/share/edk2/x64/OVMF_VARS.4m.fd", ROOT / "vars.fd")
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    command = [
        "qemu-system-x86_64",
        "-name",
        "credentiallife",
        "-machine",
        "q35,accel=kvm",
        "-cpu",
        "host",
        "-smp",
        "2",
        "-m",
        "4096",
        "-nodefaults",
        "-no-user-config",
        "-display",
        "none",
        "-monitor",
        "none",
        "-serial",
        f"file:{ROOT}/serial.log",
        "-drive",
        "if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF_CODE.4m.fd",
        "-drive",
        f"if=pflash,format=raw,file={ROOT}/vars.fd",
        "-device",
        "virtio-rng-pci",
        "-drive",
        f"file={ROOT}/disk.qcow2,if=virtio,format=qcow2",
        "-drive",
        f"file={ROOT}/seed.iso,if=ide,format=raw,media=cdrom,readonly=on",
        "-netdev",
        f"user,id=n,hostfwd=tcp:127.0.0.1:{port}-:22",
        "-device",
        "virtio-net-pci,netdev=n",
        "-no-reboot",
    ]
    ssh = [
        "ssh",
        "-i",
        str(ROOT / "key"),
        "-p",
        str(port),
        "-o",
        "StrictHostKeyChecking=accept-new",
        "-o",
        f"UserKnownHostsFile={ROOT}/known_hosts",
        "-o",
        "ConnectTimeout=3",
        "root@127.0.0.1",
    ]
    vm = subprocess.Popen(command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        deadline = time.monotonic() + 240
        while time.monotonic() < deadline:
            try:
                run(ssh + ["systemctl is-active sshd.service"])
                break
            except subprocess.CalledProcessError:
                if vm.poll() is not None:
                    raise RuntimeError("VM exited")
                time.sleep(5)
        else:
            raise TimeoutError("VM SSH deadline")
        try:
            run(
                ssh
                + [
                    "pacman -Syu --noconfirm --needed gnome-keyring dbus python xorg-server-xvfb xdotool imagemagick qt6-declarative qt6-svg qt6-wayland openssl-3 openbox tint2"
                ]
            )
        except subprocess.CalledProcessError as error:
            (EVIDENCE / "packages.log").write_text(error.stdout + error.stderr)
            raise
        run(
            ssh
            + [
                "id fixture >/dev/null 2>&1 || useradd -m fixture; install -d -m700 -o fixture -g fixture /home/fixture/runtime"
            ]
        )
        (ROOT / "connection.json").write_text(
            json.dumps({"ssh": ssh, "vm_pid": vm.pid})
        )
        deadline = time.monotonic() + 3600
        while not (ROOT / "finish").exists():
            if time.monotonic() > deadline or vm.poll() is not None:
                raise RuntimeError("VM acceptance deadline")
            time.sleep(1)
    finally:
        try:
            run(ssh + ["poweroff"])
            vm.wait(timeout=20)
        except (subprocess.SubprocessError, OSError):
            pass
        if vm.poll() is None:
            vm.terminate()
        try:
            vm.wait(timeout=15)
        except subprocess.TimeoutExpired:
            vm.kill()
            vm.wait()
        os.close(base_fd)
        os.close(root_fd)


if __name__ == "__main__":
    execute()
