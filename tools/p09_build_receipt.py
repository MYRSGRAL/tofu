"""Fail-closed receipt фактической P09 сборки; не backend probe и не launcher."""

from __future__ import annotations

import argparse
import hashlib
import json
import shlex
import shutil
from pathlib import Path

TARGET = "x86_64-unknown-linux-gnu"
FORBIDDEN_CFG = {"libc", "miri", "rustix_use_libc", "rustix_no_linux_raw"}
FORBIDDEN_FEATURES = {"use-libc", "rustc-dep-of-std"}


def require(condition: object, message: object) -> None:
    if not condition:
        raise AssertionError(str(message))


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def values(argv: list[str], option: str) -> list[str]:
    return [
        argv[i + 1] if word == option else word[len(option) + 1 :]
        for i, word in enumerate(argv)
        if (word == option and i + 1 < len(argv)) or word.startswith(option + "=")
    ]


def check_raw(argv: list[str]) -> dict:
    cfg = set(values(argv, "--cfg"))
    features = {json.loads(v[8:]) for v in cfg if v.startswith("feature=")}
    require(values(argv, "--target") == [TARGET], "не exact target")
    require({"linux_raw", "linux_raw_dep"} <= cfg, "нет raw backend")
    require(not FORBIDDEN_CFG & cfg, "unsupported backend cfg")
    require(not FORBIDDEN_FEATURES & features, "unsupported rustix features")
    require({"fs", "std", "process"} <= features, "неполный process feature closure")
    return dict(cfg=sorted(cfg), features=sorted(features))


