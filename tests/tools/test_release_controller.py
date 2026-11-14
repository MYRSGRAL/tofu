"""Контролируемый producer: synthetic runner не является production evidence."""

import importlib
import io
import json
import os
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

TOOLS = Path(__file__).resolve().parents[2] / "tools"
sys.path.insert(0, str(TOOLS))
c = importlib.import_module("release_controller")


def elf() -> bytes:
    ident = b"\x7fELF\x02\x01\x01" + bytes(9)
    header = struct.pack(
        "<16sHHIQQQIHHHHHH", ident, 2, 62, 1, 64, 64, 0, 0, 64, 56, 1, 64, 0, 0
    )
    return header + struct.pack("<IIQQQQQQ", 1, 5, 0, 0, 0, 120, 120, 4096)


class FakeTrustedRunner:
    """Только test injection во внутренний collect, без production регистрации."""

    def __init__(self) -> None:
        self.commands: list[dict] = []
        self.fail = False
        self.bad_offer = False
        self.dep_versions = {engine: "v1.0.0" for engine in c.VERSIONS}
        self.zip_bytes = {"https://fixture.invalid/module.zip": b"fixture zip"}

    def run(self, argv: list[str], cwd: Path, env: dict[str, str]) -> bytes:
        self.commands.append({"argv": argv, "cwd": str(cwd), "exit": 0})
        if self.fail:
            raise RuntimeError("fixture build failed")
        if argv[0] in ("rustc", "cargo") and "--version" in argv:
            return b"fixture toolchain\n"
        if argv[:3] == [c.SLOT, "cargo", "clean"]:
            shutil.rmtree(Path(env["CARGO_TARGET_DIR"]))
            return b""
        if argv[0] == c.SLOT:
            target = Path(env["CARGO_TARGET_DIR"]) / "release"
            target.mkdir(parents=True, exist_ok=True)
            for name in ("yonik-desktop", "yonik-helper", "yonik-worker"):
                (target / name).write_bytes(elf())
            return b""
        if "--engine" in argv:
            engine = argv[argv.index("--engine") + 1]
            output = Path(argv[argv.index("--output-dir") + 1])
            output.mkdir()
            (output / engine).write_bytes(elf())
            metadata = c.core_metadata(cwd)[engine]
            manifest = {
                "engine": engine,
                "engine_version": metadata["lock"]["version"],
                "source_commit": metadata["lock"]["commit"],
                "source_tag": metadata["lock"]["tag"],
                "source_url": metadata["lock"]["source_url"],
                "binary_sha256": c.sha(output / engine),
                "module_sha256": metadata["lock"]["module_sha256"],
                "native_codec_component": {
                    "frozen_spec_commit": c.SPEC,
                    "patch_sha256": metadata["codec"]["patch_sha256"],
                    "runtime_hooks_connected": False,
                    "callsite_files_sha256": metadata["codec"]["callsite_files_sha256"],
                },
                "patches": [],
                "go_env": {"GOOS": "linux", "GOARCH": "amd64"},
                "go_version": "go version fixture",
            }
            prebuild = {
                key: manifest[key] for key in ("source_commit", "module_sha256")
            }
            (output / "prebuild.json").write_text(json.dumps(prebuild))
            manifest["prebuild_sha256"] = c.sha(output / "prebuild.json")
            (output / "manifest.json").write_text(json.dumps(manifest))
            source = (
                Path(argv[argv.index("--build-root") + 1]) / f"build-source-{engine}"
            )
            source.mkdir(parents=True)
            (source / "proof.go").write_bytes(b"proof")
            (source / "go.mod").write_bytes(b"module fixture")
            (source / "go.sum").write_bytes(b"sum")
            archive = source.parent / "upstream.tar"
            with tarfile.open(archive, "w") as stream:
                data = b"upstream"
                info = tarfile.TarInfo("src/source")
                info.size = len(data)
                stream.addfile(info, io.BytesIO(data))
            return b""
        if argv[:2] == ["git", "clone"]:
            source = Path(argv[-1])
            source.mkdir()
            (source / "proof.go").write_bytes(b"proof")
            (source / "go.mod").write_bytes(b"module fixture")
            (source / "go.sum").write_bytes(b"sum")
            with tarfile.open(source.parent / "upstream.tar", "w") as stream:
                data = b"upstream"
                info = tarfile.TarInfo("src/source")
                info.size = len(data)
                stream.addfile(info, io.BytesIO(data))
            return b""
        if argv[:2] == ["git", "rev-parse"]:
            return ("a" * 40 + "\n").encode()
        if argv[:2] == ["git", "apply"]:
            return b""
        if argv[:3] == ["go", "version", "-m"]:
            return (
                b"fixture: go1.27\n\tpath\tfixture\n"
                + f"\tdep\tfixture/module\t{self.dep_versions[Path(argv[-1]).name]}\th1:fixture\n".encode()
                + b"\tbuild\tGOOS=linux\n\tbuild\tGOARCH=amd64\n"
            )
        if argv[0] == "curl":
            target = Path(argv[argv.index("--output") + 1])
            target.write_bytes(b"wrong" if self.bad_offer else self.zip_bytes[argv[-1]])
            return b""
        if argv[:2] == ["git", "archive"]:
            return (cwd.parent / "upstream.tar").read_bytes()
        raise AssertionError(argv)


class ControllerTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.registry = self.root / "registry"
        self.task = self.root / "task"
        self.runner = FakeTrustedRunner()
        self.make_source()
        self.commit = self.git("rev-parse", "HEAD").decode().strip()

    def git(self, *args: str) -> bytes:
        return subprocess.check_output(
            ["git", "-C", str(self.repo), *args], stderr=subprocess.DEVNULL
        )

    def make_source(self) -> None:
        self.git("init")
        for directory in ("systemd", "sysusers", "polkit", "dbus"):
            (self.repo / "packaging" / directory).mkdir(parents=True)
            # Git хранит файл, чтобы snapshot сохранял каталог.
            (self.repo / "packaging" / directory / "fixture").write_text("fixture")
        recipe = self.repo / "packaging/arch"
        recipe.mkdir(parents=True)
        (recipe / "PKGBUILD").write_bytes(
            (TOOLS.parent / "packaging/arch/PKGBUILD").read_bytes()
        )
        (recipe / "release-recipe.json").write_text(json.dumps(c.RECIPE))
        icon = self.repo / "crates/yonik-desktop/assets/yonik.svg"
        icon.parent.mkdir(parents=True)
        icon.write_text("<svg/>")
        licenses = self.repo / "packaging/licenses"
        (licenses / "composition").mkdir(parents=True)
        (licenses / "linux-inventory.json").write_text('{"packages":[]}')
        (licenses / "composition/go-sources.json").write_text(
            json.dumps(
                {
                    "records": [
                        {
                            "module": "fixture/module",
                            "version": "v1.0.0",
                            "h1": "h1:fixture",
                            "zip_sha256": c.digest(b"fixture zip"),
                            "zip_url": "https://fixture.invalid/module.zip",
                            "texts": [],
                        }
                    ]
                }
            )
        )
        for name in ("composition/P25-SCOPE.md", "SOURCE-OBLIGATIONS.md"):
            (licenses / name).write_text("fixture scope")
        (self.repo / "LICENSE").write_text("fixture license")
        (self.repo / "tools").mkdir()
        (self.repo / "tools/build_native_adapter.py").write_text("fixture builder")
        self.upstream = self.root / "upstream.tar"
        with tarfile.open(self.upstream, "w") as stream:
            data = b"upstream"
            info = tarfile.TarInfo("src/source")
            info.size = len(data)
            stream.addfile(info, io.BytesIO(data))
        for engine, version in c.VERSIONS.items():
            folder = self.repo / f"adapters/{engine}/v{version}"
            folder.mkdir(parents=True)
            (folder / "native-broker-codec.patch").write_bytes(b"patch")
            lock = {
                "engine": engine,
                "version": version,
                "tag": "v" + version,
                "commit": "a" * 40,
                "source_url": "https://fixture.invalid/source",
                "module_sha256": {
                    "go.mod": c.digest(b"module fixture"),
                    "go.sum": c.digest(b"sum"),
                },
                "archive_sha256": c.sha(self.upstream),
            }
            codec = {
                "frozen_spec_commit": c.SPEC,
                "patch_sha256": c.digest(b"patch"),
                "package_files_sha256": {},
                "callsite_files_sha256": {"proof.go": c.digest(b"proof")},
                "runtime_hooks_connected": False,
                "frozen_sources_sha256": {},
            }
            (folder / "source-lock.json").write_text(json.dumps(lock))
            (folder / "native-broker-codec.json").write_text(json.dumps(codec))
        self.git("add", ".")
        self.git(
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.org",
            "commit",
            "-m",
            "fixture",
        )

    def freeze(self) -> None:
        c._freeze(self.repo, self.registry, "unit", self.commit)

    def collect(self) -> dict:
        return c._collect(self.repo, self.registry, "unit", self.task, self.runner)

    def test_real_pipeline_with_fake_runner(self) -> None:
        self.freeze()
        # Mutable checkout не должен влиять на frozen recipe/compliance.
        (self.repo / "LICENSE").write_text("changed checkout")
        context = self.collect()
        self.assertEqual(context["state"], "COLLECTED")
        self.assertFalse((self.task / "target").exists())
        package = self.registry / "unit/package.tar"
        result = c._verify(
            self.repo, self.registry, "unit", package, self.registry / "unit"
        )
        self.assertEqual(result["status"], "PASS")
        self.assertFalse(result["final_linux_acceptance"])
        self.assertTrue(any(row["argv"][0] == c.SLOT for row in self.runner.commands))
        self.assertEqual(
            os.stat(self.registry / "unit/frozen.json").st_mode & 0o777, 0o600
        )

    def test_capture_retained_before_clean(self) -> None:
        self.freeze()
        original = self.runner.run

        def checked(argv: list[str], cwd: Path, env: dict[str, str]) -> bytes:
            if argv[:3] == [c.SLOT, "cargo", "clean"]:
                manifest = json.loads(
                    (self.registry / "unit/build-capture-manifest.json").read_bytes()
                )
                self.assertEqual(manifest["status"], "MISSING")
                self.assertIn("qt:observed-query-and-tool", manifest["producer_missing"])
            return original(argv, cwd, env)

        self.runner.run = checked
        self.collect()

    def test_retention_failure_preserves_target(self) -> None:
        self.freeze()
        with patch.object(c, "retain_build_capture", side_effect=OSError("fixture retention")):
            with self.assertRaises(OSError):
                self.collect()
        self.assertTrue((self.task / "target").exists())
        self.assertFalse(any(row["argv"][:3] == [c.SLOT, "cargo", "clean"]
                             for row in self.runner.commands))

    def test_uncollected_and_fabricated_context(self) -> None:
        self.freeze()
        with self.assertRaises(ValueError):
            c._verify(self.repo, self.registry, "unit", self.upstream, self.root)
        # Candidate input directory не может быть источником registry record.
        (self.root / "frozen.json").write_text('{"state":"COLLECTED"}')
        with self.assertRaises(ValueError):
            c._verify(self.repo, self.registry, "missing", self.upstream, self.root)

    def test_failed_job_never_registers_collected(self) -> None:
        self.freeze()
        self.runner.fail = True
        with self.assertRaises(RuntimeError):
            self.collect()
        self.assertEqual(c.read_context(self.registry, "unit")["state"], "FROZEN")

    def test_wrong_core_and_compliance_payload(self) -> None:
        self.freeze()
        self.collect()
        original = self.registry / "unit/package.tar"
        for member in ("usr/lib/yonik/cores/xray", "usr/share/licenses/yonik/LICENSE"):
            target = self.root / "bad.tar"
            with (
                tarfile.open(original) as incoming,
                tarfile.open(target, "w") as outgoing,
            ):
                for entry in incoming:
                    data = incoming.extractfile(entry).read()
                    if entry.name == member:
                        data = b"wrong payload"
                        entry.size = len(data)
                    outgoing.addfile(entry, io.BytesIO(data))
            with self.subTest(member=member), self.assertRaises(ValueError):
                c._verify(self.repo, self.registry, "unit", target, original.parent)

    def test_incompatible_elf_and_metadata(self) -> None:
        for data in (b"not an ELF", elf()[:20], elf()[:18] + b"\xb7\x00" + elf()[20:]):
            with self.assertRaises(ValueError):
                c.elf_header(data)
        self.freeze()
        metadata = c.core_metadata(self.repo)
        folder = self.repo / "adapters/xray/v26.10.31"
        record = json.loads((folder / "native-broker-codec.json").read_text())
        record["patch_sha256"] = "0" * 64
        (folder / "native-broker-codec.json").write_text(json.dumps(record))
        with self.assertRaises(ValueError):
            c.core_metadata(self.repo)
        self.assertEqual(metadata["xray"]["codec"]["frozen_spec_commit"], c.SPEC)

    def test_no_overwrite_and_path_refusal(self) -> None:
        self.freeze()
        with self.assertRaises((ValueError, FileExistsError)):
            self.freeze()
        for release in ("../evil", "/evil", "."):
            with self.subTest(release=release), self.assertRaises(ValueError):
                c._freeze(self.repo, self.registry, release, self.commit)

    def test_ingest_only_controller_selected_job(self) -> None:
        self.freeze()
        job = self.root / "trusted-job"
        job.mkdir()
        for engine in c.VERSIONS:
            self.runner.run(
                [
                    sys.executable,
                    "builder",
                    "--engine",
                    engine,
                    "--output-dir",
                    str(job / engine),
                    "--build-root",
                    str(self.root / ("native-" + engine)),
                ],
                self.repo,
                {},
            )
        patches = {
            path.relative_to(self.repo).as_posix(): c.sha(path)
            for path in (self.repo / "adapters").rglob("*")
            if path.is_file()
        }
        (job / "acceptance-context.json").write_text(
            json.dumps(
                {
                    "builder_sha256": c.digest(b"fixture builder"),
                    "patches": patches,
                }
            )
        )
        hashes = {engine: c.sha(job / engine / engine) for engine in c.VERSIONS}
        (job / "capability-abi-manifest.json").write_text(
            json.dumps(
                {
                    engine: {
                        "binary_sha256": hashes[engine],
                        "frozen_spec_commit": c.SPEC,
                    }
                    for engine in c.VERSIONS
                }
            )
        )
        self.runner.commands.clear()
        with (
            patch.object(c, "JOB_SOURCE", self.commit),
            patch.object(c, "JOB_BINARIES", hashes),
        ):
            c._ingest_core_job(self.repo, self.registry, "unit", job)
            self.collect()
        self.assertFalse(any("--engine" in row["argv"] for row in self.runner.commands))
        self.assertTrue(
            any(row["argv"][:2] == ["git", "apply"] for row in self.runner.commands)
        )

    def test_redirect_inputs_and_extraneous_compliance(self) -> None:
        self.freeze()
        self.collect()
        package = self.registry / "unit/package.tar"
        (self.root / "source.tar").write_bytes(b"attacker source")
        with self.assertRaises(ValueError):
            c._verify(self.repo, self.registry, "unit", package, self.root)
        target = self.root / "extra.tar"
        with tarfile.open(package) as incoming, tarfile.open(target, "w") as outgoing:
            for entry in incoming:
                outgoing.addfile(entry, incoming.extractfile(entry))
            entry = tarfile.TarInfo("usr/share/licenses/yonik/extraneous")
            entry.mode = 0o644
            entry.size = 5
            outgoing.addfile(entry, io.BytesIO(b"wrong"))
        # Даже если заменить только package hash, frozen payload closure откажет.
        context = c.read_context(self.registry, "unit")
        context["package_sha256"] = c.sha(target)
        with self.assertRaises(ValueError):
            c.validate_payload(target, self.task / "source", context)

    def test_builder_wrong_arch_and_pins_never_register(self) -> None:
        self.freeze()
        run = self.runner.run

        def bad(argv: list[str], cwd: Path, env: dict[str, str]) -> bytes:
            result = run(argv, cwd, env)
            if "--engine" in argv:
                output = Path(argv[argv.index("--output-dir") + 1])
                manifest = json.loads((output / "manifest.json").read_text())
                manifest["engine_version"] = "incompatible"
                (output / "manifest.json").write_text(json.dumps(manifest))
            return result

        with (
            patch.object(self.runner, "run", side_effect=bad),
            self.assertRaises(ValueError),
        ):
            self.collect()
        self.assertFalse((self.registry / "unit/collected.json").exists())

    def test_actual_dependency_source_offer_mismatch(self) -> None:
        self.freeze()
        self.runner.bad_offer = True
        with self.assertRaisesRegex(ValueError, "source offer"):
            self.collect()
        self.assertFalse((self.registry / "unit/collected.json").exists())

    def two_version_fixture(self) -> None:
        path = self.repo / "packaging/licenses/composition/go-sources.json"
        data = json.loads(path.read_text())
        second = {
            **data["records"][0],
            "version": "v2.0.0",
            "zip_url": "https://fixture.invalid/module-v2.zip",
            "zip_sha256": c.digest(b"fixture zip v2"),
        }
        data["records"].append(second)
        path.write_text(json.dumps(data))
        self.runner.dep_versions["sing-box"] = "v2.0.0"
        self.runner.zip_bytes[second["zip_url"]] = b"fixture zip v2"
        self.git("add", ".")
        self.git("commit", "-qm", "fixture two versions")
        self.commit = self.git("rev-parse", "HEAD").decode().strip()

    def test_two_cores_resolve_distinct_versions_and_payloads(self) -> None:
        self.two_version_fixture()
        self.freeze()
        context = self.collect()
        first = context["cores"]["xray"]["dependencies"][0]
        second = context["cores"]["sing-box"]["dependencies"][0]
        self.assertEqual((first["version"], second["version"]), ("v1.0.0", "v2.0.0"))
        self.assertNotEqual(first["zip_sha256"], second["zip_sha256"])
        for row in (first, second):
            self.assertIn(
                f"usr/share/yonik/sources/go/{row['zip_sha256']}.zip",
                context["payload"],
            )
        self.assertEqual(
            c._verify(
                self.repo,
                self.registry,
                "unit",
                self.registry / "unit/package.tar",
                self.registry / "unit",
            )["status"],
            "PASS",
        )

    def test_two_version_package_cannot_omit_one_source_offer(self) -> None:
        self.two_version_fixture()
        self.freeze()
        context = self.collect()
        missing = context["cores"]["sing-box"]["dependencies"][0]["zip_sha256"]
        del context["payload"][f"usr/share/yonik/sources/go/{missing}.zip"]
        with self.assertRaisesRegex(c.NotReady, "source offer"):
            c.validate_payload(
                self.registry / "unit/package.tar", self.task / "source", context
            )

    def test_conflicting_same_module_version_hash_is_rejected(self) -> None:
        path = self.repo / "packaging/licenses/composition/go-sources.json"
        data = json.loads(path.read_text())
        data["records"].append({**data["records"][0], "zip_sha256": "f" * 64})
        path.write_text(json.dumps(data))
        with self.assertRaises(c.NotReady):
            c.source_offers(self.repo)

    def test_missing_second_version_offer_is_rejected(self) -> None:
        self.runner.dep_versions["sing-box"] = "v2.0.0"
        self.freeze()
        with self.assertRaises(c.NotReady):
            self.collect()

    def test_exact_duplicate_offer_is_deduplicated(self) -> None:
        path = self.repo / "packaging/licenses/composition/go-sources.json"
        data = json.loads(path.read_text())
        data["records"].append(data["records"][0])
        path.write_text(json.dumps(data))
        self.assertEqual(len(c.source_offers(self.repo)), 1)

    def prior_rust_fixture(self) -> dict:
        prior = "linux-controller-rehearsal-14229843-msrv85-2"
        for name in c.tooling_hashes():
            shutil.copyfile(TOOLS / name, self.repo / "tools" / name)
        self.git("add", ".")
        self.git("commit", "-qm", "fixture reviewed controller")
        self.commit = self.git("rev-parse", "HEAD").decode().strip()
        c._freeze(self.repo, self.registry, prior, self.commit)
        folder = self.registry / prior
        logs = folder / "controller-logs"
        logs.mkdir(mode=0o700)
        old_task = c.TRANSIENT / ("release-controller-" + prior)
        env = {
            "RUSTUP_TOOLCHAIN": c.RUST_PROFILE,
            "RUSTC": str(c.RUST_BIN / "rustc"),
            "PATH": str(c.RUST_BIN) + ":/usr/bin",
            "CARGO_BUILD_JOBS": "1",
            "CARGO_TARGET_DIR": str(old_task / "target"),
            "TMPDIR": str(old_task / "tmp"),
            "YONIK_BUILD_ID": "0.1.0+" + self.commit,
        }
        commands = [
            ["rustc", "--version"],
            ["cargo", "--version"],
            *[[c.SLOT, *command] for command in c.RECIPE["rust_commands"]],
        ]
        for i, command in enumerate(commands):
            c.write_record(
                logs / f"{i}.command.json",
                {
                    "argv": command,
                    "cwd": str(old_task / "source"),
                    "exit": 0,
                    "controlled_env": env,
                },
            )
        for i in (0, 1):
            (logs / f"{i}.stdout").write_bytes(b"fixture toolchain\n")
        for name in c.RUST:
            (folder / name).write_bytes(elf())
        c.write_record(
            folder / "failure.json",
            {
                "status": "BLOCKED",
                "final_linux_acceptance": False,
                "reason": "Некорректный frozen Go source offer",
            },
        )
        return {
            p.relative_to(folder).as_posix(): {
                "size": p.stat().st_size,
                "sha256": c.sha(p),
            }
            for p in folder.rglob("*")
            if p.is_file()
        }

    def test_reuse_rust_has_lineage_and_no_cargo_build(self) -> None:
        pins = self.prior_rust_fixture()
        self.freeze()
        with (
            patch.object(c, "rust_reuse_pins", return_value=pins),
            patch.object(
                c,
                "rust_toolchain_hashes",
                return_value={"rustc": "a" * 64, "cargo": "b" * 64},
            ),
            patch.object(c, "RUST_REUSE_REVIEW", self.commit),
            patch.object(c, "RUST_REUSE_TARGET", "unit"),
        ):
            context = self.collect()
        self.assertEqual(context["rust_lineage"]["build_performed"], False)
        self.assertFalse(any(row["argv"][0] == c.SLOT for row in self.runner.commands))
        self.assertFalse((self.task / "target").exists())
        self.assertTrue((self.registry / "unit/rust-lineage.json").is_file())

    def test_reuse_tampered_elf_fails_without_build_fallback(self) -> None:
        pins = self.prior_rust_fixture()
        (
            self.registry / "linux-controller-rehearsal-14229843-msrv85-2/yonik-helper"
        ).write_bytes(b"wrong")
        self.freeze()
        with (
            patch.object(c, "rust_reuse_pins", return_value=pins),
            patch.object(c, "RUST_REUSE_REVIEW", self.commit),
            patch.object(c, "RUST_REUSE_TARGET", "unit"),
            self.assertRaises(c.NotReady),
        ):
            self.collect()
        self.assertFalse(any(row["argv"][0] == c.SLOT for row in self.runner.commands))

    def test_reuse_symlink_rejected_even_with_matching_bytes(self) -> None:
        pins = self.prior_rust_fixture()
        path = (
            self.registry / "linux-controller-rehearsal-14229843-msrv85-2/yonik-helper"
        )
        copy = self.root / "helper-copy"
        copy.write_bytes(path.read_bytes())
        path.unlink()
        path.symlink_to(copy)
        self.freeze()
        with (
            patch.object(c, "rust_reuse_pins", return_value=pins),
            patch.object(c, "RUST_REUSE_REVIEW", self.commit),
            patch.object(c, "RUST_REUSE_TARGET", "unit"),
            self.assertRaises((c.NotReady, OSError)),
        ):
            self.collect()

    def test_reuse_source_and_command_mismatch_fail_closed(self) -> None:
        pins = self.prior_rust_fixture()
        self.freeze()
        record = c.read_context(self.registry, "unit")
        folder = self.registry / "unit"
        toolchains = {name: "fixture toolchain\n" for name in ("rustc", "cargo")}
        with (
            patch.object(c, "rust_reuse_pins", return_value=pins),
            patch.object(c, "RUST_REUSE_REVIEW", self.commit),
            patch.object(c, "RUST_REUSE_TARGET", "unit"),
        ):
            wrong = {**record, "source": {**record["source"], "commit": "b" * 40}}
            with self.assertRaisesRegex(c.NotReady, "source/tree"):
                c.reuse_rust(
                    self.repo, self.registry, "unit", wrong, folder, toolchains
                )
            command = (
                self.registry / c.RUST_REUSE_PRIOR / "controller-logs/2.command.json"
            )
            row = json.loads(command.read_text())
            row["exit"] = 1
            command.write_text(json.dumps(row))
            # Даже иной доверенный pin не делает exit=1 пригодным для reuse.
            pins["controller-logs/2.command.json"] = {
                "size": command.stat().st_size,
                "sha256": c.sha(command),
            }
            with self.assertRaisesRegex(c.NotReady, "exact Rust"):
                c.reuse_rust(
                    self.repo, self.registry, "unit", record, folder, toolchains
                )

    def test_reuse_hardlink_and_wrong_record_mode_fail_closed(self) -> None:
        pins = self.prior_rust_fixture()
        folder = self.registry / c.RUST_REUSE_PRIOR
        os.link(folder / "yonik-worker", self.root / "extra-link")
        with self.assertRaises(c.NotReady):
            c.read_pinned_partial(folder, "yonik-worker", pins["yonik-worker"])
        (folder / "failure.json").chmod(0o644)
        with self.assertRaises(c.NotReady):
            c.read_pinned_partial(folder, "failure.json", pins["failure.json"])

    def test_two_versions_wrong_second_archive_rejected(self) -> None:
        self.two_version_fixture()
        self.runner.zip_bytes["https://fixture.invalid/module-v2.zip"] = (
            b"wrong second archive"
        )
        self.freeze()
        with self.assertRaisesRegex(c.NotReady, "source offer hash"):
            self.collect()

    def test_local_runner_records_real_exit_codes(self) -> None:
        runner = c.LocalRunner(self.root / "logs")
        output = runner.run(
            [sys.executable, "-c", "import sys; sys.stdout.write('observed')"],
            self.root,
            dict(os.environ),
        )
        self.assertEqual(output, b"observed")
        with self.assertRaises(subprocess.CalledProcessError):
            runner.run(
                [sys.executable, "-c", "raise SystemExit(7)"],
                self.root,
                dict(os.environ),
            )
        self.assertEqual([row["exit"] for row in runner.commands], [0, 7])
        self.assertEqual(
            json.loads((self.root / "logs/1.command.json").read_text())["exit"], 7
        )

    def test_malformed_registry_is_blocked(self) -> None:
        self.freeze()
        record = self.registry / "unit/collected.json"
        record.write_text("{broken")
        record.chmod(0o600)
        with self.assertRaises(c.NotReady):
            c._verify(self.repo, self.registry, "unit", self.upstream, self.root)


