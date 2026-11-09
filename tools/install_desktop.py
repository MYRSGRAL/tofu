#!/usr/bin/env python3
"""Установка пользовательского пункта меню yonik без запуска приложения."""

from __future__ import annotations

import argparse
import configparser
import hashlib
import logging
import os
import stat
import tempfile
import unicodedata
import xml.etree.ElementTree as ET
from pathlib import Path

APP_ID = "cloud.highpriority.yonik.local"
LEGACY_APP_ID = "cloud.highpriority.tofu.local"
DEFAULT_ICON = (
    Path(__file__).resolve().parents[1] / "crates/yonik-desktop/assets/yonik.svg"
)


def absolute_path(value: Path) -> Path:
    """Запрет управляющих символов исключает внедрение строк desktop-entry."""
    if not value.is_absolute() or any(
        unicodedata.category(char) == "Cc" for char in str(value)
    ):
        raise ValueError(
            f"Требуется абсолютный путь без управляющих символов: {value!r}"
        )
    return value


def exec_argument(value: Path) -> str:
    """Два уровня экранирования: Exec quoting, затем desktop string escaping."""
    text = str(absolute_path(value)).replace("%", "%%")
    quoted = "".join("\\" + char if char in '\\"$`' else char for char in text)
    return '"' + quoted.replace("\\", "\\\\") + '"'


def render_entry(binary: Path, engine_dir: Path, icon_hash: str) -> str:
    if "=" in str(binary):
        raise ValueError("Спецификация Exec запрещает '=' в пути исполняемого файла")
    return (
        "[Desktop Entry]\n"
        "Type=Application\n"
        "Name=yonik\n"
        f"Exec={exec_argument(binary)} --engine-dir {exec_argument(engine_dir)}\n"
        f"Icon={APP_ID}\n"
        "Terminal=false\n"
        "StartupNotify=true\n"
        "Categories=Network;\n"
        "X-Yonik-Managed=true\n"
        f"X-Yonik-Icon-SHA256={icon_hash}\n"
    )


def stable_metadata(metadata: os.stat_result) -> tuple[int, ...]:
    """atime меняется от нашего чтения; остальные поля проверяем с точностью ns."""
    return (
        metadata.st_mode,
        metadata.st_ino,
        metadata.st_dev,
        metadata.st_nlink,
        metadata.st_uid,
        metadata.st_gid,
        metadata.st_size,
        metadata.st_mtime_ns,
        metadata.st_ctime_ns,
    )


def read_owned_file(path: Path) -> bytes | None:
    """Не читаем чужие файлы, специальные файлы и ссылки в месте установки."""
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        return None
    if (
        not stat.S_ISREG(metadata.st_mode)
        or metadata.st_uid != os.getuid()
        or metadata.st_nlink != 1
    ):
        raise ValueError(f"Нельзя изменять чужой файл или ссылку: {path}")
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as source:
        if stable_metadata(os.fstat(source.fileno())) != stable_metadata(metadata):
            raise ValueError(f"Файл изменился при проверке: {path}")
        return source.read()


def ensure_directory(path: Path) -> None:
    path.mkdir(exist_ok=True)
    metadata = path.lstat()
    if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != os.getuid():
        raise ValueError(f"Каталог установки должен принадлежать пользователю: {path}")


def atomic_write(path: Path, data: bytes, previous: bytes | None) -> None:
    descriptor, temporary = tempfile.mkstemp(prefix=".yonik-", dir=path.parent)
    temporary_path = Path(temporary)
    try:
        with os.fdopen(descriptor, "wb") as destination:
            destination.write(data)
            destination.flush()
            os.fchmod(destination.fileno(), 0o644)
            os.fsync(destination.fileno())
        if read_owned_file(path) != previous:
            raise ValueError(f"Файл изменился после проверки: {path}")
        if previous is None:
            # link не перезаписывает файл, появившийся между проверкой и записью.
            os.link(temporary_path, path, follow_symlinks=False)
        else:
            os.replace(temporary_path, path)
    finally:
        temporary_path.unlink(missing_ok=True)


def legacy_snapshot(
    applications: Path, icon_directory: Path
) -> list[tuple[Path, bytes, os.stat_result]]:
    """Чужой или изменённый старый launcher не препятствует новой установке."""
    entry = applications / f"{LEGACY_APP_ID}.desktop"
    icon = icon_directory / f"{LEGACY_APP_ID}.svg"
    try:
        snapshots = []
        for path in (entry, icon):
            try:
                metadata = path.lstat()
            except FileNotFoundError:
                continue
            data = read_owned_file(path)
            if data is not None:
                if stable_metadata(metadata) != stable_metadata(path.lstat()):
                    return []
                snapshots.append((path, data, metadata))
        if not snapshots or snapshots[0][0] != entry:
            return []
        configuration = configparser.ConfigParser(interpolation=None, strict=True)
        configuration.read_string(snapshots[0][1].decode("utf-8"))
        section = configuration["Desktop Entry"]
        if section.get("X-Tofu-Managed") != "true":
            return []
        if len(snapshots) == 2 and section.get("X-Tofu-Icon-SHA256") != (
            hashlib.sha256(snapshots[1][1]).hexdigest()
        ):
            return []
        return snapshots
    except (OSError, ValueError, KeyError, configparser.Error):
        return []


