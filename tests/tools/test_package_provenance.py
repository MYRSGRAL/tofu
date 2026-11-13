"""Negative controls для post-native packaging boundary без Rust/VM."""

import copy
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

TOOLS = Path(__file__).resolve().parents[2] / "tools"
sys.path.insert(0, str(TOOLS))
spec = importlib.util.spec_from_file_location(
    "provenance", TOOLS / "check_package_provenance.py"
)
p = importlib.util.module_from_spec(spec)
spec.loader.exec_module(p)


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
                    p.verify(candidate, self.expected, self.root, self.root)
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
                    p.verify(self.expected, self.expected, self.root, self.root)
                path.write_bytes(original)

    def test_tree_and_missing_binding(self) -> None:
        candidate = copy.deepcopy(self.expected)
        candidate["source"]["tree"] = "0" * 40
        with self.assertRaises(ValueError):
            p.verify(candidate, candidate, self.root, self.root)
        (self.root / "record").write_text(json.dumps({"binary_sha256": "0" * 64}))
        candidate = copy.deepcopy(self.expected)
        candidate["artifacts"]["build_record"]["sha256"] = p.sha(self.root / "record")
        with self.assertRaises(ValueError):
            p.verify(candidate, candidate, self.root, self.root)

    def test_valid_binding_and_immutable_output(self) -> None:
        original = subprocess.check_output
        with (
            patch("check_arch_package.validate_archive"),
            patch.object(
                p.subprocess, "check_output", wraps=subprocess.check_output
            ) as command,
        ):

            def output(args: list[str]) -> bytes:
                if args[0] == "bsdtar":
                    key = {
                        "usr/bin/yonik-desktop": "gui",
                        "usr/lib/yonik/yonik-helper": "helper",
                        "usr/lib/yonik/yonik-worker": "worker",
                    }[args[-1]]
                    return (self.root / key).read_bytes()
                return original(args)

            command.side_effect = output
            self.assertEqual(
                p.verify(self.expected, self.expected, self.root, self.root)["status"],
                "PASS",
            )
        for name in ("candidate", "expected"):
            (self.root / name).write_text(json.dumps(self.expected))
        final = self.root / "final"
        final.write_text("retained")
        args = [
            "check",
            "--candidate",
            str(self.root / "candidate"),
            "--expected",
            str(self.root / "expected"),
            "--repo",
            str(self.root),
            "--inputs",
            str(self.root),
            "--output",
            str(final),
        ]
        with (
            patch.object(sys, "argv", args),
            patch.object(p, "verify", return_value={"status": "PASS"}),
        ):
            with self.assertRaises(FileExistsError):
                p.main()
        self.assertEqual(final.read_text(), "retained")
