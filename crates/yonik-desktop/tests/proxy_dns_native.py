"""DesktopHost: DNS-over-TCP через оба native SOCKS, без system lookup."""

from __future__ import annotations

import os
import socket
import struct
import sys
import threading
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tools"))
from dns_fixture import Reply, make_response, read_question
from proxy_native import main as native_main


def main() -> None:
    os.environ["YONIK_PROXY_ZERO_SYSTEM_DNS"] = "1"
    errors: list[Exception] = []
    queries: list[int] = []
    stop = threading.Event()
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        listener.settimeout(0.1)
        os.environ["YONIK_PROXY_FETCH_DNS"] = f"127.0.0.1:{listener.getsockname()[1]}"

        def serve() -> None:
            try:
                while not stop.is_set():
                    try:
                        stream, _ = listener.accept()
                    except socket.timeout:
                        continue
                    with stream:
                        stream.settimeout(5)

                        def exact(size: int) -> bytes:
                            data = b""
                            while len(data) < size:
                                chunk = stream.recv(size - len(data))
                                if not chunk:
                                    raise ValueError("Обрезан DNS frame")
                                data += chunk
                            return data

                        size = struct.unpack("!H", exact(2))[0]
                        packet = exact(size)
                        name, qtype, _ = read_question(packet)
                        assert name == "subscription.example"
                        queries.append(qtype)
                        reply = make_response(
                            packet,
                            Reply(addresses=("127.0.0.1",) if qtype == 1 else ()),
                        )
                        stream.sendall(struct.pack("!H", len(reply)) + reply)
            except Exception as error:
                errors.append(error)

        thread = threading.Thread(target=serve)
        thread.start()
        try:
            native_main()
        finally:
            stop.set()
            thread.join(timeout=6)
        assert not thread.is_alive() and not errors, errors
        assert len(queries) >= 12, queries
        sys.stdout.write(f"native_dns_queries={len(queries)}\n")


if __name__ == "__main__":
    main()
