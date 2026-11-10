#!/usr/bin/env python3
"""Проверить архив без распаковки и исполнения пакетных hooks (tar или .zst)."""

import argparse
import hashlib
import subprocess
import tarfile
from pathlib import Path, PurePosixPath

DESKTOP = """[Desktop Entry]
Type=Application
Name=yonik
Comment=Управление прокси-подключениями
Exec=/usr/bin/yonik-desktop
Icon=cloud.highpriority.yonik
Terminal=false
Categories=Network;
""".encode()


def expected_files(root: Path) -> dict[str, tuple[int, bytes | None]]:
    files = {
        name: (0o755, None)
        for name in (
            "usr/bin/yonik-desktop",
            "usr/lib/yonik/yonik-helper",
            "usr/lib/yonik/yonik-worker",
        )
    }
    for source, destination in (
        ("systemd", "usr/lib/systemd/system"),
        ("sysusers", "usr/lib/sysusers.d"),
        ("polkit", "usr/share/polkit-1/actions"),
        ("dbus", "usr/share/dbus-1"),
    ):
        for path in (root / "packaging" / source).iterdir():
            parent = destination
            if source == "dbus":
                parent += "/system.d" if path.suffix == ".conf" else "/system-services"
            files[f"{parent}/{path.name}"] = (0o644, path.read_bytes())
    files["usr/share/icons/hicolor/scalable/apps/cloud.highpriority.yonik.svg"] = (
        0o644,
        (root / "crates/yonik-desktop/assets/yonik.svg").read_bytes(),
    )
    files["usr/share/applications/cloud.highpriority.yonik.desktop"] = (0o644, DESKTOP)
    return files


def validate_archive(path: Path, root: Path) -> None:
    expected = expected_files(root)
    seen = set()
    # zstd декодирует архив makepkg без дополнительного Python-модуля.
    process = None
    if path.suffix == ".zst":
        process = subprocess.Popen(["zstd", "-dc", str(path)], stdout=subprocess.PIPE)
    try:
        with (
            tarfile.open(fileobj=process.stdout, mode="r|")
            if process
            else tarfile.open(path) as archive
        ):
            for entry in archive:
                name = entry.name.removeprefix("./").rstrip("/")
                if (
                    name in seen
                    or ".." in PurePosixPath(name).parts
                    or name.startswith("/")
                ):
                    raise ValueError(f"Недопустимый путь: {name}")
                seen.add(name)
                if len(seen) > 128 or entry.size > 512 * 1024 * 1024:
                    raise ValueError("Превышен лимит пакетного архива")
                if entry.uid != 0 or entry.gid != 0 or entry.mode & 0o7022:
                    raise ValueError(f"Небезопасные права: {name}")
                if entry.isdir():
                    if entry.mode != 0o755:
                        raise ValueError(f"Неверные права каталога: {name}")
                    if not any(key.startswith(name + "/") for key in expected):
                        raise ValueError(f"Лишний каталог: {name}")
                    continue
                if not entry.isfile():
                    raise ValueError(f"Ссылка/специальный файл: {name}")
                if name in (".PKGINFO", ".BUILDINFO", ".MTREE"):
                    continue
                if name not in expected:
                    raise ValueError(f"Лишний файл/hook: {name}")
                mode, content = expected[name]
                if entry.mode != mode:
                    raise ValueError(f"Неверный mode: {name}")
                stream = archive.extractfile(entry)
                assert stream is not None
                if mode == 0o755:
                    if stream.read(4) != b"\x7fELF":
                        raise ValueError(f"Ожидался ELF: {name}")
                elif content is not None and (
                    entry.size != len(content)
                    or stream.read(len(content) + 1) != content
                ):
                    raise ValueError(f"Подменены metadata: {name}")
        if not expected.keys() <= seen:
            raise ValueError(f"Отсутствуют файлы: {expected.keys() - seen}")
    finally:
        if process:
            process.stdout.close()
            if process.wait() != 0:
                raise ValueError("Ошибка декомпрессии")


def validate_recipe(path: Path) -> None:
    """Fail-closed seal: любое изменение исполняемого shell требует нового review.

    Не пытаемся распознавать произвольный Bash через blacklist: подстановки,
    функции и aliases делают такой анализ ненадёжным. Hash фиксирует целиком
    рецепт без hooks, автозапуска и сетевой установки engine.
    """
    approved = "ac9471bafbb87f50d5552c7d1f0e377786912ecb9db9b55abdb32d1784ba2326"
    if hashlib.sha256(path.read_bytes()).hexdigest() != approved:
        raise ValueError(
            "Изменён рецепт: требуется независимая проверка и обновление seal"
        )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path, nargs="?")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    validate_recipe(root / "packaging/arch/PKGBUILD")
    if args.archive:
        validate_archive(args.archive, root)


if __name__ == "__main__":
    main()
