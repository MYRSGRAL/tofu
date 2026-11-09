"""Проверка сетевого fixture серии без запуска GUI или внешней сети."""

import socket
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from urllib.parse import urlsplit

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import desktop_smoke


class DesktopTcpSeriesFixtureTests(unittest.TestCase):
    def test_fixture_accepts_three_payload_free_connections_per_reachable_case(
        self,
    ) -> None:
        connections = []

        def fake_gui(*args: object, **kwargs: object) -> dict[str, object]:
            status = kwargs["tcp_status"]
            profile = urlsplit(str(kwargs["profile"]))
            if status == "reachable":
                for _ in range(3):
                    with socket.create_connection(
                        ("127.0.0.1", profile.port), timeout=2
                    ):
                        connections.append(str(args[3]))
            return {"case": args[3], "status": status}

        with tempfile.TemporaryDirectory(prefix="tofu-tcp-series-test-") as temporary:
            with patch.object(desktop_smoke, "run_case", side_effect=fake_gui):
                results = desktop_smoke.run_tcp(
                    Path("unused-gui"), Path(temporary), native_dns=True, namespace=True
                )
        self.assertEqual(connections.count("tcp-literal"), 3)
        self.assertEqual(connections.count("tcp-resolved"), 3)
        self.assertEqual(len(connections), 6)
        self.assertEqual(
            [result["status"] for result in results],
            ["reachable", "resolver_unavailable", "reachable", "connect_failed"],
        )