class ControlledProfileTests(unittest.TestCase):
    def test_fixed_msrv_profile_ignores_caller_rust_settings(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            with patch.dict(
                os.environ,
                {
                    "RUSTUP_TOOLCHAIN": "untrusted",
                    "RUSTC": "/bad/rustc",
                    "RUSTFLAGS": "untrusted",
                },
            ):
                env = c.build_env(Path(directory), "a" * 40)
            expected = (
                "/home/myrs/.rustup/toolchains/1.85.1-x86_64-unknown-linux-gnu/bin"
            )
            self.assertEqual(env["PATH"].split(os.pathsep)[0], expected)
            self.assertEqual(env["RUSTUP_TOOLCHAIN"], "1.85.1-x86_64-unknown-linux-gnu")
            self.assertEqual(env["RUSTC"], expected + "/rustc")
            self.assertEqual(env["CARGO_BUILD_JOBS"], "1")
            self.assertNotIn("RUSTFLAGS", env)

    def test_real_command_records_controlled_environment(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            env = c.build_env(root, "a" * 40)
            env["PRIVATE_UNRELATED_SECRET"] = "must-not-log"
            runner = c.LocalRunner(root / "logs")
            runner.run([sys.executable, "-c", "pass"], root, env)
            record = json.loads((root / "logs/0.command.json").read_text())
            self.assertEqual(record["controlled_env"]["RUSTC"], env["RUSTC"])
            self.assertEqual(
                record["controlled_env"]["CARGO_TARGET_DIR"], env["CARGO_TARGET_DIR"]
            )
            self.assertNotIn("PRIVATE_UNRELATED_SECRET", record["controlled_env"])


class CollectedReuseTests(ControllerTest):
    def test_collected_reuse_never_builds_and_preserves_prior(self) -> None:
        for name in c.tooling_hashes():
            shutil.copyfile(TOOLS / name, self.repo / "tools" / name)
        self.git("add", ".")
        self.git("commit", "-qm", "fixture controller")
        self.commit = self.git("rev-parse", "HEAD").decode().strip()
        self.freeze()
        prior = self.collect()
        folder = self.registry / "unit"
        for directory in folder.rglob("*"):
            if directory.is_dir():
                directory.chmod(0o700)
        pins = {
            p.relative_to(folder).as_posix(): {
                "size": p.stat().st_size,
                "sha256": c.sha(p),
            }
            for p in folder.rglob("*")
            if p.is_file()
        }
        c._freeze(self.repo, self.registry, "next", self.commit)
        shutil.rmtree(self.task)
        with (
            patch.object(c, "COLLECTED_REUSE_TARGET", "next"),
            patch.object(c, "COLLECTED_REUSE_PRIOR", "unit"),
            patch.object(c, "collected_reuse_pins", return_value=pins),
            patch.object(c, "COLLECTED_REUSE_REVIEW", self.commit),
        ):
            result = c._collect(
                self.repo, self.registry, "next", self.task, self.runner
            )
        self.assertEqual(result["collected_lineage"]["build_performed"], False)
        self.assertEqual(result["commands"], [])
        self.assertIn(".MTREE", result["payload"])
        self.assertEqual(
            c.sha(folder / "collected.json"), pins["collected.json"]["sha256"]
        )
        self.assertEqual(result["source"], prior["source"])

    def test_collected_reuse_changed_input_rejected(self) -> None:
        for name in c.tooling_hashes():
            shutil.copyfile(TOOLS / name, self.repo / "tools" / name)
        self.git("add", ".")
        self.git("commit", "-qm", "fixture controller")
        self.commit = self.git("rev-parse", "HEAD").decode().strip()
        self.freeze()
        self.collect()
        folder = self.registry / "unit"
        for directory in folder.rglob("*"):
            if directory.is_dir():
                directory.chmod(0o700)
        pins = {
            p.relative_to(folder).as_posix(): {
                "size": p.stat().st_size,
                "sha256": c.sha(p),
            }
            for p in folder.rglob("*")
            if p.is_file()
        }
        (folder / "yonik-desktop").write_bytes(b"tampered")
        c._freeze(self.repo, self.registry, "next", self.commit)
        shutil.rmtree(self.task)
        with (
            patch.object(c, "COLLECTED_REUSE_TARGET", "next"),
            patch.object(c, "COLLECTED_REUSE_PRIOR", "unit"),
            patch.object(c, "collected_reuse_pins", return_value=pins),
            self.assertRaises(c.NotReady),
        ):
            c._collect(self.repo, self.registry, "next", self.task, self.runner)

    def test_collected_reuse_source_mismatch_rejected(self) -> None:
        self.freeze()
        context = self.collect()
        folder = self.registry / "unit"
        for directory in folder.rglob("*"):
            if directory.is_dir():
                directory.chmod(0o700)
        pins = {
            p.relative_to(folder).as_posix(): {
                "size": p.stat().st_size,
                "sha256": c.sha(p),
            }
            for p in folder.rglob("*")
            if p.is_file()
        }
        wrong = {**context, "source": {**context["source"], "commit": "b" * 40}}
        with (
            patch.object(c, "COLLECTED_REUSE_TARGET", "next"),
            patch.object(c, "COLLECTED_REUSE_PRIOR", "unit"),
            patch.object(c, "collected_reuse_pins", return_value=pins),
            self.assertRaisesRegex(c.NotReady, "source/tree"),
        ):
            c.reuse_collected(
                self.repo, self.registry, "next", wrong, folder, self.repo
            )


class PackageMtreeTests(unittest.TestCase):
    def test_mtree_matches_archive_metadata_and_escaped_paths(self) -> None:
        import gzip

        with tempfile.TemporaryDirectory() as folder:
            package = Path(folder) / "package.tar"
            c.write_package(
                package, {"usr/share/test space#=\\é": (0o644, b"payload")}, b"info"
            )
            with tarfile.open(package) as archive:
                self.assertIn(".MTREE", archive.getnames())
                stream = archive.extractfile(".MTREE")
                self.assertIsNotNone(stream)
                text = gzip.decompress(stream.read()).decode("ascii")
                self.assertNotIn("./.MTREE ", text)
                self.assertIn(
                    "./.PKGINFO type=file uid=0 gid=0 mode=644 time=0 size=4 sha256digest=",
                    text,
                )
                self.assertIn(r"./usr/share/test\040space\043\075\134\303\251 ", text)
                for member in archive:
                    self.assertEqual((member.uid, member.gid, member.mtime), (0, 0, 0))

    def test_builder_rejects_unsafe_and_reserved_names(self) -> None:
        for name in (
            "../escape",
            "/absolute",
            "usr//bad",
            ".MTREE",
            ".PKGINFO",
            "usr/../bad",
        ):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as folder:
                with self.assertRaises(ValueError):
                    c.write_package(
                        Path(folder) / "package.tar", {name: (0o644, b"x")}, b"info"
                    )
