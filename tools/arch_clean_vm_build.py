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
IMAGE = "Arch-Linux-x86_64-cloudimg-20261101.620452.qcow2"
BASE = Path("/home/myrs/projects/yonik/local/validation/vm-base")
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


def archive_manifest(packages: tuple[Path, Path], evidence: Path) -> None:
    """До VM effects фиксирует настоящие архивы и ELF; не перепаковывает их."""
    if any(
        (evidence / name).exists()
        for name in ("versioned-inputs.json", "versioned-run.json")
    ):
        raise ValueError(
            "Evidence уже использован; FAIL/PASS run нельзя перезаписывать"
        )
    rows = []
    for path, expected, revision in zip(
        packages,
        (
            "e53cb758ca1b9a2167a80c90ca50b1ab5eb4f6b67c5c8ec01c2bf0ccb856e8b0",
            "a10f0840737b2ababc572fbd432335abfd5c232c59ec0493aa0483d749abafcc",
        ),
        (
            "eec7a56b6b03f2b022a87682f9097ce78b61fa0b",
            "6cf65a4cf9760a7df68b8eee10f012770b38604d",
        ),
        strict=True,
    ):
        actual = digest(path)
        if actual != expected:
            raise ValueError(f"Не совпал принятый checksum: {path}")
        files = {}
        for member in (
            ".PKGINFO",
            ".BUILDINFO",
            "usr/bin/yonik-desktop",
            "usr/lib/yonik/yonik-helper",
            "usr/lib/yonik/yonik-worker",
        ):
            data = subprocess.check_output(["bsdtar", "-xOf", str(path), member])
            if member.startswith("usr/") and not data.startswith(b"\x7fELF"):
                raise ValueError("В архиве ожидался ELF")
            files[member] = {
                "sha256": hashlib.sha256(data).hexdigest(),
                "bytes": len(data),
            }
            if member.startswith("."):
                files[member]["text"] = data.decode()
        rows.append(
            {
                "path": str(path),
                "sha256": actual,
                "source_revision": revision,
                "accepted_source_archive_sha256": {
                    "eec7a56b6b03f2b022a87682f9097ce78b61fa0b": "71e9c347ffc363da062b42b0c5ffc6ccb0a97b0988202ce21391dd1c2553a1d3",
                    "6cf65a4cf9760a7df68b8eee10f012770b38604d": "5a3d3d3b4a418d5208436934c3a7f1032d23fc83444d1948060d3c802087f665",
                }[revision],
                "members": files,
            }
        )
    (evidence / "versioned-inputs.json").write_text(
        json.dumps(
            {
                "archives": rows,
                "rust_build": False,
                "provenance": "Принятые historical #78 / checkpoint #119; archive ELF hashes не являются новым source→ELF proof",
            },
            indent=2,
            ensure_ascii=False,
        )
    )
    (evidence / "versioned-run.json").write_text(
        json.dumps({"status": "PLANNED", "effects_started": False})
    )


