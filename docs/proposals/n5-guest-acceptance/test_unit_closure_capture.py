"""Чистые mock-регрессии: никакого доступа к proc/systemd."""

import hashlib
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

import collect_observations as c
import unit_closure_capture as u
from test_collect_observations import CASE, NONCE, Fixture


def command(*args: str) -> str:
    return f"{{ path={args[0]} ; argv[]={' '.join(args)} ; ignore_errors=no ; start_time=[n/a] ; stop_time=[n/a] ; pid=0 ; code=(null) ; status=0/0 }}"


class ClosureFixture(Fixture):
    def __init__(self) -> None:
        super().__init__()
        self.files["/usr/bin/systemctl"] = b"mock manager client"
        self.manager = {}
        for name in u.UNITS:
            service = name.endswith(".service")
            helper = name == u.UNITS[0]
            check = "-check" in name
            props = dict.fromkeys(u.COMMON + (u.SERVICE if service else u.SOCKET), "")
            props.update(
                Id=name,
                LoadState="loaded",
                FragmentPath=f"/usr/lib/systemd/system/{name}",
                Transient="no",
                NeedDaemonReload="no",
                ActiveState="inactive",
                SubState="dead",
            )
            if service:
                props.update(
                    User="root" if helper else "yonik-worker",
                    Group="root" if helper else "yonik-worker",
                    Type="dbus" if helper else "exec",
                    BusName="cloud.highpriority.yonik.Network1" if helper else "",
                    KillMode="control-group",
                    ProtectSystem="strict",
                    ProtectHome="yes",
                    NoNewPrivileges="yes",
                    PrivateNetwork="yes" if check else "no",
                    PrivateDevices="yes" if check or helper else "no",
                    CapabilityBoundingSet="" if check else "cap_net_admin",
                    AmbientCapabilities="cap_net_admin"
                    if not helper and not check
                    else "",
                    DynamicUser="no",
                    MainPID="0",
                    ExecMainStartTimestampMonotonic="0",
                )
                props["ExecStart"] = (
                    command("/usr/lib/yonik/yonik-helper")
                    if helper
                    else command(
                        "/usr/lib/yonik/yonik-worker", "--check" if check else "--run"
                    )
                )
                if helper:
                    props.update(
                        ActiveState="active",
                        SubState="running",
                        ControlGroup="/system.slice/yonik-helper.service",
                        MainPID="7",
                        InvocationID="d" * 32,
                        ExecMainStartTimestampMonotonic="1234",
                        ExecStartPre=command(
                            "/usr/lib/yonik/yonik-helper", "--check-worker-fence"
                        ),
                    )
            else:
                props.update(
                    Listen=f"/run/yonik-worker/{'check' if check else 'run'}.sock (Stream)",
                    SocketUser="root",
                    SocketGroup="root",
                    SocketMode="0600",
                    DirectoryMode="0700",
                    Accept="no",
                    RemoveOnStop="yes",
                    Triggers=name.replace(".socket", ".service"),
                )
            self.manager[name] = props

    def read(
        self,
        path: str,
        *,
        sealed: bool = False,
        cap: int = c.LIMIT,
        trusted: bool = False,
    ) -> bytes:
        return super().read(path, sealed=sealed, cap=cap)

    def command(self, name: str) -> bytes:
        self.calls.append(name)
        return "".join(
            f"{key}={value}\n"
            for key, value in self.manager[name.removeprefix("closure:")].items()
        ).encode()

    def helper_image(self, pid: int) -> dict:
        self.calls.append("image")
        return {
            "pid": pid,
            "start_ticks": 123,
            "uid": 0,
            "exe_sha256": hashlib.sha256(b"actual image").hexdigest(),
            "exe_size": 12,
        }


