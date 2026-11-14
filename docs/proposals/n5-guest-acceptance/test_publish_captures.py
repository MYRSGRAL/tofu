"""Изолированные mocks и только собственные временные файлы; runtime запрещён."""

import copy
import errno
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

import collect_observations as c
import publish_captures as p
import unit_closure_capture as u
from test_collect_observations import CASE, NONCE, Fixture
from test_unit_closure_capture import ClosureFixture


class PublisherTests(unittest.TestCase):
    def setUp(self) -> None:
        self.fixture = ClosureFixture()
        self.capture = c.collect("baseline", NONCE, CASE, backend=Fixture())
        self.closure = u.collect_unit_closure(NONCE, CASE, self.fixture)
        for target in ("command",):
            guard = patch.object(
                c.SystemBackend, target, side_effect=AssertionError("runtime")
            )
            guard.start()
            self.addCleanup(guard.stop)
        guard = patch.object(c.subprocess, "Popen", side_effect=AssertionError("Popen"))
        guard.start()
        self.addCleanup(guard.stop)

    def invoke(
        self,
        data: dict,
        *,
        closure: bool = False,
        stage: str = "baseline",
        store: Mock | None = None,
    ) -> Mock:
        store = store if store is not None else Mock()
        with (
            patch.object(c, "SystemBackend", return_value=self.fixture),
            patch.object(p, "_parent", return_value=111),
            patch.object(p, "_absent"),
            patch.object(p.os, "close"),
            patch.object(p, "_store", store),
            patch.object(c, "collect", return_value=data),
            patch.object(u, "collect_unit_closure", return_value=data),
        ):
            if closure:
                p.publish_unit_closure(NONCE, CASE)
            else:
                p.publish_capture(stage, NONCE, CASE)
            return store

    def test_success(self) -> None:
        for data, closure, name in (
            (self.capture, False, "capture-baseline.json"),
            (self.closure, True, "unit-closure.json"),
        ):
            store = self.invoke(data, closure=closure)
            self.assertEqual(store.call_args.args[1], name)
            self.assertEqual(json.loads(store.call_args.args[2]), data)

    def test_all_fixed_stages(self) -> None:
        for stage in c.STAGES:
            data = copy.deepcopy(self.capture)
            data["provenance"]["stage"] = stage
            if stage == "stopped":
                data["observations"]["tracked_processes"] = []
            store = self.invoke(data, stage=stage)
            self.assertEqual(store.call_args.args[1], f"capture-{stage}.json")

    def test_rust_consumer_identity_bounds_before_store(self) -> None:
        for field, value in (
            ("start_ticks", 2**64),
            ("uids", [1000] * 4),
            ("pid", 2**31 - 1),
        ):
            data = copy.deepcopy(self.capture)
            data["observations"]["collector"][field] = value
            if field == "pid":
                data["provenance"]["collector_pid"] = value
            store = Mock()
            with self.subTest(field=field):
                with self.assertRaises(p.PublicationError):
                    self.invoke(data, store=store)
                store.assert_not_called()

    def test_rust_consumer_bounds_for_units_and_tracked_processes(self) -> None:
        for location in ("main", "members", "tracked_processes"):
            for field, value in (
                ("pid", 2**31 - 1),
                ("start_ticks", 2**64),
                ("uids", [2**32 - 1] * 4),
            ):
                if location == "tracked_processes" and field == "uids":
                    continue
                data = copy.deepcopy(self.capture)
                identity = copy.deepcopy(data["observations"]["collector"])
                identity[field] = value
                stage = "baseline"
                if location == "tracked_processes":
                    stage = "stopped"
                    data["provenance"]["stage"] = stage
                    identity.pop("uids")
                    identity["absent"] = True
                    data["observations"][location] = [identity]
                else:
                    unit = data["observations"]["units"][u.UNITS[0]]
                    unit[location] = identity if location == "main" else [identity]
                store = Mock()
                with self.subTest(location=location, field=field):
                    with self.assertRaises(p.PublicationError):
                        self.invoke(data, stage=stage, store=store)
                    store.assert_not_called()
        data = copy.deepcopy(self.capture)
        data["provenance"]["stage"] = "stopped"
        data["observations"]["tracked_processes"] = [
            {"pid": 1, "start_ticks": 1, "absent": True}
        ] * 513
        store = Mock()
        with self.assertRaises(p.PublicationError):
            self.invoke(data, stage="stopped", store=store)
        store.assert_not_called()

    def test_rust_consumer_maximum_valid_identity(self) -> None:
        data = copy.deepcopy(self.capture)
        collector = data["observations"]["collector"]
        collector.update(pid=2**31 - 2, start_ticks=2**64 - 1)
        data["provenance"]["collector_pid"] = collector["pid"]
        unit = data["observations"]["units"][u.UNITS[0]]
        unit["members"] = [{**collector, "uids": [2**32 - 2] * 4}]
        store = self.invoke(data)
        store.assert_called_once()

    def test_gate_before_query_or_write(self) -> None:
        for change in ("uid", "grant", "input", "nonce", "stage"):
            fixture = Fixture()
            if change == "uid":
                fixture.uid = lambda: 1000
            elif change in ("grant", "input"):
                fixture.files[
                    c.ROOT
                    + ("/authorization.json" if change == "grant" else "/input.json")
                ] = b"{}"
            with (
                patch.object(c, "SystemBackend", return_value=fixture),
                patch.object(c, "collect") as collect,
                patch.object(p, "_parent") as parent,
                self.assertRaises(p.PublicationError),
            ):
                try:
                    p.publish_capture(
                        "bad" if change == "stage" else "baseline",
                        "bad" if change == "nonce" else NONCE,
                        CASE,
                    )
                finally:
                    collect.assert_not_called()
                    parent.assert_not_called()
            self.assertFalse(any(item.startswith("/proc") for item in fixture.calls))

    def test_invalid_and_context_rejected(self) -> None:
        for closure in (False, True):
            original = self.closure if closure else self.capture
            variants = [{"status": "UNAVAILABLE"}, {}, {**original, "extra": 1}]
            for key in original["provenance"]:
                data = copy.deepcopy(original)
                data["provenance"][key] = "wrong"
                variants.append(data)
            data = copy.deepcopy(original)
            del data["helper" if closure else "observations"]
            variants.append(data)
            for data in variants:
                with (
                    self.subTest(closure=closure, data=data),
                    self.assertRaises(p.PublicationError),
                ):
                    self.invoke(data, closure=closure)

    def test_gate_changed_after_collection(self) -> None:
        original = c._authorize
        with patch.object(
            c,
            "_authorize",
            side_effect=[original(self.fixture, NONCE, CASE), c.Unavailable()],
        ):
            with self.assertRaises(p.PublicationError):
                self.invoke(self.capture)


