
#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>

typedef struct { char* data; int64_t len; } yk_string;

void yk_string_init_ptr(yk_string* s, const char* data, int64_t len) {
    s->data = (char*)data;
    s->len = len;
}

yk_string* yk_string_from_int(int64_t v) {
    char buf[64];
    int n = snprintf(buf, sizeof(buf), "%lld", (long long)v);
    yk_string* s = (yk_string*)malloc(sizeof(yk_string));
    s->data = (char*)malloc(n + 1);
    memcpy(s->data, buf, n + 1);
    s->len = n;
    return s;
}

yk_string* yk_string_concat_ptr(yk_string* a, yk_string* b) {
    yk_string* s = (yk_string*)malloc(sizeof(yk_string));
    s->data = (char*)malloc(a->len + b->len + 1);
    memcpy(s->data, a->data, a->len);
    memcpy(s->data + a->len, b->data, b->len);
    s->data[a->len + b->len] = '\0';
    s->len = a->len + b->len;
    return s;
}

int64_t yk_string_len_ptr(yk_string* s) { return s->len; }

void yk_print_int(int64_t v) { printf("%lld\n", (long long)v); }
void yk_print_real(double v) { printf("%g\n", v); }
void yk_print_bool(bool v) { printf("%s\n", v ? "true" : "false"); }
void yk_print_str_ptr(yk_string* s) { printf("%.*s\n", (int)s->len, s->data); }
