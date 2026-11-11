"""Чтение production encrypted profile между реальными Arch package versions."""

from __future__ import annotations

import csv
import hashlib
import io
import json
import os
import select
import shutil
import signal
import sqlite3
import subprocess
import time
from pathlib import Path

HOME = Path("/home/builder/production")
OUT = Path("/home/builder/production-evidence")
DB = HOME / "storage/subscriptions.sqlite"
BINARIES = (
    "usr/bin/yonik-desktop",
    "usr/lib/yonik/yonik-helper",
    "usr/lib/yonik/yonik-worker",
)


def call(args: list[str]) -> str:
    process = subprocess.run(
        args, env=os.environ, text=True, capture_output=True, timeout=45
    )
    with (OUT / "route-commands.log").open("a") as log:
        # В argv нет key/keyring password; URL содержит только fixture.
        log.write(
            f"{args!r}\n{process.stdout}{process.stderr}status={process.returncode}\n"
        )
    process.check_returncode()
    return process.stdout


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def capture(label: str) -> str:
    path = OUT / f"{label}.png"
    call(["import", "-display", ":77", "-window", "root", str(path)])
    text = call(["tesseract", str(path), "stdout", "-l", "eng+rus"])
    (OUT / f"{label}.txt").write_text(text)
    return text


def write_refused(text: str) -> bool:
    normalized = "".join(
        c for c in text.casefold().replace("he ", "не ") if c.isalnum()
    )
    return "preferences.json" in text and "неудалосьсохранить" in normalized


def click(x: int, y: int) -> None:
    call(["xdotool", "mousemove", str(x), str(y), "click", "1"])
    time.sleep(0.4)


