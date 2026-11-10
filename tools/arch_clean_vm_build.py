#!/usr/bin/env python3
"""Одноразовая официальная Arch VM: clean makepkg, установка и Qt smoke.

Не устанавливает пакеты на host. Task root удерживается flock всё время;
тяжёлая guest-команда занимает общий host Rust slot. Evidence остаётся вне TTL.
"""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import logging
import os
import shutil
import socket
import subprocess
import time
import uuid
from pathlib import Path

LOG = logging.getLogger(__name__)
PRIMARY = "1B9A16984A4E8CB448712D2AE0B78BF4326C6F8F"
IMAGE = "Arch-Linux-x86_64-cloudimg.qcow2"
URL = "https://mirror.pkgbuild.com/images/latest/"
SLOT = "/home/myrs/projects/yonik/local/orchestration/with-rust-slot"


def run(command: list[str], log: Path | None = None) -> str:
    """Исполняет команду с fail-closed статусом и постоянным логом."""
    LOG.info("Запуск: %s", command[0])
    result = subprocess.run(
        command, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT
    )
    if log:
        log.write_text(result.stdout)
    result.check_returncode()
    return result.stdout


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--task-root", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--arch-public-key", type=Path, required=True)
    args = parser.parse_args()
    root = args.task_root.resolve()
    evidence = args.evidence.resolve()
    repo = Path(__file__).resolve().parents[1]
    if root.exists() or not root.is_relative_to(Path("/var/tmp/codex-myrs/transient")):
        raise ValueError("Нужен новый собственный transient task root")
    if evidence.is_relative_to(root):
        raise ValueError("Evidence должен находиться вне TTL")
    root.mkdir(mode=0o700)
    evidence.mkdir(parents=True, exist_ok=True)
    lock = os.open(root, os.O_RDONLY)
    try:
        fcntl.flock(lock, fcntl.LOCK_EX)
        execute(root, evidence, repo, args.arch_public_key)
    finally:
        os.close(lock)


