"""Isolated native-membership overlay; не production Apply и не app attribution."""
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import threading
import time

root = Path(sys.argv[1])
subprocess.run(["ip", "link", "set", "lo", "up"], check=True)

def exact(sock: socket.socket, size: int) -> bytes:
    body = b""
    while len(body) < size:
        part = sock.recv(size - len(body))
        if not part:
            raise EOFError
        body += part
    return body

def upstream_client(sock: socket.socket) -> None:
    with sock:
        try:
            version, methods = exact(sock, 2)
            assert version == 5
            exact(sock, methods)
            sock.sendall(b"\x05\x00")
            version, command, reserved, kind = exact(sock, 4)
            assert (version, command, reserved) == (5, 1, 0)
            if kind == 1:
                exact(sock, 4)
            elif kind == 4:
                exact(sock, 16)
            else:
                exact(sock, exact(sock, 1)[0])
            exact(sock, 2)
            sock.sendall(b"\x05\x00\x00\x01\x7f\x00\x00\x01\x00\x00")
            exact(sock, 1)
            sock.sendall(b"CONTROLLED-PROXY")
        except (OSError, EOFError):
            pass

def upstream(server: socket.socket) -> None:
    while True:
        sock, _ = server.accept()
        threading.Thread(target=upstream_client, args=(sock,), daemon=True).start()

server = socket.socket()
server.bind(("127.0.0.1", 18081))
server.listen()
threading.Thread(target=upstream, args=(server,), daemon=True).start()

def request(destination: str) -> bool:
    try:
        with socket.create_connection(("127.0.0.1", 10808), timeout=2) as sock:
            sock.settimeout(2)
            sock.sendall(b"\x05\x01\x00")
            if exact(sock, 2) != b"\x05\x00":
                return False
            try:
                address = socket.inet_pton(socket.AF_INET, destination)
                address = b"\x01" + address
            except OSError:
                name = destination.encode()
                address = b"\x03" + bytes([len(name)]) + name
            sock.sendall(b"\x05\x01\x00" + address + struct.pack("!H", 443))
            header = exact(sock, 4)
            if header[1] != 0:
                return False
            if header[3] == 1:
                exact(sock, 4)
            elif header[3] == 4:
                exact(sock, 16)
            else:
                exact(sock, exact(sock, 1)[0])
            exact(sock, 2)
            sock.sendall(b"x")
            return exact(sock, 16) == b"CONTROLLED-PROXY"
    except (OSError, EOFError):
        return False

results = []
for engine, binary in [("xray", "/home/myrs/projects/yonik/local/engines/xray-26.10.31/xray"), ("sing", "/home/myrs/projects/yonik/local/engines/sing-box-1.14.4/sing-box")]:
    version = subprocess.check_output([binary, "version"], text=True)
    assert ("Xray 26.10.31" if engine == "xray" else "sing-box version 1.14.4") in version
    for kind in ["domains", "cidrs"]:
        config = root / f"{engine}-{kind}.json"
        env = dict(os.environ, XRAY_LOCATION_ASSET=str(root))
        check = [binary, "run", "-test", "-config", str(config)] if engine == "xray" else [binary, "check", "-c", str(config)]
        subprocess.run(check, cwd=root, env=env, check=True, stdout=subprocess.DEVNULL)
        command = [binary, "run", "-config", str(config)] if engine == "xray" else [binary, "run", "-c", str(config)]
        process = subprocess.Popen(command, cwd=root, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            for _ in range(100):
                if process.poll() is not None:
                    raise RuntimeError("Ядро завершилось до проверки")
                try:
                    with socket.create_connection(("127.0.0.1", 10808), timeout=0.1):
                        break
                except OSError:
                    time.sleep(0.02)
            else:
                raise TimeoutError("Listener не появился")
            cases = [("d0.example.org", False), ("d99999.example.org", False), ("d100000.example.org", True), ("sub.d0.example.org", True), ("miss.example.org", True), ("inline.example.org", False), ("sub.inline.example.org", True)] if kind == "domains" else [("10.0.0.0", False), ("10.0.195.79", False), ("10.0.195.80", True), ("9.255.255.255", True), ("10.1.0.0", True), ("192.0.2.255", False), ("192.0.3.0", True)]
            for destination, expected in cases:
                actual = request(destination)
                assert actual == expected, (engine, kind, destination, expected, actual)
                results.append({"engine": engine, "kind": kind, "destination": destination, "proxy_reached": actual})
        finally:
            process.terminate()
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
print(json.dumps({"native_membership": results, "app_attribution": "Unavailable: production safety gate preserved", "production_apply": False}))