class PathTests(unittest.TestCase):
    def test_parent_rejects_untrusted_components_and_symlink(self) -> None:
        from types import SimpleNamespace

        for uid, mode in ((1000, 0o40755), (0, 0o40777), (0, 0o100755)):
            with (
                patch.object(p.os, "open", return_value=71),
                patch.object(p.os, "close") as close,
                patch.object(
                    p.os,
                    "fstat",
                    return_value=SimpleNamespace(st_uid=uid, st_mode=mode),
                ),
                self.assertRaises(c.Unavailable),
            ):
                try:
                    p._parent()
                finally:
                    close.assert_called_once_with(71)
        with (
            patch.object(
                p.os, "open", side_effect=[71, OSError(errno.ELOOP, "symlink")]
            ) as opened,
            patch.object(p.os, "close"),
            patch.object(
                p.os, "fstat", return_value=SimpleNamespace(st_uid=0, st_mode=0o40755)
            ),
            self.assertRaises(OSError),
        ):
            try:
                p._parent()
            finally:
                self.assertTrue(
                    all(call.args[1] & os.O_NOFOLLOW for call in opened.call_args_list)
                )

    def test_identity_mismatch_rejected(self) -> None:
        from types import SimpleNamespace

        with (
            patch.object(p, "_parent", return_value=72),
            patch.object(p.os, "close") as close,
            patch.object(
                p.os,
                "fstat",
                side_effect=[
                    SimpleNamespace(st_dev=1, st_ino=1),
                    SimpleNamespace(st_dev=1, st_ino=2),
                ],
            ),
            self.assertRaises(c.Unavailable),
        ):
            try:
                p._identity(71)
            finally:
                close.assert_called_once_with(72)


class FileTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.fd = os.open(self.root, os.O_RDONLY | os.O_DIRECTORY)
        self.addCleanup(os.close, self.fd)
        real = os.fstat

        def root_stat(fd: int) -> os.stat_result:
            values = list(real(fd))
            values[4] = 0
            return os.stat_result(values)

        for guard in (
            patch.object(p, "_identity"),
            patch.object(p.os, "fstat", side_effect=root_stat),
        ):
            guard.start()
            self.addCleanup(guard.stop)

    def store(self) -> None:
        p._store(self.fd, "capture-baseline.json", b"{}")

    def test_file_mode_single_link_and_no_reuse(self) -> None:
        self.store()
        target = self.root / "capture-baseline.json"
        self.assertEqual(target.read_bytes(), b"{}")
        self.assertEqual(target.stat().st_mode & 0o777, 0o444)
        self.assertEqual(target.stat().st_nlink, 1)
        with self.assertRaises(p.PublicationError):
            self.store()
        self.assertEqual(len(list(self.root.iterdir())), 1)

    def test_symlink_and_hardlink_existing_rejected(self) -> None:
        target = self.root / "capture-baseline.json"
        own = self.root / "own"
        own.write_bytes(b"old")
        for symbolic in (True, False):
            if symbolic:
                target.symlink_to(own)
            else:
                os.link(own, target)
            with self.assertRaises(p.PublicationError):
                self.store()
            self.assertEqual(own.read_bytes(), b"old")
            target.unlink()

    def test_failures_before_link_leave_no_capture(self) -> None:
        for method in ("write", "fchmod", "fsync", "link"):
            with patch.object(p.os, method, side_effect=OSError(errno.EIO, "injected")):
                with self.assertRaises(OSError):
                    self.store()
            self.assertEqual(list(self.root.iterdir()), [])

    def test_dir_fsync_failure_retains_unknown(self) -> None:
        with patch.object(
            p.os, "fsync", side_effect=[None, OSError(errno.EIO, "injected")]
        ):
            with self.assertRaises(p.PublicationUnknown):
                self.store()
        self.assertEqual((self.root / "capture-baseline.json").read_bytes(), b"{}")
        self.assertEqual(len(list(self.root.iterdir())), 1)

    def test_short_writes(self) -> None:
        original = os.write
        with patch.object(
            p.os, "write", side_effect=lambda fd, raw: original(fd, raw[:1])
        ):
            self.store()
        self.assertEqual((self.root / "capture-baseline.json").read_bytes(), b"{}")

    def test_link_race_is_exclusive(self) -> None:
        original = os.link

        def race(*args: str, **kwargs: object) -> None:
            (self.root / "capture-baseline.json").write_bytes(b"racer")
            original(*args, **kwargs)

        with (
            patch.object(p.os, "link", side_effect=race),
            self.assertRaises(FileExistsError),
        ):
            self.store()
        self.assertEqual((self.root / "capture-baseline.json").read_bytes(), b"racer")

    def test_size_bound_before_write(self) -> None:
        with self.assertRaises(c.Unavailable):
            p._store(self.fd, "capture-baseline.json", b"x" * (c.LIMIT + 1))
        self.assertEqual(list(self.root.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
