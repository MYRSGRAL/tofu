"""Пользовательский Qt-путь batch на синтетическом loopback-источнике."""
from __future__ import annotations
import socket
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tools"))
from desktop_ui_smoke import execute, ui

slow = socket.socket()
slow.bind(("127.0.0.1", 0))
slow.listen(0)

class Handler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        port = slow.getsockname()[1] if self.path == "/slow" else 1
        body = "\n".join(f"trojan://fixture-{i}@127.0.0.1:{port}#Цель{i}" for i in range(250)).encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, format: str, *args: object) -> None:
        pass

binary, output = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve()
output.mkdir(parents=True, exist_ok=True)
server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
worker = threading.Thread(target=server.serve_forever)
worker.start()
try:
    for slow_run in (False, True):
        run_output = output / ("slow" if slow_run else "many")
        run_output.mkdir(exist_ok=True)
        path = "slow" if slow_run else "fixture"
        steps = [
            {"when": {"ready": True}, "command": {
                "command": "import_source", "name": "Fixture",
                "url": f"http://127.0.0.1:{server.server_port}/{path}",
                "local_exception": True, "route": "direct",
            }},
            ui("click", "settingsButton", when={"total": 250, "busy": False}),
            ui("click", "settingsDiagnosticsButton"),
            ui("click", "batchCheckButton"),
        ]
        if slow_run:
            steps.append(ui("click", "batchCancelButton",
                expect={"batchCancelButton": {"visible": True}}, capture="cancel-ready.png"))
            condition = {"text": {"contains": "отменено"}}
        else:
            condition = {"text": "250 / 250"}
        steps.append({"when": {}, "ui_when": {"batchProgress": condition},
            "capture_name": "result.png", "command": {"command": "shutdown"}})
        with tempfile.TemporaryDirectory(prefix="yonik-batch-qt-") as root:
            execute(binary, Path(root), steps, run_output, "1")
finally:
    server.shutdown()
    server.server_close()
    worker.join()
    slow.close()
