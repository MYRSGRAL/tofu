"""Проверки oracle, ограниченного транспорта и lifecycle без реальных окон."""

import argparse
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import ui_benchmark as bench


class OracleTests(unittest.TestCase):
    def test_known_hash_and_exact_host(self) -> None:
        oracle = bench.Oracle(1000)
        self.assertEqual(bench.page_hash(oracle.rows[:1]), "f84088cf6817a1b8")
        self.assertEqual(bench.page_hash([]), "cbf29ce484222325")
        oracle.apply(bench.query("server-0.example.invalid"))
        report = oracle.expected("frame")
        self.assertEqual(
            (report["total"], report["first_id"], report["revision"]), (1, 1, 2)
        )
        oracle.apply(dict(op="select", id=1))
        oracle.apply(dict(op="favorite", id=1, value=True))
        oracle.apply(bench.query(favorites_only=True))
        self.assertEqual(oracle.expected("frame")["total"], 1)
        self.assertNotEqual(oracle.expected("frame")["page_hash"], "f84088cf6817a1b8")
        oracle.apply(bench.query(sort="name"))
        self.assertEqual(oracle.expected("frame")["last_id"], 797)
        self.assertEqual(oracle.expected("frame")["selected_id"], 1)
        revision = oracle.revision
        oracle.apply(dict(op="scroll", index=199))
        self.assertEqual(oracle.expected("frame")["scroll_index"], 199)
        self.assertEqual(oracle.apply(dict(op="hide")), "hidden")
        self.assertEqual(oracle.apply(dict(op="show")), "shown")
        self.assertEqual(oracle.revision, revision)

    def test_stale_revision_bad_hash_and_missing_null_are_rejected(self) -> None:
        oracle = bench.Oracle(1000)
        valid = oracle.expected("frame") | dict(
            toolkit="fake", created_total=0, alive_rows=None, visible_rows=None
        )
        oracle.validate(valid, "frame")
        for key, value in (
            ("revision", 0),
            ("revision", True),
            ("page_hash", "0"),
            ("selected_id", 1),
            ("event", "shown"),
            ("created_total", -1),
        ):
            with (
                self.subTest(key=key, value=value),
                self.assertRaises(bench.BenchmarkError),
            ):
                oracle.validate(valid | {key: value}, "frame")
        del valid["selected_id"]
        with self.assertRaises(bench.BenchmarkError):
            oracle.validate(valid, "frame")

    def test_proc_fields_handle_comm_with_spaces_and_parentheses(self) -> None:
        fields = ["S"] + ["0"] * 49
        fields[11], fields[12] = "123", "45"
        result = bench.parse_proc(
            "123 (name ) with space) " + " ".join(fields),
            "Name:\ttest\nvoluntary_ctxt_switches:\t12\nnonvoluntary_ctxt_switches:\t7\n",
            "Pss: 42 kB\nPrivate_Clean: 3 kB\nPrivate_Dirty: 5 kB\nPrivate_Hugetlb: 2 kB\n",
        )
        self.assertEqual(result["cpu_ticks"], 168)
        self.assertEqual(result["pss_bytes"], 42 * 1024)
        self.assertEqual(result["uss_bytes"], 10 * 1024)
        self.assertEqual(sum(result["main_thread_context_switches_proxy"].values()), 19)

    def test_locked_unknown_or_nonwayland_session_is_invalid(self) -> None:
        for state in (
            {},
            dict(Type="wayland", Active="yes", LockedHint="yes"),
            dict(Type="x11", Active="yes", LockedHint="no"),
        ):
            with (
                patch.object(bench, "session_state", return_value=state),
                self.assertRaises(bench.BenchmarkError),
            ):
                bench.require_unlocked()


class ChildTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory(prefix="yonik-ui-driver-test-")
        self.root = Path(self.directory.name)
        self.children: list[bench.Child] = []

    def tearDown(self) -> None:
        for child in self.children:
            child.close()
        self.directory.cleanup()

    def spawn(self, code: str) -> bench.Child:
        child = bench.Child(
            [sys.executable, "-u", "-c", code],
            self.root / f"stderr-{len(self.children)}.log",
        )
        self.children.append(child)
        return child

    def test_exit_observation_keeps_leader_until_last_group_signal(self) -> None:
        for operation in ("alive", "ack", "send", "finish"):
            with self.subTest(operation=operation):
                child = self.spawn("pass")
                deadline = time.monotonic() + 2
                while (
                    os.waitid(
                        os.P_PID,
                        child.process.pid,
                        os.WEXITED | os.WNOHANG | os.WNOWAIT,
                    )
                    is None
                ):
                    self.assertLess(time.monotonic(), deadline)
                    time.sleep(0.005)
                signals = []
                original = os.killpg

                def guarded_signal(pid: int, number: int) -> None:
                    # Oracle не посылает сигнал, если PID уже освобождён.
                    self.assertIsNone(child.process.returncode, "signal after reap")
                    held = os.waitid(
                        os.P_PID, pid, os.WEXITED | os.WNOHANG | os.WNOWAIT
                    )
                    self.assertIsNotNone(held)
                    signals.append(number)
                    original(pid, number)

                with patch.object(bench.os, "killpg", side_effect=guarded_signal):
                    try:
                        if operation == "send":
                            child.send(dict(op="quit"), 0.1)
                        elif operation in ("ack", "finish"):
                            getattr(child, operation)(0.1)
                        else:
                            child.alive()
                    except bench.BenchmarkError:
                        pass
                    child.close()
                    child.close()
                self.assertEqual(signals, [signal.SIGTERM, signal.SIGKILL])
                self.assertEqual(child.process.returncode, 0)

    def test_external_reap_forbids_numeric_group_signals(self) -> None:
        child = self.spawn("pass")
        child.process.wait(timeout=2)
        with (
            patch.object(bench.os, "killpg") as killpg,
            self.assertRaisesRegex(bench.BenchmarkError, "reservation"),
        ):
            child.close()
        killpg.assert_not_called()
        child.close()

    def test_external_waitpid_without_popen_returncode_forbids_signals(self) -> None:
        for observe_first in (False, True):
            with self.subTest(observe_first=observe_first):
                child = self.spawn("pass")
                os.waitpid(child.process.pid, 0)
                self.assertIsNone(child.process.returncode)
                with patch.object(bench.os, "killpg") as killpg:
                    if observe_first:
                        with self.assertRaisesRegex(
                            bench.BenchmarkError, "reservation"
                        ):
                            child.alive()
                    with self.assertRaisesRegex(bench.BenchmarkError, "reservation"):
                        child.close()
                    child.close()
                killpg.assert_not_called()

    def test_signal_oracle_rejects_reaped_leader_without_real_signal(self) -> None:
        child = self.spawn("pass")
        child.close()

        # Отрицательный контроль: только mock, без killpg освобождённого PID.
        def oracle(pid: int, number: int) -> None:
            self.assertIsNone(child.process.returncode, "signal after reap")

        with (
            patch.object(bench.os, "killpg", side_effect=oracle),
            self.assertRaisesRegex(AssertionError, "signal after reap"),
        ):
            bench.os.killpg(child.process.pid, signal.SIGKILL)

    def test_cleanup_on_timeout_cancel_and_exception_keeps_pid_reserved(self) -> None:
        for failure in ("timeout", "cancel", "exception"):
            with self.subTest(failure=failure):
                child = self.spawn(
                    "import signal,time\nsignal.signal(signal.SIGTERM,signal.SIG_IGN)\n"
                    'print("{}",flush=True)\ntime.sleep(30)'
                )
                child.ack(2)
                original = os.killpg
                signals = []

                def guarded_signal(pid: int, number: int) -> None:
                    self.assertIsNone(child.process.returncode)
                    # ECHILD здесь означает, что reservation уже потеряна.
                    os.waitid(os.P_PID, pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
                    signals.append(number)
                    original(pid, number)

                with patch.object(bench.os, "killpg", side_effect=guarded_signal):
                    if failure == "timeout":
                        with self.assertRaises(bench.BenchmarkError):
                            child.ack(0.01)
                        child.close()
                    else:
                        error = (
                            KeyboardInterrupt if failure == "cancel" else RuntimeError
                        )
                        original_sleep = time.sleep
                        interrupted = False

                        def interrupt_once(seconds: float) -> None:
                            nonlocal interrupted
                            if not interrupted:
                                interrupted = True
                                raise error("cleanup")
                            original_sleep(seconds)

                        with (
                            patch.object(
                                bench.time, "sleep", side_effect=interrupt_once
                            ),
                            self.assertRaises(error),
                        ):
                            child.close()
                    child.close()
                self.assertEqual(signals, [signal.SIGTERM, signal.SIGKILL])
                self.assertEqual(child.process.returncode, -signal.SIGKILL)

    def test_group_cleanup_kills_term_ignoring_descendant_after_leader_exit(
        self,
    ) -> None:
        marker = self.root / "descendant.pid"
        child = self.spawn(
            "import os,signal,time,pathlib\n"
            "pid=os.fork()\n"
            "if pid == 0:\n"
            " signal.signal(signal.SIGTERM,signal.SIG_IGN)\n"
            f" ready=pathlib.Path({str(marker.with_suffix('.ready'))!r})\n"
            " ready.write_text(str(os.getpid()))\n"
            f" ready.replace({str(marker)!r})\n"
            " time.sleep(30)\n"
            "else:\n"
            f" while not pathlib.Path({str(marker)!r}).exists(): time.sleep(.005)\n"
        )
        deadline = time.monotonic() + 2
        while not marker.exists():
            self.assertLess(time.monotonic(), deadline)
            time.sleep(0.005)
        descendant = int(marker.read_text())
        child.finish(0.1)
        child.close()
        deadline = time.monotonic() + 2
        while True:
            stat = Path(f"/proc/{descendant}/stat")
            if not stat.exists() or stat.read_text().split(") ")[1].startswith("Z"):
                break
            self.assertLess(time.monotonic(), deadline)
            time.sleep(0.005)

    def test_fake_child_waits_for_command_and_real_ack(self) -> None:
        child = self.spawn(
            'import json,sys,time\nfor line in sys.stdin:\n command=json.loads(line)\n time.sleep(.05)\n print(json.dumps({"event":"frame","id":command["id"]}),flush=True)'
        )
        start = child.send(dict(op="select", id=7), 1)
        report, received = child.ack(2)
        self.assertEqual(report, dict(event="frame", id=7))
        self.assertGreater(received - start, 40_000_000)

    def test_ack_timeout_and_term_ignoring_process_cleanup(self) -> None:
        child = self.spawn(
            'import signal,time\nsignal.signal(signal.SIGTERM,signal.SIG_IGN)\nprint("{\\"event\\":\\"ready\\"}",flush=True)\ntime.sleep(20)'
        )
        child.ack(2)
        with self.assertRaisesRegex(bench.BenchmarkError, "Timeout"):
            child.ack(0.05)
        child.close()
        self.children.remove(child)
        self.assertEqual(child.process.returncode, -signal.SIGKILL)

    def test_long_line_and_duplicate_ack_fail(self) -> None:
        for code, message in [
            ('import time\nprint("x"*9000,flush=True)\ntime.sleep(2)', "длинная"),
            ("import time\nprint('{}\\n{}',flush=True)\ntime.sleep(2)", "одного ack"),
        ]:
            child = self.spawn(code)
            with self.assertRaisesRegex(bench.BenchmarkError, message):
                child.ack(2)

    def test_observed_partial_duplicate_ack_is_rejected(self) -> None:
        child = self.spawn('import os,time\nos.write(1,b"{}\\n{")\ntime.sleep(20)')
        with self.assertRaisesRegex(bench.BenchmarkError, "одного ack"):
            child.ack(2)

    def test_stderr_flood_is_drained_but_file_is_bounded(self) -> None:
        child = self.spawn(
            'import os,time\nos.write(2,b"x"*2000000)\nprint("{}",flush=True)\ntime.sleep(2)'
        )
        self.assertEqual(child.ack(3)[0], {})
        # ACK в stdout не подтверждает чтение независимого stderr pipe.
        deadline = time.monotonic() + 1
        while (
            child.stderr_bytes < 2_000_000
            and (remaining := deadline - time.monotonic()) > 0
        ):
            child.pump(remaining)
        self.assertEqual(child.stderr_bytes, 2_000_000)
        self.assertEqual(child.stderr_saved, bench.MAX_STDERR)
        self.assertEqual(child.log.tell(), bench.MAX_STDERR)

    def test_premature_exit_during_idle_is_failure(self) -> None:
        child = self.spawn("import time\ntime.sleep(.05)")
        with (
            patch.object(bench, "require_unlocked", return_value={}),
            self.assertRaisesRegex(bench.BenchmarkError, "exit"),
        ):
            child.idle(0.3, True)

    def test_quit_exits_without_extra_ack(self) -> None:
        child = self.spawn(
            'import json,sys\nfor line in sys.stdin:\n if json.loads(line)["op"]=="quit": break'
        )
        child.send(dict(op="quit"), 1)
        child.finish(2)
        self.assertEqual(child.process.returncode, 0)

    def test_stdout_during_idle_is_failure(self) -> None:
        child = self.spawn(
            'import time\ntime.sleep(.05)\nprint("{}",flush=True)\ntime.sleep(2)'
        )
        with (
            patch.object(bench, "require_unlocked", return_value={}),
            self.assertRaisesRegex(bench.BenchmarkError, "idle"),
        ):
            child.idle(0.3, True)

    def test_full_run_fake_binary_and_evidence(self) -> None:
        # Fake shell проверяет порядок команд и отдаёт заранее вычисленные ответы.
        oracle = bench.Oracle(1000)
        commands = [
            bench.query(),
            bench.query("server-0.example.invalid"),
            dict(op="select", id=1),
            dict(op="favorite", id=1, value=True),
            bench.query(favorites_only=True),
            bench.query(),
            bench.query(sort="name"),
            dict(op="scroll", index=199),
            dict(op="hide"),
            dict(op="show"),
        ]
        reports = [oracle.expected("frame")]
        for command in commands:
            reports.append(oracle.expected(oracle.apply(command)))
        for report in reports:
            report.update(
                toolkit="fake", created_total=200, alive_rows=None, visible_rows=None
            )
        fake = self.root / "fake-gui"
        fake.write_text(
            f"#!{sys.executable}\nimport json,sys\ncommands={commands!r}\nreports={reports!r}\nprint(json.dumps(reports[0]),flush=True)\nfor command,report in zip(commands,reports[1:]):\n assert json.loads(sys.stdin.readline())==command\n print(json.dumps(report),flush=True)\nassert json.loads(sys.stdin.readline())=={{'op':'quit'}}\n"
        )
        fake.chmod(0o700)
        output = self.root / "evidence"
        args = argparse.Namespace(
            binary=str(fake),
            output=str(output),
            rows=1000,
            idle_seconds=0.01,
            timeout=2,
        )
        with (
            patch.object(bench, "environment", return_value={}),
            patch.object(bench, "require_unlocked", return_value={}),
            patch.object(bench, "session_state", return_value={}),
        ):
            evidence = bench.run(args)
        self.assertTrue(evidence["valid"], evidence.get("error"))
        self.assertEqual(len(evidence["actions"]), 10)
        self.assertEqual(evidence["exit_code"], 0)
        self.assertIsNone(evidence["first_frame"]["process"]["gpu"])
        self.assertEqual(
            json.loads((output / "evidence.json").read_text())["rows"], 1000
        )
        self.assertTrue((output / "stderr.log").exists())

    def test_lock_preflight_saves_invalid_evidence_without_spawning(self) -> None:
        args = argparse.Namespace(
            binary="/nonexistent",
            output=str(self.root / "invalid"),
            rows=1000,
            idle_seconds=0.01,
            timeout=1,
        )
        with patch.object(
            bench, "environment", side_effect=bench.BenchmarkError("locked")
        ):
            evidence = bench.run(args)
        self.assertFalse(evidence["valid"])
        self.assertEqual(evidence["error"], "locked")
        self.assertNotIn("first_frame", evidence)

    def test_log_open_failure_does_not_spawn(self) -> None:
        log = self.root / "existing.log"
        log.write_text("existing")
        with (
            patch.object(bench.subprocess, "Popen") as popen,
            self.assertRaises(FileExistsError),
        ):
            bench.Child([sys.executable], log)
        popen.assert_not_called()
        self.assertEqual(log.read_text(), "existing")

    def test_setup_failure_cleans_owned_group(self) -> None:
        original = bench.subprocess.Popen
        spawned = []

        def spawn(*args: object, **kwargs: object) -> subprocess.Popen:
            process = original(*args, **kwargs)
            spawned.append(process)
            return process

        with (
            patch.object(bench.subprocess, "Popen", side_effect=spawn),
            patch.object(
                bench.selectors,
                "DefaultSelector",
                side_effect=RuntimeError("setup failure"),
            ),
            self.assertRaisesRegex(RuntimeError, "setup failure"),
        ):
            bench.Child(
                [sys.executable, "-c", "import time; time.sleep(20)"],
                self.root / "setup.log",
            )
        self.assertEqual(len(spawned), 1)
        self.assertIsNotNone(spawned[0].poll())
        self.assertTrue(spawned[0].stdout.closed)

    def test_sigterm_cleans_child_and_restores_handlers(self) -> None:
        fake = self.root / "fake-blocking-gui"
        marker = self.root / "child.pid"
        report = bench.Oracle(1000).expected("frame") | dict(
            toolkit="fake", created_total=200, alive_rows=None, visible_rows=None
        )
        fake.write_text(
            f"#!{sys.executable}\nimport json,os,pathlib,sys,time\npathlib.Path({str(marker)!r}).write_text(str(os.getpid()))\nprint(json.dumps({report!r}),flush=True)\nsys.stdin.readline()\ntime.sleep(30)\n"
        )
        fake.chmod(0o700)
        output = self.root / "signal-evidence"
        wrapper = self.root / "driver-wrapper.py"
        wrapper.write_text(
            f"import sys,signal,argparse\nsys.path.insert(0,{str(Path(bench.__file__).parent)!r})\nimport ui_benchmark as b\nb.environment=lambda:{{}}\nprevious=signal.getsignal(signal.SIGTERM)\nresult=b.run(argparse.Namespace(binary={str(fake)!r},output={str(output)!r},rows=1000,idle_seconds=1,timeout=20))\nassert signal.getsignal(signal.SIGTERM)==previous\nassert not result['valid']\n"
        )
        driver = subprocess.Popen(
            [sys.executable, str(wrapper)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        pid = None
        try:
            deadline = time.monotonic() + 3
            while not marker.exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            self.assertTrue(marker.exists(), "Fake child не стартовал")
            pid = int(marker.read_text())
            driver.send_signal(signal.SIGTERM)
            self.assertEqual(driver.wait(timeout=5), 0)
            self.assertFalse(Path(f"/proc/{pid}").exists())
            evidence = json.loads((output / "evidence.json").read_text())
            self.assertFalse(evidence["valid"])
            self.assertIn("сигнал", evidence["error"])
        finally:
            if driver.poll() is None:
                driver.kill()
                driver.wait(timeout=3)

    def test_signal_in_spawn_window_is_deferred_until_child_is_owned(self) -> None:
        fake = self.root / "spawn-window-child"
        fake.write_text(f"#!{sys.executable}\nimport time\ntime.sleep(20)\n")
        fake.chmod(0o700)
        original = bench.subprocess.Popen
        spawned = []
        previous = signal.getsignal(signal.SIGTERM)

        def spawn(*args: object, **kwargs: object) -> subprocess.Popen:
            process = original(*args, **kwargs)
            spawned.append(process)
            os.kill(os.getpid(), signal.SIGTERM)
            return process

        args = argparse.Namespace(
            binary=str(fake),
            output=str(self.root / "spawn-signal-evidence"),
            rows=1000,
            idle_seconds=1,
            timeout=1,
        )
        with (
            patch.object(bench, "environment", return_value={}),
            patch.object(bench.subprocess, "Popen", side_effect=spawn),
        ):
            evidence = bench.run(args)
        self.assertFalse(evidence["valid"])
        self.assertIn("сигнал", evidence["error"])
        self.assertIsNotNone(spawned[0].poll())
        self.assertEqual(signal.getsignal(signal.SIGTERM), previous)


if __name__ == "__main__":
    unittest.main()
