"""Без boot/seed/overlay/host commands: adversarial frames и owned temp fixtures."""

from __future__ import annotations

import ast
import base64
import hashlib
import importlib
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools"))
protocol = importlib.import_module("a4_serial_protocol")
collector = importlib.import_module("a4_serial_collector")
driver = importlib.import_module("a4_serial_driver")
projection = importlib.import_module("a4_serial_projection")
TOKEN = "a" * 32
SOURCES = {"collector_sha256": "b" * 64, "projection_sha256": "c" * 64}
BOOT = "12345678-1234-1234-1234-123456789abc"


def frame(seq: int, kind: str, payload: dict, phase: str = "initial") -> dict:
    return {
        "seq": seq,
        "phase": phase,
        "monotonic_ns": seq,
        "run_token": TOKEN,
        "boot_id": BOOT,
        "pid": 10 if phase == "initial" else 11,
        "starttime": 100 if phase == "initial" else 101,
        "kind": kind,
        "payload": payload,
    }


def encode(value: dict) -> bytes:
    return protocol.PREFIX + json.dumps(value).encode() + b"\n"


class FramingTests(unittest.TestCase):
    def setUp(self) -> None:
        self.reader = protocol.Stream(TOKEN, SOURCES)

    def test_chunked_projection_has_no_token_or_raw_secrets(self) -> None:
        data = b"non-frame console line\n" + encode(frame(1, "start", SOURCES))
        for offset in range(0, len(data), 7):
            self.reader.feed(data[offset : offset + 7])
        public = json.dumps(self.reader.result())
        self.assertNotIn(TOKEN, public)
        self.assertEqual(self.reader.result()["capture"], "UNKNOWN")

    def test_token_sequence_identity_and_malformed_rejected(self) -> None:
        for delta in (
            {"run_token": "d" * 32},
            {"seq": 2},
            {"seq": True},
            {"boot_id": "bad"},
            {"pid": 0},
            {"extra": "secret-marker"},
            {"kind": "arbitrary"},
        ):
            with self.subTest(delta=delta), self.assertRaises(ValueError):
                protocol.Stream(TOKEN, SOURCES).feed(
                    encode(frame(1, "start", SOURCES) | delta)
                )
        with self.assertRaises(ValueError):
            self.reader.feed(protocol.PREFIX + b'{"seq":1,"seq":1}\n')

    def test_duplicate_and_partial_source_or_child_mismatch(self) -> None:
        self.reader.feed(encode(frame(1, "start", SOURCES)))
        with self.assertRaises(ValueError):
            self.reader.feed(encode(frame(2, "start", SOURCES)))
        with self.assertRaises(ValueError):
            protocol.Stream(TOKEN, SOURCES).feed(
                encode(frame(1, "start", SOURCES | {"collector_sha256": "d" * 64}))
            )
        fresh = protocol.Stream(TOKEN, SOURCES)
        fresh.feed(encode(frame(1, "start", SOURCES))[:-2])
        self.assertEqual(fresh.result()["capture"], "UNKNOWN")
        with self.assertRaises(ValueError):
            protocol.Stream(TOKEN, SOURCES).feed(
                encode(frame(1, "start", SOURCES, "delayed"))
            )

    def test_closed_payload_and_limits(self) -> None:
        with self.assertRaises(ValueError):
            protocol.checked_payload(
                "jobs",
                {
                    "jobs": [
                        {
                            "unit": "sshd.service",
                            "id": 1,
                            "type": "start",
                            "state": "waiting",
                            "secret": "secret-marker",
                        }
                    ],
                    "foreign_count": 0,
                    "unknown": False,
                },
            )
        with self.assertRaises(ValueError):
            protocol.checked_payload(
                "properties",
                {
                    "unit": "sshd.service",
                    "fields": {"Environment": "secret-marker"},
                    "unknown_fields": 0,
                },
            )
        with self.assertRaises(ValueError):
            self.reader.feed(b"x" * (protocol.MAX_STREAM + 1))
        with self.assertRaises(ValueError):
            protocol.Stream(TOKEN, SOURCES).feed(
                protocol.PREFIX + b"x" * protocol.MAX_RECORD + b"\n"
            )

    def test_two_phases_required_and_partial_not_complete(self) -> None:
        seq = 0
        for phase in ("initial", "delayed"):
            records = [
                ("start", SOURCES),
                (
                    "identity",
                    {"pid1": "systemd", "dmi": "QEMU", "cgroup2": True, "masks": 5},
                ),
                ("masks", {"inactive": True, "unknown": False}),
            ]
            for unit in projection.REQUIRED:
                records.append(("properties", projection.project_properties(unit, {})))
                if phase == "initial":
                    records.append(
                        (
                            "definition",
                            {
                                "unit": unit,
                                "index": 0,
                                "definition": {"unit": unit, "status": "RED_READ"},
                            },
                        )
                    )
            records.append(("jobs", {"jobs": [], "foreign_count": 0, "unknown": True}))
            if phase == "initial":
                records.append(("spawn", {"pid": 11, "starttime": 101}))
            records.append(("end", {"outcome": "CAPTURED"}))
            for kind, payload in records:
                seq += 1
                self.reader.feed(encode(frame(seq, kind, payload, phase)))
            if phase == "initial":
                self.assertEqual(self.reader.result()["capture"], "UNKNOWN")
        self.assertEqual(self.reader.result()["capture"], "CAPTURED")
        # Transport CAPTURED никогда не integrity/graph PASS.
        self.assertEqual(self.reader.result()["graph_status"], "UNKNOWN")
        self.assertFalse(self.reader.result()["fullA4"])

    def test_changed_boot_or_child_and_product_fail_stop(self) -> None:
        self.reader.feed(encode(frame(1, "start", SOURCES)))
        with self.assertRaises(ValueError):
            self.reader.feed(
                encode(
                    frame(
                        2,
                        "identity",
                        {"pid1": "systemd", "dmi": "QEMU", "cgroup2": True, "masks": 5},
                    )
                    | {"boot_id": "22345678-1234-1234-1234-123456789abc"}
                )
            )
        with self.assertRaises(ValueError):
            protocol.checked_payload("failure", {"code": "secret-marker"})
        with self.assertRaisesRegex(ValueError, "PRODUCT_JOB_FAIL"):
            self.reader.feed(encode(frame(2, "failure", {"code": "PRODUCT_JOB_FAIL"})))


