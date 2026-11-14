#!/usr/bin/env python3
"""P-03 integrated: новый source-bound ELF; preflight перед 3×100 campaign."""

from __future__ import annotations

import argparse
import hashlib
import json
import shlex
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def trace_source() -> dict[str, str]:
    production = (ROOT / "crates/yonik-desktop/src/bridge_impl.rs").read_text()
    private_path = ROOT / "crates/yonik-desktop/examples/p03_integrated/bridge_impl.rs"
    private = private_path.read_text()

    def span(value: str) -> tuple[int, int]:
        start = value.index("    pub fn dispatch(")
        return start, value.index("\n    pub fn copy_diagnostics(", start)

    a, b = span(private)
    c, d = span(production)
    restored = (private[:a] + production[c:d] + private[b:]).replace(
        "../../src/bridge_pending.rs", "bridge_pending.rs"
    )
    assert restored == production, (
        "Private bridge drift outside dispatch trace/fault hooks"
    )
    paths = [
        private_path,
        ROOT / "crates/yonik-desktop/examples/p03_integrated.rs",
        ROOT / "tools/p03_integrated_observer.cpp",
        ROOT / "tools/p03_integrated.py",
        ROOT / "crates/yonik-desktop/qml/Main.qml",
        ROOT / "crates/yonik-desktop/src/backend.rs",
        ROOT / "crates/yonik-desktop/src/desktop_schedule.rs",
        ROOT / "crates/yonik-desktop/src/application.cpp",
        ROOT / "crates/yonik-desktop/src/bridge_impl.rs",
    ]
    return {
        str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in paths
    }


def run(binary: Path, output: Path, stage: str) -> None:
    hashes = trace_source()
    output.mkdir(mode=0o700)
    source = ROOT / "tools/p03_integrated_observer.cpp"
    subprocess.run(
        [
            "/usr/lib/qt6/moc",
            str(source),
            "-o",
            str(output / "p03_integrated_observer.moc"),
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
    cases = (
        [
            ("oracle-positive", "", 3, None),
            ("negative-frame", "P03_SUPPRESS_FRAME=1", 1, "visible frame canary"),
            (
                "negative-hidden-gate",
                "P03_DROP_VISIBILITY=1",
                1,
                "backend hidden release gate",
            ),
            (
                "negative-observer-gate",
                "P03_SUPPRESS_GATE=1",
                1,
                "initial backend visible gate",
            ),
            (
                "negative-duplicate-admission",
                "P03_DUPLICATE_ADMISSION=1",
                1,
                "initial single visibility admission",
            ),
            (
                "negative-mixed-fields",
                "P03_CORRUPT_FIELDS=1",
                1,
                "mixed fields preserved across views",
            ),
        ]
        if stage == "preflight"
        else [(f"positive-{i}", "", 100, None) for i in range(3)]
    )
    if stage == "model-preflight":
        cases = [
            ("model-positive", "", 3, None),
            (
                "negative-retained-rule-control",
                "P03_RETAIN_RULE_CONTROL=1",
                1,
                "editor tree release",
            ),
        ]
    runs = []
    for name, fault, cycles, reason in cases:
        work = output / name
        work.mkdir(mode=0o700)
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
            'mount --make-rprivate / && mount --bind "$1" /mnt && ip link set lo up && exec dbus-run-session -- env -u LD_LIBRARY_PATH LD_PRELOAD="$2" QT_FORCE_STDERR_LOGGING=1 QT_QPA_PLATFORM=offscreen QT_QPA_PLATFORMTHEME=kde QT_QUICK_BACKEND=software P03_DATA_DIR=/mnt/host P03_TRACE=/mnt/trace.jsonl P03_CYCLES="$4" "$5" "$3" /mnt/host',
            "p03",
            str(work),
            str(library),
            str(binary),
            str(cycles),
            fault or "P03_POSITIVE=1",
        ]
        with (work / "qt.log").open("w") as log:
            process = subprocess.run(
                command, stdout=log, stderr=log, timeout=125, check=False
            )
        text = (work / "qt.log").read_text()
        assert "P03I MAIN_SHA256 " + hashes["crates/yonik-desktop/qml/Main.qml"] in text
        assert not any(
            e in text for e in ("TypeError", "ReferenceError", "Binding loop")
        )
        records = [
            json.loads(line.split("P03I ", 1)[1])
            for line in text.splitlines()
            if "P03I {" in line
        ]
        events = [
            json.loads(line) for line in (work / "trace.jsonl").read_text().splitlines()
        ]
        if reason:
            assert (
                process.returncode != 0
                and records[-1]["phase"] == "FAIL"
                and records[-1]["reason"] == reason
            ), text[-4000:]
        else:
            assert process.returncode == 0 and records[-1]["phase"] == "PASS", text[
                -4000:
            ]
            for phase in (
                "rules_ready_frame",
                "advanced_ready_frame",
                "simple_ready_frame",
                "reopen_ready_frame",
                "released",
                "closed_gate",
                "hidden_quiet",
                "reopened_gate",
            ):
                assert len([r for r in records if r["phase"] == phase]) == cycles, phase
            for r in records:
                if "dispatch_to_frame_ms" in r:
                    assert r["dispatch_to_frame_ms"] >= r["dispatch_to_ready_ms"] >= 0
            admitted = [e["request"] for e in events if e["kind"] == "admitted"]
            assert len([e for e in admitted if e["command"] == "edit_routing"]) == 2
            visibility = [
                e["visible"]
                for e in admitted
                if e["command"] == "subscription_visibility"
            ]
            assert visibility == [True] + [
                v for _ in range(cycles) for v in (False, True)
            ]
            gates = [
                e["visible"]
                for e in events
                if e["kind"] == "schedule_gate" and e["loaded"]
            ]
            transitions = [
                value
                for i, value in enumerate(gates)
                if i == 0 or gates[i - 1] != value
            ]
            assert transitions in (
                [False, True] + [v for _ in range(cycles) for v in (False, True)],
                [True] + [v for _ in range(cycles) for v in (False, True)],
            ), transitions
        runs.append(
            {
                "name": name,
                "exit": process.returncode,
                "records": records,
                "trace": events,
            }
        )
        (output / "results.json").write_text(
            json.dumps(
                {
                    "stage": stage,
                    "source_sha256": hashes,
                    "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                    "runs": runs,
                },
                indent=2,
            )
            + "\n"
        )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument(
        "--stage", choices=["preflight", "model-preflight", "campaign"], required=True
    )
    args = parser.parse_args()
    run(args.binary.resolve(), args.output.resolve(), args.stage)


if __name__ == "__main__":
    main()
