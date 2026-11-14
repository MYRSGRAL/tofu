"""Offline image bounds и privacy/path guards до actual extraction."""

import importlib
import re
import struct
import subprocess
import sys
import tempfile
import unittest
import zlib
from pathlib import Path
from unittest.mock import Mock

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools"))
offline = importlib.import_module("a4_offline_bootstrap_extract")
units = importlib.import_module("a4_offline_bootstrap_units")


class OfflineExtractTests(unittest.TestCase):
    def test_window_bounds_reject_overflow_and_unaligned(self) -> None:
        offline.check_window(512, 1024, 4096)
        for start, length in [(-1, 512), (1, 512), (512, 0), (3584, 1024)]:
            with self.assertRaises(ValueError):
                offline.check_window(start, length, 4096)

    def test_existing_qemu_sparse_raw_window_exact_bytes(self) -> None:
        with tempfile.TemporaryDirectory(prefix="a4-window-test-") as directory:
            source = Path(directory) / "source.raw"
            output = Path(directory) / "slice.raw"
            source.write_bytes(b"a" * 512 + b"b" * 1024 + b"c" * 2560)
            before = source.read_bytes()
            argv = offline.window_args(source, output, 512, 1024, 4096, "raw")
            subprocess.run(argv, check=True, capture_output=True)
            self.assertEqual(output.read_bytes(), b"b" * 1024)
            self.assertEqual(source.read_bytes(), before)

    def test_gpt_crc_and_partition_bounds(self) -> None:
        data = bytearray(64 * 512)
        data[510:512] = b"\x55\xaa"
        table = bytearray(4 * 128)
        table[:16] = b"a" * 16
        struct.pack_into("<QQ", table, 32, 40, 55)
        data[1024:1536] = table
        header = bytearray(92)
        header[:8] = b"EFI PART"
        struct.pack_into("<I", header, 12, 92)
        struct.pack_into("<QQQQ", header, 24, 1, 63, 34, 62)
        struct.pack_into("<QIII", header, 72, 2, 4, 128, zlib.crc32(table))
        struct.pack_into("<I", header, 16, zlib.crc32(header))
        data[512:604] = header
        rows = offline.gpt_partitions(bytes(data), len(data))
        self.assertEqual(rows[0]["start"], 40 * 512)
        data[1030] ^= 1
        with self.assertRaises(ValueError):
            offline.gpt_partitions(bytes(data), len(data))

    def test_forbidden_guest_paths_rejected(self) -> None:
        for path in [
            "/etc/ssh/ssh_host_ed25519_key",
            "/etc/shadow",
            "/home/fixture/storage/subscriptions.sqlite",
            "/var/lib/cloud/instance/user-data.txt",
            "/usr/lib/systemd/system/../../ssh/key",
        ]:
            with self.assertRaises(ValueError):
                offline.allowed_guest_path(path)
        offline.allowed_guest_path("/usr/lib/systemd/system/sshd.service")
        offline.allowed_guest_path("/var/log/journal/" + "a" * 32 + "/system.journal")

    def test_sanitized_journal_drops_nonce_and_key_payload(self) -> None:
        nonce = "f" * 32
        value = offline.sanitize_journal(
            {
                "_PID": "1",
                "MESSAGE": "Waiting for network.target " + nonce,
                "_CMDLINE": "private unexported",
            },
            nonce,
        )
        self.assertNotIn(nonce, str(value))
        self.assertNotIn("_CMDLINE", value)
        value = offline.sanitize_journal(
            {"_PID": "1", "MESSAGE": "ssh-ed25519 secret-payload"}, nonce
        )
        self.assertNotIn("secret-payload", str(value))

    def test_units_only_selector_excludes_unreadable_logs_and_credentials(self) -> None:
        pattern = units.units_pattern()
        self.assertRegex("/usr/lib/systemd/system/sshd.service", pattern)
        self.assertRegex("/etc/systemd/system/yonik-worker.socket", pattern)
        self.assertRegex(
            "/usr/lib/systemd/system/sshd.service.d/10-cloud-init.conf", pattern
        )
        for path in [
            "/var/log/cloud-init-output.log",
            "/var/log/journal/id/system.journal",
            "/etc/ssh/ssh_host_ed25519_key",
            "/home/fixture/storage/subscriptions.sqlite",
        ]:
            self.assertIsNone(re.fullmatch(pattern, path))

    def test_journal_directory_rejects_parent_escape_and_symlink(self) -> None:
        with tempfile.TemporaryDirectory(prefix="a4-journal-path-") as directory:
            root = Path(directory) / "own"
            root.mkdir()
            other = Path(directory) / "other"
            other.mkdir()
            with self.assertRaises(ValueError):
                offline.checked_journal_directory(root / ".." / "other", root)
            (root / "link").symlink_to(other, target_is_directory=True)
            with self.assertRaises(ValueError):
                offline.checked_journal_directory(root / "link", root)
            self.assertEqual(offline.checked_journal_directory(root, root), root)
            reader = offline.Extraction.__new__(offline.Extraction)
            reader.out = root
            reader.run = Mock(
                side_effect=AssertionError("must reject before journalctl")
            )
            with self.assertRaises(ValueError):
                reader.journals(str(root / ".." / "other"))
            reader.run.assert_not_called()

    def test_multiline_private_key_payload_fully_redacted(self) -> None:
        text = "prefix\n-----BEGIN OPENSSH PRIVATE KEY-----\nsecret-base64-payload\n-----END OPENSSH PRIVATE KEY-----"
        self.assertNotIn("secret-base64-payload", offline.sanitize_text(text, "f" * 32))

    def test_nonrelevant_journal_not_exported(self) -> None:
        self.assertIsNone(
            offline.sanitize_journal(
                {
                    "_PID": "500",
                    "_SYSTEMD_UNIT": "unrelated.service",
                    "MESSAGE": "private",
                },
                "f" * 32,
            )
        )
