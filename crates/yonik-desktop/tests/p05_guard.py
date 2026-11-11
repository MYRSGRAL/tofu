"""Controls resolver и DIRECT fence только для тестового executable."""

from __future__ import annotations

import importlib.util
import os
import subprocess
import sys
import tempfile
from pathlib import Path


def main() -> None:
    binary = Path(sys.argv[1]).resolve()
    module_path = (
        Path(__file__).resolve().parents[2]
        / "yonik-linux/tests/subscription_proxy_dns.py"
    )
    spec = importlib.util.spec_from_file_location("dns_guard", module_path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    # Direct fence смотрит exe, поэтому не затрагивает дочерние pinned cores.
    direct = r"""
#include <sys/socket.h>
#include <string.h>
#include <errno.h>
int connect(int fd, const struct sockaddr *address, socklen_t size) {
    char exe[4096];
    ssize_t len = readlink("/proc/self/exe", exe, sizeof(exe)-1);
    const char *target = getenv("P05_TEST_EXE");
    if (len > 0 && target) {
        exe[len] = 0;
        unsigned short port = 0;
        if (address->sa_family == AF_INET) port = ntohs(((const struct sockaddr_in *)address)->sin_port);
        if (address->sa_family == AF_INET6) port = ntohs(((const struct sockaddr_in6 *)address)->sin6_port);
        char needle[16];
        snprintf(needle,sizeof(needle),",%u,",port);
        const char *ports = getenv("P05_FORBIDDEN_PORTS");
        if (!strcmp(exe,target) && ports && strstr(ports,needle)) {
            const char *path = getenv("P05_DIRECT_CALLS");
            int out = open(path,O_WRONLY|O_CREAT|O_APPEND,0600);
            if(out >= 0) { write(out,"direct\n",7); close(out); }
            errno = ECONNREFUSED;
            return -1;
        }
    }
    int (*real_connect)(int,const struct sockaddr *,socklen_t) = dlsym(RTLD_NEXT,"connect");
    return real_connect(fd,address,size);
}
"""
    with tempfile.TemporaryDirectory(prefix="p05-guard-") as tmp:
        work = Path(tmp)
        source, library, calls = work / "guard.c", work / "guard.so", work / "dns.calls"
        source.write_text(module.INTERPOSER + "\n#include <stdio.h>\n" + direct)
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
        control = dict(
            os.environ,
            LD_PRELOAD=str(library),
            YONIK_DNS_CALLS=str(calls),
            P05_TEST_EXE=str(Path(sys.executable).resolve()),
        )
        # Два controls: известный DNS lookup и TCP connect на fenced literal port.
        subprocess.run(
            [
                sys.executable,
                "-c",
                "import socket;\ntry: socket.getaddrinfo('control.invalid',443)\nexcept socket.gaierror: pass",
            ],
            env=control,
            check=True,
            timeout=5,
        )
        assert calls.read_text().splitlines() == ["lookup"]
        calls.unlink()
        direct_calls = work / "direct.calls"
        env = dict(
            os.environ,
            LD_PRELOAD=str(library),
            YONIK_DNS_CALLS=str(calls),
            P05_DIRECT_CALLS=str(direct_calls),
            P05_TEST_EXE=str(binary),
        )
        control.update(P05_FORBIDDEN_PORTS=",9,", P05_DIRECT_CALLS=str(direct_calls))
        subprocess.run(
            [
                sys.executable,
                "-c",
                "import socket;\ns=socket.socket()\ntry: s.connect(('127.0.0.1',9))\nexcept OSError: pass",
            ],
            env=control,
            check=True,
            timeout=5,
        )
        assert direct_calls.read_text().splitlines() == ["direct"]
        direct_calls.unlink()
        subprocess.run([str(binary), *sys.argv[2:]], env=env, check=True, timeout=180)
        assert not calls.exists(), "system lookup"
        assert not direct_calls.exists(), "DIRECT fallback"
        sys.stdout.write(
            "dns_positive_control=1 direct_positive_control=1 system_lookups=0 hidden_direct=0\n"
        )


if __name__ == "__main__":
    main()
