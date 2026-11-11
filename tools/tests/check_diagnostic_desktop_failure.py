#!/usr/bin/env python3
"""Настоящий DesktopHost storage failure → production errorText → clipboard.

Запуск: unshare -Urnpf --mount-proc dbus-run-session -- env
QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=software python3 THIS BINARY TASK.
Каталог TASK должен принадлежать тесту. Keyring не достигается: mode=0755.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shlex
import subprocess
import tempfile
from pathlib import Path

from check_diagnostic_actual_copy import INTERPOSE, ROOT


def main() -> None:
    """Подменяет только начальный QML; snapshot приходит из DesktopHost."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("task", type=Path)
    parser.add_argument("--v2", action="store_true")
    args = parser.parse_args()
    if os.environ.get("QT_QPA_PLATFORM") != "offscreen" or not os.environ.get(
        "DBUS_SESSION_BUS_ADDRESS"
    ):
        parser.error("нужны offscreen и private bus")
    task = args.task.resolve()
    task.mkdir(mode=0o700, parents=True, exist_ok=True)
    data = task / "data"
    data.mkdir(exist_ok=True)
    data.chmod(0o755)
    runtime_owner = tempfile.TemporaryDirectory(prefix="yp11a-")
    runtime = Path(runtime_owner.name)
    main_qml = (ROOT / "crates/yonik-desktop/qml/Main.qml").read_text()
    mapping = main_qml.split("    function errorText(key) {", 1)[1].split(
        "\n    DesktopBridge", 1
    )[0]
    qml = (
        """import QtQuick
import QtQuick.Controls
import cloud.highpriority.yonik
ApplicationWindow {
 id: window; visible: true; width: 640; height: 480
 property string language: "ru"
 function tr(ru, en) { return language === "ru" ? ru : en }
 function errorText(key) {"""
        + mapping
        + """
 DesktopBridge { id: backend }
 TextEdit { id: probe }
 function require(ok, message) { if (!ok) { console.error(message); Qt.exit(3); throw Error(message) } }
 function clipboard() { probe.clear(); probe.paste(); return probe.text }
 Component.onCompleted: backend.initialize()
 Timer { interval: 100; running: true; repeat: true
 property int attempts: 0
 onTriggered: {
   attempts++
   const state = JSON.parse(backend.snapshot_json)
   if (!state.error) { require(attempts < 100, "DesktopHost failure missing"); return }
   stop()
   require(state.error === "storage", "unexpected failure")
   for (let locale of ["ru", "en"]) {
     window.language = locale
     probe.text = window.errorText(state.error)
     require(probe.text === (locale === "ru" ? "Ошибка локального хранилища. Проверьте доступ к данным приложения." : "Local storage error. Check access to application data."), "UI mapping mismatch")
     console.log("desktopFailure UI " + locale + " " + probe.text)
     require(backend.copy_diagnostics(), "copy rejected real snapshot")
     const exported = clipboard()
     const metadata = JSON.parse(exported)
     require(metadata.errors.length === 1 && metadata.errors[0].code === "STORAGE" && metadata.errors[0].message_key === "storage", "wrong metadata")
     require(metadata.errors[0].stage === "operation" && metadata.errors[0].retryable === false, "legacy contract changed")
     console.log("desktopFailure EXPORT " + locale + " " + exported)
   }
   const previous = clipboard()
   // Отдельный controlled negative, не источник production failure выше.
   backend.snapshot_json = JSON.stringify({error: "https://secret:CANARY@example.invalid/\\u001b"})
   require(!backend.copy_diagnostics() && clipboard() === previous, "negative changed clipboard")
   console.log("desktopFailure PASS negative clipboard unchanged")
   Qt.quit()
 } }
}
"""
    )
    if args.v2:
        qml = qml.replace("backend.copy_diagnostics()", "backend.copy_diagnostics_v2()")
        qml = qml.replace(
            "metadata.errors.length === 1", "metadata.errors.length === 2"
        )
        qml = qml.replace(
            'metadata.errors[0].stage === "operation"',
            'metadata.schema_version === 2 && metadata.errors[0].stage === "storage_open" && metadata.errors[0].reason === "data_directory_permissions"',
        )
        qml = qml.replace(
            "const previous = clipboard()",
            """const previous = clipboard()
   for (let [field, canary] of [["reason", "CANARY\\u001b"], ["params", {url:"https://secret:CANARY@example.invalid"}], ["extra", true], ["request_id", "01"], ["generation", "18446744073709551616"]]) {
     const bad = JSON.parse(JSON.stringify(state))
     bad.error_contexts[0][field] = canary
     backend.snapshot_json = JSON.stringify(bad)
     require(!backend.copy_diagnostics_v2() && clipboard() === previous, "V2 canary changed clipboard")
   }
""",
        )
    fixture = task / "desktop-failure.qml"
    fixture.write_text(qml)
    source = task / "interpose.cpp"
    source.write_text(INTERPOSE)
    library = task / "interpose.so"
    flags = shlex.split(
        subprocess.check_output(
            ["pkg-config", "--cflags", "--libs", "Qt6Qml"], text=True
        )
    )
    subprocess.run(
        [
            "clang++",
            "-shared",
            "-fPIC",
            str(source),
            "-o",
            str(library),
            *flags,
            "-ldl",
        ],
        check=True,
    )
    env = dict(
        os.environ,
        LD_PRELOAD=str(library),
        YONIK_TEST_QML=str(fixture),
        XDG_RUNTIME_DIR=str(runtime),
        QT_FORCE_STDERR_LOGGING="1",
        QT_LOGGING_RULES="qml.debug=true;qml.warning=true",
    )
    result = subprocess.run(
        [str(args.binary.resolve()), "--data-dir", str(data)],
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        timeout=20,
        check=False,
    )
    print(result.stdout, end="")
    result.check_returncode()
    for marker in [
        "UI ru",
        "UI en",
        "EXPORT ru",
        "EXPORT en",
        "PASS negative clipboard unchanged",
    ]:
        if result.stdout.count("desktopFailure " + marker) != 1:
            raise RuntimeError("Нет свидетельства: " + marker)
    print(
        json.dumps(
            {
                "binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(),
                "qml_sha256": hashlib.sha256(main_qml.encode()).hexdigest(),
                "fixture_sha256": hashlib.sha256(qml.encode()).hexdigest(),
            },
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
