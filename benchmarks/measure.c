#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/resource.h>
#include <sys/wait.h>

int main(int argc, char** argv) {
    if (argc != 3) {
        fprintf(stderr, "usage: measure <program> <stdout-file>\n");
        return 2;
    }
    struct timespec start, end;
    clock_gettime(CLOCK_MONOTONIC, &start);
    pid_t pid = fork();
    if (pid < 0) return 2;
    if (pid == 0) {
        int out = open(argv[2], O_WRONLY | O_CREAT | O_TRUNC, 0600);
        int null = open("/dev/null", O_WRONLY);
        if (out < 0 || null < 0) _exit(127);
        dup2(out, 1);
        dup2(null, 2);
        execl(argv[1], argv[1], (char*)NULL);
        _exit(127);
    }
    int status = 0;
    struct rusage usage;
    if (wait4(pid, &status, 0, &usage) < 0) return 2;
    clock_gettime(CLOCK_MONOTONIC, &end);
    long long ns = (long long)(end.tv_sec - start.tv_sec) * 1000000000LL + (end.tv_nsec - start.tv_nsec);
#ifdef __APPLE__
    long rss_kb = usage.ru_maxrss / 1024;
#else
    long rss_kb = usage.ru_maxrss;
#endif
    printf("%lld %ld %d\n", ns, rss_kb, WIFEXITED(status) ? WEXITSTATUS(status) : 128);
    return 0;
}