class CollectorTests(unittest.TestCase):
    def test_dirfd_read_and_escape_alias_special_hardlink_limits(self) -> None:
        with tempfile.TemporaryDirectory(prefix="a4-serial-unit-") as name:
            root = Path(name)
            unit = root / "usr/lib/systemd/system/sshd.service"
            unit.parent.mkdir(parents=True)
            unit.write_bytes(b"[Unit]\nAfter=network.target\n")
            fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
            try:
                path = "/usr/lib/systemd/system/sshd.service"
                self.assertEqual(
                    collector._read_unit(path, "sshd.service", fd), unit.read_bytes()
                )
                for invalid in (
                    "/etc/shadow",
                    "/usr/lib/systemd/system/../shadow",
                    "/usr/lib/systemd/system/other.service",
                ):
                    with self.assertRaises(ValueError):
                        collector._read_unit(invalid, "sshd.service", fd)
                with self.assertRaises(ValueError):
                    collector._read_unit(path, "sshd.service", fd, remaining=1)
                unit.unlink()
                unit.symlink_to("/etc/shadow")
                with self.assertRaises(ValueError):
                    collector._read_unit(path, "sshd.service", fd)
                unit.unlink()
                unit.symlink_to("sshd.service")
                with self.assertRaises(ValueError):
                    collector._read_unit(path, "sshd.service", fd)
                unit.unlink()
                os.mkfifo(unit)
                with self.assertRaises(ValueError):
                    collector._read_unit(path, "sshd.service", fd)
                unit.unlink()
                unit.write_bytes(b"x" * 16385)
                with self.assertRaises(ValueError):
                    collector._read_unit(path, "sshd.service", fd)
                unit.unlink()
                unit.write_bytes(b"[Unit]\n")
                os.link(unit, unit.with_name("hardlink"))
                with self.assertRaises(ValueError):
                    collector._read_unit(path, "sshd.service", fd)
            finally:
                os.close(fd)

    def test_inroot_alias_and_changed_bytes_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="a4-serial-alias-") as name:
            root = Path(name)
            target = root / "usr/lib/systemd/system/sshd.service"
            target.parent.mkdir(parents=True)
            target.write_bytes(b"[Unit]\n")
            alias = root / "etc/systemd/system/sshd.service"
            alias.parent.mkdir(parents=True)
            alias.symlink_to("/usr/lib/systemd/system/sshd.service")
            fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
            try:
                self.assertEqual(
                    collector._read_unit(
                        "/etc/systemd/system/sshd.service", "sshd.service", fd
                    ),
                    b"[Unit]\n",
                )
                original = os.read

                def changed(descriptor: int, count: int) -> bytes:
                    data = original(descriptor, count)
                    target.write_bytes(b"[Unit]\nChanged=yes\n")
                    return data

                with (
                    patch.object(collector.os, "read", side_effect=changed),
                    self.assertRaises(ValueError),
                ):
                    collector._read_unit(
                        "/etc/systemd/system/sshd.service", "sshd.service", fd
                    )
            finally:
                os.close(fd)

    def test_no_generic_exec_and_guard_precedes_subprocess(self) -> None:
        for kind, unit in [
            ("exec", ""),
            ("properties", "yonik-helper.service"),
            ("paths", "../../etc/shadow"),
        ]:
            with self.assertRaises(ValueError):
                collector._argv(kind, unit)
        with (
            patch.object(collector, "_identity", side_effect=ValueError("guard")),
            patch.object(collector.subprocess, "Popen") as spawn,
            patch.object(collector.os, "fork") as fork,
        ):
            with self.assertRaises(ValueError):
                collector.main()
            spawn.assert_not_called()
            fork.assert_not_called()

    def test_job_and_mask_errors_are_not_pass(self) -> None:
        jobs, product = collector._jobs(
            "42 yonik-worker.service start waiting\nraw secret-marker\n"
        )
        self.assertTrue(product)
        self.assertTrue(jobs["unknown"])
        self.assertNotIn("secret-marker", json.dumps(jobs))
        self.assertEqual(
            collector._mask_state(None), {"inactive": False, "unknown": True}
        )
        self.assertFalse(
            collector._mask_state("Id=yonik-worker.service\nActiveState=active\n")[
                "inactive"
            ]
        )

    def test_deadline_prevents_command_and_snapshot_budget(self) -> None:
        with (
            patch.object(collector.time, "monotonic", return_value=10),
            patch.object(collector.subprocess, "Popen") as spawn,
        ):
            self.assertIsNone(collector._command("jobs", 9))
            spawn.assert_not_called()


