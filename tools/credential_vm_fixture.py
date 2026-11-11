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
ROOT = Path("/var/tmp/codex-myrs/transient/credentiallife-5b952cef")
EVIDENCE = Path("/home/myrs/projects/yonik/local/validation/credentiallife")
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
    binary = EVIDENCE / "credential_lifecycle"
    manifest = {
        "base": provenance,
        "head": run(["git", "rev-parse", "HEAD"]).strip(),
        "binary_sha256": digest(binary),
        "sources": {
            p: digest(Path(p))
            for p in run(["git", "ls-files"]).splitlines()
            if Path(p).is_file()
        },
        "fixture_sha256": digest(Path(__file__)),
        "example_sha256": digest(
            Path("crates/yonik-desktop/examples/credential_lifecycle.rs")
        ),
    }
    (EVIDENCE / "manifest.json").write_text(json.dumps(manifest, indent=2))
    if not (ROOT / "key").exists():
        run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(ROOT / "key")])
    seed = ROOT / "seed"
    seed.mkdir(exist_ok=True)
    (seed / "meta-data").write_text("instance-id: credentiallife-5b952cef\n")
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
        "2048",
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
            run(ssh + ["pacman -Syu --noconfirm --needed gnome-keyring dbus python"])
        except subprocess.CalledProcessError as error:
            (EVIDENCE / "packages.log").write_text(error.stdout + error.stderr)
            raise
        run(
            ssh
            + [
                "id fixture >/dev/null 2>&1 || useradd -m fixture; install -d -m700 -o fixture -g fixture /home/fixture/runtime"
            ]
        )
        subprocess.run(
            ssh
            + [
                "cat > /usr/local/bin/credential_lifecycle; chmod 755 /usr/local/bin/credential_lifecycle"
            ],
            input=binary.read_bytes(),
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        guest = Path(__file__).with_name("credential_vm_guest.py").read_text()
        subprocess.run(
            ssh
            + [
                "cat > /home/fixture/guest.py; chown fixture:fixture /home/fixture/guest.py"
            ],
            input=guest.encode(),
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        try:
            result = run(ssh + ["runuser -u fixture -- python /home/fixture/guest.py"])
        except subprocess.CalledProcessError:
            partial = run(ssh + ["cat /home/fixture/results.json"])
            (EVIDENCE / "partial-service.json").write_text(
                json.dumps(json.loads(partial), indent=2)
            )
            raise
        # Guest stdout содержит только публичные результаты, никогда D-Bus payload.
        parsed = json.loads(result)
        (EVIDENCE / "actual-service.json").write_text(json.dumps(parsed, indent=2))
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
