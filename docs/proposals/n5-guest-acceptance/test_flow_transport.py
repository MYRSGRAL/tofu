"""Pure-регрессии framing и IP canonicalization; сеть не открывается."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

BASE = (
    Path(__file__).resolve().parents[3]
    / "crates/yonik-desktop/examples/n5_guest_driver"
)


def load(name: str):
    spec = importlib.util.spec_from_file_location(name, BASE / (name + ".py"))
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


CLIENT = load("n5-flow-client")
RESPONDER = load("n5-flow-responder")


class FlowTransportTests(unittest.TestCase):
    def test_client_exchange_collects_complete_fragmented_tcp_reply(self) -> None:
        flow = {
            "endpoint": "192.0.2.4:100",
            "source_ip": "192.0.2.2",
            "expected_peer_ip": "192.0.2.3",
            "transport": "tcp",
            "token": "token",
        }
        connection = Mock()
        connection.__enter__ = Mock(return_value=connection)
        connection.__exit__ = Mock(return_value=False)
        connection.getsockname.return_value = ("192.0.2.2", 123)
        connection.recv.side_effect = [b"token:n:", b"server\n"]
        with patch.object(CLIENT.socket, "socket", return_value=connection):
            result = CLIENT.exchange(flow, "n")
        self.assertEqual(result["reply"], "token:n:server")
        self.assertEqual(connection.recv.call_count, 2)

    def test_fragmented_request_and_response(self) -> None:
        for module in (CLIENT, RESPONDER):
            for chunks, expected in (
                ([b"token:n:", b"server\n"], b"token:n:server"),
                ([b'{"token":', b'"n"}', b"\n"], b'{"token":"n"}'),
            ):
                with self.subTest(module=module.__name__, expected=expected):
                    connection = Mock()
                    connection.recv.side_effect = chunks
                    with patch.object(module.time, "monotonic", return_value=1):
                        self.assertEqual(
                            module.read_frame(connection, 3, 1024), expected
                        )
                    self.assertEqual(connection.recv.call_count, len(chunks))

    def test_truncated_oversized_and_extra_frame_are_rejected(self) -> None:
        for module in (CLIENT, RESPONDER):
            for chunks in ([b"partial", b""], [b"x" * 8], [b"one\ntwo\n"]):
                with self.subTest(module=module.__name__, chunks=chunks):
                    connection = Mock()
                    connection.recv.side_effect = chunks
                    with patch.object(module.time, "monotonic", return_value=1):
                        with self.assertRaises(ValueError):
                            module.read_frame(connection, 3, 7)

    def test_fragments_share_absolute_deadline(self) -> None:
        for module in (CLIENT, RESPONDER):
            connection = Mock()
            connection.recv.side_effect = [b"partial", b"never\n"]
            with patch.object(module.time, "monotonic", side_effect=[1, 3]):
                with self.assertRaises(TimeoutError):
                    module.read_frame(connection, 3, 1024)
            connection.recv.assert_called_once()
            connection.settimeout.assert_called_once_with(2)

    def test_ipv6_controls_and_wire_use_same_canonical_form(self) -> None:
        expanded = {
            "endpoint": "[2001:0db8:0000::4]:100",
            "source_ip": "2001:0db8::2",
            "expected_peer_ip": "2001:0db8::3",
        }
        wire = {
            "endpoint": "[2001:db8::4]:100",
            "source_ip": "2001:db8::2",
            "expected_peer_ip": "2001:db8::3",
        }
        self.assertEqual(CLIENT.normalize_flow(expanded), wire)
        self.assertEqual(CLIENT.normalize_flow(wire), wire)
        self.assertEqual(RESPONDER.endpoint_key(expanded["endpoint"]), wire["endpoint"])
        self.assertEqual(RESPONDER.endpoint_key(wire["endpoint"]), wire["endpoint"])


if __name__ == "__main__":
    unittest.main()