def versioned_lifecycle(
    ssh: list[str], packages: tuple[Path, Path], repo: Path, evidence: Path
) -> None:
    """Продолжает после reboot только с принятыми готовыми пакетами."""
    for path, name in zip(packages, ("old", "new"), strict=True):
        with path.open("rb") as stream:
            subprocess.run(
                ssh + [f"cat > /home/builder/{name}.pkg.tar.zst"],
                stdin=stream,
                check=True,
            )
    with (repo / "tools/arch_versioned_package.py").open("rb") as stream:
        subprocess.run(
            ssh + ["cat > /home/builder/versioned.py"], stdin=stream, check=True
        )
    result = "FAIL"
    try:
        run(ssh + ["python /home/builder/versioned.py"], evidence / "versioned.log")
        result = "PASS"
    finally:
        with (evidence / "versioned-artifacts.tar").open("wb") as stream:
            subprocess.run(
                ssh
                + [
                    "tar -C /home/builder -cf - versioned-evidence versioned versioned.py"
                ],
                stdout=stream,
                check=True,
            )
        (evidence / "versioned-run.json").write_text(
            json.dumps({"status": result, "effects_started": True, "rust_build": False})
        )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--task-root", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--arch-public-key", type=Path, required=True)
    parser.add_argument("--old-package", type=Path)
    parser.add_argument("--new-package", type=Path)
    args = parser.parse_args()
    if bool(args.old_package) != bool(args.new_package):
        parser.error("Для versioned lifecycle нужны оба архива")
    root = args.task_root.resolve()
    evidence = args.evidence.resolve()
    repo = Path(__file__).resolve().parents[1]
    if root.exists() or not root.is_relative_to(Path("/var/tmp/codex-myrs/transient")):
        raise ValueError("Нужен новый собственный transient task root")
    if evidence.is_relative_to(root):
        raise ValueError("Evidence должен находиться вне TTL")
    evidence.mkdir(parents=True, exist_ok=True)
    packages = None
    if args.old_package:
        packages = (args.old_package.resolve(), args.new_package.resolve())
        archive_manifest(packages, evidence)
    root.mkdir(mode=0o700)
    lock = os.open(root, os.O_RDONLY)
    try:
        fcntl.flock(lock, fcntl.LOCK_EX)
        base_lock = os.open(BASE, os.O_RDONLY)
        try:
            fcntl.flock(base_lock, fcntl.LOCK_SH)
            with (BASE / "manifest.json").open() as manifest:
                if (
                    json.load(manifest)["sha256"]
                    != "794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003"
                ):
                    raise ValueError("Изменён base manifest")
            if packages:
                (evidence / "versioned-run.json").write_text(
                    json.dumps(
                        {
                            "status": "RUNNING",
                            "effects_started": True,
                            "task_root": str(root),
                        }
                    )
                )
            execute(root, evidence, repo, args.arch_public_key, packages)
        finally:
            os.close(base_lock)
    except Exception as error:
        if packages:
            (evidence / "versioned-run.json").write_text(
                json.dumps(
                    {
                        "status": "FAIL",
                        "effects_started": True,
                        "task_root": str(root),
                        "error": str(error),
                    }
                )
            )
        raise
    finally:
        os.close(lock)


