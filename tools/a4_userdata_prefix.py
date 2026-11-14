"""Own prefix: pinned installed copies + GPG-verified runtime archives; без hooks."""

from __future__ import annotations

import hashlib
import json
import os
import posixpath
import re
import shutil
import stat
import subprocess
import tarfile
from pathlib import Path

BASE = Path(__file__).resolve().parents[1]
OUT = BASE / "docs/evidence/a4-userdata-142"
SAVED = BASE / "local/validation/a4-userdata-142"


def sha(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def own_hardlink(root: Path, source_name: str, target: Path) -> None:
    parts = source_name.split("/")
    if not source_name.startswith("usr/") or any(
        part in ("", ".", "..") for part in parts
    ):
        raise ValueError("Небезопасный runtime hardlink source")
    source = root / source_name
    if source == target:
        raise ValueError("Runtime hardlink не может заменить свой source")
    if not target.is_relative_to(root / "usr") or ".." in target.parts:
        raise ValueError("Небезопасный runtime hardlink target")
    for path in (source, target):
        if any(parent.is_symlink() for parent in path.parents):
            raise ValueError("Runtime hardlink parent symlink")
    info = source.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid():
        raise ValueError("Runtime hardlink source не own regular file")
    target.parent.mkdir(parents=True, exist_ok=True)
    if target.is_symlink():
        raise ValueError("Runtime hardlink target symlink")
    if target.exists():
        target.unlink()
    os.link(source, target, follow_symlinks=False)


def populate(root: Path, out: Path) -> None:
    old = BASE / "docs/evidence/a4-mtree4/gui-run/dependency-files.json"
    copied = {}
    signed_paths: set[str] = set()

    def copy(path: Path, expected: str | None = None) -> None:
        if path.parts[1:2] in (("lib",), ("lib64",)):
            path = Path("/usr/lib") / path.relative_to("/" + path.parts[1])
        if str(path) in signed_paths:
            return
        if str(path) in copied:
            return
        assert (
            path.is_relative_to("/usr")
            or path.is_relative_to("/etc/fonts")
            or path.is_relative_to("/etc/ImageMagick-7")
        )
        checksum = sha(path)
        if expected is not None:
            assert checksum == expected, str(path)
        dest = root / str(path).lstrip("/")
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, dest)
        assert sha(dest) == checksum
        copied[str(path)] = {
            "sha256": checksum,
            "resolved": str(path.resolve()),
            "mode": oct(path.stat().st_mode & 0o7777),
        }

    for row in json.loads(old.read_text()):
        copy(Path(row["path"]), row["sha256"])
    downloaded = []
    unselected_optional = []
    elf_paths = []
    for record in (
        "runtime-inputs.json",
        "runtime-inputs-wm.json",
        "runtime-inputs-closure.json",
        "runtime-inputs-qt.json",
        "runtime-inputs-crypto.json",
    ):
        for package in json.loads((OUT / record).read_text())["packages"]:
            archive = Path(package["path"])
            assert sha(archive) == package["sha256"]
            assert "[GNUPG:] VALIDSIG " in package["signature_status"]
            process = subprocess.Popen(
                ["zstd", "-dc", str(archive)], stdout=subprocess.PIPE
            )
            links = []
            hardlinks = []
            members = []
            try:
                with tarfile.open(fileobj=process.stdout, mode="r|") as stream:
                    for member in stream:
                        name = member.name.removeprefix("./").rstrip("/")
                        if name in (".PKGINFO", ".BUILDINFO", ".MTREE", ".INSTALL"):
                            continue
                        if not name:
                            continue
                        parts = name.split("/")
                        assert all(part not in ("", ".", "..") for part in parts)
                        assert (
                            not name.startswith("/")
                            and len(name) <= 4096
                            and member.uid == member.gid == 0
                        )
                        assert (
                            member.mode & 0o7000 == 0
                            and member.size <= 128 * 1024 * 1024
                        )
                        if name in {
                            "usr/lib/qt6/plugins/sqldrivers/libqsqlibase.so",
                            "usr/lib/qt6/plugins/sqldrivers/libqsqlmysql.so",
                            "usr/lib/qt6/plugins/sqldrivers/libqsqlodbc.so",
                            "usr/lib/qt6/plugins/sqldrivers/libqsqlpsql.so",
                        }:
                            unselected_optional.append(
                                {
                                    "package": package["name"],
                                    "path": name,
                                    "reason": "Optional Qt SQL driver outside normal GUI/Rust SQLite route; not copied or executed",
                                }
                            )
                            continue
                        signed_paths.add("/" + name)
                        target = root / name
                        assert not any(
                            p.is_symlink() for p in target.parents if p != root.parent
                        )
                        if member.isdir():
                            target.mkdir(parents=True, exist_ok=True)
                        elif member.isfile():
                            target.parent.mkdir(parents=True, exist_ok=True)
                            assert not target.is_symlink()
                            incoming = stream.extractfile(member)
                            assert incoming is not None
                            with target.open("wb") as dest:
                                shutil.copyfileobj(incoming, dest)
                            target.chmod(member.mode)
                            with target.open("rb") as check:
                                if check.read(4) == b"\x7fELF":
                                    elf_paths.append(target)
                        elif member.islnk():
                            hardlinks.append(
                                (target, member.linkname.removeprefix("./"))
                            )
                        elif member.issym():
                            resolved = posixpath.normpath(
                                posixpath.join(
                                    posixpath.dirname("/" + name), member.linkname
                                )
                            )
                            assert resolved.startswith("/usr/") or resolved.startswith(
                                "/etc/"
                            )
                            links.append((target, member.linkname))
                        else:
                            raise ValueError("Запрещённый тип runtime member")
                        members.append(
                            {
                                "path": name,
                                "mode": oct(member.mode),
                                "type": member.type.decode(),
                                "uid": member.uid,
                                "gid": member.gid,
                                "size": member.size,
                                "link": member.linkname,
                            }
                        )
                assert process.wait() == 0
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
            for path, link in hardlinks:
                own_hardlink(root, link, path)
                with path.open("rb") as check:
                    if check.read(4) == b"\x7fELF":
                        elf_paths.append(path)
            for path, link in links:
                path.parent.mkdir(parents=True, exist_ok=True)
                if path.is_symlink():
                    path.unlink()
                if path.exists():
                    assert path.is_file()
                    path.unlink()
                path.symlink_to(link)
            downloaded.append(
                {
                    "package": package["name"],
                    "version": package["version"],
                    "sha256": package["sha256"],
                    "members": members,
                }
            )
    initial = [
        Path("/usr/bin") / name for name in ("xdotool", "import", "xauth", "xkbcomp")
    ]
    initial += [Path("/usr/lib/qt6/plugins/platforms/libqxcb.so")]
    for directory in ("/usr/share/X11/xkb", "/etc/ImageMagick-7"):
        initial += [p for p in Path(directory).rglob("*") if p.is_file()]
    initial += [
        p for p in Path("/usr/lib").glob("ImageMagick-*/modules*/coders/png.so")
    ]
    initial += [p for p in Path("/usr/lib").glob("ImageMagick-*/modules*/coders/x.so")]
    for path in initial:
        copy(path)
        elf_paths.append(root / str(path).lstrip("/"))
    # Разрешение динамической closure: downloaded libs приоритетны; host libs только copies.
    env = {
        "PATH": "/usr/bin",
        "LANG": "C",
        "LD_LIBRARY_PATH": str(root / "usr/lib") + ":" + str(root / "usr/lib/openbox"),
    }
    seen = set()
    missing = []
    while elf_paths:
        path = elf_paths.pop()
        if path in seen:
            continue
        seen.add(path)
        with path.open("rb") as stream:
            if stream.read(4) != b"\x7fELF":
                continue
        result = subprocess.run(
            ["ldd", str(path)], env=env, capture_output=True, text=True
        )
        if "not found" in result.stdout:
            missing.append({"path": str(path), "output": result.stdout})
        for raw in re.findall(r"(/[^\s()]+)", result.stdout):
            dep = Path(raw)
            if dep.is_relative_to(root):
                continue
            if not dep.is_file():
                continue
            if dep.parts[1:2] in (("lib",), ("lib64",)):
                dep = Path("/usr/lib") / dep.relative_to("/" + dep.parts[1])
            if not (root / str(dep).lstrip("/")).exists():
                copy(dep)
                elf_paths.append(root / str(dep).lstrip("/"))
    (out / "missing-dynamic-closure.json").write_text(
        json.dumps(missing, indent=2) + "\n"
    )
    assert not missing
    (out / "unselected-optional-runtime.json").write_text(
        json.dumps(unselected_optional, indent=2) + "\n"
    )
    owners = subprocess.run(
        ["pacman", "-Qoq", *[str(Path(p).resolve()) for p in copied]],
        env={"PATH": "/usr/bin", "LANG": "C"},
        capture_output=True,
        text=True,
    )
    (out / "installed-file-owners.log").write_text(owners.stdout + owners.stderr)
    assert owners.returncode == 0
    names = sorted(set(owners.stdout.splitlines()))
    versions = subprocess.check_output(["pacman", "-Q", *names], text=True)
    (out / "installed-package-versions.log").write_text(versions)
    (out / "installed-runtime-copies.json").write_text(
        json.dumps(copied, indent=2) + "\n"
    )
    (out / "downloaded-runtime-members.json").write_text(
        json.dumps(downloaded, indent=2) + "\n"
    )
    for name, dest in (("bin", "usr/bin"), ("lib", "usr/lib"), ("lib64", "usr/lib")):
        (root / name).symlink_to(dest)
    if not (root / "usr/bin/sh").exists():
        (root / "usr/bin/sh").symlink_to("bash")
    for name in (
        "tmp",
        "run",
        "proc",
        "dev",
        "home/fixture",
        "home/fixture/runtime",
        "home/fixture/storage",
        "home/fixture/storage/copies",
        "home/fixture/data",
        "home/fixture/config",
        "home/fixture/evidence",
    ):
        (root / name).mkdir(mode=0o700, parents=True, exist_ok=True)
    for name in ("home/fixture", "home/fixture/runtime", "home/fixture/storage"):
        (root / name).chmod(0o700)
    (root / "etc/passwd").write_text(
        "root:x:0:0:root:/root:/usr/bin/bash\nfixture:x:1000:1000:fixture:/home/fixture:/usr/bin/bash\n"
    )
    (root / "etc/group").write_text("root:x:0:\nfixture:x:1000:\n")
    (root / "etc/os-release").write_bytes(Path("/etc/os-release").read_bytes())
