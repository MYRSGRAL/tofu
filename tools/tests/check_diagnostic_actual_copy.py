#!/usr/bin/env python3
"""Проверяет настоящий CXX-Qt copy на контролируемом snapshot, без DesktopHost.

Запускать только внутри private D-Bus с QT_QPA_PLATFORM=offscreen. Перехват
загрузки QML действует только в дочернем тестовом процессе; production не меняет.
"""

from __future__ import annotations

import argparse
import os
import shlex
import shutil
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
INTERPOSE = r"""
#include <QQmlApplicationEngine>
#include <QUrl>
#include <dlfcn.h>
#include <cstdlib>
#include <cstdio>
void QQmlApplicationEngine::load(const QUrl&) {
    using Load = void (*)(QQmlApplicationEngine*, const QUrl&);
    auto original = reinterpret_cast<Load>(dlsym(RTLD_NEXT, "_ZN21QQmlApplicationEngine4loadERK4QUrl"));
    if (!original) std::abort();
    std::fputs("controlled-QML loaded\n", stderr);
    original(this, QUrl::fromLocalFile(QString::fromUtf8(std::getenv("YONIK_TEST_QML"))));
}
"""


def main() -> None:
    """Собирает изолированный test interposer и запускает production binary."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("task_dir", type=Path)
    args = parser.parse_args()
    if os.environ.get("QT_QPA_PLATFORM") != "offscreen" or not os.environ.get(
        "DBUS_SESSION_BUS_ADDRESS"
    ):
        parser.error("требуются offscreen и private D-Bus")
    task = args.task_dir.resolve()
    source = task / "qml-load-test.cpp"
    library = task / "qml-load-test.so"
    source.write_text(INTERPOSE)
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
        QT_LOGGING_RULES="qml.debug=true;qml.warning=true",
        QT_FORCE_STDERR_LOGGING="1",
        YONIK_TEST_QML=str(ROOT / "tools/tests/qml/diagnostic-actual-copy.qml"),
    )
    runtime = tempfile.mkdtemp(prefix="yp11-")
    try:
        env["XDG_RUNTIME_DIR"] = runtime
        result = subprocess.run(
            [
                str(args.binary.resolve()),
                "--smoke-test",
                "--data-dir",
                str(task / "data"),
            ],
            env=env,
            check=True,
            timeout=30,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        print(result.stdout, end="")
        expected = [
            "cancelled",
            "cleanup_failed",
            "storage",
            "invalid_json",
            "check_timeout",
            "dns_failed",
            "system_timeout",
            "negative clipboard unchanged",
        ]
        expected = [
            key + " " + locale for locale in ("ru", "en") for key in expected[:-1]
        ] + [expected[-1]]
        for key in expected:
            if result.stdout.count("actualCopy PASS " + key + "\n") != 1:
                raise RuntimeError("Не подтверждён actualCopy: " + key)
    finally:
        shutil.rmtree(runtime)


if __name__ == "__main__":
    main()
