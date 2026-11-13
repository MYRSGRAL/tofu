"""Сборка отдельных N4 test ELF из accepted pinned source и test overlays."""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import subprocess
import sys
from contextlib import ExitStack
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO / "tools"))
import build_native_adapter as builder  # noqa: E402


def build(root: Path, output: Path) -> None:
    """Не переиспользовать чужой checkout; удерживать lock всего task root."""
    output.mkdir(parents=True, exist_ok=True)
    if output.is_relative_to(root):
        raise ValueError("Evidence должна находиться вне transient")
    descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
    with ExitStack() as cleanup:
        cleanup.callback(os.close, descriptor)
        fcntl.flock(descriptor, fcntl.LOCK_SH)
        env = {
            key: value for key, value in os.environ.items() if not key.startswith("GO")
        }
        env.update(
            GOTOOLCHAIN="local",
            GOENV="off",
            GOWORK="off",
            CGO_ENABLED="0",
            GOMAXPROCS="1",
            GOCACHE=str(root / "go-cache"),
            GOMODCACHE=str(root / "go-mod"),
        )
        for engine, tag, pin, url in [
            ("xray", builder.XRAY_TAG, builder.XRAY_COMMIT, builder.XRAY_URL),
            ("sing-box", builder.SOURCE_TAG, builder.SOURCE_COMMIT, builder.SOURCE_URL),
        ]:
            source = root / engine
            if source.exists():
                raise ValueError(f"Нужен новый task source: {source}")
            with (output / f"{engine}-build.log").open("w") as log:

                def run(args: list[str], cwd: Path) -> None:
                    log.write(json.dumps(args) + "\n")
                    log.flush()
                    subprocess.run(
                        args,
                        cwd=cwd,
                        env=env,
                        stdout=log,
                        stderr=log,
                        timeout=900,
                        check=True,
                    )

                run(
                    ["git", "clone", "--depth=1", "--branch", tag, url, str(source)],
                    root,
                )
                head = subprocess.check_output(
                    ["git", "rev-parse", "HEAD"], cwd=source, text=True
                ).strip()
                if head != pin:
                    raise ValueError("Неверный pinned commit")
                patches = (
                    [
                        builder.DEFAULT_PATCH,
                        *[path for path, _ in builder.EXTRA_PATCHES],
                    ]
                    if engine == "sing-box"
                    else []
                ) + [REPO / "adapters" / engine / tag / "native-broker-codec.patch"]
                modules = {
                    name: builder.sha256(source / name) for name in ("go.mod", "go.sum")
                }
                metadata = json.loads(patches[-1].with_suffix(".json").read_text())
                if builder.sha256(patches[-1]) != metadata["patch_sha256"]:
                    raise ValueError("Codec patch SHA256")
                for patch in patches:
                    run(["git", "apply", str(patch)], source)
                if modules != {name: builder.sha256(source / name) for name in modules}:
                    raise ValueError("Патчи изменили go.mod/go.sum")
                for name, digest in metadata["package_files_sha256"].items():
                    if builder.sha256(source / "common/yonikbroker" / name) != digest:
                        raise ValueError(f"Package hash: {name}")
                for name, digest in metadata["callsite_files_sha256"].items():
                    if builder.sha256(source / name) != digest:
                        raise ValueError(f"Callsite hash: {name}")
                package = source / "common/yonikbroker"
                overlays = {
                    str(package / "n4_bridge_test.go"): str(
                        Path(__file__).with_name("bridge_test.go")
                    ),
                    str(package / "n4_pipeline_test.go"): str(
                        Path(__file__).with_name(f"{engine}_pipeline_test.go")
                    ),
                }
                overlay = output / f"{engine}-overlay.json"
                overlay.write_text(json.dumps({"Replace": overlays}, indent=2))
                manifest = {
                    str(path.relative_to(source)): builder.sha256(path)
                    for path in source.rglob("*")
                    if path.is_file() and ".git" not in path.relative_to(source).parts
                }
                (output / f"{engine}-source.json").write_text(
                    json.dumps(
                        {
                            "pin": pin,
                            "files": manifest,
                            "patches": {
                                str(path.relative_to(REPO)): builder.sha256(path)
                                for path in patches
                            },
                            "overlays": {
                                path: builder.sha256(Path(path))
                                for path in overlays.values()
                            },
                        },
                        indent=2,
                    )
                )
                binary = output / f"{engine}-consumer.test"
                run(
                    [
                        "go",
                        "test",
                        "-c",
                        "-p=1",
                        "-mod=readonly",
                        "-trimpath",
                        "-buildvcs=false",
                        "-overlay",
                        str(overlay),
                        "-o",
                        str(binary),
                        "./common/yonikbroker",
                    ],
                    source,
                )
                (output / f"{engine}-binary.sha256").write_text(
                    builder.sha256(binary) + "\n"
                )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    build(args.root.resolve(strict=True), args.output.resolve())


if __name__ == "__main__":
    main()
