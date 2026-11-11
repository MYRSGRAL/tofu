"""Один guest-only эксперимент historical Git/makepkg; без checkout/network/source hooks."""

from __future__ import annotations

import hashlib
import json
import subprocess
from pathlib import Path


def sha(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main() -> None:
    root = Path("/opt/qt-experiment")
    output = root / "result"
    output.mkdir()
    (root / "empty-home").mkdir()
    environment = {
        "PATH": "/usr/bin:/bin",
        "HOME": str(root / "empty-home"),
        "XDG_CONFIG_HOME": str(root / "empty-home"),
        "LC_ALL": "C",
        "GIT_CONFIG_SYSTEM": "/dev/null",
        "GIT_CONFIG_GLOBAL": "/dev/null",
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_ATTR_NOSYSTEM": "1",
        "GIT_CONFIG_COUNT": "0",
        "GIT_TERMINAL_PROMPT": "0",
        "GIT_NO_LAZY_FETCH": "1",
        "MAKEPKG_GIT_CONFIG": "/dev/null",
    }
    records = []
    versions = {"qt6-base": "4", "qt6-declarative": "3", "qt6-svg": "2"}
    repos = {
        "qt6-base": "qtbase",
        "qt6-declarative": "qtdeclarative",
        "qt6-svg": "qtsvg",
    }
    for row in json.loads((root / "qt-sources.json").read_text()):
        repo = root / "objects" / repos[row["pkgbase"]]
        version = "7.1.0.r9.g54d9411-" + versions[row["pkgbase"]]
        tools = root / "tools" / ("pacman-" + version + "-x86_64")
        library = tools / "usr/share/makepkg"
        record = {
            "pkgbase": row["pkgbase"],
            "expected_commit": row["commit"],
            "expected_source_checksum": row["arch_source_checksum"],
            "previous_host_archive_sha256": row["git_archive_sha256"],
            "historical_pacman_version": version,
            "environment": environment,
            "git_binary_sha256": sha(Path("/usr/bin/git")),
            "makepkg_script_sha256": sha(tools / "usr/bin/makepkg"),
            "calc_library_sha256": sha(library / "source/git.sh"),
        }
        if not repo.exists():
            record.update(
                {
                    "status": "unavailable_objects",
                    "detail": "pinned repository не доставлен",
                }
            )
            records.append(record)
            continue
        actual = subprocess.check_output(
            ["/usr/bin/git", "-C", str(repo), "rev-parse", row["tag"] + "^{}"],
            env=environment,
            text=True,
        ).strip()
        if actual != row["commit"]:
            raise ValueError("pinned commit mismatch")
        subprocess.run(
            [
                "/usr/bin/git",
                "-C",
                str(repo),
                "config",
                "core.attributesFile",
                "/dev/null",
            ],
            env=environment,
            check=True,
        )
        subprocess.run(
            ["/usr/bin/git", "-C", str(repo), "config", "core.hooksPath", "/dev/null"],
            env=environment,
            check=True,
        )
        missing = subprocess.check_output(
            [
                "/usr/bin/git",
                "-C",
                str(repo),
                "rev-list",
                "--objects",
                "--missing=print",
                row["commit"],
            ],
            env=environment,
            text=True,
        )
        absent = [line[1:] for line in missing.splitlines() if line.startswith("?")]
        if absent:
            record.update({"status": "unavailable_objects", "missing_objects": absent})
            records.append(record)
            continue
        # Адаптеры задают только три входа исходной makepkg checksum-функции.
        script = """set -eo pipefail
source "$MAKEPKG_LIBRARY/source/git.sh"
get_filepath() { printf '%s\\n' "$QT_REPO"; }
get_url() { printf '%s\\n' "git+https://code.qt.io/qt/$QT_NAME.git#tag=v6.12.0"; }
get_uri_fragment() { printf '%s\\n' 'tag=v6.12.0'; }
calc_checksum_git "$QT_NAME" sha256
"""
        run_env = {
            **environment,
            "MAKEPKG_LIBRARY": str(library),
            "QT_REPO": str(repo),
            "QT_NAME": repos[row["pkgbase"]],
        }
        command = [
            "unshare",
            "-n",
            "/usr/bin/env",
            "-i",
            *[key + "=" + value for key, value in run_env.items()],
            "/usr/bin/bash",
            "-c",
            script,
        ]
        result = subprocess.run(command, capture_output=True, text=True, timeout=120)
        record.update(
            {
                "actual_commit": actual,
                "command": command,
                "exit": result.returncode,
                "stdout": result.stdout,
                "stderr": result.stderr,
                "status": "observed" if result.returncode == 0 else "failed",
            }
        )
        if result.returncode == 0:
            actual_hash = result.stdout.strip()
            record.update(
                {
                    "archive_sha256": actual_hash,
                    "matches_arch_checksum": actual_hash == row["arch_source_checksum"],
                    "matches_previous_host": actual_hash == row["git_archive_sha256"],
                }
            )
        records.append(record)
    info = {
        "schema": 1,
        "git_version": subprocess.check_output(
            ["/usr/bin/git", "--version", "--build-options"], env=environment, text=True
        ),
        "installed_git_package": subprocess.check_output(
            ["pacman", "-Q", "git"], text=True
        ),
        "records": records,
        "limits": "Только один набор checksum runs. Не реконструкция всего Qt build environment; не Rust build.",
    }
    (output / "qt-historical-experiment.json").write_text(
        json.dumps(info, ensure_ascii=False, indent=2) + "\n"
    )


if __name__ == "__main__":
    main()
