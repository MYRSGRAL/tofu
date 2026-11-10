"""Отказы IPv6 fixture не должны маскироваться под запрет IPv4."""

import ast
import socket
import struct
import sys
import unittest
from collections.abc import Callable
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import dns_fixture  # noqa: E402


def fixture_functions() -> dict:
    path = (
        Path(__file__).resolve().parents[2]
        / "crates/yonik-engine/tests/fixtures/dns_ipv6.py"
    )
    source = ast.parse(path.read_text())
    names = {"disabled_family_reply", "guarded", "upstream_response"}
    selected = [
        node
        for node in source.body
        if isinstance(node, ast.FunctionDef) and node.name in names
    ]
    if len(selected) != len(names):
        raise AssertionError("Не найдены функции реальной fixture")
    namespace = {
        "os": SimpleNamespace(urandom=lambda _: b"\x12\x34"),
        "socket": socket,
        "struct": struct,
        "errors": [],
        "queries": [],
        "Callable": Callable,
    }
    namespace.update(
        {
            name: getattr(dns_fixture, name)
            for name in [
                "Reply",
                "read_question",
                "make_response",
                "question_bytes",
                "response_addresses",
            ]
        }
    )
    exec(
        compile(ast.Module(body=selected, type_ignores=[]), str(path), "exec"),
        namespace,
    )
    return namespace


def request(qtype: int) -> bytes:
    return (
        b"\x12\x34"
        + struct.pack("!HHHHH", 0x0100, 1, 0, 0, 0)
        + dns_fixture.question_bytes("disabled.test", qtype)
    )


class GateTest(unittest.TestCase):
    def test_upstream_returns_correct_a_and_aaaa(self) -> None:
        namespace = fixture_functions()
        for kind, expected in [(1, "192.0.2.20"), (28, "2001:db8::20")]:
            packet = namespace["upstream_response"](request(kind))
            self.assertEqual(
                dns_fixture.response_addresses(
                    packet, b"\x12\x34", "disabled.test", kind
                ),
                [expected],
            )
        self.assertEqual([row["qtype"] for row in namespace["queries"]], [1, 28])

    def test_thread_failure_is_reported(self) -> None:
        namespace = fixture_functions()

        def failing() -> None:
            raise ValueError("broken fixture")

        namespace["guarded"](failing)
        self.assertEqual(len(namespace["errors"]), 1)
        self.assertIn("broken fixture", namespace["errors"][0])

    def test_disabled_reply_requires_valid_negative_response(self) -> None:
        for reply, error in [
            (dns_fixture.Reply(), None),
            (dns_fixture.Reply(rcode=3), None),
            (dns_fixture.Reply(rcode=5), None),
            (dns_fixture.Reply(addresses=("192.0.2.20",)), AssertionError),
            (dns_fixture.Reply(rcode=2), AssertionError),
            (b"short", ValueError),
            (TimeoutError(), TimeoutError),
        ]:
            with self.subTest(reply=reply):
                namespace = fixture_functions()
                payload = (
                    dns_fixture.make_response(request(1), reply)
                    if isinstance(reply, dns_fixture.Reply)
                    else reply
                )

                class Client:
                    def __enter__(self) -> "Client":
                        return self

                    def __exit__(self, *_: object) -> None:
                        pass

                    def settimeout(self, _: float) -> None:
                        pass

                    def connect(self, _: tuple[str, int]) -> None:
                        pass

                    def send(self, _: bytes) -> None:
                        pass

                    def recv(self, _: int) -> bytes:
                        if isinstance(payload, Exception):
                            raise payload
                        return payload

                namespace["socket"] = SimpleNamespace(
                    AF_INET=socket.AF_INET,
                    SOCK_DGRAM=socket.SOCK_DGRAM,
                    socket=lambda *_: Client(),
                )
                if error:
                    with self.assertRaises(error):
                        namespace["disabled_family_reply"]()
                else:
                    self.assertEqual(
                        namespace["disabled_family_reply"]()["rcode"], reply.rcode
                    )
