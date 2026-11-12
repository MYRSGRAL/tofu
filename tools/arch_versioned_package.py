"""Versioned lifecycle только внутри одноразовой Arch VM, без сборки и keyring."""

from __future__ import annotations

import hashlib
import json
import sqlite3
import subprocess
from pathlib import Path

HOME = Path("/home/builder/versioned")
DATA = HOME / "data"
OUT = Path("/home/builder/versioned-evidence")
BINARIES = (
    "usr/bin/yonik-desktop",
    "usr/lib/yonik/yonik-helper",
    "usr/lib/yonik/yonik-worker",
)


def command(args: list[str]) -> str:
    result = subprocess.run(
        args, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT
    )
    with (OUT / "transactions.log").open("a") as log:
        log.write(f"{args!r}\n{result.stdout}\nstatus={result.returncode}\n")
    result.check_returncode()
    return result.stdout


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inactive() -> None:
    result = subprocess.run(
        ["systemctl", "is-active", "yonik-helper.service"],
        capture_output=True,
        text=True,
    )
    assert (result.returncode, result.stdout.strip()) == (3, "inactive")


def snapshot(label: str) -> dict:
    result = {
        "files": {
            p.name: {
                "sha256": sha(p),
                "mode": oct(p.stat().st_mode & 0o777),
                "uid": p.stat().st_uid,
            }
            for p in DATA.iterdir()
            if p.is_file()
            and p.name in {"subscriptions.sqlite", "keyfile", "preferences.json"}
        },
        "elf": {
            name: sha(Path("/") / name)
            for name in BINARIES
            if (Path("/") / name).exists()
        },
        "foreign_nft": command(
            ["nft", "list", "table", "inet", "yonik_versioned_fixture"]
        ),
        "foreign_link": command(["ip", "-j", "link", "show", "yonik-versioned"]),
    }
    with sqlite3.connect(DATA / "subscriptions.sqlite") as db:
        assert db.execute("SELECT value FROM validation_sentinel").fetchall() == [
            ("preserve-own-db",)
        ]
    (OUT / f"{label}.json").write_text(json.dumps(result, indent=2))
    return result


def archive_elf(package: str) -> dict[str, str]:
    return {
        name: hashlib.sha256(
            subprocess.check_output(
                ["bsdtar", "-xOf", f"/home/builder/{package}.pkg.tar.zst", name]
            )
        ).hexdigest()
        for name in BINARIES
    }


def startup(label: str) -> None:
    if label == "old-initial":
        steps = [{"when": {"ready": True}, "command": {"command": "shutdown"}}]
    else:
        # Новый smoke process имеет другой ephemeral key. Проверяем наблюдаемый
        # fail-closed на прежней БД, без fresh-config обхода и заявления миграции.
        steps = [
            {"when": {"error": "storage", "ready": False}, "ui": {"action": "close"}}
        ]
    (HOME / "steps.json").write_text(json.dumps({"steps": steps}))
    trace = OUT / f"{label}.jsonl"
    result = subprocess.run(
        [
            "runuser",
            "-u",
            "builder",
            "--",
            "env",
            f"HOME={HOME}",
            f"XDG_RUNTIME_DIR={HOME / 'runtime'}",
            "QT_QPA_PLATFORM=offscreen",
            "QT_QUICK_BACKEND=software",
            "QT_QUICK_CONTROLS_STYLE=Basic",
            "/usr/bin/yonik-desktop",
            "--smoke-test",
            "--data-dir",
            str(DATA),
            "--smoke-script",
            str(HOME / "steps.json"),
            "--smoke-trace",
            str(trace),
        ],
        capture_output=True,
        text=True,
        timeout=60,
    )
    (OUT / f"{label}.log").write_text(result.stdout + result.stderr)
    (OUT / f"{label}-status.json").write_text(json.dumps({"exit": result.returncode}))
    result.check_returncode()
    events = [json.loads(line) for line in trace.read_text().splitlines()]
    assert events[-1].get("closed"), "Нет штатного завершения smoke"
    if label == "old-initial":
        assert any(event.get("ready") for event in events), "Нет ready snapshot"
    else:
        assert any(
            event.get("error") == "storage" and not event.get("ready")
            for event in events
        )
        (OUT / f"{label}-constraint.json").write_text(
            json.dumps(
                {
                    "startup": "fail_closed_storage",
                    "cause": "smoke per-process ephemeral key differs",
                    "preferences_parse": "not reached; preservation only",
                    "same_data_directory": str(DATA),
                }
            )
        )