def execute(
    root: Path,
    evidence: Path,
    repo: Path,
    key: Path,
    packages: tuple[Path, Path] | None = None,
) -> None:
    """Весь жизненный цикл одной VM под lock вызывающей функции."""
    vm = None
    ssh: list[str] = []
    archive_success = False
    try:
        # Cache удерживается shared BSD lock до завершения VM; запись запрещена.
        for suffix in ("", ".SHA256", ".sig"):
            if not (BASE / (IMAGE + suffix)).is_file():
                raise ValueError("Неполный закреплённый base cache")
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
                str(BASE / (IMAGE + ".sig")),
                str(BASE / IMAGE),
            ],
            evidence / "image-signature.log",
        )
        if not any(
            line.startswith("[GNUPG:] VALIDSIG ") and line.split()[-1] == PRIMARY
            for line in signature.splitlines()
        ):
            raise ValueError("Подпись не принадлежит закреплённому primary Arch CI")
        run(["gpgconf", "--homedir", str(gnupg), "--kill", "gpg-agent"])
        actual = digest(BASE / IMAGE)
        if (
            actual != "794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003"
            or actual != (BASE / (IMAGE + ".SHA256")).read_text().split()[0]
        ):
            raise ValueError("SHA256 образа не совпал")
        info = json.loads(run(["qemu-img", "info", "--output=json", str(BASE / IMAGE)]))
        if (
            info["format"] != "qcow2"
            or info.get("backing-filename")
            or info["format-specific"]["data"].get("data-file")
        ):
            raise ValueError("Внешние ссылки в официальном образе")
        (evidence / "image.json").write_text(
            json.dumps(
                {
                    "cache": str(BASE / IMAGE),
                    "sha256": actual,
                    "primary": PRIMARY,
                    "public_key_sha256": digest(key),
                    "signature_sha256": digest(BASE / (IMAGE + ".sig")),
                },
                indent=2,
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
                str(BASE / IMAGE),
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
                "set -e; pacman -Syu --noconfirm --needed qt6-base qt6-declarative qt6-svg sqlite dbus polkit nftables iproute2 python"
                + ("" if packages else " base-devel clang pkgconf rustup")
            ],
            evidence / "dependencies.log",
        )
        # После -Syu kernel и modules должны относиться к одной загрузке.
        before_boot = run(ssh + ["cat /proc/sys/kernel/random/boot_id"]).strip()
        before_kernel = run(ssh + ["uname -r"]).strip()
        (evidence / "reboot-before.json").write_text(
            json.dumps({"boot_id": before_boot, "kernel": before_kernel})
        )
        run(ssh + ["systemctl reboot"])
        reboot_deadline = time.monotonic() + 240
        while time.monotonic() < reboot_deadline:
            time.sleep(5)
            probe = subprocess.run(
                ssh
                + [
                    "test -d /usr/lib/modules/$(uname -r) && cat /proc/sys/kernel/random/boot_id"
                ],
                capture_output=True,
                text=True,
            )
            if probe.returncode == 0 and probe.stdout.strip() != before_boot:
                break
        else:
            raise TimeoutError("Гость не загрузился с согласованными kernel/modules")
        after_kernel = run(ssh + ["uname -r"]).strip()
        (evidence / "reboot-after.json").write_text(
            json.dumps({"boot_id": probe.stdout.strip(), "kernel": after_kernel})
        )
        if packages and after_kernel == before_kernel:
            raise ValueError("Для этого gate требуется actual kernel change")
        run(
            ssh
            + [
                "set -e; useradd -m builder; install -d -o builder -g builder /home/builder/input /home/builder/task; echo 'builder ALL=(ALL) NOPASSWD: /usr/bin/pacman' > /etc/sudoers.d/yonik-build"
            ]
        )
        if packages:
            versioned_lifecycle(ssh, packages, repo, evidence)
            archive_success = True
            return
        transfer = [
            "tar",
            "-C",
            str(repo),
            "-cf",
            str(root / "input.tar"),
            "packaging/arch/PKGBUILD",
            "tools/check_arch_package.py",
            "tools/arch_package_lifecycle.sh",
            "tools/compile_notices.py",
            "tools/check_linux_notices.py",
            "packaging/licenses",
            "LICENSE",
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
        run(
            ssh
            + [
                "su - builder -s /bin/bash -c 'rustup toolchain install 1.85.1 --profile minimal'"
            ],
            evidence / "toolchain.log",
        )
        guest = """set -euo pipefail
cd /home/builder/input/packaging/arch
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
                "set -e; cd /home/builder/input/packaging/arch; sha256sum PKGBUILD yonik-*.tar.gz *.pkg.tar.zst /home/builder/task/target/release/yonik-desktop /home/builder/task/target/release/yonik-helper /home/builder/task/target/release/yonik-worker; bsdtar -xOf *.pkg.tar.zst .BUILDINFO; strings /home/builder/task/target/release/yonik-desktop | grep -F '0.1.0+6cf65a4cf9760a7df68b8eee10f012770b38604d'"
            ],
            evidence / "binary-provenance.log",
        )
        run(
            ssh
            + [
                'set -e; cd /home/builder/input; python tools/check_arch_package.py packaging/arch/*.pkg.tar.zst; pacman -U --noconfirm packaging/arch/*.pkg.tar.zst; pacman -Qkk yonik; if helper_state=$(systemctl is-active yonik-helper.service); then exit 1; else test $? = 3 && test "$helper_state" = inactive; fi'
            ],
            evidence / "install.log",
        )
        run(
            ssh
            + ["bash /home/builder/input/tools/arch_package_lifecycle.sh installed"],
            evidence / "integration.log",
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
            ssh + ["bash /home/builder/input/tools/arch_package_lifecycle.sh cycles"],
            evidence / "lifecycle.log",
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
        if archive_success:
            if vm is None or vm.returncode != 0:
                raise RuntimeError(
                    "VM завершилась нештатно; собственный overlay сохранён"
                )
            shutil.rmtree(root)
        # При ошибке диск сохраняется для диагностики; автоматическая очистка только после успеха.
    shutil.rmtree(root)


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO)
    main()