def remove_legacy(snapshots: list[tuple[Path, bytes, os.stat_result]]) -> None:
    """Повторная проверка пары перед удалением исключает подмену при установке."""
    for path, data, metadata in snapshots:
        try:
            unchanged = read_owned_file(path) == data and stable_metadata(
                path.lstat()
            ) == stable_metadata(metadata)
        except (OSError, ValueError):
            unchanged = False
        if not unchanged:
            logging.warning("Старый launcher изменился, сохранён: %s", path)
            return
    for path, data, metadata in reversed(snapshots):
        try:
            unchanged = read_owned_file(path) == data and stable_metadata(
                path.lstat()
            ) == stable_metadata(metadata)
        except (OSError, ValueError):
            unchanged = False
        if not unchanged:
            logging.warning("Старый launcher изменился, сохранён: %s", path)
            return
        # Entry удаляем последним: после сбоя он остаётся маркером владения,
        # даже если старая иконка уже удалена. Повтор завершит очистку пары.
        # Ошибка удаления возвращается вызывающему: повтор сможет завершить
        # миграцию, а успешная установка не оставляет управляемый дубликат меню.
        path.unlink()


def install_desktop(
    binary: Path,
    engine_dir: Path,
    icon: Path = DEFAULT_ICON,
    data_home: Path | None = None,
) -> Path:
    """Возвращает установленный desktop-entry; бинарник и движки не изменяются."""
    binary, engine_dir, icon = map(absolute_path, (binary, engine_dir, icon))
    # GIO проверяет существование Exec до раскрытия %% в имени программы.
    if "%" in str(binary):
        raise ValueError(
            "GIO не поддерживает '%' в пути бинарника; перенесите артефакт"
        )
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError(f"Бинарник должен быть исполняемым обычным файлом: {binary}")
    if not engine_dir.is_dir():
        raise ValueError(f"Каталог движков не найден: {engine_dir}")
    if not icon.is_file():
        raise ValueError(f"SVG-файл не найден: {icon}")
    icon_data = icon.read_bytes()
    if ET.fromstring(icon_data).tag != "{http://www.w3.org/2000/svg}svg":
        raise ValueError(f"Требуется SVG-иконка: {icon}")
    digest = hashlib.sha256(icon_data).hexdigest()
    entry_data = render_entry(binary, engine_dir, digest).encode("utf-8")
    if data_home is None:
        configured = os.environ.get("XDG_DATA_HOME")
        data_home = Path(configured) if configured else Path.home() / ".local/share"
    data_home = absolute_path(data_home)
    # Родитель XDG может ещё отсутствовать; его содержимое и права не меняются.
    data_home.parent.mkdir(parents=True, exist_ok=True)
    ensure_directory(data_home)
    applications = data_home / "applications"
    ensure_directory(applications)
    icon_directory = data_home
    for component in ("icons", "hicolor", "scalable", "apps"):
        icon_directory /= component
        ensure_directory(icon_directory)
    entry = applications / f"{APP_ID}.desktop"
    installed_icon = icon_directory / f"{APP_ID}.svg"
    legacy = legacy_snapshot(applications, icon_directory)
    old_entry = read_owned_file(entry)
    old_icon = read_owned_file(installed_icon)
    if old_entry is not None:
        configuration = configparser.ConfigParser(interpolation=None, strict=True)
        configuration.read_string(old_entry.decode("utf-8"))
        section = configuration["Desktop Entry"]
        if section.get("X-Yonik-Managed") != "true":
            raise ValueError(f"Существующий пункт меню не управляется yonik: {entry}")
        if (
            old_icon is not None
            and section.get("X-Yonik-Icon-SHA256")
            != hashlib.sha256(old_icon).hexdigest()
            and old_icon != icon_data
        ):
            raise ValueError(f"Существующая иконка изменена извне: {installed_icon}")
    elif old_icon is not None and old_icon != icon_data:
        raise ValueError(f"Существующая иконка не управляется yonik: {installed_icon}")
    # После сбоя между двумя atomic_write наша точная иконка уже на месте,
    # а entry ещё старый/отсутствует. Повтор завершает установку; чужие bytes
    # по-прежнему запрещены. Ссылки/владелец проверяются read_owned_file выше.
    atomic_write(installed_icon, icon_data, old_icon)
    atomic_write(entry, entry_data, old_entry)
    remove_legacy(legacy)
    return entry


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--engine-dir", required=True, type=Path)
    parser.add_argument("--icon", type=Path, default=DEFAULT_ICON)
    arguments = parser.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    try:
        entry = install_desktop(arguments.binary, arguments.engine_dir, arguments.icon)
    except (OSError, ValueError, KeyError, configparser.Error, ET.ParseError) as error:
        logging.error("Установка пункта меню не выполнена: %s", error)
        return 1
    logging.info("Пункт меню yonik установлен: %s", entry)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
