#include <stdio.h>
int main(void) {
    long total = 0;
    for (long i = 0; i < 500000000; i++) total += i % 7;
    printf("%ld\n", total);
    return 0;
}
