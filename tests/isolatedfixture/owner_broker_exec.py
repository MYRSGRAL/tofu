"""Socket holder сохраняет PID и FD при смене executable."""
import json
import os
import socket
import sys

with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as owned:
    owned.bind(("127.0.0.1", 0))
    owned.connect(("127.0.0.1", 45679))
    owned.set_inheritable(True)
    print(json.dumps({"port": owned.getsockname()[1]}), flush=True)
    sys.stdin.buffer.read(1)
    os.execv("/usr/bin/sleep", ["sleep", "10"])
