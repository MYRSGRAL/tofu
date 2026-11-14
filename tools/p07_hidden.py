#!/usr/bin/env python3
"""Focused P07: пассивные actual consumer counters, без повторения TLS/batch campaign."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shlex
import shutil
import subprocess
import sys
from pathlib import Path

from native_cancel_matrix import remaining, snapshot

ROOT = Path(__file__).resolve().parents[1]
ZERO_KEYS = (
    "serial_notify",
    "serial_value",
    "page_apply",
    "model_notify",
    "count_notify",
    "delegate_data_notify",
    "delegate_text_notify",
    "delegate_created",
)


def static_trace() -> dict:
    view = ROOT / "crates/yonik-desktop/qml/LogsView.qml"
    main = ROOT / "crates/yonik-desktop/qml/Main.qml"
    text = view.read_text()
    assert "Timer" not in text
    assert "root.serial += 1" in text and 'command: "logs_read"' in text
    assert (
        'root.visible && root.pending !== "" && root.response && root.response.request_id === root.pending'
        in text
    )
    assert (
        "model: root.visible && !root.stale && root.page ? root.page.events : []"
        in text
    )
    assert text.count("root.read(") == 2
    assert "const next = JSON.parse(raw)" in main.read_text()
    original = (ROOT / "crates/yonik-desktop/src/bridge_impl.rs").read_text()
    private = (
        ROOT / "crates/yonik-desktop/examples/p07_hidden/bridge_impl.rs"
    ).read_text()

    def span(value: str) -> tuple[int, int]:
        start = value.index("    pub fn dispatch(")
        return start, value.index("\n    pub fn copy_diagnostics(", start)

    a, b = span(private)
    c, d = span(original)
    restored = (private[:a] + original[c:d] + private[b:]).replace(
        "../../src/bridge_pending.rs", "bridge_pending.rs"
    )
    assert restored == original, "Private bridge drift вне наблюдаемого dispatch hook"
    return {
        "sha256": {
            str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in [view, main]
        },
        "read_calls": 2,
        "no_timer": True,
        "matching_visible_pending_guard": True,
        "visible_stale_page_model_guard": True,
        "bridge_difference": "только private dispatch: producer hook и actual successful logs_read admission trace",
    }


def records(text: str) -> dict:
    result = {}
    for line in text.splitlines():
        for marker in (
            "VISIBLE_CANARY",
            "FRAME_CANARY",
            "QUIET_BEGIN",
            "QUIET_END",
            "HIDDEN_BEGIN",
            "HIDDEN_END",
            "POST_VISIBLE_CANARY",
            "LOGS",
        ):
            prefix = "P07 " + marker + " "
            if prefix in line:
                result[marker] = json.loads(line.split(prefix, 1)[1])
    return result


def interval(text: str, begin: str, end: str) -> str:
    return text.split("P07 " + begin + " ", 1)[1].split("P07 " + end + " ", 1)[0]


def inside(args: argparse.Namespace) -> None:
    assert Path("/proc/self/ns/pid").readlink() == Path("/proc/1/ns/pid").readlink()
    assert os.readlink("/proc/self/ns/net") != args.parent_net
    assert [
        line.split(":")[0].strip()
        for line in Path("/proc/net/dev").read_text().splitlines()[2:]
    ] == ["lo"]
    trace = static_trace()
    build = args.output / "observer"
    build.mkdir()
    source = build / "p07_hidden_observer.cpp"
    shutil.copyfile(ROOT / "tools/p07_hidden_observer.cpp", source)
    subprocess.run(
        [
            "/usr/lib/qt6/moc",
            str(source),
            "-o",
            str(build / "p07_hidden_observer.moc"),
        ],
        check=True,
    )
    flags = shlex.split(
        subprocess.check_output(
            ["pkg-config", "--cflags", "--libs", "Qt6Quick", "Qt6Qml"], text=True
        )
    )
    library = build / "observer.so"
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
    baseline = snapshot()
    runs = []
    for name in [
        "positive-0",
        "positive-1",
        "positive-2",
        "negative-notify-disconnected",
        "negative-frame-disconnected",
    ]:
        work = args.output / name
        work.mkdir(mode=0o700)
        negative = name.startswith("negative")
        env = dict(
            os.environ,
            LD_PRELOAD=str(library),
            QT_QPA_PLATFORM="offscreen",
            QT_QUICK_BACKEND="software",
            QT_QUICK_CONTROLS_STYLE="Basic",
            QT_FORCE_STDERR_LOGGING="1",
        )
        if name == "negative-notify-disconnected":
            env["P07_SUPPRESS_NOTIFY"] = "1"
        else:
            env.pop("P07_SUPPRESS_NOTIFY", None)
        if name == "negative-frame-disconnected":
            env["P07_SUPPRESS_FRAME"] = "1"
        else:
            env.pop("P07_SUPPRESS_FRAME", None)
        with (work / "qt.log").open("w") as log:
            process = subprocess.run(
                [args.binary, str(work / "host"), str(work / "no-engines")],
                env=env,
                stdout=log,
                stderr=log,
                timeout=22,
                check=False,
            )
        text = (work / "qt.log").read_text()
        parsed = records(text)
        terminal = remaining(snapshot(), baseline)
        assert not terminal, terminal
        if name == "negative-frame-disconnected":
            assert process.returncode != 0 and "frameSwapped canary" in text
            assert (
                parsed["VISIBLE_CANARY"]["serial_notify"] > 0
                and parsed["FRAME_CANARY"]["delta"] == 0
            )
            assert "P07 HIDDEN_PASS" not in text
        elif negative:
            assert process.returncode != 0 and "visible refresh notify canary" in text
            assert (
                "P07 READ_ADMITTED" in text
                and parsed["VISIBLE_CANARY"]["serial_notify"] == 0
            )
            assert "P07 HIDDEN_PASS" not in text
        else:
            assert process.returncode == 0 and "P07 HIDDEN_PASS" in text, text[-5000:]
            assert not any(
                error in text
                for error in (
                    "HIDDEN_FAIL",
                    "TypeError",
                    "ReferenceError",
                    "Binding loop",
                )
            )
            expected_hash = trace["sha256"]["crates/yonik-desktop/qml/Main.qml"]
            assert "P07 MAIN_SHA256 " + expected_hash in text
            hidden = parsed["HIDDEN_END"]
            assert hidden["t_ms"] >= 1200
            assert (
                all(hidden[key] == 0 for key in ZERO_KEYS)
                and hidden["frame_swapped"] == 0
            )
            segment = interval(text, "HIDDEN_BEGIN", "HIDDEN_END")
            assert "P07 READ_ADMITTED" not in segment
            progress = [
                int(line.split("P07 PRODUCER_PROGRESS ", 1)[1])
                for line in segment.splitlines()
                if "P07 PRODUCER_PROGRESS " in line
            ]
            assert len(progress) >= 2 and progress[-1] - progress[0] >= 100250, progress
            assert "P07 READ_ADMITTED" not in interval(text, "QUIET_BEGIN", "QUIET_END")
            assert text.count("P07 READ_ADMITTED") == 2
            parsed["hidden_producer_progress"] = progress
            parsed["hidden_actual_read_admissions"] = 0
            parsed["total_actual_read_admissions"] = 2
        runs.append(
            {
                "name": name,
                "exit": process.returncode,
                "records": parsed,
                "terminal": terminal,
            }
        )
        (args.output / "results.json").write_text(
            json.dumps({"static_trace": trace, "runs": runs}, indent=2) + "\n"
        )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary")
    parser.add_argument("output", type=Path)
    parser.add_argument("--inside", action="store_true")
    parser.add_argument("--parent-net")
    args = parser.parse_args()
    if args.inside:
        inside(args)
        return
    args.output.mkdir(mode=0o700)
    subprocess.run(
        [
            "unshare",
            "--user",
            "--map-root-user",
            "--net",
            "--mount",
            "--pid",
            "--fork",
            "--kill-child=SIGKILL",
            "--mount-proc",
            "sh",
            "-c",
            'mount --make-rprivate / && mount --bind "$1" /mnt && shift && ip link set lo up && exec dbus-run-session -- "$@"',
            "p07-hidden",
            str(args.output.resolve()),
            sys.executable,
            str(Path(__file__).resolve()),
            str(Path(args.binary).resolve()),
            "/mnt",
            "--inside",
            "--parent-net",
            os.readlink("/proc/self/ns/net"),
        ],
        check=True,
        timeout=100,
    )


if __name__ == "__main__":
    main()