class DriverTests(unittest.TestCase):
    def test_missing_authority_has_no_side_effects(self) -> None:
        instance = driver.SerialDriver()
        with (
            tempfile.TemporaryDirectory(prefix="a4-no-authority-") as name,
            patch.object(driver, "DECISION", Path(name) / "absent"),
            patch.object(driver.subprocess, "Popen") as spawn,
        ):
            with self.assertRaises(FileNotFoundError):
                instance.execute()
            spawn.assert_not_called()
            self.assertFalse(instance.task.exists())
            self.assertFalse(instance.out.exists())

    def test_bootstrap_exact_subsequence_and_sources_no_seed_write(self) -> None:
        config = driver.cloud_config(TOKEN, "fixture-public")
        commands = config["bootcmd"]
        collector_argv = commands.pop(6)
        self.assertEqual(collector_argv[:3], ["/usr/bin/python", "-I", "-c"])
        old = driver.common.Driver.__new__(driver.common.Driver)
        old.nonce = TOKEN
        expected = old.cloud_config("fixture-public")
        self.assertEqual(commands, expected["bootcmd"] + expected["runcmd"])
        self.assertNotIn("runcmd", config)
        self.assertNotIn("write_files", config)
        self.assertNotIn("packages", config)

    def test_inline_delivery_closed_names_digests_and_no_write_files_dependency(
        self,
    ) -> None:
        program = driver.loader_source(TOKEN)
        parsed = ast.parse(program)
        encoded = next(
            node.args[0].value
            for node in ast.walk(parsed)
            if isinstance(node, ast.Call)
            and isinstance(node.func, ast.Attribute)
            and node.func.attr == "b64decode"
        )
        payload = json.loads(base64.b64decode(encoded))
        self.assertEqual(set(payload["files"]), set(driver.NAMES[1:4]))
        for name, source in payload["files"].items():
            self.assertEqual(
                hashlib.sha256(source.encode()).hexdigest(),
                payload["contract"]["sources"][name],
            )
        self.assertLess(len(program.encode()), 98305)
        self.assertIn("os.O_EXCL|os.O_NOFOLLOW", program)
        self.assertIn("runpy.run_path('/run/a4-serial/a4_serial_collector.py'", program)
        self.assertLess(program.index("os.getuid()"), program.index("os.mkdir("))

    def test_no_ssh_scp_package_or_arbitrary_host_kind(self) -> None:
        instance = driver.SerialDriver()
        with patch.object(driver.subprocess, "Popen") as spawn:
            for method in (
                instance.guest,
                instance.scp,
                instance.workload,
                instance._host,
            ):
                with self.assertRaises(ValueError):
                    method("arbitrary")
            spawn.assert_not_called()

    def test_future_qemu_argv_and_own_path_guard(self) -> None:
        task = driver.TRANSIENT / ("a4-serial-only-" + TOKEN)
        argv = driver.qemu_argv(task, TOKEN, 22022)
        self.assertEqual(argv[0], "/usr/bin/qemu-system-x86_64")
        self.assertIn("file:" + str(task / "serial.log"), argv)
        self.assertIn("user,id=n,restrict=on,hostfwd=tcp:127.0.0.1:22022-:22", argv)
        with self.assertRaises(ValueError):
            driver.qemu_argv(Path("/tmp/foreign"), TOKEN, 22022)

    def test_authority_wrong_scope_or_hash_rejected(self) -> None:
        contract = {
            "decision": "APPROVED one bounded serial-only diagnostic boot",
            "basis": "59748be719b01cb74f8b3f4d210230e6093b3902 serial proposal; existing147 standing authorization",
            "source": driver.SOURCE142,
            "sources": driver.sources(),
            "input_manifest_sha256": driver.common.sha(
                driver.common.PROPOSAL / "readonly-inputs.json"
            ),
            "qemu_contract_sha256": driver.common.sha(
                driver.common.PROPOSAL / "execution-contract.json"
            ),
            "limits": driver.LIMITS,
            "launcher_sha256": driver.launcher_pins(),
            "forbidden": [
                "SSH",
                "package transactions",
                "product activation",
                "host changes",
                "download/build",
                "mount/repair",
                "clock/network changes",
                "userdata",
            ],
        }
        driver.validate_authority(contract)
        for delta in (
            {"decision": "approved"},
            {"sources": {}},
            {"limits": driver.LIMITS | {"boots": 2}},
            {"input_manifest_sha256": "0" * 64},
        ):
            with self.assertRaises(ValueError):
                driver.validate_authority(contract | delta)

    def _fixture(self, root: Path) -> driver.SerialDriver:
        instance = driver.SerialDriver()
        instance.task = root / ("a4-serial-only-" + instance.token)
        instance.out = root / "evidence"
        instance.task.mkdir(mode=0o700)
        instance.out.mkdir(mode=0o700)
        instance.created_task = True
        instance.base_stat = (1, 2, 3, 4)
        (instance.task / "owner.json").write_text(
            json.dumps({"token": instance.token, "uid": os.getuid()})
        )
        (instance.task / "serial.log").write_bytes(b"private fixture serial\n")
        return instance

    def test_partial_bootstrap_cleanup_retains_overlay_and_reaps_before_delete(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory(prefix="a4-serial-clean-") as name:
            root = Path(name)
            instance = self._fixture(root)
            (instance.task / "disk.qcow2").write_bytes(b"fixture-not-vm")
            instance.vm = Mock(returncode=-15)
            instance.vm.poll.return_value = None
            instance._check_vm = Mock()
            with (
                patch.object(driver, "TRANSIENT", root),
                patch.object(driver.common, "references", return_value=[]),
                patch.object(driver.common, "sha", return_value=driver.BASE_SHA),
                patch.object(
                    instance, "_base_identity", return_value=instance.base_stat
                ),
            ):
                result = instance.cleanup()
            instance.vm.terminate.assert_called_once()
            instance.vm.wait.assert_called_once_with(timeout=10)
            instance.vm.kill.assert_not_called()
            self.assertTrue(result["removed"])
            self.assertEqual(
                (instance.out / "blocked-disk.qcow2").read_bytes(), b"fixture-not-vm"
            )
            self.assertEqual(
                (instance.out / "serial-private.log").stat().st_mode & 0o777, 0o600
            )

    def test_references_or_changed_base_keep_owned_root(self) -> None:
        for refs, stable in ([{"pid": 123}], True), ([], False):
            with (
                self.subTest(refs=refs, stable=stable),
                tempfile.TemporaryDirectory(prefix="a4-serial-keep-") as name,
            ):
                root = Path(name)
                instance = self._fixture(root)
                with (
                    patch.object(driver, "TRANSIENT", root),
                    patch.object(driver.common, "references", return_value=refs),
                    patch.object(
                        driver.common,
                        "sha",
                        return_value=driver.BASE_SHA if stable else "changed",
                    ),
                    patch.object(
                        instance, "_base_identity", return_value=instance.base_stat
                    ),
                ):
                    result = instance.cleanup()
                self.assertFalse(result["removed"])
                self.assertTrue(instance.task.exists())

    def test_foreign_ownership_and_changed_pid_never_signal_or_delete(self) -> None:
        with tempfile.TemporaryDirectory(prefix="a4-serial-pid-") as name:
            root = Path(name)
            instance = self._fixture(root)
            instance.vm = Mock()
            instance.vm.poll.return_value = None
            with (
                patch.object(driver, "TRANSIENT", root),
                patch.object(
                    instance, "_check_vm", side_effect=ValueError("changed identity")
                ),
            ):
                with self.assertRaises(ValueError):
                    instance.cleanup()
            instance.vm.terminate.assert_not_called()
            instance.vm.kill.assert_not_called()
            self.assertTrue(instance.task.exists())
            (instance.task / "owner.json").write_text("{}")
            with patch.object(driver, "TRANSIENT", root), self.assertRaises(ValueError):
                instance.cleanup()
            instance.vm.terminate.assert_not_called()

    def test_identity_rechecked_before_kill(self) -> None:
        with tempfile.TemporaryDirectory(prefix="a4-serial-kill-") as name:
            root = Path(name)
            instance = self._fixture(root)
            instance.vm = Mock()
            instance.vm.poll.return_value = None
            instance.vm.wait.side_effect = subprocess.TimeoutExpired("qemu", 10)
            instance._check_vm = Mock(side_effect=[None, ValueError("reused PID")])
            with patch.object(driver, "TRANSIENT", root), self.assertRaises(ValueError):
                instance.cleanup()
            instance.vm.terminate.assert_called_once()
            instance.vm.kill.assert_not_called()
            self.assertTrue(instance.task.exists())


class SchedulingTests(unittest.TestCase):
    def test_parent_launches_exactly_one_child_and_initial_snapshot(self) -> None:
        with (
            tempfile.TemporaryFile() as serial,
            patch.object(collector, "_identity", return_value=BOOT),
            patch.object(
                collector, "_contract", return_value={"token": TOKEN, "sources": {}}
            ),
            patch.object(collector, "Emitter") as emitter,
            patch.object(collector, "_snapshot") as snapshot,
            patch.object(collector.os, "fork", return_value=123) as fork,
            patch.object(collector, "_starttime", return_value=100),
            patch.object(collector.os, "open", return_value=os.dup(serial.fileno())),
            patch.object(collector.signal, "signal"),
            patch.object(collector.signal, "alarm"),
        ):
            self.assertEqual(collector.main(), 0)
            fork.assert_called_once_with()
            self.assertEqual(snapshot.call_args.args[2], True)
            snapshot.assert_called_once()
            emitter.return_value.emit.assert_any_call(
                "spawn", {"pid": 123, "starttime": 100}
            )

    def test_delayed_branch_single_snapshot_no_second_fork_or_definition(self) -> None:
        with (
            tempfile.TemporaryFile() as serial,
            patch.object(collector, "_identity", return_value=BOOT),
            patch.object(
                collector, "_contract", return_value={"token": TOKEN, "sources": {}}
            ),
            patch.object(collector, "Emitter"),
            patch.object(collector, "_snapshot") as snapshot,
            patch.object(collector.os, "fork", return_value=0) as fork,
            patch.object(
                collector.os,
                "open",
                side_effect=[os.dup(serial.fileno()), os.dup(serial.fileno())],
            ),
            patch.object(collector.os, "dup2"),
            patch.object(collector.os, "_exit", side_effect=SystemExit),
            patch.object(collector.signal, "signal"),
            patch.object(collector.signal, "alarm"),
            patch.object(collector.time, "sleep") as sleep,
        ):
            with self.assertRaises(SystemExit):
                collector.main()
            fork.assert_called_once_with()
            self.assertEqual(
                [call.args[2] for call in snapshot.call_args_list], [True, False]
            )
            sleep.assert_called_once()

    def test_child_identity_mismatch_rejected(self) -> None:
        stream = protocol.Stream(TOKEN, SOURCES)
        for value in (
            frame(1, "start", SOURCES),
            frame(2, "spawn", {"pid": 11, "starttime": 101}),
            frame(3, "end", {"outcome": "CAPTURED"}),
        ):
            stream.feed(encode(value))
        with self.assertRaises(ValueError):
            stream.feed(encode(frame(4, "start", SOURCES, "delayed") | {"pid": 12}))


class BoundedCommandTests(unittest.TestCase):
    def test_timeout_kills_and_reaps_only_mocked_child(self) -> None:
        process = Mock()
        process.stdout.fileno.return_value = 42
        process.poll.return_value = None
        with (
            patch.object(collector.subprocess, "Popen", return_value=process) as spawn,
            patch.object(collector.os, "set_blocking"),
            patch.object(collector.time, "monotonic", side_effect=[0, 0, 0, 0, 2]),
            patch.object(collector.select, "select", return_value=([], [], [])),
        ):
            self.assertIsNone(collector._command("jobs", 10))
            spawn.assert_called_once()
        process.kill.assert_called_once()
        process.wait.assert_called_once_with(timeout=1)
        process.stdout.close.assert_called_once()

    def test_bounded_stdout_no_raw_error_or_retry(self) -> None:
        process = Mock()
        process.stdout.fileno.return_value = 42
        process.poll.return_value = None
        with (
            patch.object(collector.subprocess, "Popen", return_value=process) as spawn,
            patch.object(collector.os, "set_blocking"),
            patch.object(collector.os, "read", return_value=b"x" * 4096),
            patch.object(collector.time, "monotonic", return_value=0),
            patch.object(
                collector.select, "select", return_value=([process.stdout], [], [])
            ),
        ):
            self.assertIsNone(collector._command("jobs", 10))
            spawn.assert_called_once()
        process.kill.assert_called_once()
        process.wait.assert_called_once_with(timeout=1)

    def test_unknown_masks_and_active_masks_distinguished(self) -> None:
        emitter = Mock(
            boot=BOOT,
            contract={
                "sources": {
                    "a4_serial_collector.py": SOURCES["collector_sha256"],
                    "a4_serial_projection.py": SOURCES["projection_sha256"],
                }
            },
        )
        with (
            patch.object(collector, "_identity", return_value=BOOT),
            patch.object(collector, "_runtime_guard"),
            patch.object(collector, "_command", return_value=None),
        ):
            with self.assertRaisesRegex(ValueError, "UNKNOWN"):
                collector._snapshot(emitter, time.monotonic() + 100, True)
        emitter.emit.assert_any_call("failure", {"code": "GUARD_RED"})
        self.assertNotIn(
            unittest.mock.call("failure", {"code": "PRODUCT_JOB_FAIL"}),
            emitter.emit.call_args_list,
        )


class AdditionalGuardTests(unittest.TestCase):
    _fixture = DriverTests._fixture

    def test_pin_drift_stops_before_bootstrap_commands(self) -> None:
        instance = driver.SerialDriver()
        with (
            patch.object(driver.os, "getuid", return_value=1000),
            patch.object(driver.os, "geteuid", return_value=1000),
            patch.object(driver.os, "access", return_value=True),
            patch.object(driver.common, "sha", return_value="drift"),
            patch.object(driver.subprocess, "Popen") as spawn,
        ):
            with self.assertRaisesRegex(ValueError, "Pinned input/tool drift"):
                instance._verify()
            spawn.assert_not_called()

    def test_retention_failure_still_stops_own_vm_and_keeps_root(self) -> None:
        with tempfile.TemporaryDirectory(prefix="a4-serial-retention-") as name:
            root = Path(name)
            instance = self._fixture(root)
            instance.vm = Mock(returncode=-15)
            instance.vm.poll.return_value = None
            instance._check_vm = Mock()
            with (
                patch.object(driver, "TRANSIENT", root),
                patch.object(
                    instance, "_retain", side_effect=OSError("fixture copy failed")
                ),
                patch.object(driver.common, "references", return_value=[]),
                patch.object(driver.common, "sha", return_value=driver.BASE_SHA),
                patch.object(
                    instance, "_base_identity", return_value=instance.base_stat
                ),
            ):
                result = instance.cleanup()
            instance.vm.terminate.assert_called_once()
            instance.vm.wait.assert_called_once_with(timeout=10)
            self.assertTrue(instance.task.exists())
            self.assertFalse(result["removed"])
            self.assertTrue((instance.out / "retention-before-stop.json").exists())


class OnceAuthorityTests(unittest.TestCase):
    def test_same_authority_cannot_be_claimed_twice(self) -> None:
        instance = driver.SerialDriver()
        with tempfile.TemporaryDirectory(prefix="a4-once-authority-") as name:
            instance.out = Path(name) / "unused-output"
            instance._claim_authority("a" * 64)
            with self.assertRaises(FileExistsError):
                instance._claim_authority("a" * 64)
            marker = Path(name) / ("authority-used-" + "a" * 64 + ".json")
            self.assertEqual(marker.stat().st_mode & 0o777, 0o600)
            self.assertEqual(json.loads(marker.read_text())["attempts"], 1)
