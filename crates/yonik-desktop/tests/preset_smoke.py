"""Настоящий Qt workflow: конфигурация, загрузка, активация, invalidation."""

from __future__ import annotations

import hashlib
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tools"))
from desktop_ui_smoke import execute, ui

DOMAINS = b"example.org\n"
IPS = b"0.0.0.0/0\n"


class Source(BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        body = {"/domains": DOMAINS, "/ips": IPS}.get(self.path)
        if body is None:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format: str, *args: object) -> None:
        """Тест не журналирует URL ресурсов."""


def run(binary: Path, output: Path) -> None:
    output.mkdir(parents=True, exist_ok=False)
    server = ThreadingHTTPServer(("127.0.0.1", 0), Source)
    thread = threading.Thread(target=server.serve_forever)
    thread.start()
    address = f"http://127.0.0.1:{server.server_port}"
    steps = [
        ui("click", "settingsButton", when={"ready": True}),
        ui("click", "settingsRoutingButton"),
        ui("click", "presetConfigure"),
        ui("text", "presetVersion", value="fixture-v1"),
        ui("text", "presetDomainsURL", value=address + "/domains"),
        ui("text", "presetIpsURL", value=address + "/ips"),
        ui("text", "presetDomainHash", value=hashlib.sha256(DOMAINS).hexdigest()),
        ui("text", "presetIpHash", value=hashlib.sha256(IPS).hexdigest()),
        ui("key", "presetFetchRoute", key="Down", capture="source-form.png"),
        ui("click", "presetLocal"),
        ui("click", "presetUpdateButton"),
        ui(
            "click",
            "presetConfigure",
            expect={
                "presetActivate": {"enabled": True},
                "presetDomainsURL": {"text": ""},
                "presetIpsURL": {"text": ""},
            },
        ),
        ui("click", "presetActivate"),
        ui(
            "text",
            "presetTarget",
            value="8.8.8.8",
            expect={"presetActivate": {"checked": True}},
        ),
        ui("click", "presetPrepareButton"),
        ui(
            "text",
            "presetTarget",
            value="example.org",
            expect={
                "presetDecision": {"visible": True, "text": {"contains": "Preset(2)"}}
            },
        ),
        ui("click", "presetPrepareButton"),
        {
            "when": {},
            "ui_when": {
                "presetDecision": {"visible": True, "text": {"contains": "Preset(1)"}}
            },
            "command": {
                "command": "routing_default",
                "expected_revision": 1,
                "request_id": 101,
                "action": "block",
            },
        },
        ui(
            "text",
            "presetTarget",
            value="8.8.8.8",
            expect={"presetDecision": {"visible": False}},
        ),
        ui("click", "presetPrepareButton"),
        ui(
            "text",
            "presetTarget",
            value="invalid IP",
            expect={"presetDecision": {"visible": True}},
        ),
        ui("click", "presetPrepareButton"),
        {
            "when": {},
            "ui_when": {"presetDecision": {"visible": False}},
            "command": {"command": "shutdown"},
        },
    ]
    try:
        execute(binary.resolve(), output, steps, output, "1")
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)
        if thread.is_alive():
            raise RuntimeError("Не завершился собственный source server")


if __name__ == "__main__":
    run(Path(sys.argv[1]), Path(sys.argv[2]).resolve())
