"""Проверки локального стенда, без прокси и изменения маршрутов хоста."""

from __future__ import annotations

import socket
import threading
import unittest

from tools.traffic_echo import RESPONSE_PREFIX, echo_servers, receive_exact


class EchoTests(unittest.TestCase):
    def test_tcp_udp_ipv4_ipv6_echo_and_cleanup(self) -> None:
        before = set(threading.enumerate())
        with echo_servers(port=0) as endpoints:
            self.assertEqual(len(endpoints), 4)
            for (family, kind), address in endpoints.items():
                with (
                    self.subTest(family=family, kind=kind),
                    socket.socket(family, kind) as client,
                ):
                    client.settimeout(2)
                    client.connect(address)
                    client.sendall(b"x" * 32)
                    expected = RESPONSE_PREFIX + b"x" * 32
                    actual = (
                        receive_exact(client, len(expected))
                        if kind == socket.SOCK_STREAM
                        else client.recv(4096)
                    )
                    self.assertEqual(actual, expected)
        self.assertEqual(set(threading.enumerate()), before)

    def test_truncated_tcp_response_is_an_error(self) -> None:
        sender, receiver = socket.socketpair()
        with sender, receiver:
            sender.sendall(b"short")
            sender.shutdown(socket.SHUT_WR)
            with self.assertRaises(ConnectionError):
                receive_exact(receiver, 32)


if __name__ == "__main__":
    unittest.main()
