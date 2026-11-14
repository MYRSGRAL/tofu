"""Единственная fixture-only сборка exact frozen142 API под task flock и Rust slot."""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

import release_controller as c

BASE = Path(__file__).resolve().parents[1]
OUT = BASE / "docs/evidence/a4-userdata-142"
SAVED = BASE / "local/validation/a4-userdata-142"
SOURCE = "14229843f9a11bf98ec4f5992df00f3247102ea2"


def main() -> None:
    global OUT
    parser = argparse.ArgumentParser()
    parser.add_argument("--recovery", action="store_true")
    args = parser.parse_args()
    if args.recovery:
        assert (OUT / "recovery-preflight.json").exists()
        assert (
            json.loads((OUT / "witness-build-binding.json").read_text())["exit"] == -15
        )
        OUT = OUT / "witness-attempt2"
        OUT.mkdir(exist_ok=False)
    marker = OUT / "witness-build-attempt.json"
    # Нельзя незаметно превратить один разрешённый build в серию попыток.
    with marker.open("x") as stream:
        json.dump(
            {
                "source": SOURCE,
                "build_attempt_index": 2 if args.recovery else 1,
                "maximum_authorized_attempts": 2 if args.recovery else 1,
            },
            stream,
        )
    task = Path(
        tempfile.mkdtemp(prefix="a4-witness-142-", dir="/var/tmp/codex-myrs/transient")
    )
    fd = os.open(task, os.O_RDONLY | os.O_DIRECTORY)
    fcntl.flock(fd, fcntl.LOCK_EX)
    root = task / "source"
    target = task / "target"
    env = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith(("LD_", "CARGO_", "RUST", "YONIK_"))
    }
    env.update(
        PATH=str(c.RUST_BIN) + ":/usr/bin",
        RUSTUP_TOOLCHAIN=c.RUST_PROFILE,
        RUSTC=str(c.RUST_BIN / "rustc"),
        CARGO_TARGET_DIR=str(target),
        CARGO_BUILD_JOBS="1",
        RUSTC_WRAPPER="",
        RUSTC_WORKSPACE_WRAPPER="",
    )
    registry = (
        Path("/home/myrs/projects/yonik/local/release-controller")
        / c.COLLECTED_REUSE_TARGET
    )
    record = json.loads((registry / "frozen.json").read_text())
    assert record["source"]["commit"] == SOURCE
    try:
        c.snapshot(c.canonical_repo(), record, registry / "source.tar", root)
        fixture = BASE / "tools/fixtures/a4_prefix_witness.rs"
        example = root / "crates/yonik-linux/examples/a4_prefix_witness.rs"
        example.parent.mkdir(exist_ok=True)
        shutil.copyfile(fixture, example)
        command = [
            c.SLOT,
            str(c.RUST_BIN / "cargo"),
            "--config",
            'build.rustc-wrapper=""',
            "--config",
            'build.rustc-workspace-wrapper=""',
            "build",
            "-j",
            "1",
            "--locked",
            "--offline",
            "--release",
            "-p",
            "yonik-linux",
            "--example",
            "a4_prefix_witness",
        ]
        (OUT / "effective-env-before-exec.json").write_text(
            json.dumps(
                {
                    key: env[key]
                    for key in (
                        "PATH",
                        "RUSTUP_TOOLCHAIN",
                        "RUSTC",
                        "CARGO_TARGET_DIR",
                        "CARGO_BUILD_JOBS",
                        "RUSTC_WRAPPER",
                        "RUSTC_WORKSPACE_WRAPPER",
                    )
                },
                indent=2,
            )
            + "\n"
        )
        with (OUT / "witness-build.log").open("w") as log:
            process = subprocess.Popen(
                command, cwd=root, env=env, stdout=log, stderr=log
            )
            code = process.wait()
        binding = {
            "source": record["source"],
            "archive_sha256": c.sha(registry / "source.tar"),
            "cargo_lock_sha256": c.sha(root / "Cargo.lock"),
            "fixture_sha256": c.sha(fixture),
            "rustc_sha256": c.sha(c.RUST_BIN / "rustc"),
            "cargo_sha256": c.sha(c.RUST_BIN / "cargo"),
            "rust_profile": c.RUST_PROFILE,
            "argv": command,
            "cwd": str(root),
            "pid": process.pid,
            "exit": code,
            "env": {
                key: env[key]
                for key in (
                    "PATH",
                    "RUSTUP_TOOLCHAIN",
                    "RUSTC",
                    "CARGO_TARGET_DIR",
                    "CARGO_BUILD_JOBS",
                    "RUSTC_WRAPPER",
                    "RUSTC_WORKSPACE_WRAPPER",
                )
            },
        }
        if code == 0:
            binary = target / "release/examples/a4_prefix_witness"
            saved = SAVED / "a4_prefix_witness"
            shutil.copyfile(binary, saved)
            saved.chmod(0o755)
            binding["elf_sha256"] = c.sha(saved)
            c.elf_file(saved)
        (OUT / "witness-build-binding.json").write_text(
            json.dumps(binding, indent=2) + "\n"
        )
        if code:
            raise RuntimeError("Единственная fixture witness build отказала; см. log")
    finally:
        if target.exists():
            with (OUT / "witness-cargo-clean.log").open("w") as log:
                result = subprocess.run(
                    [c.SLOT, "cargo", "clean", "--target-dir", str(target)],
                    cwd=root,
                    env=env,
                    stdout=log,
                    stderr=log,
                )
                if result.returncode:
                    raise RuntimeError(
                        "cargo clean отказал; task сохранён " + str(task)
                    )
        shutil.rmtree(task)
        (OUT / "witness-cleanup.json").write_text(
            json.dumps(
                {"task": str(task), "exists": task.exists(), "children_waited": True}
            )
            + "\n"
        )
        os.close(fd)


if __name__ == "__main__":
    main()
