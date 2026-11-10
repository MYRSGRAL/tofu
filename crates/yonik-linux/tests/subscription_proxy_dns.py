"""Изолированная проверка отсутствия системного DNS у Proxy test binary."""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

INTERPOSER = r"""
#define _GNU_SOURCE
#include <arpa/inet.h>
#include <netdb.h>
#include <dlfcn.h>
#include <fcntl.h>
#include <stdlib.h>
#include <unistd.h>
int getaddrinfo(const char *node, const char *service,
                const struct addrinfo *hints, struct addrinfo **result) {
    const char *path = getenv("YONIK_DNS_CALLS");
    if (path) {
        int fd = open(path, O_WRONLY | O_CREAT | O_APPEND, 0600);
        if (fd >= 0) { write(fd, "lookup\n", 7); close(fd); }
    }
    return EAI_NONAME;
}
"""


def main() -> None:
    """Перехватить getaddrinfo только дочернего процесса; host DNS не меняется."""
    with tempfile.TemporaryDirectory(prefix="yonik-proxy-dns-") as temporary:
        directory = Path(temporary)
        source = directory / "resolver.c"
        library = directory / "resolver.so"
        calls = directory / "calls"
        source.write_text(INTERPOSER)
        subprocess.run(
            [
                "cc",
                "-shared",
                "-fPIC",
                "-Wall",
                "-Werror",
                str(source),
                "-ldl",
                "-o",
                str(library),
            ],
            check=True,
            timeout=15,
        )
        env = dict(os.environ, LD_PRELOAD=str(library), YONIK_DNS_CALLS=str(calls))
        subprocess.run([sys.argv[1], *sys.argv[2:]], env=env, check=True, timeout=60)
        count = len(calls.read_text().splitlines()) if calls.exists() else 0
        sys.stderr.write(f"system_resolver_calls={count}\n")
        if count:
            raise AssertionError("Proxy вызвал системный resolver")


if __name__ == "__main__":
    main()