class CaptureTests(unittest.TestCase):
    def test_positive_exact_schema_and_order(self) -> None:
        fixture = ClosureFixture()
        result = u.collect_unit_closure(NONCE, CASE, fixture)
        self.assertEqual(set(result), {"schema", "provenance", "units", "helper"})
        self.assertEqual(result["schema"], "N5_UNIT_CLOSURE_V1")
        self.assertEqual(len(result["provenance"]), 8)
        self.assertEqual(len(result["helper"]), 7)
        self.assertEqual(
            result["provenance"]["collector_sha256"],
            hashlib.sha256(b"mock source").hexdigest(),
        )
        queries = [
            item
            for item in fixture.calls
            if item.startswith("closure:") or item == "image"
        ]
        self.assertEqual(
            queries,
            [
                *("closure:" + name for name in u.UNITS),
                "image",
                *("closure:" + name for name in u.UNITS),
            ],
        )

    def test_auth_failure_before_queries(self) -> None:
        for path in ("authorization.json", "input.json"):
            fixture = ClosureFixture()
            fixture.files[c.ROOT + "/" + path] = b"{}"
            self.assertEqual(
                u.collect_unit_closure(NONCE, CASE, fixture)["status"], "UNAVAILABLE"
            )
            self.assertFalse(
                any(
                    item.startswith("closure:")
                    or item.startswith("/proc")
                    or item == "image"
                    for item in fixture.calls
                )
            )

    def test_missing_duplicate_unknown_overflow_properties(self) -> None:
        for raw in (b"Id=x\n", b"Id=x\nId=x\n", b"Unknown=x\n", b"x" * 65537):
            fixture = ClosureFixture()
            fixture.command = lambda name: raw
            self.assertEqual(
                u.collect_unit_closure(NONCE, CASE, fixture)["status"], "UNAVAILABLE"
            )

    def test_manager_mutation(self) -> None:
        fixture = ClosureFixture()
        original = fixture.helper_image

        def image(pid: int) -> dict:
            fixture.manager[u.UNITS[0]]["InvocationID"] = "e" * 32
            return original(pid)

        fixture.helper_image = image
        self.assertEqual(
            u.collect_unit_closure(NONCE, CASE, fixture)["status"], "UNAVAILABLE"
        )

    def test_permission_denied(self) -> None:
        fixture = ClosureFixture()
        fixture.helper_image = Mock(side_effect=PermissionError)
        self.assertEqual(
            u.collect_unit_closure(NONCE, CASE, fixture)["status"], "UNAVAILABLE"
        )

    def test_fixed_argv(self) -> None:
        for name in u.UNITS:
            argv = c.COMMANDS["closure:" + name]
            self.assertEqual(
                argv[:6],
                ("/usr/bin/systemctl", "--system", "--no-pager", "--all", "show", name),
            )
            self.assertEqual(argv[6], "--property")
            self.assertEqual(
                argv[7].split(","),
                u.COMMON + (u.SERVICE if name.endswith(".service") else u.SOCKET),
            )


class ImageTests(unittest.TestCase):
    def image(
        self,
        *,
        reused: bool = False,
        mutated: bool = False,
        dead: bool = False,
        overflow: bool = False,
    ) -> dict:
        info = SimpleNamespace(
            st_uid=0,
            st_mode=0o100755,
            st_dev=1,
            st_ino=2,
            st_size=3,
            st_mtime_ns=4,
            st_ctime_ns=5,
        )
        altered = SimpleNamespace(**vars(info))
        altered.st_mtime_ns = 6
        poller = Mock()
        poller.poll.side_effect = [[], [1] if dead else []]
        identity = [(123, (0, 0, 0, 0)), (124 if reused else 123, (0, 0, 0, 0))]
        with (
            patch.object(u.os, "pidfd_open", return_value=10),
            patch.object(u.os, "open", side_effect=[11, 12, 13, 14, 15]),
            patch.object(u.os, "close") as close,
            patch.object(
                u.os,
                "fstat",
                side_effect=[
                    info,
                    info,
                    info,
                    info,
                    altered if mutated else info,
                    info,
                ],
            ),
            patch.object(
                u.os, "read", side_effect=[b"xxxx" if overflow else b"abc", b""]
            ),
            patch.object(u.select, "poll", return_value=poller),
            patch.object(u, "_identity", side_effect=identity),
        ):
            try:
                return u.ClosureBackend().helper_image(7)
            finally:
                self.assertEqual(
                    {call.args[0] for call in close.call_args_list},
                    {10, 11, 12, 13, 14, 15}
                    if not overflow and not mutated
                    else {10, 11, 12, 13, 14},
                )

    def test_actual_image_hash(self) -> None:
        self.assertEqual(self.image()["exe_sha256"], hashlib.sha256(b"abc").hexdigest())

    def test_pid_reuse_mutation_exit_size(self) -> None:
        for option in ("reused", "mutated", "dead", "overflow"):
            with self.subTest(option=option), self.assertRaises(c.Unavailable):
                self.image(**{option: True})