def main() -> None:
    OUT.mkdir()
    command(
        [
            "install",
            "-d",
            "-m700",
            "-o",
            "builder",
            "-g",
            "builder",
            str(HOME),
            str(DATA),
            str(HOME / "runtime"),
        ]
    )
    command(["chown", "builder:builder", str(OUT)])
    steps = HOME / "steps.json"
    steps.write_text(
        json.dumps(
            {
                "steps": [
                    {
                        "when": {"ready": True},
                        "ui": {"action": "resize", "width": 420, "height": 760},
                    }
                ]
            }
        )
    )
    command(["nft", "add", "table", "inet", "yonik_versioned_fixture"])
    command(["ip", "link", "add", "yonik-versioned", "type", "dummy"])
    try:
        command(["pacman", "-U", "--noconfirm", "/home/builder/old.pkg.tar.zst"])
        inactive()
        assert command(["pacman", "-Q", "yonik"]).strip() == "yonik 0.1.0.r20261110-2"
        command(["pacman", "-Qkk", "yonik"])
        startup("old-initial")
        inactive()
        # Приложение создало настоящую БД; контрольная строка не затрагивает его таблицы.
        with sqlite3.connect(DATA / "subscriptions.sqlite") as db:
            db.execute("CREATE TABLE validation_sentinel (value TEXT NOT NULL)")
            db.execute("INSERT INTO validation_sentinel VALUES ('preserve-own-db')")
        key = DATA / "keyfile"
        key.write_bytes(b"fixture-only-key-material\n")
        key.chmod(0o600)
        prefs = DATA / "preferences.json"
        prefs.write_text(
            json.dumps(
                {
                    "version": 1,
                    "engine": "xray",
                    "port": None,
                    "probe_url": "https://example.com/",
                    "probe_status": 204,
                    "language": "ru",
                    "proxy_fetch_dns": None,
                    "subscription_interval_seconds": 86400,
                    "subscription_background": False,
                }
            )
        )
        prefs.chmod(0o600)
        command(["chown", "-R", "builder:builder", str(DATA)])
        before = snapshot("before-upgrade")
        assert before["elf"] == archive_elf("old")
        original = before["files"]
        for label, package, version in (
            ("new", "new", "0.1.0.r20261111-1"),
            ("rollback", "old", "0.1.0.r20261110-2"),
        ):
            command(
                ["pacman", "-U", "--noconfirm", f"/home/builder/{package}.pkg.tar.zst"]
            )
            inactive()
            assert command(["pacman", "-Q", "yonik"]).strip() == f"yonik {version}"
            command(["pacman", "-Qkk", "yonik"])
            installed = snapshot(f"{label}-installed")
            assert installed["files"] == original
            assert installed["foreign_nft"] == before["foreign_nft"]
            assert installed["foreign_link"] == before["foreign_link"]
            assert installed["elf"] == archive_elf(package)
            startup(label)
            inactive()
            after = snapshot(f"{label}-after-startup")
            assert after["files"] == original
            assert after["foreign_nft"] == before["foreign_nft"]
            assert after["foreign_link"] == before["foreign_link"]
        command(["pacman", "-R", "--noconfirm", "yonik"])
        assert all(not (Path("/") / name).exists() for name in BINARIES)
        removed = snapshot("uninstalled")
        assert removed["files"] == original
        assert removed["foreign_nft"] == before["foreign_nft"]
        assert removed["foreign_link"] == before["foreign_link"]
        (OUT / "result.json").write_text(
            json.dumps(
                {
                    "old_new_old_uninstall": "PASS",
                    "later_startup": "observed fail_closed_storage; ephemeral key changes per process",
                    "future_preferences_parse": "not reached; byte preservation checked",
                    "encrypted_data_migration": "not certified",
                    "same_data_directory": str(DATA),
                    "keyring": "not used; smoke ephemeral key",
                    "rust_build": False,
                }
            )
        )
    finally:
        command(["ip", "link", "del", "yonik-versioned"])
        command(["nft", "delete", "table", "inet", "yonik_versioned_fixture"])


if __name__ == "__main__":
    main()
