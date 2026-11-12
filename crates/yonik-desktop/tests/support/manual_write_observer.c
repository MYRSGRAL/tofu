/* Пассивный observer synthetic fixture: исходная write всегда выполняется первой. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/syscall.h>
#include <stdio.h>

ssize_t write(int fd, const void *buffer, size_t count) {
    ssize_t (*actual_write)(int, const void *, size_t) = dlsym(RTLD_NEXT, "write");
    ssize_t result = actual_write(fd, buffer, count);
    int original_errno = errno;
    const char *destination = getenv("YONIK_MANUAL_WRITE_OBSERVER");
    if (destination && result == (ssize_t)count && count > 32 && count < 1048576 &&
        ((const char *)buffer)[0] == '{' && memmem(buffer, count, "\"outbounds\"", 11)) {
        char path[64], target[128];
        snprintf(path, sizeof(path), "/proc/self/fd/%d", fd);
        ssize_t length = readlink(path, target, sizeof(target));
        if (length > 5 && !memcmp(target, "pipe:", 5)) {
            int output = syscall(SYS_openat, AT_FDCWD, destination,
                                 O_WRONLY | O_CREAT | O_APPEND | O_CLOEXEC, 0600);
            if (output >= 0) {
                syscall(SYS_write, output, buffer, count);
                syscall(SYS_write, output, "\n", 1);
                syscall(SYS_close, output);
            }
        }
    }
    errno = original_errno;
    return result;
}
