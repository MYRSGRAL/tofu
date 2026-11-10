"""Запуск реальных managed-network Rust-тестов только в отдельной Linux-сети."""

from __future__ import annotations

import argparse
import json
import logging
import os
import subprocess
import sys
from pathlib import Path

from engine_smoke import install_signal_handlers, run_sandbox

LOG = logging.getLogger("yonik.managed-network")
CASES = {
    "owned-tun": "managed_network::tun::tests::real_namespace_tun_ownership_reopen_and_cleanup",
    "public-block": "managed_network::nft::block::tests::real_public_block_tcp_udp_dual_stack_and_lifetime",
    "durable-policy": "managed_network::nft::policy::tests::sigkill_recovery_preserves_desired_previous_and_unlock_intents",
    "service": "helper::protection_host::tests::isolated_service_applies_preserves_and_unlocks_real_rules",
}
_BOOTSTRAP = """
import json, os, subprocess, sys
from pathlib import Path
host = tuple(json.loads(sys.argv[1]))
current = os.stat('/proc/self/ns/net')
if (current.st_dev, current.st_ino) == host:
    raise SystemExit('Нет изоляции network namespace')
for target in ('/tmp', '/run'):
    subprocess.run(['/usr/bin/mount','-t','tmpfs','-o','size=16M','tmpfs',target], check=True, timeout=5)
subprocess.run(['/usr/bin/mount','-t','sysfs','sysfs','/sys'], check=True, timeout=5)
runtime = Path('/tmp/yonik-private-runtime')
runtime.mkdir(mode=0o700)
env = os.environ.copy()
env['XDG_RUNTIME_DIR'] = str(runtime)
env['YONIK_NFT_TEST_HOST_NETNS'] = f'{host[0]}:{host[1]}'
listing = subprocess.run([sys.argv[2], '--list', '--format', 'terse', '--exact', sys.argv[5]], env=env, capture_output=True, text=True, timeout=5)
if listing.returncode or listing.stdout.strip() != f'{sys.argv[5]}: test':
    raise SystemExit('Нужный Rust test отсутствует; ноль тестов не является успехом')
raise SystemExit(subprocess.run(sys.argv[2:], env=env, timeout=90).returncode)
"""


def command(binary: Path, case: str) -> list[str]:
    namespace = os.stat("/proc/self/ns/net")
    return [
        "/usr/bin/unshare",
        "--user",
        "--map-root-user",
        "--net",
        "--mount",
        "--pid",
        "--fork",
        "--mount-proc",
        sys.executable,
        "-c",
        _BOOTSTRAP,
        json.dumps((namespace.st_dev, namespace.st_ino)),
        str(binary),
        "--ignored",
        "--exact",
        CASES[case],
        "--nocapture",
    ]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--case", choices=CASES, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError("Нужен исполняемый Rust test binary")
    run_sandbox(command(binary, args.case), timeout=100)
    return 0


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    install_signal_handlers()
    try:
        raise SystemExit(main())
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        LOG.error("Проверка не выполнена: %s", error)
        raise SystemExit(1) from None