def click_text(text: str, label: str) -> None:
    capture(label)
    tsv = call(
        ["tesseract", str(OUT / f"{label}.png"), "stdout", "-l", "eng+rus", "tsv"]
    )
    rows = list(csv.DictReader(io.StringIO(tsv), delimiter="\t"))
    matches = [r for r in rows if text.casefold() in r["text"].casefold()]
    assert len(matches) == 1, (text, [r["text"] for r in matches])
    r = matches[0]
    click(int(r["left"]) + int(r["width"]) // 2, int(r["top"]) + int(r["height"]) // 2)


def inactive(label: str) -> None:
    p = subprocess.run(
        ["systemctl", "is-active", "yonik-helper.service"],
        capture_output=True,
        text=True,
    )
    (OUT / f"{label}-inactive.json").write_text(
        json.dumps({"status": p.returncode, "state": p.stdout.strip()})
    )
    assert (p.returncode, p.stdout.strip()) == (3, "inactive")


def secret_state(label: str) -> dict:
    prefix = [
        "busctl",
        "--address=" + os.environ["DBUS_SESSION_BUS_ADDRESS"],
        "--json=short",
        "call",
    ]
    owner = json.loads(
        call(
            prefix
            + [
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "GetNameOwner",
                "s",
                "org.freedesktop.secrets",
            ]
        )
    )["data"][0]
    pid = json.loads(
        call(
            prefix
            + [
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "GetConnectionUnixProcessID",
                "s",
                "org.freedesktop.secrets",
            ]
        )
    )["data"][0]
    assert Path(f"/proc/{pid}").stat().st_uid == os.getuid()
    items = json.loads(
        call(
            prefix
            + [
                "org.freedesktop.secrets",
                "/org/freedesktop/secrets",
                "org.freedesktop.Secret.Service",
                "SearchItems",
                "a{ss}",
                "3",
                "application",
                "cloud.highpriority.tofu",
                "purpose",
                "subscription-master-key",
                "schema",
                "1",
            ]
        )
    )["data"]
    assert len(items[0]) == 1 and items[1] == []
    result = {
        "owner": owner,
        "pid": pid,
        "unlocked_items": items[0],
        "locked_items": items[1],
        "get_secret_called_by_fixture": False,
    }
    (OUT / f"{label}-secret-service.json").write_text(json.dumps(result))
    return result


def snapshot(label: str) -> dict:
    # mode=ro и contextlib closing не оставляют root-owned WAL/shm файлов.
    db = sqlite3.connect(f"file:{DB}?mode=ro", uri=True)
    try:
        rows = db.execute("SELECT id,payload FROM profiles").fetchall()
        assert len(rows) == 1 and len(rows[0][1]) > 40
        payload = rows[0][1]
        assert b"P24StableProfile" not in payload and b"p24-fixture" not in payload
        profiles = {
            "count": len(rows),
            "id": rows[0][0],
            "payload_sha256": hashlib.sha256(payload).hexdigest(),
            "ciphertext_bytes": len(payload),
        }
    finally:
        db.close()
    files = {}
    for p in [
        DB,
        HOME / "storage/preferences.json",
        HOME / "preserve-keyfile",
        *(HOME / "xdg-data/keyrings").glob("*"),
    ]:
        if p.is_file():
            files[str(p.relative_to(HOME))] = {
                "sha256": digest(p),
                "mode": oct(p.stat().st_mode & 0o777),
                "uid": p.stat().st_uid,
            }
    result = {
        "files": files,
        "profiles": profiles,
        "elf": {
            name: digest(Path("/") / name)
            for name in BINARIES
            if (Path("/") / name).exists()
        },
    }
    shutil.copyfile(DB, OUT / f"{label}-encrypted.sqlite")
    (OUT / f"{label}-snapshot.json").write_text(json.dumps(result, indent=2))
    return result


def close_existing() -> None:
    ids = call(["xdotool", "search", "--onlyvisible", "--name", "^yonik"]).splitlines()
    assert len(ids) == 1, ids
    pid = int(call(["xdotool", "getwindowpid", ids[0]]).strip())
    fd = os.pidfd_open(pid)
    try:
        call(["xdotool", "key", "Escape"])
        call(["xdotool", "mousemove", "466", "16", "click", "3"])
        time.sleep(0.3)
        capture(f"quit-menu-{pid}")
        click(530, 119)
        assert select.select([fd], [], [], 15)[0], "GUI не завершился"
    finally:
        os.close(fd)


def open_app(label: str) -> subprocess.Popen:
    inactive(label + "-before-start")
    with (OUT / f"{label}.log").open("w") as log:
        app = subprocess.Popen(
            ["/usr/bin/yonik-desktop", "--data-dir", str(HOME / "storage")],
            env=os.environ,
            stdout=log,
            stderr=log,
        )
    time.sleep(3)
    assert app.poll() is None
    (OUT / f"{label}-process.json").write_text(
        json.dumps(
            {
                "pid": app.pid,
                "argv": ["/usr/bin/yonik-desktop", "--data-dir", str(HOME / "storage")],
                "smoke_test": False,
                "ephemeral_key": False,
            }
        )
    )
    return app


def read_profile(label: str) -> None:
    text = capture(label + "-home")
    assert "Disconnected" in text or "Отключено" in text
    click(210, 165)
    text = capture(label + "-profile")
    assert "P24StableProfile" in text and "profileexampleinvalid" in "".join(
        c for c in text.lower() if c.isalnum()
    )
    inactive(label + "-after-read")


def archive_elf(name: str) -> dict:
    return {
        p: hashlib.sha256(
            subprocess.check_output(
                ["bsdtar", "-xOf", f"/home/builder/{name}.pkg.tar.zst", p]
            )
        ).hexdigest()
        for p in BINARIES
    }


def install(name: str, version: str) -> None:
    call(
        [
            "sudo",
            "-n",
            "/usr/bin/pacman",
            "-U",
            "--noconfirm",
            f"/home/builder/{name}.pkg.tar.zst",
        ]
    )
    assert call(["pacman", "-Q", "yonik"]).strip() == f"yonik {version}"
    call(["pacman", "-Qkk", "yonik"])
    inactive(name + "-installed")
    assert {p: digest(Path("/") / p) for p in BINARIES} == archive_elf(name)


def seed_profile() -> None:
    db = sqlite3.connect(f"file:{DB}?mode=ro", uri=True)
    try:
        exists = db.execute("SELECT COUNT(*) FROM profiles").fetchone()[0]
    finally:
        db.close()
    if exists:
        # Сохраняет actual seed этап, выполненный при осмотре собственного GUI.
        return
    click(210, 165)
    click(270, 114)
    call(
        [
            "xdotool",
            "type",
            "--clearmodifiers",
            "--",
            (OUT / "source-url.txt").read_text(),
        ]
    )
    click(105, 422)
    click(55, 465)
    click(334, 580)
    time.sleep(2)
    assert "P24StableProfile" in capture("old-created-profile")
    click(370, 50)
    click(210, 430)
    click(280, 198)
    call(["xdotool", "key", "End", "Return"])
    time.sleep(0.5)
    for _ in range(3):
        click(50, 50)


def main() -> None:
    assert os.getuid() != 0 and Path.home() == HOME
    assert Path("/sys/class/dmi/id/sys_vendor").read_text().strip() == "QEMU"
    seed_profile()
    prefs = HOME / "storage/preferences.json"
    old_prefs = json.loads(prefs.read_text())
    assert old_prefs["language"] == "en" and "proxy_fetch_dns" not in old_prefs
    read_profile("old-reopened-before-upgrade")
    capture("old-produced-prefs")
    old_secret = secret_state("old-created")
    close_existing()
    preserve = HOME / "preserve-keyfile"
    preserve.write_bytes(b"fixture-only-own-keyfile\n")
    preserve.chmod(0o600)
    baseline = snapshot("old-closed")
    assert baseline["elf"] == archive_elf("old")
    fd = os.pidfd_open(old_secret["pid"])
    os.kill(old_secret["pid"], signal.SIGTERM)
    assert select.select([fd], [], [], 10)[0]
    os.close(fd)
    with (HOME / "password").open("rb") as password:
        restarted = subprocess.Popen(
            [
                "gnome-keyring-daemon",
                "--foreground",
                "--unlock",
                "--components=secrets",
                "--control-directory",
                str(HOME / "runtime/control"),
            ],
            env=os.environ,
            stdin=password,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    time.sleep(2)
    after_restart = secret_state("persisted-provider-restart")
    assert after_restart["pid"] == restarted.pid
    assert after_restart["owner"] != old_secret["owner"]
    assert after_restart["unlocked_items"] == old_secret["unlocked_items"]
    # Provider остаётся на private bus до завершения owning session fixture.
    (OUT / "restarted-provider.json").write_text(json.dumps({"pid": restarted.pid}))
    install("new", "0.1.0.r20261111-1")
    assert snapshot("new-installed")["files"] == baseline["files"]
    app = open_app("new")
    try:
        read_profile("new")
        assert (
            secret_state("new-reopened")["unlocked_items"]
            == old_secret["unlocked_items"]
        )
        assert json.loads(prefs.read_text()) == old_prefs
        click(370, 50)
        click_text("Appearance", "new-settings")
        click(280, 198)
        # Два реальных изменения языка вызывают production serializer нового ELF.
        call(["xdotool", "key", "Home", "Return"])
        time.sleep(0.5)
        click(280, 198)
        call(["xdotool", "key", "End", "Return"])
        time.sleep(0.5)
        new_prefs = json.loads(prefs.read_text())
        assert (
            new_prefs["language"] == "en"
            and "proxy_fetch_dns" in new_prefs
            and "subscription_interval_seconds" in new_prefs
        )
        capture("new-produced-future-prefs")
        close_existing()
        assert app.wait(timeout=5) == 0
    finally:
        if app.poll() is None:
            app.terminate()
            app.wait(timeout=15)
    before_rollback = snapshot("new-closed")
    assert before_rollback["profiles"] == baseline["profiles"]
    assert (
        before_rollback["files"]["storage/subscriptions.sqlite"]
        == baseline["files"]["storage/subscriptions.sqlite"]
    )
    assert (
        before_rollback["files"]["preserve-keyfile"]
        == baseline["files"]["preserve-keyfile"]
    )
    install("old", "0.1.0.r20261110-2")
    assert snapshot("rollback-installed")["files"] == before_rollback["files"]
    app = open_app("rollback")
    try:
        read_profile("rollback")
        assert (
            secret_state("rollback-reopened")["unlocked_items"]
            == old_secret["unlocked_items"]
        )
        click(370, 50)
        text = capture("rollback-future-warning")
        assert "preferences.json" in text and (
            "заблокирована" in text or "blocked" in text
        )
        click_text("Интерфейс", "rollback-settings")
        before = prefs.read_bytes()
        click_text("Русский", "rollback-language")
        call(["xdotool", "key", "End", "Return"])
        time.sleep(0.5)
        assert prefs.read_bytes() == before
        text = capture("rollback-write-refused")
        assert write_refused(text)
        close_existing()
        assert app.wait(timeout=5) == 0
    finally:
        if app.poll() is None:
            app.terminate()
            app.wait(timeout=15)
    closed = snapshot("rollback-closed")
    assert closed["profiles"] == baseline["profiles"]
    assert closed["files"] == before_rollback["files"]
    assert closed["elf"] == baseline["elf"]
    call(["sudo", "-n", "/usr/bin/pacman", "-R", "--noconfirm", "yonik"])
    removed = snapshot("uninstalled")
    assert (
        removed["files"] == closed["files"]
        and removed["profiles"] == baseline["profiles"]
        and removed["elf"] == {}
    )
    assert (
        secret_state("after-uninstall")["unlocked_items"]
        == old_secret["unlocked_items"]
    )
    (OUT / "result.json").write_text(
        json.dumps(
            {
                "status": "PASS",
                "profile_old_new_old": "actual production GUI decrypted profile",
                "stable_secret_service_item": True,
                "ephemeral_key": False,
                "old_preferences_read_by_new": True,
                "future_preferences_produced_by_new": True,
                "rollback_future_preferences": "unsupported; warning + write rejected; bytes unchanged",
                "uninstall_preserved": True,
                "rust_build": False,
            },
            indent=2,
        )
    )
    restarted.terminate()
    restarted.wait(timeout=10)


if __name__ == "__main__":
    main()