def verify(evidence: Path) -> dict:
    freeze = json.loads((evidence / "freeze.json").read_text())
    revision = json.loads((evidence / "verifier-revision.json").read_text())
    own_path = str(Path(__file__).resolve())
    require(revision["path"] == own_path, "receipt validation failed")
    require(
        revision["before_sha256"] == freeze["sources"][own_path],
        "receipt validation failed",
    )
    require(
        revision["after_sha256"] == digest(Path(own_path)), "receipt validation failed"
    )
    for path, expected in freeze["sources"].items():
        require(
            digest(Path(path))
            == (revision["after_sha256"] if path == own_path else expected),
            path,
        )
    require(freeze["profile"]["Seccomp"] == "0", "unsupported launch profile")
    require(freeze["rustc_version"].startswith("rustc 1.85.1 "), "не MSRV toolchain")
    require(
        all(value == "" for value in freeze["build_env"].values()), "build overrides"
    )
    events = []
    commands = []
    for stem in ("example", "tests"):
        rows = []
        for line in (evidence / f"{stem}.jsonl").read_text().splitlines():
            if line.startswith("{"):
                rows.append(json.loads(line))
            else:
                # Cargo -vv печатает stdout build scripts рядом с JSON.
                require(line.startswith("["), "неизвестный stdout format")
        require(
            rows[-1] == {"reason": "build-finished", "success": True},
            "receipt validation failed",
        )
        events.extend(rows)
        for line in (evidence / f"{stem}.stderr").read_text().splitlines():
            if "Running `" in line and line.endswith("`"):
                argv = shlex.split(line.split("Running `", 1)[1][:-1])
                if "--crate-name" in argv and values(argv, "--target") == [TARGET]:
                    compiler = next(word for word in argv if "=" not in word)
                    require(
                        Path(compiler).resolve()
                        == Path(freeze["rustc_path"]).resolve(),
                        "receipt validation failed",
                    )
                    require(
                        digest(Path(compiler)) == freeze["rustc_sha256"],
                        "receipt validation failed",
                    )
                    commands.append(argv)
    artifacts = [e for e in events if e.get("reason") == "compiler-artifact"]
    records: dict = {}
    chains = []
    backends: dict = {}
    pending: set[Path] = set()

    def artifact_command(path: Path) -> list[str]:
        matches = [
            a
            for a in commands
            if values(a, "--out-dir") == [str(path.parent)]
            and any(
                path.stem.endswith(value.split("=", 1)[1])
                for value in values(a, "-C")
                if value.startswith("extra-filename=")
            )
        ]
        require(len(matches) == 1, f"ambiguous artifact invocation {path}")
        require(
            any(str(path) in a["filenames"] for a in artifacts),
            "receipt validation failed",
        )
        require(
            "link" in values(matches[0], "--emit")[0].split(","),
            "receipt validation failed",
        )
        pending.add(path)
        return matches[0]

    def linked(argv: list[str], dependency: str) -> Path:
        externs = sorted(
            {
                v.split("=", 1)[1]
                for v in values(argv, "--extern")
                if v.startswith(dependency + "=")
            }
        )
        require(len(externs) == 1, f"ambiguous extern {dependency}")
        path = Path(externs[0])
        require(path.suffix in (".rlib", ".rmeta"), "receipt validation failed")
        # Cargo1.85 передаёт rmeta для metadata, rlib того же compiler artifact
        # используется linker; проверяем оба output и exact extra-filename.
        artifact_command(path)
        rlib = path.with_suffix(".rlib")
        artifact_command(rlib)
        chains.append(
            dict(
                caller=values(argv, "--crate-name")[0],
                extern=str(path),
                linked_rlib=str(rlib),
            )
        )
        return rlib

    executables = {}
    for name, dependencies in (
        (
            "local_socks_alternating",
            ("yonik_desktop", "yonik_linux", "yonik_nsfs", "rustix"),
        ),
        ("local_parent_death", ("yonik_nsfs", "rustix")),
    ):
        candidates = [a for a in commands if values(a, "--crate-name") == [name]]
        require(len(candidates) == 1, f"ambiguous executable command {name}")
        argv = candidates[0]
        output = {
            a["executable"]
            for a in artifacts
            if a["target"]["name"] == name and a["executable"]
        }
        require(len(output) == 1, "receipt validation failed")
        executable = Path(output.pop())
        pending.add(executable)
        executables[name] = str(executable)
        for dependency in dependencies:
            rlib = linked(argv, dependency)
            argv = artifact_command(rlib)
        backend = check_raw(argv)
        package = next(
            a["package_id"] for a in artifacts if str(rlib) in a["filenames"]
        )
        require(package.endswith("#rustix@1.1.6"), "receipt validation failed")
        out_dirs = [v.split("=", 1)[1] for v in argv if v.startswith("OUT_DIR=")]
        require(len(out_dirs) == 1, "receipt validation failed")
        scripts = {
            a["out_dir"]: a
            for a in events
            if a.get("reason") == "build-script-executed" and a["package_id"] == package
        }
        require(out_dirs[0] in scripts, "receipt validation failed")
        script = scripts[out_dirs[0]]
        cfgs = set(script["cfgs"])
        require(
            {"linux_raw", "linux_raw_dep"} <= cfgs and not FORBIDDEN_CFG & cfgs,
            "receipt validation failed",
        )
        output_file = Path(script["out_dir"]).parent / "output"
        output_cfg = {
            line.split("=", 1)[1]
            for line in output_file.read_text().splitlines()
            if line.startswith(("cargo:rustc-cfg=", "cargo::rustc-cfg="))
        }
        require(cfgs == output_cfg, "receipt validation failed")
        require(
            cfgs == {c for c in values(argv, "--cfg") if not c.startswith("feature=")},
            "receipt validation failed",
        )
        backends[str(rlib)] = dict(
            backend=backend,
            argv=argv,
            script=script,
            output_sha256=digest(output_file),
            package_id=package,
        )
    require(
        len(backends)
        == len([a for a in commands if values(a, "--crate-name") == ["rustix"]]),
        "unbound rustix instance",
    )
    retained = evidence / "artifacts"
    retained.mkdir(exist_ok=True)
    for path in sorted(pending):
        destination = retained / path.name
        if destination.exists():
            require(digest(destination) == digest(path), "retained artifact collision")
        else:
            shutil.copy2(path, destination)
        require(digest(path) == digest(destination), "receipt validation failed")
        records[str(path)] = dict(retained=str(destination), sha256=digest(destination))
    return dict(
        status="VERIFIED_LINUX_RAW",
        target=TARGET,
        backend_instances=backends,
        link_chains=chains,
        artifacts=records,
        executables={name: records[path] for name, path in executables.items()},
        freeze_sha256=digest(evidence / "freeze.json"),
        verifier_sha256=digest(Path(__file__)),
        verifier_revision=revision,
        launch_policy="known Seccomp=0; no errno emulation",
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path)
    args = parser.parse_args()
    result = verify(args.evidence.resolve())
    (args.evidence / "receipt.json").write_text(json.dumps(result, indent=2))
    print(result["status"])
