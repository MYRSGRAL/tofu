// Собственный ELF: положительный marker control и безвредный процесс-кандидат.
#include <fcntl.h>
#include <unistd.h>
#include <sys/prctl.h>
#include <string.h>
int main(int argc, char **argv) {
    if(argc==2 && strcmp(argv[1],"canary")==0) {
        char byte;
        int fd=open("/proc/self/cmdline",O_RDONLY); if(fd<0) return 2;
        if(read(fd,&byte,1)!=1) return 3;
        close(fd);
        fd=open("/proc/self/environ",O_RDONLY); if(fd<0) return 4;
        if(read(fd,&byte,1)!=1) return 5;
        close(fd); return 0;
    }
    if(argc==2) { int fd=open(argv[1],O_CREAT|O_WRONLY|O_EXCL,0600); if(fd<0) return 6; close(fd); return 0; }
    if(prctl(PR_SET_NAME,"a08-helper",0,0,0)!=0) return 7;
    for(;;) pause();
}
