#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <math.h>

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

yk_string* yk_string_from_real(double v) {
    char buf[64];
    int n = snprintf(buf, sizeof(buf), "%g", v);
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

typedef struct { double real; double imag; } yk_complex;

void yk_complex_set(yk_complex* c, double r, double i) { c->real = r; c->imag = i; }
double yk_complex_real(yk_complex* c) { return c->real; }
double yk_complex_imag(yk_complex* c) { return c->imag; }
double yk_complex_mod(yk_complex* c) { return sqrt(c->real * c->real + c->imag * c->imag); }
double yk_complex_arg(yk_complex* c) { return atan2(c->imag, c->real); }
void yk_complex_conj(yk_complex* r, yk_complex* c) { r->real = c->real; r->imag = -c->imag; }
void yk_complex_add(yk_complex* r, yk_complex* a, yk_complex* b) { r->real = a->real + b->real; r->imag = a->imag + b->imag; }
void yk_complex_sub(yk_complex* r, yk_complex* a, yk_complex* b) { r->real = a->real - b->real; r->imag = a->imag - b->imag; }
void yk_complex_mul(yk_complex* r, yk_complex* a, yk_complex* b) { r->real = a->real*b->real - a->imag*b->imag; r->imag = a->real*b->imag + a->imag*b->real; }
void yk_complex_div(yk_complex* r, yk_complex* a, yk_complex* b) { double d = b->real*b->real + b->imag*b->imag; r->real = (a->real*b->real + a->imag*b->imag)/d; r->imag = (a->imag*b->real - a->real*b->imag)/d; }
void yk_print_complex(yk_complex* c) { printf("%g + %gi\n", c->real, c->imag); }

yk_string* yk_string_from_bool(bool v) {
    const char* s = v ? "true" : "false";
    yk_string* r = (yk_string*)malloc(sizeof(yk_string));
    int n = (int)strlen(s);
    r->data = (char*)malloc(n + 1);
    memcpy(r->data, s, n + 1);
    r->len = n;
    return r;
}

yk_string* yk_string_from_complex(yk_complex* c) {
    char buf[128];
    int n;
    if (c->imag < 0)
        n = snprintf(buf, sizeof(buf), "%g%gi", c->real, c->imag);
    else
        n = snprintf(buf, sizeof(buf), "%g+%gi", c->real, c->imag);
    yk_string* s = (yk_string*)malloc(sizeof(yk_string));
    s->data = (char*)malloc(n + 1);
    memcpy(s->data, buf, n + 1);
    s->len = n;
    return s;
}

yk_string* yk_string_from_char(int32_t c) {
    yk_string* s = (yk_string*)malloc(sizeof(yk_string));
    s->data = (char*)malloc(2);
    s->data[0] = (char)c;
    s->data[1] = '\0';
    s->len = 1;
    return s;
}
