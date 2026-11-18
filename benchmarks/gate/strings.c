#include <stdio.h>
#include <stdlib.h>
#include <string.h>
static char* make(long i) {
    char digits[32];
    int n = snprintf(digits, sizeof digits, "%ld", i);
    char* s = malloc(5 + n + 1);
    memcpy(s, "item-", 5);
    memcpy(s + 5, digits, n + 1);
    return s;
}
int main(void) {
    long total = 0;
    for (long i = 0; i < 3000000; i++) { char* s = make(i); total += strlen(s); free(s); }
    printf("%ld\n", total);
    return 0;
}
