
#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>

typedef struct { char* data; int64_t len; } yk_string;

yk_string yk_string_make(const char* s) {
    int64_t len = (int64_t)strlen(s);
    char* data = (char*)malloc(len + 1);
    memcpy(data, s, len + 1);
    return (yk_string){data, len};
}

yk_string yk_string_from_int(int64_t v) {
    char buf[64];
    int n = snprintf(buf, sizeof(buf), "%lld", (long long)v);
    char* data = (char*)malloc(n + 1);
    memcpy(data, buf, n + 1);
    return (yk_string){data, n};
}

yk_string yk_string_concat(yk_string a, yk_string b) {
    char* data = (char*)malloc(a.len + b.len + 1);
    memcpy(data, a.data, a.len);
    memcpy(data + a.len, b.data, b.len);
    data[a.len + b.len] = '\0';
    return (yk_string){data, a.len + b.len};
}

int64_t yk_string_len(yk_string s) { return s.len; }

void yk_print_int(int64_t v) { printf("%lld\n", (long long)v); }
void yk_print_real(double v) { printf("%g\n", v); }
void yk_print_bool(bool v) { printf("%s\n", v ? "true" : "false"); }
void yk_print_str(yk_string s) { printf("%.*s\n", (int)s.len, s.data); }

#define yk_print_val(v) _Generic((v), \
    int64_t: yk_print_int, \
    double: yk_print_real, \
    bool: yk_print_bool, \
    yk_string: yk_print_str \
)(v)


int main(int argc, char** argv) {
    (void)argc; (void)argv;
    x = 0;
    y = (0 + 4);
    yk_print_val((y.mod));
    {
        int64_t yk_start_0 = 10;
        int64_t yk_end_0 = 10;
        for (int64_t i = yk_start_0; i < yk_end_0; i++) {
            yk_print_val(((yk_string_concat(yk_string_make(" "), str(x)) + yk_string_make(" : ")) + str(i)));
        }
    }
    return 0;
}
