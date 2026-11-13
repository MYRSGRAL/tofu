"""Negative controls для post-native packaging boundary без Rust/VM."""

import copy
import importlib
import io
import json
import os
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

TOOLS = Path(__file__).resolve().parents[2] / "tools"
sys.path.insert(0, str(TOOLS))
p = importlib.import_module("check_package_provenance")


class ProvenanceTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

        def run(*args: str) -> bytes:
            return subprocess.check_output(
                args, cwd=self.root, stderr=subprocess.DEVNULL
            )

        run("git", "init")
        (self.root / "source").write_text("исходник\n")
        run("git", "add", "source")
        run(
            "git",
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.org",
            "commit",
            "-m",
            "fixture",
        )
        source = {
            "commit": run("git", "rev-parse", "HEAD").decode().strip(),
            "tree": run("git", "rev-parse", "HEAD^{tree}").decode().strip(),
        }
        (self.root / "archive.tar").write_bytes(
            run("git", "archive", "--prefix=src/", "HEAD")
        )
        names = (
            "gui",
            "helper",
            "worker",
            "xray",
            "sing_box",
            "xray_patched_package",
            "sing_box_patched_package",
            "notices",
            "licenses",
            "source_offers",
            "package",
        )
        artifacts = {}
        for name in names:
            (self.root / name).write_bytes(b"fixture-" + name.encode())
            artifacts[name] = {"path": name, "sha256": p.sha(self.root / name)}
        artifacts["source_archive"] = {
            "path": "archive.tar",
            "sha256": p.sha(self.root / "archive.tar"),
        }
        build = {
            "command": ["cargo", "build", "--locked"],
            "toolchain": "rustc fixture",
            "target": "x86_64-unknown-linux-gnu",
        }
        record = {
            **build,
            "source": source,
            "source_archive_sha256": artifacts["source_archive"]["sha256"],
            "outputs": {k: artifacts[k]["sha256"] for k in names[:7]},
            "abi": 260,
        }
        (self.root / "record").write_text(json.dumps(record))
        artifacts["build_record"] = {
            "path": "record",
            "sha256": p.sha(self.root / "record"),
        }
        self.expected = {
            "schema": 1,
            "status": "FROZEN",
            "arch": "x86_64",
            "abi": 260,
            "source": source,
            "build": build,
            "artifacts": artifacts,
            "capabilities": {"file_capabilities": [], "setuid": False},
            "safety_gates": {
                "host_install": False,
                "autostart": False,
                "runtime_core_download": False,
            },
        }

    def test_negative_controls(self) -> None:
        for field, value in (
            ("source", {}),
            ("arch", "aarch64"),
            ("abi", 259),
            ("capabilities", {"setuid": True}),
            ("build", {}),
        ):
            with self.subTest(field=field):
                candidate = copy.deepcopy(self.expected)
                candidate[field] = value
                with self.assertRaises(ValueError):
                    p._validate_materials(
                        candidate, self.expected, self.root, self.root
                    )
        for name in (
            "gui",
            "xray",
            "sing_box",
            "notices",
            "licenses",
            "source_offers",
            "build_record",
        ):
            with self.subTest(artifact=name):
                path = self.root / self.expected["artifacts"][name]["path"]
                original = path.read_bytes()
                path.write_bytes(b"wrong")
                with self.assertRaises(ValueError):
                    p._validate_materials(
                        self.expected, self.expected, self.root, self.root
                    )
                path.write_bytes(original)

    def test_tree_and_missing_binding(self) -> None:
        candidate = copy.deepcopy(self.expected)
        candidate["source"]["tree"] = "0" * 40
        with self.assertRaises(ValueError):
            p._validate_materials(candidate, candidate, self.root, self.root)
        (self.root / "record").write_text(json.dumps({"binary_sha256": "0" * 64}))
        candidate = copy.deepcopy(self.expected)
        candidate["artifacts"]["build_record"]["sha256"] = p.sha(self.root / "record")
        with self.assertRaises(ValueError):
            p._validate_materials(candidate, candidate, self.root, self.root)

    def test_unsafe_source_members(self) -> None:
        for names in (
            ["../source"],
            ["src/../source"],
            ["src/source", "other/"],
            ["src/source", "src/../"],
            ["src/source", "/absolute/"],
            ["src/source", "src/empty/"],
        ):
            with self.subTest(names=names):
                archive = self.root / "archive.tar"
                with tarfile.open(archive, "w") as stream:
                    for name in names:
                        entry = tarfile.TarInfo(name)
                        entry.mode = 0o644
                        data = "исходник\n".encode()
                        if name.endswith("/"):
                            entry.type = tarfile.DIRTYPE
                        else:
                            entry.size = len(data)
                        stream.addfile(entry, io.BytesIO(data))
                with self.assertRaisesRegex(ValueError, "source|путь|prefix"):
                    p.validate_source_members(archive)

    def test_symlink_input(self) -> None:
        (self.root / "link").symlink_to(self.root / "gui")
        candidate = copy.deepcopy(self.expected)
        candidate["artifacts"]["gui"]["path"] = "link"
        with self.assertRaisesRegex(ValueError, "путь"):
            p._validate_materials(candidate, candidate, self.root, self.root)

    def test_source_link_refusal(self) -> None:
        archive = self.root / "linked.tar"
        for kind in (tarfile.SYMTYPE, tarfile.LNKTYPE, tarfile.FIFOTYPE):
            with self.subTest(kind=kind):
                with tarfile.open(archive, "w") as stream:
                    entry = tarfile.TarInfo("src/source")
                    entry.type = kind
                    entry.linkname = "source" if kind != tarfile.FIFOTYPE else ""
                    stream.addfile(entry)
                with self.assertRaises(ValueError):
                    p.validate_source_members(archive)

    def test_input_parent_link_and_traversal(self) -> None:
        (self.root / "directory").mkdir()
        (self.root / "alias").symlink_to(
            self.root / "directory", target_is_directory=True
        )
        for name in ("../source", "/source", "alias/source", "./source"):
            with self.subTest(name=name), self.assertRaises(ValueError):
                p.input_path(self.root, name)

    def test_materials_use_frozen_snapshot_without_provenance_pass(self) -> None:
        # Mutable checkout отличается от frozen tree; checker должен видеть Git blob.
        (self.root / "source").write_text("подмена checkout")

        def check_frozen(package: Path, frozen: Path) -> None:
            self.assertNotEqual(frozen, self.root)
            self.assertEqual((frozen / "source").read_text(), "исходник\n")

        original = subprocess.check_output
        with (
            patch("check_arch_package.validate_archive", side_effect=check_frozen),
            patch.object(
                p.subprocess, "check_output", wraps=subprocess.check_output
            ) as command,
        ):

            def output(args: list[str], **kwargs: object) -> bytes:
                if args[0] == "bsdtar":
                    key = {
                        "usr/bin/yonik-desktop": "gui",
                        "usr/lib/yonik/yonik-helper": "helper",
                        "usr/lib/yonik/yonik-worker": "worker",
                    }[args[-1]]
                    return (self.root / key).read_bytes()
                return original(args, **kwargs)

            command.side_effect = output
            self.assertEqual(
                p._validate_materials(
                    self.expected, self.expected, self.root, self.root
                )["status"],
                "MATERIALS_CHECKED",
            )

    def test_fabricated_record_cannot_authorize(self) -> None:
        # Даже корректный самосогласованный FROZEN не является production proof.
        with (
            patch("release_controller.release_lock") as lock,
            patch("release_controller._read_request", return_value=self.expected),
        ):
            lock.return_value.__enter__.return_value = self.root
            with self.assertRaisesRegex(p.NotReady, "controller-collected"):
                p.verify_release("fabricated", self.root / "package", self.root)

    def test_registry_absence_and_redirect_inputs(self) -> None:
        with patch("release_controller._read_request", side_effect=FileNotFoundError):
            for inputs in (self.root, self.root / "attacker-registry"):
                with self.subTest(inputs=inputs), self.assertRaises(p.NotReady):
                    p.verify_release("missing", self.root / "package", inputs)

    def test_wrong_core_and_compliance_cannot_authorize(self) -> None:
        for key in ("xray", "sing_box", "notices", "licenses", "source_offers"):
            forged = copy.deepcopy(self.expected)
            forged["artifacts"][key]["sha256"] = "0" * 64
            with (
                self.subTest(key=key),
                patch.object(p, "_read_request", return_value=forged),
                self.assertRaises(p.NotReady),
            ):
                p.verify_release("forged", self.root / "package", self.root)

    def test_controller_has_no_import_attestation_path(self) -> None:
        result = subprocess.run(
            [
                sys.executable,
                str(TOOLS / "release_controller.py"),
                "collect",
                "--release-id",
                "test",
                "--build-record",
                str(self.root / "record"),
            ],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 2)

    def test_private_registry_record(self) -> None:
        registry = self.root / "registry"
        registry.mkdir(mode=0o700)
        release = registry / "test"
        release.mkdir(mode=0o700)
        record = release / "frozen.json"
        record.write_text(json.dumps(self.expected))
        record.chmod(0o600)
        self.assertEqual(p._read_request(registry, "test"), self.expected)
        record.chmod(0o644)
        with self.assertRaises(p.NotReady):
            p._read_request(registry, "test")
        record.chmod(0o600)
        (release / "alias").hardlink_to(record)
        with self.assertRaises(p.NotReady):
            p._read_request(registry, "test")
        (release / "alias").unlink()
        record.unlink()
        record.symlink_to(self.root / "record")
        with self.assertRaises(OSError):
            p._read_request(registry, "test")

    def test_cli_refuses_attestation_arguments(self) -> None:
        result = subprocess.run(
            [
                sys.executable,
                str(TOOLS / "check_package_provenance.py"),
                "--candidate",
                "fabricated",
                "--expected",
                "fabricated",
                "--repo",
                str(self.root),
            ],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 2)
        self.assertNotIn('"status": "PASS"', result.stdout)

    def test_cli_missing_release_is_blocked(self) -> None:
        result = subprocess.run(
            [
                sys.executable,
                str(TOOLS / "check_package_provenance.py"),
                "--release-id",
                "nonexistent-test-release",
                "--package",
                str(self.root / "package"),
                "--inputs",
                str(self.root),
            ],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 2)
        self.assertEqual(json.loads(result.stdout)["status"], "BLOCKED")

    def test_git_environment_does_not_redirect_anchor(self) -> None:
        with patch.dict(
            os.environ,
            {"GIT_DIR": str(self.root / ".git"), "GIT_WORK_TREE": str(self.root)},
        ):
            self.assertEqual(p.canonical_repo(), Path("/home/myrs/projects/yonik"))

    def test_registry_directory_modes_and_links(self) -> None:
        registry = self.root / "registry"
        registry.mkdir(mode=0o755)
        with self.assertRaises(p.NotReady):
            p._read_request(registry, "test")
        registry.chmod(0o700)
        alias = self.root / "alias"
        alias.symlink_to(registry, target_is_directory=True)
        with self.assertRaises(OSError):
            p._read_request(alias, "test")
        release = registry / "test"
        release.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(OSError):
            p._read_request(registry, "test")
