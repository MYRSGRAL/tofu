"""Регрессии безопасного support metadata в штатном CI discovery."""

import json
import re
import subprocess
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_diagnostic_export as export


def fixture() -> dict:
    return {
        "schema_version": 1,
        "platform": "linux",
        "errors": [
            {
                "code": "CANCELLED",
                "message_key": "cancelled",
                "stage": "operation",
                "retryable": False,
                "params": {},
            }
        ],
    }


class ExportTests(unittest.TestCase):
    def test_known_error_gap(self) -> None:
        for key in (
            "storage",
            "engine_failed",
            "check_timeout",
            "system_timeout",
            "invalid_json",
            "dns_failed",
        ):
            value = fixture()
            value["errors"][0].update(code=key.upper(), message_key=key)
            export.export_metadata(json.dumps(value).encode())

    def test_every_catalog_key_and_strict_pair(self) -> None:
        self.assertEqual(len(export.CATALOG["messages"]), len(export.CODES))
        self.assertEqual(len(export.CODES), len(export.MESSAGES))
        for code, key in export.CODES.items():
            value = fixture()
            value["errors"][0].update(code=code, message_key=key)
            self.assertEqual(
                json.loads(export.export_metadata(json.dumps(value).encode())), value
            )
            for bad in (True, 1, [], {}, None, "CANARY", key + "\x1b"):
                value["errors"][0]["message_key"] = bad
                self.reject(value)

    def test_backend_error_factories_are_covered(self) -> None:
        source = (
            Path(__file__).resolve().parents[2] / "crates/yonik-desktop/src/backend.rs"
        ).read_text()
        keys = set(re.findall(r'DesktopError\("([a-z_]+)"\)', source))
        keys.update(
            re.findall(
                r'(?:IssueCode|ReviewFlag|CredentialError|TcpProbeStatus|ProbeResult)::[^\n]*?=> "([a-z_]+)"',
                source,
            )
        )
        keys -= {"reachable", "not_applicable", "http"}
        self.assertFalse(keys - export.MESSAGES.keys())
        for item in export.CATALOG["messages"]:
            self.assertEqual(item["stage"], "operation")
            self.assertIs(item["retryable"], False)

    def test_roundtrip_and_languages(self) -> None:
        result = export.export_metadata(json.dumps(fixture()).encode())
        self.assertEqual(json.loads(result), fixture())
        for ru, en in export.MESSAGES.values():
            self.assertTrue(ru)
            self.assertTrue(en)

    def test_catalog_and_maximum_document(self) -> None:
        catalog = (
            Path(__file__).resolve().parents[2] / "crates/yonik-desktop/qml/Main.qml"
        ).read_text()
        for key, pair in export.MESSAGES.items():
            match = re.search(r'(?<![a-z_])"?' + key + r'"?:\s*(\[[^\n]+?\])', catalog)
            if key.startswith("system_"):
                reference = (
                    Path(__file__).resolve().parents[2]
                    / "docs/evidence/p11-gui88-error-reference.txt"
                ).read_text()
                match = re.search(
                    r'(?<![a-z_])"?' + key + r'"?:\s*(\[[^\n]+?\])', reference
                )
            if key == "catalog":
                continue  # Реальная фабрика; отдельного GUI-перевода пока нет.
            self.assertIsNotNone(match, key)
            candidates = re.findall(
                r'(?<![a-z_])"?' + key + r'"?:\s*(\[[^\n]+?\])', catalog
            )
            if key.startswith("system_"):
                candidates = [match[1]]
            self.assertIn(
                list(pair), [json.loads(candidate) for candidate in candidates]
            )
        value = fixture()
        value["errors"] *= export.MAX_ERRORS
        self.assertEqual(
            json.loads(export.export_metadata(json.dumps(value).encode())), value
        )
        value["errors"][0].update(code="CLEANUP_FAILED", message_key="cleanup_failed")
        export.export_metadata(json.dumps(value).encode())

    def test_canaries_and_unknown_fields(self) -> None:
        canaries = [
            "https://user:password@host/?token=CANARY",
            "vless://CANARY",
            '{"password":"CANARY"}',
            "/home/user/secret",
            "CANARY",
        ]
        for canary in canaries:
            for field in ("credentials", "raw_source", "native_json", "cause", "url"):
                value = fixture()
                value["errors"][0][field] = canary
                self.reject(value)
            value = fixture()
            value["errors"][0]["message_key"] = canary
            self.reject(value)
            value = fixture()
            value["errors"][0]["params"] = {"detail": canary}
            self.reject(value)

    def reject(self, value: object) -> None:
        with self.assertRaises(export.ExportError) as caught:
            export.export_metadata(json.dumps(value).encode())
        self.assertNotIn("CANARY", str(caught.exception))

    def test_controls(self) -> None:
        for number in (*range(32), *range(127, 160)):
            for field in ("code", "message_key", "stage"):
                value = fixture()
                value["errors"][0][field] += chr(number)
                self.reject(value)

    def test_strict_types_and_schema(self) -> None:
        for field, bad in [
            ("schema_version", True),
            ("schema_version", 1.0),
            ("platform", "android"),
            ("errors", {}),
            ("extra", 1),
        ]:
            value = fixture()
            value[field] = bad
            self.reject(value)
        for field, bad in [
            ("retryable", 1),
            ("params", []),
            ("code", "STORAGE"),
            ("stage", None),
            ("message_key", "storage"),
        ]:
            value = fixture()
            value["errors"][0][field] = bad
            self.reject(value)
        value = fixture()
        del value["errors"][0]["params"]
        self.reject(value)

    def test_bounds_and_malformed_json(self) -> None:
        raw = json.dumps(fixture()).encode()
        padded = raw + b" " * (export.MAX_BYTES - len(raw))
        self.assertEqual(json.loads(export.export_metadata(padded)), fixture())
        with self.assertRaises(export.ExportError):
            export.export_metadata(padded + b" ")
        for raw in (
            b"[" * 10000,
            b" " * (export.MAX_BYTES + 1),
            b"\xff",
            b'{"schema_version":1,"schema_version":1}',
            b"NaN",
            b"{} trailing",
        ):
            with self.assertRaises(export.ExportError):
                export.export_metadata(raw)
        value = fixture()
        value["errors"] *= export.MAX_ERRORS + 1
        self.reject(value)

    def test_cli_no_partial_output_or_input_echo(self) -> None:
        script = Path(export.__file__)
        result = subprocess.run(
            [sys.executable, str(script)],
            input=b"CANARY\x1b[31m",
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 2)
        self.assertEqual(result.stdout, b"")
        self.assertNotIn(b"CANARY", result.stderr)
        result = subprocess.run(
            [sys.executable, str(script)],
            input=json.dumps(fixture()).encode(),
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(json.loads(result.stdout), fixture())
