"""Ограниченные localhost echo endpoints для синтетических TUN-проверок."""

from __future__ import annotations

import socket
import socketserver
import threading
from collections.abc import Iterator
from contextlib import ExitStack, contextmanager

RESPONSE_PREFIX = b"yonik-tun:"
PAYLOAD_SIZE = 32


def receive_exact(connection: socket.socket, size: int) -> bytes:
    data = bytearray()
    while len(data) < size:
        chunk = connection.recv(size - len(data))
        if not chunk:
            raise ConnectionError("Соединение закрыто до полного ответа")
        data.extend(chunk)
    return bytes(data)


class TcpEcho(socketserver.BaseRequestHandler):
    def handle(self) -> None:
        self.request.settimeout(2)
        payload = receive_exact(self.request, PAYLOAD_SIZE)
        self.request.sendall(RESPONSE_PREFIX + payload)


class UdpEcho(socketserver.BaseRequestHandler):
    def handle(self) -> None:
        payload, connection = self.request
        connection.sendto(RESPONSE_PREFIX + payload, self.client_address)


class TcpV6(socketserver.TCPServer):
    address_family = socket.AF_INET6


class UdpV6(socketserver.UDPServer):
    address_family = socket.AF_INET6


def join_thread(thread: threading.Thread) -> None:
    thread.join(timeout=3)
    if thread.is_alive():
        raise RuntimeError("Echo-сервер не остановился")


@contextmanager
def echo_servers(port: int = 24081) -> Iterator[dict[tuple[int, int], tuple[str, int]]]:
    endpoints: dict[tuple[int, int], tuple[str, int]] = {}
    with ExitStack() as stack:
        for server_type, handler, address in [
            (socketserver.TCPServer, TcpEcho, "127.0.0.1"),
            (socketserver.UDPServer, UdpEcho, "127.0.0.1"),
            (TcpV6, TcpEcho, "::1"),
            (UdpV6, UdpEcho, "::1"),
        ]:
            server = stack.enter_context(server_type((address, port), handler))
            thread = threading.Thread(
                target=server.serve_forever, kwargs={"poll_interval": 0.05}
            )
            thread.start()
            stack.callback(join_thread, thread)
            stack.callback(server.shutdown)
            endpoints[(server.address_family, server.socket_type)] = (
                address,
                server.server_address[1],
            )
        yield endpoints
