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
        self.temporary = tempfile.TemporaryDirectory(prefix="tofu-launcher-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.data = self.root / "xdg"
        self.binary = self.root / "tofu"
        self.binary.write_text("#!/bin/sh\nexit 0\n")
        self.binary.chmod(0o755)
        self.engines = self.root / "engines"
        self.engines.mkdir()

    def install(self) -> Path:
        return installer.install_desktop(self.binary, self.engines, data_home=self.data)

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
            entry = installer.install_desktop(self.binary, self.engines)
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
        self.binary = self.root / "tofu %f"
        self.binary.write_text("#!/bin/sh\nexit 0\n")
        self.binary.chmod(0o755)
        with self.assertRaisesRegex(ValueError, "GIO"):
            self.install()
        self.assertFalse(self.data.exists())

    @unittest.skipUnless(shutil.which("gio"), "GIO не установлен")
    def test_gio_launch_preserves_literal_arguments(self) -> None:
        output = self.root / "arguments.json"
        self.binary = self.root / 'tofu space "quote" $d `tick` \\ slash'
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
            hashlib.sha256(installer.DEFAULT_ICON.read_bytes()).hexdigest(),
            entry.read_text(),
        )


if __name__ == "__main__":
    unittest.main()
