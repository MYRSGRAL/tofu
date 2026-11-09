"""Проверки закреплённой сборки без сети и настоящего Go toolchain."""

from __future__ import annotations

import fcntl
import hashlib
import json
import os
import select
import signal
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import build_native_adapter as builder  # noqa: E402


class BuilderTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="tofu-builder-test-")
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.root = self.base / "build"
        self.source = self.root / "build-source-sing-box"
        (self.source / "release").mkdir(parents=True)
        (self.source / "release/DEFAULT_BUILD_TAGS_OTHERS").write_text(
            "with_quic,with_utls\n"
        )
        (self.source / "go.mod").write_text("module example\n")
        (self.source / "go.sum").write_text("fixture\n")
        self.patch_file = self.base / "adapter.patch"
        self.patch_file.write_bytes(builder.DEFAULT_PATCH.read_bytes())
        self.output = self.base / "output"
        self.head = builder.SOURCE_COMMIT
        self.dirty = ""
        self.fail_go = False
        self.modify_module = False
        self.commands: list[list[str]] = []

    def fake_run(self, runner: builder.Runner, argv: list[str], cwd: Path) -> str:
        self.commands.append(argv)
        # Эксклюзивная блокировка не должна проходить, пока запущена команда.
        descriptor = os.open(self.root, os.O_RDONLY | os.O_DIRECTORY)
        try:
            with self.assertRaises(BlockingIOError):
                fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        finally:
            os.close(descriptor)
        for name in ("GOCACHE", "GOMODCACHE", "GOPATH", "GOTMPDIR", "TMPDIR"):
            self.assertTrue(Path(runner.env[name]).is_relative_to(self.root))
        self.assertEqual(runner.env["GOTOOLCHAIN"], "local")
        self.assertEqual(runner.env["GOENV"], "off")
        self.assertNotIn("GOFLAGS", runner.env)
        self.assertEqual(runner.env.get("HOME"), os.environ.get("HOME"))
        if argv[:3] == ["git", "rev-parse", "HEAD"]:
            return self.head + "\n"
        if argv[:2] == ["git", "rev-parse"]:
            return builder.SOURCE_COMMIT + "\n"
        if argv[:2] == ["git", "status"]:
            return self.dirty
        if argv[:2] == ["go", "version"]:
            return "go version go1.25.0 linux/amd64\n"
        if argv[:2] == ["go", "env"]:
            return json.dumps({"GOOS": "linux", "GOARCH": "amd64", "GOEXPERIMENT": ""})
        if argv[:2] == ["go", "build"]:
            if self.fail_go:
                raise builder.BuildError("Go завершился с ошибкой")
            if self.modify_module:
                (self.source / "go.sum").write_text("unexpected change")
            Path(argv[argv.index("-o") + 1]).write_bytes(b"fixture binary")
        return ""

    def build(self) -> Path:
        with patch.object(
            builder.Runner, "run", autospec=True, side_effect=self.fake_run
        ):
            return builder.build(self.root, self.output, self.patch_file)

    def test_unknown_patch_cannot_claim_strict_complete_feature(self) -> None:
        self.patch_file.write_text("unrelated README-only patch")
        with self.assertRaisesRegex(builder.BuildError, "каноническ"):
            self.build()
        self.assertFalse(self.commands)
        self.assertFalse((self.output / "manifest.json").exists())

    @unittest.skipUnless(hasattr(os, "pidfd_open"), "Требуется Linux pidfd")
    def test_timeout_stops_descendants_before_return(self) -> None:
        runner = builder.Runner(self.root)
        pid_file = self.base / "child.pid"
        script = (
            "import subprocess,sys,time; from pathlib import Path; "
            "p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)']); "
            "Path(sys.argv[1]).write_text(str(p.pid)); time.sleep(60)"
        )
        original_run = subprocess.run

        def short_run(
            *args: object, **kwargs: object
        ) -> subprocess.CompletedProcess[bytes]:
            kwargs["timeout"] = 0.3
            return original_run(*args, **kwargs)

        child_pid = None
        try:
            with (
                patch.object(builder, "COMMAND_TIMEOUT", 0.3, create=True),
                patch.object(builder.subprocess, "run", side_effect=short_run),
            ):
                with self.assertRaisesRegex(builder.BuildError, "лимит времени"):
                    runner.run([sys.executable, "-c", script, str(pid_file)], self.root)
            child_pid = int(pid_file.read_text())
            try:
                descriptor = os.pidfd_open(child_pid)
            except ProcessLookupError:
                return  # Потомок уже завершён и reaped.
            try:
                self.assertTrue(
                    select.select([descriptor], [], [], 0.5)[0],
                    "Потомок пережил timeout builder",
                )
            finally:
                os.close(descriptor)
        finally:
            if child_pid is None and pid_file.exists():
                child_pid = int(pid_file.read_text())
            if child_pid is not None:
                try:
                    os.kill(child_pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass

    def test_signal_during_handler_restore_still_stops_process_group(self) -> None:
        runner = builder.Runner(self.root)
        events = (signal.SIGINT, signal.SIGTERM)
        previous = {event: signal.getsignal(event) for event in events}
        original_signal = signal.signal
        interrupted = False

        def restore_with_interrupt(event: int, handler: object) -> object:
            nonlocal interrupted
            result = original_signal(event, handler)
            if (
                event == signal.SIGINT
                and handler is previous[event]
                and not interrupted
            ):
                interrupted = True
                signal.raise_signal(signal.SIGINT)
            return result

        try:
            with (
                patch.object(builder.subprocess, "Popen") as popen,
                patch.object(builder.os, "killpg") as kill_group,
                patch.object(
                    builder.signal, "signal", side_effect=restore_with_interrupt
                ),
            ):
                popen.return_value.pid = 12345
                with self.assertRaises(KeyboardInterrupt):
                    runner.run(["git", "status"], self.root)
                kill_group.assert_called_once_with(12345, signal.SIGKILL)
        finally:
            for event, handler in previous.items():
                original_signal(event, handler)

    def test_wrong_head_refused_before_patch(self) -> None:
        self.head = "0" * 40
        with self.assertRaisesRegex(builder.BuildError, "HEAD"):
            self.build()
        self.assertFalse(any("apply" in command for command in self.commands))

    def test_dirty_source_refused(self) -> None:
        self.dirty = " M go.mod\n"
        with self.assertRaisesRegex(builder.BuildError, "чист"):
            self.build()

    def test_success_records_patch_binary_and_exact_build_settings(self) -> None:
        manifest = json.loads(self.build().read_text())
        self.assertEqual(
            manifest["patch_sha256"],
            hashlib.sha256(self.patch_file.read_bytes()).hexdigest(),
        )
        self.assertEqual(
            manifest["binary_sha256"], hashlib.sha256(b"fixture binary").hexdigest()
        )
        self.assertEqual(manifest["source_commit"], builder.SOURCE_COMMIT)
        self.assertEqual(manifest["source_tag"], "v1.14.4")
        self.assertEqual(manifest["engine"], "sing-box")
        self.assertEqual(manifest["build_tags"], ["with_quic", "with_utls"])
        self.assertTrue(manifest["adapter_features"]["strict_complete_dns"])
        self.assertFalse(manifest["strictDNSGate"])
        self.assertFalse(manifest["strict_dns_gate"])
        self.assertIn("-mod=readonly", manifest["build_args"])
        self.assertEqual(manifest["build_args"][-1], "./cmd/sing-box")

    def test_failed_go_does_not_publish_manifest(self) -> None:
        self.fail_go = True
        with self.assertRaises(builder.BuildError):
            self.build()
        self.assertFalse((self.output / "manifest.json").exists())
        self.assertFalse((self.output / "sing-box").exists())

    def test_modified_module_does_not_publish(self) -> None:
        self.modify_module = True
        with self.assertRaisesRegex(builder.BuildError, "go.mod/go.sum"):
            self.build()
        self.assertFalse((self.output / "manifest.json").exists())

    def test_partial_prior_build_is_not_restarted(self) -> None:
        self.fail_go = True
        with self.assertRaises(builder.BuildError):
            self.build()
        self.fail_go = False
        with self.assertRaisesRegex(builder.BuildError, "предыдущ"):
            self.build()

    def test_output_inside_build_root_refused(self) -> None:
        self.output = self.root / "output"
        with self.assertRaisesRegex(builder.BuildError, "output"):
            self.build()

    def test_runner_keeps_failed_process_stdout_stderr(self) -> None:
        runner = builder.Runner(self.root)
        with self.assertRaisesRegex(builder.BuildError, "кодом 7"):
            runner.run(
                [
                    sys.executable,
                    "-c",
                    "import sys; sys.stdout.write('out'); sys.stderr.write('err'); sys.exit(7)",
                ],
                self.root,
            )
        self.assertEqual(next(runner.logs.glob("*.stdout.log")).read_text(), "out")
        self.assertEqual(next(runner.logs.glob("*.stderr.log")).read_text(), "err")

    def test_runner_limits_git_and_go_build_duration(self) -> None:
        runner = builder.Runner(self.root)
        for argv, timeout in ((["git", "clone"], 180), (["go", "build"], 900)):
            with (
                self.subTest(argv=argv),
                patch.object(builder.subprocess, "Popen") as popen,
                patch.object(builder.os, "killpg") as kill_group,
            ):
                process = popen.return_value
                process.pid = 12345
                process.wait.side_effect = [subprocess.TimeoutExpired(argv, timeout), 0]
                with self.assertRaisesRegex(builder.BuildError, "лимит времени"):
                    runner.run(argv, self.root)
                self.assertEqual(
                    process.wait.call_args_list[0].kwargs["timeout"], timeout
                )
                self.assertTrue(popen.call_args.kwargs["start_new_session"])
                kill_group.assert_called_once_with(12345, signal.SIGKILL)


if __name__ == "__main__":
    unittest.main()
