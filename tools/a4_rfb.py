"""Минимальный RFB raw client только для Unix socket own isolated QVNC."""

from __future__ import annotations

import socket
import struct
import time
from pathlib import Path


class Rfb:
    def __init__(self, path: Path) -> None:
        self.socket = socket.socket(socket.AF_UNIX)
        self.socket.settimeout(10)
        self.socket.connect(str(path))
        version = self.read(12)
        self.socket.sendall(version)
        if version >= b"RFB 003.007\n":
            types = self.read(self.read(1)[0])
            assert 1 in types
            self.socket.sendall(b"\x01")
            assert self.read(4) == bytes(4)
        else:
            assert self.read(4) == b"\0\0\0\1"
        self.socket.sendall(b"\x01")
        self.width, self.height = struct.unpack(">HH", self.read(4))
        assert 0 < self.width <= 4096 and 0 < self.height <= 4096
        self.read(16)
        self.read(struct.unpack(">I", self.read(4))[0])
        self.socket.sendall(
            b"\0\0\0\0"
            + struct.pack(">BBBBHHHBBBxxx", 32, 24, 0, 1, 255, 255, 255, 16, 8, 0)
        )
        self.socket.sendall(struct.pack(">BBHi", 2, 0, 1, 0))

    def read(self, count: int) -> bytes:
        assert 0 <= count <= 64 * 1024 * 1024
        data = bytearray()
        while len(data) < count:
            chunk = self.socket.recv(count - len(data))
            if not chunk:
                raise EOFError("QVNC закрыл socket")
            data.extend(chunk)
        return bytes(data)

    def capture(self, path: Path) -> None:
        from PIL import Image

        image = Image.new("RGB", (self.width, self.height))
        rectangles = []
        for _ in range(3):
            self.socket.sendall(
                struct.pack(">BBHHHH", 3, 0, 0, 0, self.width, self.height)
            )
            while True:
                message = self.read(1)[0]
                if message == 2:
                    continue
                if message == 3:
                    self.read(3)
                    self.read(struct.unpack(">I", self.read(4))[0])
                    continue
                assert message == 0, message
                self.read(1)
                count = struct.unpack(">H", self.read(2))[0]
                for _ in range(count):
                    x, y, w, h, encoding = struct.unpack(">HHHHi", self.read(12))
                    assert encoding == 0, encoding
                    assert x + w <= self.width and y + h <= self.height
                    rectangles.append((x, y, w, h))
                    image.paste(
                        Image.frombytes(
                            "RGB", (w, h), self.read(w * h * 4), "raw", "BGRX"
                        ),
                        (x, y),
                    )
                break
            time.sleep(0.2)
        image.save(path)
        path.with_suffix(".rectangles.json").write_text(
            __import__("json").dumps(rectangles)
        )

    def click(self, x: int, y: int) -> None:
        for buttons in (1, 0):
            self.socket.sendall(struct.pack(">BBHH", 5, buttons, x, y))

    def key(self, key: int) -> None:
        for down in (1, 0):
            self.socket.sendall(struct.pack(">BBHI", 4, down, 0, key))

    def text(self, text: str) -> None:
        for character in text:
            self.key(ord(character))

    def close(self) -> None:
        self.socket.close()
