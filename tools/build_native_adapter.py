"""Закреплённая локальная сборка sing-box с патчем strict_complete_dns."""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import logging
import os
import re
import shutil
import signal
import subprocess
from pathlib import Path

SOURCE_URL = "https://github.com/SagerNet/sing-box.git"
SOURCE_TAG = "v1.14.4"
SOURCE_COMMIT = "c005f35300ce76427dc285481ec5695f5b179cd2"
TRANSIENT = Path("/var/tmp/codex-myrs/transient")
DEFAULT_ROOT = TRANSIENT / "tofu-native-adapter"
DEFAULT_PATCH = (
    Path(__file__).resolve().parents[1]
    / "adapters/sing-box/v1.14.4/strict-complete-dns.patch"
)
LOG = logging.getLogger("tofu.native_adapter")
COMMAND_TIMEOUT = 180
BUILD_TIMEOUT = 900


class BuildError(RuntimeError):
    """Сборка остановлена; оставленные исходники и логи требуют проверки."""


def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


class Runner:
    """Команды без shell, с отдельными stdout/stderr и локальным Go cache."""

    def __init__(self, root: Path) -> None:
        self.logs = root / "build-native-adapter-logs"
        try:
            self.logs.mkdir()
        except FileExistsError as error:
            raise BuildError(
                f"Обнаружен предыдущий запуск: {self.logs}; проверьте его вручную"
            ) from error
        self.sequence = 0
        # GOFLAGS, GOENV, GOCACHEPROG и прочие внешние настройки не должны
        # незаметно менять закреплённую команду или запускать сторонний код.
        self.env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith("GO") or key in {"GOOS", "GOARCH", "GOEXPERIMENT"}
        }
        cache = root / "build-native-adapter-cache"
        for variable, name in {
            "GOCACHE": "go-build",
            "GOMODCACHE": "go-mod",
            "GOPATH": "gopath",
            "GOTMPDIR": "go-tmp",
            "TMPDIR": "tmp",
            "XDG_CACHE_HOME": "xdg-cache",
            "XDG_CONFIG_HOME": "xdg-config",
        }.items():
            directory = cache / name
            directory.mkdir(parents=True, exist_ok=True)
            self.env[variable] = str(directory)
        self.env.update(
            GOTOOLCHAIN="local",
            GOENV="off",
            GOWORK="off",
            CGO_ENABLED="0",
            GIT_CONFIG_NOSYSTEM="1",
            GIT_CONFIG_GLOBAL=os.devnull,
            GIT_TERMINAL_PROMPT="0",
        )

    def run(self, argv: list[str], cwd: Path) -> str:
        self.sequence += 1
        prefix = self.logs / f"{self.sequence:02d}-{Path(argv[0]).name}"
        prefix.with_suffix(".command.json").write_text(
            json.dumps({"argv": argv, "cwd": str(cwd)}, ensure_ascii=False) + "\n"
        )
        stdout = prefix.with_suffix(".stdout.log")
        stderr = prefix.with_suffix(".stderr.log")
        with stdout.open("wb") as out, stderr.open("wb") as err:
            pending: list[int] = []
            signals = (signal.SIGINT, signal.SIGTERM)
            previous = {event: signal.getsignal(event) for event in signals}
            process: subprocess.Popen[bytes] | None = None
            try:
                try:
                    # Не теряем handle, если сигнал пришёл во время Popen.
                    for event in signals:
                        signal.signal(
                            event, lambda signum, _frame: pending.append(signum)
                        )
                    process = subprocess.Popen(
                        argv,
                        cwd=cwd,
                        env=self.env,
                        stdout=out,
                        stderr=err,
                        stdin=subprocess.DEVNULL,
                        start_new_session=True,
                    )
                finally:
                    for event, handler in previous.items():
                        signal.signal(event, handler)
                if pending:
                    raise SystemExit(128 + pending[0])
                try:
                    code = process.wait(
                        timeout=BUILD_TIMEOUT
                        if argv[:2] == ["go", "build"]
                        else COMMAND_TIMEOUT
                    )
                except subprocess.TimeoutExpired as error:
                    raise BuildError(
                        f"{argv[0]} превысил лимит времени; логи: {prefix}.*"
                    ) from error
            finally:
                # Cleanup охватывает и восстановление handlers после Popen.
                for event in signals:
                    signal.signal(event, lambda _signum, _frame: None)
                try:
                    if process is not None:
                        try:
                            os.killpg(process.pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                        process.wait()
                finally:
                    for event, handler in previous.items():
                        signal.signal(event, handler)
        if code:
            raise BuildError(f"{argv[0]} завершился с кодом {code}; логи: {prefix}.*")
        return stdout.read_text(errors="replace")


def build(root: Path, output: Path, patch: Path) -> Path:
    """Собрать один раз; повторный запуск не сбрасывает частичные результаты."""
    root = root.resolve()
    output = output.resolve()
    patch = patch.resolve(strict=True)
    if sha256(patch) != sha256(DEFAULT_PATCH):
        raise BuildError("Требуется канонический strict_complete_dns patch")
    if output.is_relative_to(root) or output.is_relative_to(TRANSIENT):
        raise BuildError("--output-dir должен быть вне transient/build root")
    if any((output / name).exists() for name in ("sing-box", "manifest.json")):
        raise BuildError(
            "В --output-dir уже есть binary/manifest; перезапись запрещена"
        )
    root.mkdir(parents=True, exist_ok=True)
    # Lock именно inode корня защищает всё дерево от TTL cleaner. Shared lock
    # совместим с исследованием в других подкаталогах этого task root.
    descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
    try:
        fcntl.flock(descriptor, fcntl.LOCK_SH)
        return build_locked(root, output, patch)
    finally:
        os.close(descriptor)


def build_locked(root: Path, output: Path, patch: Path) -> Path:
    runner = Runner(root)
    source = root / "build-source-sing-box"
    if source.is_symlink():
        raise BuildError(f"Исходник не должен быть symlink: {source}")
    if not source.exists():
        runner.run(
            [
                "git",
                "clone",
                "--depth=1",
                "--branch",
                SOURCE_TAG,
                "--",
                SOURCE_URL,
                str(source),
            ],
            root,
        )
    head = runner.run(["git", "rev-parse", "HEAD"], source).strip()
    tag = runner.run(["git", "rev-parse", f"{SOURCE_TAG}^{{commit}}"], source).strip()
    if head != SOURCE_COMMIT or tag != SOURCE_COMMIT:
        raise BuildError(f"Неверный HEAD/tag: ожидался {SOURCE_COMMIT}, HEAD={head}")
    dirty = runner.run(
        ["git", "status", "--porcelain", "--untracked-files=all", "--ignored"], source
    )
    if dirty.strip():
        raise BuildError(f"Исходник должен быть чистым: {source}")
    modules = {name: sha256(source / name) for name in ("go.mod", "go.sum")}
    patch_digest = sha256(patch)
    runner.run(["git", "apply", "--check", str(patch)], source)
    runner.run(["git", "apply", str(patch)], source)
    if sha256(patch) != patch_digest:
        raise BuildError("Патч изменился во время применения")
    if modules != {name: sha256(source / name) for name in modules}:
        raise BuildError("Патч изменяет go.mod/go.sum")
    raw_tags = (source / "release/DEFAULT_BUILD_TAGS_OTHERS").read_text().strip()
    tags = re.split(r"[,\s]+", raw_tags)
    if not raw_tags or any(not re.fullmatch(r"[A-Za-z0-9_]+", tag) for tag in tags):
        raise BuildError("Некорректные upstream build tags")
    go_version = runner.run(["go", "version"], source).strip()
    go_env = json.loads(
        runner.run(["go", "env", "-json", "GOOS", "GOARCH", "GOEXPERIMENT"], source)
    )
    binary = root / "build-native-adapter-sing-box"
    if binary.exists() or binary.is_symlink():
        raise BuildError(f"Обнаружен предыдущий бинарник: {binary}")
    args = [
        "go",
        "build",
        "-mod=readonly",
        "-trimpath",
        "-buildvcs=false",
        "-tags",
        ",".join(tags),
        "-o",
        str(binary),
        "./cmd/sing-box",
    ]
    try:
        runner.run(args, source)
    finally:
        if modules != {name: sha256(source / name) for name in modules}:
            raise BuildError("go.mod/go.sum изменились при сборке")
    if not binary.is_file() or binary.is_symlink():
        raise BuildError("Go не создал ожидаемый бинарник")
    manifest = {
        "engine": "sing-box",
        "source_url": SOURCE_URL,
        "source_tag": SOURCE_TAG,
        "source_commit": head,
        "patch_sha256": patch_digest,
        "binary_sha256": sha256(binary),
        "go_version": go_version,
        "go_env": go_env,
        "build_environment": {
            key: runner.env[key]
            for key in ("GOTOOLCHAIN", "GOENV", "GOWORK", "CGO_ENABLED")
        },
        "build_tags": tags,
        "build_args": args,
        "module_sha256": modules,
        "adapter_features": {"strict_complete_dns": True},
        "strictDNSGate": False,
        "strict_dns_gate": False,
        "bit_reproducibility_verified": False,
    }
    output.mkdir(parents=True, exist_ok=True)
    destination = output / "sing-box"
    with binary.open("rb") as incoming, destination.open("xb") as outgoing:
        shutil.copyfileobj(incoming, outgoing)
    destination.chmod(0o755)
    if sha256(destination) != manifest["binary_sha256"]:
        raise BuildError("SHA256 опубликованного бинарника не совпал")
    manifest_path = output / "manifest.json"
    with manifest_path.open("x") as stream:
        stream.write(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
    return manifest_path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-root", type=Path, default=DEFAULT_ROOT)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--patch", type=Path, default=DEFAULT_PATCH)
    options = parser.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(message)s")

    def terminate(signum: int, _frame: object) -> None:
        raise SystemExit(128 + signum)

    signal.signal(signal.SIGTERM, terminate)
    try:
        manifest = build(options.build_root, options.output_dir, options.patch)
    except (BuildError, OSError, ValueError) as error:
        LOG.error("Сборка остановлена: %s", error)
        return 1
    LOG.info("Бинарник и manifest сохранены: %s", manifest.parent)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