def execute(root: Path, evidence: Path, repo: Path, key: Path) -> None:
    """Весь жизненный цикл одной VM под lock вызывающей функции."""
    vm = None
    ssh: list[str] = []
    try:
        for suffix in ("", ".SHA256", ".sig"):
            run(
                [
                    "curl",
                    "-4",
                    "-fL",
                    "--connect-timeout",
                    "20",
                    "--max-time",
                    "900",
                    "--retry",
                    "1",
                    URL + IMAGE + suffix,
                    "-o",
                    str(root / (IMAGE + suffix)),
                ]
            )
        gnupg = root / "gnupg"
        gnupg.mkdir(mode=0o700)
        run(["gpg", "--homedir", str(gnupg), "--import", str(key)])
        signature = run(
            [
                "gpg",
                "--homedir",
                str(gnupg),
                "--status-fd=1",
                "--verify",
                str(root / (IMAGE + ".sig")),
                str(root / IMAGE),
            ],
            evidence / "image-signature.log",
        )
        if not any(
            line.startswith("[GNUPG:] VALIDSIG ") and line.split()[-1] == PRIMARY
            for line in signature.splitlines()
        ):
            raise ValueError("Подпись не принадлежит закреплённому primary Arch CI")
        run(["gpgconf", "--homedir", str(gnupg), "--kill", "gpg-agent"])
        actual = digest(root / IMAGE)
        if actual != (root / (IMAGE + ".SHA256")).read_text().split()[0]:
            raise ValueError("SHA256 образа не совпал")
        info = json.loads(run(["qemu-img", "info", "--output=json", str(root / IMAGE)]))
        if (
            info["format"] != "qcow2"
            or info.get("backing-filename")
            or info["format-specific"]["data"].get("data-file")
        ):
            raise ValueError("Внешние ссылки в официальном образе")
        (evidence / "image.json").write_text(
            json.dumps(
                {"url": URL + IMAGE, "sha256": actual, "primary": PRIMARY}, indent=2
            )
        )
        run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(root / "key")])
        seed = root / "seed"
        seed.mkdir()
        (seed / "meta-data").write_text(f"instance-id: yonik-clean-{uuid.uuid4()}\n")
        (seed / "user-data").write_text(
            "#cloud-config\ndisable_root: false\nssh_pwauth: false\nusers:\n  - name: root\n    ssh_authorized_keys:\n      - "
            + (root / "key.pub").read_text().strip()
            + "\nruncmd:\n  - [systemctl, enable, --now, sshd.service]\n  - [touch, /run/yonik-cloud-ready]\n"
        )
        run(
            [
                "xorriso",
                "-as",
                "mkisofs",
                "-quiet",
                "-output",
                str(root / "seed.iso"),
                "-volid",
                "cidata",
                "-joliet",
                "-rock",
                str(seed),
            ]
        )
        run(
            [
                "qemu-img",
                "create",
                "-f",
                "qcow2",
                "-F",
                "qcow2",
                "-b",
                str(root / IMAGE),
                str(root / "disk.qcow2"),
                "32G",
            ]
        )
        shutil.copyfile("/usr/share/edk2/x64/OVMF_VARS.4m.fd", root / "vars.fd")
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        command = [
            "qemu-system-x86_64",
            "-name",
            "yonik-arch-clean",
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
            f"file:{root}/serial.log",
            "-drive",
            "if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF_CODE.4m.fd",
            "-drive",
            f"if=pflash,format=raw,file={root}/vars.fd",
            "-device",
            "virtio-rng-pci",
            "-drive",
            f"file={root}/disk.qcow2,if=virtio,format=qcow2",
            "-drive",
            f"file={root}/seed.iso,if=ide,format=raw,media=cdrom,readonly=on",
            "-netdev",
            f"user,id=n,hostfwd=tcp:127.0.0.1:{port}-:22",
            "-device",
            "virtio-net-pci,netdev=n",
            "-no-reboot",
        ]
        (evidence / "qemu-command.json").write_text(json.dumps(command))
        with (root / "qemu.log").open("w") as output:
            vm = subprocess.Popen(command, stdout=output, stderr=subprocess.STDOUT)
        ssh = [
            "ssh",
            "-i",
            str(root / "key"),
            "-p",
            str(port),
            "-o",
            "StrictHostKeyChecking=accept-new",
            "-o",
            f"UserKnownHostsFile={root}/known_hosts",
            "-o",
            "ConnectTimeout=5",
            "root@127.0.0.1",
        ]
        deadline = time.monotonic() + 240
        while time.monotonic() < deadline:
            probe = subprocess.run(
                ssh + ["test -f /run/yonik-cloud-ready"], capture_output=True
            )
            if probe.returncode == 0:
                break
            if vm.poll() is not None:
                raise RuntimeError("VM завершилась до SSH")
            time.sleep(5)
        else:
            raise TimeoutError("Cloud-init не готов за 240 секунд")
        run(
            ssh
            + [
                "set -e; pacman -Syu --noconfirm --needed base-devel clang pkgconf rustup qt6-base qt6-declarative qt6-svg sqlite dbus polkit nftables iproute2 python"
            ],
            evidence / "dependencies.log",
        )
        run(
            ssh
            + [
                "set -e; useradd -m builder; install -d -o builder -g builder /home/builder/input /home/builder/task; echo 'builder ALL=(ALL) NOPASSWD: /usr/bin/pacman' > /etc/sudoers.d/yonik-build"
            ]
        )
        transfer = [
            "tar",
            "-C",
            str(repo),
            "-cf",
            str(root / "input.tar"),
            "packaging/arch/PKGBUILD",
            "tools/check_arch_package.py",
            "tools/desktop_ui_smoke.py",
            "tools/desktop_smoke.py",
            "tools/engine_compile.py",
            "tools/engine_host.py",
            "tools/engine_smoke.py",
            "tools/traffic_echo.py",
            "packaging/systemd",
            "packaging/sysusers",
            "packaging/polkit",
            "packaging/dbus",
            "crates/yonik-desktop/assets/yonik.svg",
        ]
        run(transfer)
        with (root / "input.tar").open("rb") as stream:
            subprocess.run(
                ssh
                + [
                    "set -e; tar -xf - -C /home/builder/input; chown -R builder:builder /home/builder/input"
                ],
                stdin=stream,
                check=True,
            )
        guest = """set -euo pipefail
cd /home/builder/input/packaging/arch
rustup toolchain install 1.85.1 --profile minimal
export RUSTUP_TOOLCHAIN=1.85.1 CARGO_TARGET_DIR=/home/builder/task/target CARGO_BUILD_JOBS=1
cat > /home/builder/task/makepkg.conf <<'CONF'
source /etc/makepkg.conf
unset RUSTFLAGS CFLAGS CXXFLAGS CPPFLAGS LDFLAGS
OPTIONS=(strip docs !libtool !staticlibs emptydirs zipman purge !debug !lto !autodeps)
MAKEFLAGS="-j1"
CONF
flock -s /home/builder/task makepkg --config /home/builder/task/makepkg.conf --noconfirm
sha256sum *.pkg.tar.zst
rustc --version
pacman -Q
"""
        (evidence / "guest-build.sh").write_text(guest)
        with (root / "build.sh").open("w") as out:
            out.write(guest)
        with (
            (root / "build.sh").open("rb") as stream,
            (evidence / "build.log").open("w") as output,
        ):
            subprocess.run(
                [SLOT, *ssh, "su - builder -s /bin/bash -c 'bash -s'"],
                stdin=stream,
                stdout=output,
                stderr=subprocess.STDOUT,
                check=True,
            )
        with (evidence / "package.tar").open("wb") as stream:
            subprocess.run(
                ssh
                + [
                    "cd /home/builder/input/packaging/arch; tar -cf - yonik-*.pkg.tar.zst"
                ],
                stdout=stream,
                check=True,
            )
        run(
            ssh
            + [
                "set -e; cd /home/builder/input/packaging/arch; sha256sum PKGBUILD yonik-*.tar.gz *.pkg.tar.zst; bsdtar -xOf *.pkg.tar.zst .BUILDINFO; strings /home/builder/task/target/release/yonik-desktop | grep -F '0.1.0+eec7a56b6b03f2b022a87682f9097ce78b61fa0b'"
            ],
            evidence / "binary-provenance.log",
        )
        run(
            [
                SLOT,
                *ssh,
                "su - builder -s /bin/bash -c 'set -e; rustup component add --toolchain 1.85.1 clippy rustfmt; cd /home/builder/input/packaging/arch/src/yonik-*/; export CARGO_TARGET_DIR=/home/builder/task/target CARGO_BUILD_JOBS=1 YONIK_BUILD_ID=0.1.0+eec7a56b6b03f2b022a87682f9097ce78b61fa0b; cargo +1.85.1 fmt --all -- --check; flock -s /home/builder/task cargo +1.85.1 clippy --locked --release --workspace --all-targets -- -D warnings'",
            ],
            evidence / "rust-validation.log",
        )
        run(
            ssh
            + [
                "set -e; cd /home/builder/input; python tools/check_arch_package.py packaging/arch/*.pkg.tar.zst; pacman -U --noconfirm packaging/arch/*.pkg.tar.zst; pacman -Qkk yonik; systemctl is-active yonik-helper.service || test $? = 3"
            ],
            evidence / "install.log",
        )
        run(ssh + ["install -d -m700 -o builder -g builder /home/builder/smoke-home"])
        run(
            ssh
            + [
                "su - builder -s /bin/bash -c 'cd /home/builder/input; HOME=/home/builder/smoke-home XDG_CONFIG_HOME=/home/builder/smoke-home/config XDG_CACHE_HOME=/home/builder/smoke-home/cache XDG_DATA_HOME=/home/builder/smoke-home/data python tools/desktop_ui_smoke.py /usr/bin/yonik-desktop --output /home/builder/smoke'"
            ],
            evidence / "smoke.log",
        )
        with (evidence / "artifacts.tar").open("wb") as stream:
            subprocess.run(
                ssh
                + [
                    "cd /home/builder; tar -cf - input/packaging/arch/yonik-*.pkg.tar.zst smoke"
                ],
                stdout=stream,
                check=True,
            )
        run(
            [
                SLOT,
                *ssh,
                "su - builder -s /bin/bash -c 'flock -s /home/builder/task cargo +1.85.1 clean --manifest-path /home/builder/input/packaging/arch/src/yonik-*/Cargo.toml --target-dir /home/builder/task/target'",
            ],
            evidence / "cargo-clean.log",
        )
    finally:
        if vm is not None:
            if vm.poll() is None:
                try:
                    subprocess.run(ssh + ["poweroff"], capture_output=True, timeout=15)
                except subprocess.TimeoutExpired:
                    LOG.warning(
                        "SSH poweroff не завершился; ожидается завершение своей VM"
                    )
                try:
                    vm.wait(timeout=45)
                except subprocess.TimeoutExpired:
                    vm.terminate()
                    vm.wait(timeout=15)
            (evidence / "vm-exit.json").write_text(json.dumps({"exit": vm.returncode}))
        for name in ("serial.log", "qemu.log"):
            if (root / name).exists():
                shutil.copyfile(root / name, evidence / name)
        if (root / "gnupg").exists():
            subprocess.run(
                ["gpgconf", "--homedir", str(root / "gnupg"), "--kill", "gpg-agent"],
                check=False,
            )
        # При ошибке диск сохраняется для диагностики; автоматическая очистка только после успеха.
    shutil.rmtree(root)


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO)
    main()
