#!/usr/bin/env python3
"""P-03 component: 3×100 actual lifecycle cycles; не physical GUI acceptance."""

from __future__ import annotations

import argparse
import hashlib
import json
import shlex
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def run(binary: Path, output: Path) -> None:
    output.mkdir(mode=0o700)
    source = ROOT / "tools/p03_lifecycle_observer.cpp"
    subprocess.run(
        [
            "/usr/lib/qt6/moc",
            str(source),
            "-o",
            str(output / "p03_lifecycle_observer.moc"),
        ],
        check=True,
    )
    flags = shlex.split(
        subprocess.check_output(
            [
                "pkg-config",
                "--cflags",
                "--libs",
                "Qt6Quick",
                "Qt6Qml",
                "Qt6Widgets",
                "Qt6DBus",
                "Qt6Network",
            ],
            text=True,
        )
    )
    library = output / "observer.so"
    subprocess.run(
        [
            "clang++",
            "-shared",
            "-fPIC",
            "-I" + str(output),
            str(source),
            "-o",
            str(library),
            *flags,
            "-ldl",
        ],
        check=True,
    )
    results = []
    for name in [
        "positive-0",
        "positive-1",
        "positive-2",
        "negative-disconnected",
        "negative-duplicate",
        "negative-retained",
    ]:
        work = output / name
        work.mkdir(mode=0o700)
        extra = {
            "negative-disconnected": "P03_DISCONNECT=1",
            "negative-duplicate": "P03_DUPLICATE=1",
            "negative-retained": "P03_RETAIN_EDITOR=1",
        }.get(name, "P03_CONTROL=positive")
        command = [
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
            'mount --make-rprivate / && mount --bind "$1" /mnt && ip link set lo up && exec dbus-run-session -- env -u LD_LIBRARY_PATH LD_PRELOAD="$2" QT_FORCE_STDERR_LOGGING=1 QT_QPA_PLATFORM=offscreen QT_QPA_PLATFORMTHEME=kde QT_QUICK_BACKEND=software P03_DATA_DIR=/mnt/host P03_PRIVATE_WATCHER=1 "$4" "$3" /mnt/host',
            "p03",
            str(work),
            str(library),
            str(binary),
            extra,
        ]
        with (work / "qt.log").open("w") as log:
            process = subprocess.run(
                command, stdout=log, stderr=log, timeout=120, check=False
            )
        text = (work / "qt.log").read_text()
        expected = hashlib.sha256(
            (ROOT / "crates/yonik-desktop/qml/Main.qml").read_bytes()
        ).hexdigest()
        assert "P03 MAIN_SHA256 " + expected in text
        assert not any(
            error in text for error in ("TypeError", "ReferenceError", "Binding loop")
        )
        records = [
            json.loads(line.split("P03 ", 1)[1])
            for line in text.splitlines()
            if "P03 {" in line
        ]

        reason = records[-1].get("reason")
        if name.startswith("positive"):
            assert process.returncode == 0 and records[-1]["phase"] == "completed", (
                text[-4000:]
            )
            assert len([r for r in records if r["phase"] == "reopened"]) == 100
            assert (
                len([r for r in records if r["phase"] == "policy_check" and r["equal"]])
                == 100
            )
            released = [r for r in records if r["phase"] == "editor_release"]
            assert len(released) == 100
            assert all(
                r["destroyed"] and r["observed_descendants_surviving"] == 0
                for r in released
            )
            objects = [r for r in records if r["phase"] == "editor_objects"]
            assert (
                len(objects) == 100
                and len({(r["descendants"], r["connections_objects"]) for r in objects})
                == 1
            )
        elif name == "negative-retained":
            assert reason in ("Loader retained editor", "editor descendants retained")
        else:
            assert reason == "advanced notify canary"
        results.append({"name": name, "exit": process.returncode, "records": records})
    paths = [
        binary,
        source,
        ROOT / "crates/yonik-desktop/examples/p03_lifecycle.rs",
        ROOT / "crates/yonik-desktop/qml/Main.qml",
    ]
    (output / "results.json").write_text(
        json.dumps(
            {
                "acceptance": False,
                "cycles_admitted": 300,
                "boundary": "offscreen software; empty policy; bounded object counters",
                "sha256": {
                    str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths
                },
                "runs": results,
            },
            indent=2,
        )
        + "\n"
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    run(args.binary.resolve(), args.output.resolve())


if __name__ == "__main__":
    main()