class BoundaryTests(unittest.TestCase):
    def test_root_nonce_case_and_grant_gate(self) -> None:
        for change in ("root", "nonce", "case", "grant"):
            fixture = ClosureFixture()
            nonce, case = NONCE, CASE
            if change == "root":
                fixture.uid = lambda: 1000
            elif change == "nonce":
                nonce = "e" * 64
            elif change == "case":
                case = "other"
            else:
                path = c.ROOT + "/authorization.json"
                fixture.files[path] = fixture.files[path].replace(
                    b"USER_GRANTED_GUEST_CAPTURE", b"CLOSED"
                )
            self.assertEqual(
                u.collect_unit_closure(nonce, case, fixture)["status"], "UNAVAILABLE"
            )
            self.assertFalse(
                any(
                    item.startswith("closure:") or item.startswith("/proc")
                    for item in fixture.calls
                )
            )

    def test_identity_validates_pid_start_and_all_uids(self) -> None:
        for pid, start, uid, valid in (
            (7, 123, b"0 0 0 0", True),
            (8, 123, b"0 0 0 0", False),
            (7, 0, b"0 0 0 0", False),
            (7, 123, b"0 1000 0 0", False),
        ):
            raw = f"{pid} (helper) S ".encode() + b"0 " * 18 + str(start).encode()
            with (
                patch.object(u.os, "open", return_value=20),
                patch.object(u.os, "close"),
                patch.object(u.os, "read", side_effect=[raw, b"Uid:\t" + uid + b"\n"]),
            ):
                if valid:
                    self.assertEqual(u._identity(13, 7), (123, (0, 0, 0, 0)))
                else:
                    with self.assertRaises(c.Unavailable):
                        u._identity(13, 7)

    def test_closure_child_overflow_deadline_and_reap(self) -> None:
        for failure in ("overflow", "deadline", "reap"):
            child = Mock()
            child.stdout.fileno.return_value = 5
            child.poll.return_value = None
            child.wait.return_value = 0
            selector = Mock()
            selector.select.return_value = [] if failure == "deadline" else [1]
            selector.__enter__ = Mock(return_value=selector)
            selector.__exit__ = Mock(return_value=False)
            if failure == "reap":
                child.wait.side_effect = c.subprocess.TimeoutExpired("mock", 3)
            with (
                patch.object(c.subprocess, "Popen", return_value=child),
                patch.object(c.selectors, "DefaultSelector", return_value=selector),
                patch.object(
                    c.os,
                    "read",
                    return_value=b"x" * 65537 if failure == "overflow" else b"",
                ),
            ):
                with self.assertRaises((c.Unavailable, c.subprocess.TimeoutExpired)):
                    c.SystemBackend().command("closure:" + u.UNITS[0])
            child.kill.assert_called_once()
            child.stdout.close.assert_called_once()
            self.assertTrue(child.wait.called)


class TrustedFileTests(unittest.TestCase):
    def test_manager_0755_is_trusted_but_not_sealed(self) -> None:
        info = SimpleNamespace(
            st_uid=0,
            st_mode=0o100755,
            st_nlink=1,
            st_dev=1,
            st_ino=2,
            st_size=3,
            st_mtime_ns=4,
            st_ctime_ns=5,
        )
        for sealed in (False, True):
            with (
                patch.object(c.os, "open", return_value=30),
                patch.object(c.os, "close"),
                patch.object(c.os, "fstat", return_value=info),
                patch.object(c.os, "read", side_effect=[b"abc", b""]),
            ):
                if sealed:
                    with self.assertRaises(c.Unavailable):
                        c.SystemBackend().read("/usr/bin/systemctl", sealed=True)
                else:
                    self.assertEqual(
                        c.SystemBackend().read("/usr/bin/systemctl", trusted=True),
                        b"abc",
                    )

    def test_manager_untrusted_owner_or_writable_path_rejected(self) -> None:
        for uid, mode in ((1000, 0o100755), (0, 0o100775)):
            info = SimpleNamespace(st_uid=uid, st_mode=mode)
            with (
                patch.object(c.os, "open", return_value=30),
                patch.object(c.os, "close"),
                patch.object(c.os, "fstat", return_value=info),
            ):
                with self.assertRaises(c.Unavailable):
                    c.SystemBackend().read("/usr/bin/systemctl", trusted=True)
