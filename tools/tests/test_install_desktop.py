"""Проверки пользовательского launcher без изменения настоящего XDG-каталога."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import install_desktop as installer  # noqa: E402


class InstallDesktopTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="yonik-launcher-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.data = self.root / "xdg"
        self.binary = self.root / "yonik"
        self.binary.write_text("#!/bin/sh\nexit 0\n")
        self.binary.chmod(0o755)
        self.icon = self.root / "yonik.svg"
        self.icon.write_text('<svg xmlns="http://www.w3.org/2000/svg"/>')
        self.engines = self.root / "engines"
        self.engines.mkdir()

    def install(self) -> Path:
        return installer.install_desktop(
            self.binary, self.engines, self.icon, data_home=self.data
        )

    def legacy_launcher(self) -> tuple[Path, Path]:
        entry = self.install()
        icon = self.data / f"icons/hicolor/scalable/apps/{installer.APP_ID}.svg"
        legacy_entry = entry.with_name(f"{installer.LEGACY_APP_ID}.desktop")
        legacy_icon = icon.with_name(f"{installer.LEGACY_APP_ID}.svg")
        legacy_entry.write_text(
            entry.read_text().replace("yonik", "tofu").replace("X-Yonik-", "X-Tofu-")
        )
        legacy_icon.write_bytes(icon.read_bytes())
        entry.unlink()
        icon.unlink()
        return legacy_entry, legacy_icon

    def test_managed_legacy_migrates_and_repeat_has_one_menu_entry(self) -> None:
        legacy_entry, legacy_icon = self.legacy_launcher()
        entry = self.install()
        self.assertIn("Name=yonik\n", entry.read_text())
        self.assertIn("X-Yonik-Managed=true\n", entry.read_text())
        self.assertFalse(legacy_entry.exists())
        self.assertFalse(legacy_icon.exists())
        self.install()
        self.assertEqual(list(entry.parent.glob("*.desktop")), [entry])

    def test_foreign_legacy_preserved_without_blocking_install(self) -> None:
        for content in (b"foreign", b"[Desktop Entry]\nName=tofu\n"):
            with self.subTest(content=content):
                entry, icon = self.legacy_launcher()
                entry.write_bytes(content)
                old_icon = icon.read_bytes()
                self.assertTrue(self.install().exists())
                self.assertEqual(entry.read_bytes(), content)
                self.assertEqual(icon.read_bytes(), old_icon)

    def test_altered_legacy_icon_preserves_pair(self) -> None:
        entry, icon = self.legacy_launcher()
        original = entry.read_bytes()
        icon.write_bytes(b"foreign icon")
        self.assertTrue(self.install().exists())
        self.assertEqual(entry.read_bytes(), original)
        self.assertEqual(icon.read_bytes(), b"foreign icon")

    def test_legacy_replacement_during_install_preserved(self) -> None:
        for change in ("content", "inode", "symlink"):
            with self.subTest(change=change):
                entry, icon = self.legacy_launcher()
                original = entry.read_bytes()
                write = installer.atomic_write

                def replace_entry(
                    path: Path, data: bytes, previous: bytes | None
                ) -> None:
                    write(path, data, previous)
                    if path.suffix == ".desktop":
                        replacement = self.root / "replacement.desktop"
                        replacement.write_bytes(original)
                        if change == "content":
                            entry.write_bytes(b"foreign")
                        elif change == "inode":
                            os.replace(replacement, entry)
                        else:
                            entry.unlink()
                            entry.symlink_to(replacement)

                with patch.object(installer, "atomic_write", replace_entry):
                    self.assertTrue(self.install().exists())
                self.assertTrue(entry.exists())
                self.assertTrue(icon.exists())
                if entry.is_symlink():
                    entry.unlink()

    def test_legacy_icon_symlink_and_hardlink_preserved(self) -> None:
        for kind in ("symlink", "hardlink"):
            with self.subTest(kind=kind):
                entry, icon = self.legacy_launcher()
                original = entry.read_bytes()
                foreign = self.root / f"foreign-{kind}.svg"
                foreign.write_bytes(icon.read_bytes())
                icon.unlink()
                if kind == "symlink":
                    icon.symlink_to(foreign)
                else:
                    os.link(foreign, icon)
                self.assertTrue(self.install().exists())
                self.assertEqual(entry.read_bytes(), original)
                self.assertTrue(icon.exists())
                icon.unlink()

    def test_legacy_icon_changed_during_install_preserves_pair(self) -> None:
        entry, icon = self.legacy_launcher()
        original = entry.read_bytes()
        write = installer.atomic_write

        def replace_icon(path: Path, data: bytes, previous: bytes | None) -> None:
            write(path, data, previous)
            if path.suffix == ".desktop":
                icon.write_bytes(b"foreign")

        with patch.object(installer, "atomic_write", replace_icon):
            self.assertTrue(self.install().exists())
        self.assertEqual(entry.read_bytes(), original)
        self.assertEqual(icon.read_bytes(), b"foreign")

    def test_legacy_cleanup_failure_can_be_retried(self) -> None:
        for fail_target in ("entry", "icon"):
            with self.subTest(fail_target=fail_target):
                legacy_entry, legacy_icon = self.legacy_launcher()
                unlink = Path.unlink
                blocked = legacy_entry if fail_target == "entry" else legacy_icon

                def fail_legacy(path: Path, missing_ok: bool = False) -> None:
                    if path == blocked:
                        raise PermissionError("Удаление запрещено")
                    unlink(path, missing_ok=missing_ok)

                with patch.object(Path, "unlink", fail_legacy):
                    with self.assertRaises(PermissionError):
                        self.install()
                self.assertTrue(legacy_entry.exists())
                self.assertEqual(legacy_icon.exists(), fail_target == "icon")
                self.install()
                self.assertFalse(legacy_entry.exists())
                self.assertFalse(legacy_icon.exists())

    def test_legacy_access_time_change_does_not_block_migration(self) -> None:
        entry, icon = self.legacy_launcher()
        pending = {entry, icon}
        lstat = Path.lstat

        def before_first_read(path: Path, **kwargs: bool) -> os.stat_result:
            metadata = lstat(path, **kwargs)
            if path not in pending:
                return metadata
            pending.remove(path)
            # На relatime чтение меняет atime; моделируем это и на noatime.
            fields = list(metadata)
            fields[stat.ST_ATIME] -= 1
            return os.stat_result(
                fields,
                {
                    "st_atime_ns": metadata.st_atime_ns - 1_000_000_000,
                    "st_mtime_ns": metadata.st_mtime_ns,
                    "st_ctime_ns": metadata.st_ctime_ns,
                },
            )

        with patch.object(Path, "lstat", before_first_read):
            self.assertTrue(self.install().exists())
        self.assertFalse(entry.exists())
        self.assertFalse(icon.exists())

    def test_legacy_survives_partial_write_and_retry_migrates(self) -> None:
        legacy_entry, legacy_icon = self.legacy_launcher()
        original = legacy_entry.read_bytes()
        write = installer.atomic_write

        def fail_entry(path: Path, data: bytes, previous: bytes | None) -> None:
            if path.suffix == ".desktop":
                raise OSError("Ошибка записи entry")
            write(path, data, previous)

        with patch.object(installer, "atomic_write", fail_entry):
            with self.assertRaises(OSError):
                self.install()
        self.assertEqual(legacy_entry.read_bytes(), original)
        self.assertTrue(legacy_icon.exists())
        self.assertTrue(self.install().exists())
        self.assertFalse(legacy_entry.exists())
        self.assertFalse(legacy_icon.exists())

    def test_install_and_managed_update_preserve_artifacts(self) -> None:
        before = self.binary.stat()
        entry = self.install()
        icon = self.data / f"icons/hicolor/scalable/apps/{installer.APP_ID}.svg"
        self.assertEqual(stat.S_IMODE(entry.stat().st_mode), 0o644)
        self.assertEqual(stat.S_IMODE(icon.stat().st_mode), 0o644)
        self.assertEqual(self.binary.stat(), before)
        self.assertNotIn("--port", entry.read_text())
        self.assertIn("Terminal=false\nStartupNotify=true\n", entry.read_text())
        replacement = self.root / "new engines"
        replacement.mkdir()
        self.engines = replacement
        self.assertEqual(self.install(), entry)
        self.assertIn(str(replacement), entry.read_text())
        self.assertEqual(self.binary.stat(), before)

    def test_xdg_environment_is_used(self) -> None:
        with patch.dict(os.environ, {"XDG_DATA_HOME": str(self.data)}):
            entry = installer.install_desktop(self.binary, self.engines, self.icon)
        self.assertEqual(entry.parent, self.data / "applications")

    def test_foreign_entry_and_icon_preserved(self) -> None:
        entry = self.install()
        icon = self.data / f"icons/hicolor/scalable/apps/{installer.APP_ID}.svg"
        for target in (entry, icon):
            with self.subTest(target=target):
                original = target.read_bytes()
                target.write_bytes(b"foreign content\n")
                with self.assertRaises((ValueError, installer.configparser.Error)):
                    self.install()
                self.assertEqual(target.read_bytes(), b"foreign content\n")
                target.write_bytes(original)

    def test_foreign_icon_without_entry_preserved(self) -> None:
        entry = self.install()
        entry.unlink()
        icon = self.data / f"icons/hicolor/scalable/apps/{installer.APP_ID}.svg"
        icon.write_bytes(b"foreign icon")
        with self.assertRaisesRegex(ValueError, "иконка не управляется"):
            self.install()

    def test_entry_write_failure_can_be_retried(self) -> None:
        for existing in (False, True):
            with self.subTest(existing=existing):
                if existing:
                    self.install()
                icon = self.root / f"replacement-{existing}.svg"
                icon.write_text(
                    '<svg xmlns="http://www.w3.org/2000/svg"><title>Новая</title></svg>'
                )
                write = installer.atomic_write

                def fail_entry(path: Path, data: bytes, previous: bytes | None) -> None:
                    if path.suffix == ".desktop":
                        raise OSError("Ошибка записи entry")
                    write(path, data, previous)

                with patch.object(installer, "atomic_write", fail_entry):
                    with self.assertRaises(OSError):
                        installer.install_desktop(
                            self.binary, self.engines, icon, self.data
                        )
                entry = installer.install_desktop(
                    self.binary, self.engines, icon, self.data
                )
                self.assertIn(
                    hashlib.sha256(icon.read_bytes()).hexdigest(), entry.read_text()
                )

    def test_symlink_and_hardlink_rejected(self) -> None:
        entry = self.install()
        original = entry.read_bytes()
        foreign = self.root / "foreign"
        foreign.write_bytes(original)
        for link in ("symlink", "hardlink"):
            with self.subTest(link=link):
                entry.unlink()
                if link == "symlink":
                    entry.symlink_to(foreign)
                else:
                    os.link(foreign, entry)
                with self.assertRaisesRegex(ValueError, "ссылку"):
                    self.install()
                self.assertEqual(foreign.read_bytes(), original)

    def test_directory_symlink_rejected(self) -> None:
        self.data.mkdir()
        elsewhere = self.root / "elsewhere"
        elsewhere.mkdir()
        (self.data / "applications").symlink_to(elsewhere, target_is_directory=True)
        with self.assertRaises(ValueError):
            self.install()
        self.assertEqual(list(elsewhere.iterdir()), [])

    def test_control_relative_and_invalid_artifacts_rejected(self) -> None:
        for path in (Path("relative"), self.root / "x\nExec=bad", self.root / "x\0"):
            with self.subTest(path=path), self.assertRaises(ValueError):
                installer.install_desktop(path, self.engines, data_home=self.data)
        self.binary.chmod(0o644)
        with self.assertRaisesRegex(ValueError, "исполняемым"):
            self.install()
        self.assertFalse(self.data.exists())

    def test_invalid_icon_rejected(self) -> None:
        invalid = self.root / "invalid.svg"
        invalid.write_text("<html/>")
        with self.assertRaises(ValueError):
            installer.install_desktop(
                self.binary, self.engines, invalid, data_home=self.data
            )
        self.assertFalse(self.data.exists())

    def test_percent_in_binary_rejected_before_installation(self) -> None:
        self.binary = self.root / "yonik %f"
        self.binary.write_text("#!/bin/sh\nexit 0\n")
        self.binary.chmod(0o755)
        with self.assertRaisesRegex(ValueError, "GIO"):
            self.install()
        self.assertFalse(self.data.exists())

    @unittest.skipUnless(shutil.which("gio"), "GIO не установлен")
    def test_gio_launch_preserves_literal_arguments(self) -> None:
        output = self.root / "arguments.json"
        self.binary = self.root / 'yonik space "quote" $d `tick` \\ slash'
        self.engines = self.root / 'engine space %u "quote" $d `tick` \\ slash'
        self.engines.mkdir()
        self.binary.write_text(
            f"#!{sys.executable}\n"
            "import json,sys\n"
            f"with open({str(output)!r}, 'w') as result:\n"
            "    json.dump(sys.argv, result)\n"
        )
        self.binary.chmod(0o755)
        entry = self.install()
        if shutil.which("desktop-file-validate"):
            subprocess.run(["desktop-file-validate", str(entry)], check=True)
        subprocess.run(["gio", "launch", str(entry)], check=True, timeout=5)
        deadline = time.monotonic() + 5
        while not output.exists() and time.monotonic() < deadline:
            time.sleep(0.02)
        self.assertTrue(output.exists(), "GIO не запустил проверочный бинарник")
        self.assertEqual(
            json.loads(output.read_text()),
            [str(self.binary), "--engine-dir", str(self.engines)],
        )
        self.assertIn(
            hashlib.sha256(self.icon.read_bytes()).hexdigest(),
            entry.read_text(),
        )


if __name__ == "__main__":
    unittest.main()
