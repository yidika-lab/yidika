use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use crate::diagnostics::error::{self, ErrorKind, Result};
use crate::diagnostics::span::Span;
use crate::hardware::HardwareInfo;
use crate::interpret::FnDef;
use crate::syntax::ast::*;

const RUNTIME_C: &str = r##"
#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <math.h>
#include <time.h>
#include <direct.h>
#include <process.h>
#ifdef _WIN32
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <winhttp.h>
#pragma comment(lib, "winhttp.lib")
#pragma comment(lib, "ws2_32.lib")
#include <winsock2.h>
#include <ws2tcpip.h>
#endif
#include <regex>

typedef struct { char* data; int64_t len; } yk_string;

#ifdef __cplusplus
extern "C" {
#endif

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

yk_string* yk_string_from_cstr(const char* s) {
    yk_string* result = (yk_string*)malloc(sizeof(yk_string));
    result->len = (int64_t)strlen(s);
    result->data = (char*)malloc(result->len + 1);
    memcpy(result->data, s, result->len + 1);
    return result;
}

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

yk_string* yk_json_string(yk_string* s) {
    int64_t cap = s->len * 6 + 3;
    char* out = (char*)malloc((size_t)cap);
    int64_t j = 0;
    out[j++] = '"';
    for (int64_t i = 0; i < s->len; i++) {
        char c = s->data[i];
        switch (c) {
            case '"':  out[j++] = '\\'; out[j++] = '"'; break;
            case '\\': out[j++] = '\\'; out[j++] = '\\'; break;
            case '\n': out[j++] = '\\'; out[j++] = 'n'; break;
            case '\r': out[j++] = '\\'; out[j++] = 'r'; break;
            case '\t': out[j++] = '\\'; out[j++] = 't'; break;
            case '\b': out[j++] = '\\'; out[j++] = 'b'; break;
            case '\f': out[j++] = '\\'; out[j++] = 'f'; break;
            default:
                if ((unsigned char)c < 0x20) {
                    j += sprintf(out + j, "\\u%04x", (unsigned char)c);
                } else {
                    out[j++] = c;
                }
                break;
        }
        if (j >= cap - 1) break;
    }
    out[j++] = '"';
    out[j] = '\0';
    yk_string* result = (yk_string*)malloc(sizeof(yk_string));
    result->data = out;
    result->len = j;
    return result;
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

#include <process.h>

typedef struct { void (*entry)(void*); void* ctx; HANDLE thread; int64_t result; } yk_task;

typedef struct { int64_t* data; int64_t len; int64_t cap; } yk_list;

yk_list* yk_list_new(void) {
    yk_list* l = (yk_list*)malloc(sizeof(yk_list));
    l->data = NULL; l->len = 0; l->cap = 0;
    return l;
}

void yk_list_push(yk_list* l, int64_t val) {
    if (l->len >= l->cap) {
        l->cap = l->cap ? l->cap * 2 : 8;
        l->data = (int64_t*)realloc(l->data, l->cap * sizeof(int64_t));
    }
    l->data[l->len++] = val;
}

int64_t yk_list_get(yk_list* l, int64_t idx) {
    if (idx < 0 || idx >= l->len) { return 0; }
    return l->data[idx];
}

int64_t yk_list_len(yk_list* l) { return l->len; }

void yk_list_free(yk_list* l) { if (l) { free(l->data); free(l); } }

int64_t yk_list_pop(yk_list* l) {
    if (l->len == 0) return 0;
    return l->data[--l->len];
}

void yk_list_clear(yk_list* l) { l->len = 0; }

void yk_list_sort(yk_list* l) {
    for (int64_t i = 0; i < l->len - 1; i++) {
        for (int64_t j = 0; j < l->len - i - 1; j++) {
            if (l->data[j] > l->data[j + 1]) {
                int64_t t = l->data[j]; l->data[j] = l->data[j + 1]; l->data[j + 1] = t;
            }
        }
    }
}

void yk_list_reverse(yk_list* l) {
    for (int64_t i = 0; i < l->len / 2; i++) {
        int64_t j = l->len - i - 1;
        int64_t t = l->data[i]; l->data[i] = l->data[j]; l->data[j] = t;
    }
}

void yk_list_insert(yk_list* l, int64_t idx, int64_t val) {
    if (idx < 0) idx = 0;
    if (idx > l->len) idx = l->len;
    if (l->len >= l->cap) {
        l->cap = l->cap ? l->cap * 2 : 8;
        l->data = (int64_t*)realloc(l->data, l->cap * sizeof(int64_t));
    }
    for (int64_t i = l->len; i > idx; i--) l->data[i] = l->data[i - 1];
    l->data[idx] = val;
    l->len++;
}

void yk_list_remove(yk_list* l, int64_t idx) {
    if (idx < 0 || idx >= l->len) return;
    for (int64_t i = idx; i < l->len - 1; i++) l->data[i] = l->data[i + 1];
    l->len--;
}

void yk_list_print(yk_list* l) {
    printf("[");
    for (int64_t i = 0; i < l->len; i++) {
        if (i > 0) printf(", ");
        printf("%lld", (long long)l->data[i]);
    }
    printf("]\n");
}

int64_t yk_result_str_new(int64_t data, int64_t len) {
    yk_string* s = (yk_string*)malloc(sizeof(yk_string));
    s->data = (char*)malloc(len + 1);
    memcpy(s->data, (void*)(intptr_t)data, len);
    s->data[len] = '\0';
    s->len = len;
    return (int64_t)(intptr_t)s;
}

void yk_print_result_val(int64_t val, int8_t ok) {
    yk_string* s = (yk_string*)(intptr_t)val;
    int looks_like_str = (val != 0 && s->len >= 0 && s->len < 1048576 && s->data != NULL);
    if (looks_like_str) {
        printf("%s%.*s)\n", ok ? "Ok(" : "Error(", (int)s->len, s->data);
    } else {
        printf("%s%lld)\n", ok ? "Ok(" : "Error(", (long long)val);
    }
}

static unsigned int __stdcall yk_task_start(void* arg) {
    yk_task* t = (yk_task*)arg;
    t->entry(t->ctx);
    return 0;
}

uint64_t yk_run_thread(void (*entry)(void*), void* ctx) {
    yk_task* t = (yk_task*)malloc(sizeof(yk_task));
    t->entry = entry;
    t->ctx = ctx;
    t->result = 0;
    t->thread = (HANDLE)_beginthreadex(NULL, 0, yk_task_start, t, 0, NULL);
    if (!t->thread) { free(t); return 0; }
    return (uint64_t)(intptr_t)t;
}

int64_t yk_join_thread(uint64_t handle) {
    yk_task* t = (yk_task*)(intptr_t)handle;
    WaitForSingleObject(t->thread, INFINITE);
    CloseHandle(t->thread);
    int64_t r = t->result;
    free(t);
    return r;
}

void yk_task_set_result(uint64_t handle, int64_t val) {
    yk_task* t = (yk_task*)(intptr_t)handle;
    t->result = val;
}

int64_t yk_math_abs_i64(int64_t x) { return x < 0 ? -x : x; }
double yk_math_abs_real(double x) { return x < 0 ? -x : x; }
double yk_math_sqrt(double x) { return sqrt(x); }
double yk_math_sin(double x) { return sin(x); }
double yk_math_cos(double x) { return cos(x); }
double yk_math_floor(double x) { return floor(x); }
double yk_math_ceil(double x) { return ceil(x); }
double yk_math_round(double x) { return round(x); }
double yk_math_pow(double x, double y) { return pow(x, y); }
double yk_math_max(double x, double y) { return x > y ? x : y; }
double yk_math_min(double x, double y) { return x < y ? x : y; }
int64_t yk_math_rand(int64_t max) { return rand() % max; }

yk_string* yk_time_now(void) {
    time_t t = time(NULL);
    char buf[64];
    int n = snprintf(buf, sizeof(buf), "%lld", (long long)t);
    yk_string* s = (yk_string*)malloc(sizeof(yk_string));
    s->data = (char*)malloc(n + 1);
    memcpy(s->data, buf, n + 1);
    s->len = n;
    return s;
}

void yk_time_sleep(int64_t ms) { Sleep((DWORD)ms); }

int64_t yk_time_timestamp(void) { return (int64_t)time(NULL); }

int64_t yk_sys_pid(void) { return (int64_t)GetCurrentProcessId(); }
void yk_sys_exit(int64_t code) { exit((int)code); }
yk_string* yk_sys_cwd(void) {
    char* cwd = _getcwd(NULL, 0);
    yk_string* s = (yk_string*)malloc(sizeof(yk_string));
    s->data = cwd;
    s->len = (int64_t)strlen(cwd);
    return s;
}
yk_string* yk_sys_platform(void) {
#ifdef _WIN32
    const char* p = "windows";
#elif __APPLE__
    const char* p = "macos";
#else
    const char* p = "linux";
#endif
    yk_string* s = (yk_string*)malloc(sizeof(yk_string));
    s->data = strdup(p);
    s->len = (int64_t)strlen(p);
    return s;
}
yk_string* yk_sys_env(yk_string* name) {
    char* val = getenv(name->data);
    if (!val) { val = ""; }
    yk_string* s = (yk_string*)malloc(sizeof(yk_string));
    s->data = strdup(val);
    s->len = (int64_t)strlen(val);
    return s;
}

yk_string* yk_path_join(yk_string* a, yk_string* b) {
    int64_t total = a->len + 1 + b->len;
    char* buf = (char*)malloc(total + 1);
    memcpy(buf, a->data, a->len);
    buf[a->len] = '/';
    memcpy(buf + a->len + 1, b->data, b->len);
    buf[total] = '\0';
    yk_string* s = (yk_string*)malloc(sizeof(yk_string));
    s->data = buf;
    s->len = total;
    return s;
}

yk_string* yk_path_dirname(yk_string* s) {
    int64_t i;
    for (i = s->len - 1; i >= 0; i--) {
        if (s->data[i] == '/' || s->data[i] == '\\') {
            char* buf = (char*)malloc(i + 1);
            memcpy(buf, s->data, i);
            buf[i] = '\0';
            yk_string* r = (yk_string*)malloc(sizeof(yk_string));
            r->data = buf; r->len = i;
            return r;
        }
    }
    yk_string* r = (yk_string*)malloc(sizeof(yk_string));
    r->data = strdup(""); r->len = 0;
    return r;
}

yk_string* yk_path_basename(yk_string* s) {
    int64_t i;
    for (i = s->len - 1; i >= 0; i--) {
        if (s->data[i] == '/' || s->data[i] == '\\') {
            int64_t nlen = s->len - i - 1;
            char* buf = (char*)malloc(nlen + 1);
            memcpy(buf, s->data + i + 1, nlen); buf[nlen] = '\0';
            yk_string* r = (yk_string*)malloc(sizeof(yk_string));
            r->data = buf; r->len = nlen;
            return r;
        }
    }
    return yk_string_from_cstr(s->data);
}

yk_string* yk_path_extension(yk_string* s) {
    int64_t i;
    for (i = s->len - 1; i >= 0; i--) {
        if (s->data[i] == '.') {
            int64_t nlen = s->len - i - 1;
            char* buf = (char*)malloc(nlen + 1);
            memcpy(buf, s->data + i + 1, nlen); buf[nlen] = '\0';
            yk_string* r = (yk_string*)malloc(sizeof(yk_string));
            r->data = buf; r->len = nlen;
            return r;
        }
        if (s->data[i] == '/' || s->data[i] == '\\') break;
    }
    yk_string* r = (yk_string*)malloc(sizeof(yk_string));
    r->data = strdup(""); r->len = 0;
    return r;
}

int64_t yk_path_is_absolute(yk_string* s) {
#ifdef _WIN32
    if (s->len > 2 && s->data[1] == ':') return 1;
    return 0;
#else
    if (s->len > 0 && s->data[0] == '/') return 1;
    return 0;
#endif
}

yk_string* yk_fs_read(yk_string* path) {
    HANDLE h = CreateFileA(path->data, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (h == INVALID_HANDLE_VALUE) { return yk_string_from_cstr(""); }
    DWORD size = GetFileSize(h, NULL);
    char* buf = (char*)malloc(size + 1);
    DWORD read = 0;
    ReadFile(h, buf, size, &read, NULL);
    CloseHandle(h);
    buf[read] = '\0';
    yk_string* s = (yk_string*)malloc(sizeof(yk_string));
    s->data = buf; s->len = read;
    return s;
}

void yk_fs_write(yk_string* path, yk_string* content) {
    HANDLE h = CreateFileA(path->data, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    if (h == INVALID_HANDLE_VALUE) return;
    DWORD written = 0;
    WriteFile(h, content->data, (DWORD)content->len, &written, NULL);
    CloseHandle(h);
}

void yk_fs_append(yk_string* path, yk_string* content) {
    HANDLE h = CreateFileA(path->data, FILE_APPEND_DATA, 0, NULL, OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    if (h == INVALID_HANDLE_VALUE) return;
    DWORD written = 0;
    WriteFile(h, content->data, (DWORD)content->len, &written, NULL);
    CloseHandle(h);
}

void yk_fs_remove(yk_string* path) { DeleteFileA(path->data); }

int64_t yk_fs_exists(yk_string* path) {
    DWORD attr = GetFileAttributesA(path->data);
    return (attr != INVALID_FILE_ATTRIBUTES) ? 1 : 0;
}

int64_t yk_fs_is_dir(yk_string* path) {
    DWORD attr = GetFileAttributesA(path->data);
    return (attr != INVALID_FILE_ATTRIBUTES && (attr & FILE_ATTRIBUTE_DIRECTORY)) ? 1 : 0;
}

int64_t yk_fs_is_file(yk_string* path) {
    DWORD attr = GetFileAttributesA(path->data);
    return (attr != INVALID_FILE_ATTRIBUTES && !(attr & FILE_ATTRIBUTE_DIRECTORY)) ? 1 : 0;
}

static const char b64_enc[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

yk_string* yk_base64_encode(yk_string* input) {
    int64_t len = input->len;
    int64_t out_len = ((len + 2) / 3) * 4;
    char* out = (char*)malloc(out_len + 1);
    int i, j;
    for (i = 0, j = 0; i < len; ) {
        unsigned char a = i < len ? (unsigned char)input->data[i++] : 0;
        unsigned char b = i < len ? (unsigned char)input->data[i++] : 0;
        unsigned char c = i < len ? (unsigned char)input->data[i++] : 0;
        unsigned int triple = (a << 16) | (b << 8) | c;
        out[j++] = b64_enc[(triple >> 18) & 0x3F];
        out[j++] = b64_enc[(triple >> 12) & 0x3F];
        out[j++] = (i - 1 >= len) ? '=' : b64_enc[(triple >> 6) & 0x3F];
        out[j++] = (i >= len) ? '=' : b64_enc[triple & 0x3F];
    }
    out[out_len] = '\0';
    yk_string* s = (yk_string*)malloc(sizeof(yk_string));
    s->data = out; s->len = out_len;
    return s;
}

static int b64_dec_char(char c) {
    if (c >= 'A' && c <= 'Z') return c - 'A';
    if (c >= 'a' && c <= 'z') return c - 'a' + 26;
    if (c >= '0' && c <= '9') return c - '0' + 52;
    if (c == '+') return 62;
    if (c == '/') return 63;
    return -1;
}

yk_string* yk_base64_decode(yk_string* input) {
    int64_t len = input->len;
    if (len == 0) return yk_string_from_cstr("");
    int64_t out_len = (len / 4) * 3;
    if (input->data[len-1] == '=') out_len--;
    if (len > 1 && input->data[len-2] == '=') out_len--;
    if (out_len < 0) out_len = 0;
    char* out = (char*)malloc(out_len + 1);
    int i, j;
    for (i = 0, j = 0; i < len && j < out_len; ) {
        int a = b64_dec_char(input->data[i++]);
        int b = b64_dec_char(input->data[i++]);
        int c = (i < len && input->data[i] != '=') ? b64_dec_char(input->data[i]) : 0; i++;
        int d = (i < len && input->data[i] != '=') ? b64_dec_char(input->data[i]) : 0; i++;
        if (a < 0 || b < 0) break;
        if (c < 0) c = 0;
        if (d < 0) d = 0;
        unsigned int triple = (a << 18) | (b << 12) | (c << 6) | d;
        if (j < out_len) out[j++] = (triple >> 16) & 0xFF;
        if (j < out_len) out[j++] = (triple >> 8) & 0xFF;
        if (j < out_len) out[j++] = triple & 0xFF;
    }
    out[out_len] = '\0';
    yk_string* s = (yk_string*)malloc(sizeof(yk_string));
    s->data = out; s->len = out_len;
    return s;
}

yk_string* yk_datetime_now(void) {
    time_t t = time(NULL);
    struct tm lt;
    localtime_s(&lt, &t);
    char buf[64];
    strftime(buf, sizeof(buf), "%Y-%m-%d %H:%M:%S", &lt);
    return yk_string_from_cstr(buf);
}

yk_string* yk_datetime_utc(void) {
    time_t t = time(NULL);
    struct tm gt;
    gmtime_s(&gt, &t);
    char buf[64];
    strftime(buf, sizeof(buf), "%Y-%m-%d %H:%M:%S", &gt);
    return yk_string_from_cstr(buf);
}

int64_t yk_datetime_year(int64_t ts) {
    struct tm lt; time_t t = (time_t)ts;
    localtime_s(&lt, &t);
    return (int64_t)lt.tm_year + 1900;
}

int64_t yk_datetime_month(int64_t ts) {
    struct tm lt; time_t t = (time_t)ts;
    localtime_s(&lt, &t);
    return (int64_t)lt.tm_mon + 1;
}

int64_t yk_datetime_day(int64_t ts) {
    struct tm lt; time_t t = (time_t)ts;
    localtime_s(&lt, &t);
    return (int64_t)lt.tm_mday;
}

int64_t yk_datetime_hour(int64_t ts) {
    struct tm lt; time_t t = (time_t)ts;
    localtime_s(&lt, &t);
    return (int64_t)lt.tm_hour;
}

int64_t yk_datetime_minute(int64_t ts) {
    struct tm lt; time_t t = (time_t)ts;
    localtime_s(&lt, &t);
    return (int64_t)lt.tm_min;
}

int64_t yk_datetime_second(int64_t ts) {
    struct tm lt; time_t t = (time_t)ts;
    localtime_s(&lt, &t);
    return (int64_t)lt.tm_sec;
}

yk_string* yk_datetime_format(int64_t ts, yk_string* fmt) {
    struct tm lt; time_t t = (time_t)ts;
    localtime_s(&lt, &t);
    size_t sz = fmt->len * 2 + 64;
    char* buf = (char*)malloc(sz);
    strftime(buf, sz, fmt->data, &lt);
    return yk_string_from_cstr(buf);
}

extern "C" int64_t yk_re_match(yk_string* pattern, yk_string* text) {
    try {
        std::regex re(std::string(pattern->data, pattern->len));
        std::string t(text->data, text->len);
        return std::regex_search(t, re) ? 1 : 0;
    } catch (...) { return 0; }
}

extern "C" yk_string* yk_re_replace(yk_string* pattern, yk_string* text, yk_string* replacement) {
    try {
        std::regex re(std::string(pattern->data, pattern->len));
        std::string t(text->data, text->len);
        std::string r(replacement->data, replacement->len);
        std::string result = std::regex_replace(t, re, r);
        yk_string* s = (yk_string*)malloc(sizeof(yk_string));
        s->len = (int64_t)result.size();
        s->data = (char*)malloc(s->len + 1);
        memcpy(s->data, result.c_str(), s->len + 1);
        return s;
    } catch (...) { return yk_string_from_cstr(""); }
}

extern "C" yk_string* yk_fetch(yk_string* url, yk_string* method, yk_string* body) {
    if (!url || url->len == 0) return yk_string_from_cstr("");
    HINTERNET hSession = WinHttpOpen(L"Yidika/1.0", WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, NULL, NULL, 0);
    if (!hSession) return yk_string_from_cstr("");
    URL_COMPONENTSW uc = {0};
    uc.dwStructSize = sizeof(uc);
    uc.dwHostNameLength = (DWORD)-1;
    uc.dwUrlPathLength = (DWORD)-1;
    int url_len = MultiByteToWideChar(CP_UTF8, 0, url->data, (int)url->len, NULL, 0);
    wchar_t* wurl = (wchar_t*)malloc((url_len + 1) * sizeof(wchar_t));
    MultiByteToWideChar(CP_UTF8, 0, url->data, (int)url->len, wurl, url_len);
    wurl[url_len] = L'\0';
    if (!WinHttpCrackUrl(wurl, url_len, 0, &uc)) { free(wurl); WinHttpCloseHandle(hSession); return yk_string_from_cstr(""); }
    wchar_t host[256] = {0};
    wcsncpy_s(host, 256, uc.lpszHostName, uc.dwHostNameLength);
    wchar_t path[2048] = {0};
    if (uc.lpszUrlPath && uc.dwUrlPathLength > 0)
        wcsncpy_s(path, 2048, uc.lpszUrlPath, uc.dwUrlPathLength);
    else
        path[0] = L'/';
    INTERNET_PORT port = uc.nPort;
    DWORD flags = uc.nScheme == INTERNET_SCHEME_HTTPS ? WINHTTP_FLAG_SECURE : 0;
    HINTERNET hConnect = WinHttpConnect(hSession, host, port, 0);
    if (!hConnect) { free(wurl); WinHttpCloseHandle(hSession); return yk_string_from_cstr(""); }
    const wchar_t* wmethod = L"GET";
    if (method && method->len > 0) {
        int mlen = MultiByteToWideChar(CP_UTF8, 0, method->data, (int)method->len, NULL, 0);
        wchar_t* wm = (wchar_t*)malloc((mlen + 1) * sizeof(wchar_t));
        MultiByteToWideChar(CP_UTF8, 0, method->data, (int)method->len, wm, mlen);
        wm[mlen] = L'\0';
        wmethod = wm;
    }
    HINTERNET hRequest = WinHttpOpenRequest(hConnect, wmethod, path, NULL, NULL, NULL, flags);
    if (!hRequest) { free(wurl); WinHttpCloseHandle(hConnect); WinHttpCloseHandle(hSession); return yk_string_from_cstr(""); }
    LPCWSTR headers = L"Content-Type: application/octet-stream\r\n";
    void* body_data = NULL;
    DWORD body_len = 0;
    if (body && body->len > 0) { body_data = body->data; body_len = (DWORD)body->len; }
    BOOL sent = WinHttpSendRequest(hRequest, headers, -1, body_data, body_len, body_len, 0);
    if (!sent) { free(wurl); WinHttpCloseHandle(hRequest); WinHttpCloseHandle(hConnect); WinHttpCloseHandle(hSession); return yk_string_from_cstr(""); }
    if (!WinHttpReceiveResponse(hRequest, NULL)) { free(wurl); WinHttpCloseHandle(hRequest); WinHttpCloseHandle(hConnect); WinHttpCloseHandle(hSession); return yk_string_from_cstr(""); }
    DWORD total = 0, cap = 4096;
    char* buf = (char*)malloc(cap);
    DWORD read = 0;
    while (WinHttpReadData(hRequest, buf + total, cap - total - 1, &read) && read > 0) {
        total += read;
        if (total + 1024 >= cap) { cap *= 2; buf = (char*)realloc(buf, cap); }
        read = 0;
    }
    buf[total] = '\0';
    if (method && method->len > 0) free((void*)wmethod);
    free(wurl);
    WinHttpCloseHandle(hRequest);
    WinHttpCloseHandle(hConnect);
    WinHttpCloseHandle(hSession);
    yk_string* s = (yk_string*)malloc(sizeof(yk_string));
    s->data = buf; s->len = total;
    return s;
}

// ── TCP HTTP Server (keep-alive + thread pool) ───────
typedef struct { char* method; char* path; void (*handler)(void*,void*,char*,int64_t); } yk_route;

typedef struct {
    yk_route* routes;
    int64_t count;
    int64_t cap;
    SOCKET listen_fd;
    volatile int running;
} yk_server;

int64_t yk_server_new(void) {
    yk_server* s = (yk_server*)malloc(sizeof(yk_server));
    s->routes = NULL;
    s->count = 0;
    s->cap = 0;
    s->listen_fd = INVALID_SOCKET;
    s->running = 0;
    return (int64_t)(intptr_t)s;
}

void yk_server_add_route(int64_t handle, yk_string* method, yk_string* path, void* fn_ptr) {
    yk_server* s = (yk_server*)(intptr_t)handle;
    if (s->count >= s->cap) {
        s->cap = s->cap ? s->cap * 2 : 8;
        s->routes = (yk_route*)realloc(s->routes, s->cap * sizeof(yk_route));
    }
    yk_route* r = &s->routes[s->count++];
    r->method = (char*)malloc(method->len + 1);
    memcpy(r->method, method->data, method->len); r->method[method->len] = '\0';
    r->path = (char*)malloc(path->len + 1);
    memcpy(r->path, path->data, path->len); r->path[path->len] = '\0';
    r->handler = (void (*)(void*,void*,char*,int64_t))(intptr_t)fn_ptr;
}

// Check if "Connection: keep-alive" (case-insensitive) appears in headers
static int yk_check_keep_alive(const char* headers, int len) {
    const char* conn = strstr(headers, "\r\nConnection:");
    if (!conn) conn = strstr(headers, "\r\nconnection:");
    if (!conn) return 0;
    const char* eol = strstr(conn, "\r\n");
    if (!eol) return 0;
    int line_len = (int)(eol - conn);
    // Check if "keep-alive" appears anywhere on the Connection line
    const char* ka = conn;
    while (ka < eol) {
        if (*ka == 'k' || *ka == 'K') {
            if (_strnicmp(ka, "keep-alive", 10) == 0) return 1;
        }
        ka++;
    }
    return 0;
}

// Handle one client connection. Returns when connection is closed.
static void yk_handle_client(SOCKET client_fd, yk_server* s) {
    char buf[65536];
    char method_buf[64], path_buf[4096];

    // Determine default keep-alive: HTTP/1.1 defaults to keep-alive
    int http_ver_major = 1, http_ver_minor = 1;

    while (s->running) {
        // Read HTTP request headers (up to \r\n\r\n)
        int total = 0;
        while (total < (int)sizeof(buf) - 1) {
            int n = recv(client_fd, buf + total, (int)sizeof(buf) - 1 - total, 0);
            if (n <= 0) { closesocket(client_fd); return; }
            total += n;
            if (total >= 4 && memcmp(buf + total - 4, "\r\n\r\n", 4) == 0) break;
        }
        if (total < 4) { closesocket(client_fd); return; }
        buf[total] = '\0';

        // Parse first line: METHOD /path HTTP/1.x
        char* line_end = strstr(buf, "\r\n");
        if (!line_end) { closesocket(client_fd); return; }
        int line_len = (int)(line_end - buf);
        char* space1 = (char*)memchr(buf, ' ', line_len);
        if (!space1) { closesocket(client_fd); return; }
        char* space2 = (char*)memchr(space1 + 1, ' ', line_end - space1 - 1);
        if (!space2) { closesocket(client_fd); return; }
        int method_len = (int)(space1 - buf);
        int path_len = (int)(space2 - space1 - 1);
        if (method_len > 63) method_len = 63;
        if (path_len > 4095) path_len = 4095;
        memcpy(method_buf, buf, method_len); method_buf[method_len] = '\0';
        memcpy(path_buf, space1 + 1, path_len); path_buf[path_len] = '\0';

        // Parse HTTP version (e.g. "HTTP/1.0")
        char* ver = space2 + 1;
        if (strncmp(ver, "HTTP/", 5) == 0) {
            http_ver_major = ver[5] - '0';
            http_ver_minor = ver[7] - '0';
        }

        // Check Connection header for keep-alive
        int client_ka = yk_check_keep_alive(buf, total);
        // HTTP/1.1 defaults to keep-alive unless Connection: close
        // HTTP/1.0 defaults to close unless Connection: keep-alive
        int use_keep_alive;
        if (http_ver_major >= 1 && http_ver_minor >= 1) {
            use_keep_alive = (strstr(buf, "\r\nConnection: close") == NULL &&
                             strstr(buf, "\r\nconnection: close") == NULL);
        } else {
            use_keep_alive = client_ka;
        }

        // Route matching
        yk_route* matched = NULL;
        for (int64_t i = 0; i < s->count; i++) {
            if (strcmp(s->routes[i].method, method_buf) == 0 &&
                strcmp(s->routes[i].path, path_buf) == 0) {
                matched = &s->routes[i];
                break;
            }
        }

        if (matched) {
            typedef struct { void* method; int64_t mlen; void* path; int64_t plen; void* body; int64_t blen; } yk_req;
            yk_req req;
            req.method = method_buf; req.mlen = method_len;
            req.path = path_buf; req.plen = path_len;
            req.body = NULL; req.blen = 0;

            typedef struct { void* body; int64_t body_len; int32_t status; } yk_resp;
            yk_resp resp;
            resp.body = NULL; resp.body_len = 0; resp.status = 200;

            char handler_buf[16384];
            matched->handler(&resp, &req, handler_buf, sizeof(handler_buf));

            char resp_buf[65536];
            const char* conn_hdr = use_keep_alive ? "keep-alive" : "close";
            int n = snprintf(resp_buf, sizeof(resp_buf),
                "HTTP/1.1 %d OK\r\nContent-Length: %lld\r\nContent-Type: text/plain\r\nConnection: %s\r\n\r\n",
                resp.status, (long long)resp.body_len, conn_hdr);
            if (resp.body && resp.body_len > 0 && n + resp.body_len < (int)sizeof(resp_buf)) {
                memcpy(resp_buf + n, resp.body, resp.body_len);
                n += (int)resp.body_len;
            }
            send(client_fd, resp_buf, n, 0);
        } else {
            const char* conn_hdr = use_keep_alive ? "keep-alive" : "close";
            char resp_buf[512];
            int n = snprintf(resp_buf, sizeof(resp_buf),
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: %s\r\n\r\n", conn_hdr);
            send(client_fd, resp_buf, n, 0);
        }

        if (!use_keep_alive) { closesocket(client_fd); return; }
    }
    closesocket(client_fd);
}

// Worker thread: accept connections and handle them
static unsigned __stdcall yk_worker_thread(void* arg) {
    yk_server* s = (yk_server*)arg;
    while (s->running) {
        struct sockaddr_in client;
        int client_len = sizeof(client);
        SOCKET client_fd = accept(s->listen_fd, (struct sockaddr*)&client, &client_len);
        if (client_fd == INVALID_SOCKET) { continue; }
        if (!s->running) { closesocket(client_fd); return 0; }
        yk_handle_client(client_fd, s);
    }
    return 0;
}

void yk_server_serve(int64_t handle, yk_string* addr) {
    yk_server* s = (yk_server*)(intptr_t)handle;
    if (!s || s->count == 0) { printf("No routes registered\n"); return; }

    // Parse addr: "host:port"
    char addr_buf[256];
    int addr_len = (int)(addr->len < 255 ? addr->len : 255);
    memcpy(addr_buf, addr->data, addr_len);
    addr_buf[addr_len] = '\0';
    char* port_str = strrchr(addr_buf, ':');
    if (!port_str) { printf("Invalid address format (expected host:port)\n"); return; }
    *port_str++ = '\0';
    char* host = addr_buf;
    int port = atoi(port_str);
    if (port <= 0) port = 8080;

    // Initialize Winsock
    WSADATA wsa;
    if (WSAStartup(MAKEWORD(2,2), &wsa) != 0) {
        printf("WSAStartup failed\n"); return;
    }

    // Create socket
    SOCKET server_fd = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if (server_fd == INVALID_SOCKET) {
        printf("socket() failed: %d\n", WSAGetLastError());
        WSACleanup(); return;
    }

    int opt = 1;
    setsockopt(server_fd, SOL_SOCKET, SO_REUSEADDR, (const char*)&opt, sizeof(opt));

    struct sockaddr_in sa;
    sa.sin_family = AF_INET;
    sa.sin_port = htons((unsigned short)port);
    if (strcmp(host, "*") == 0 || strcmp(host, "0.0.0.0") == 0)
        sa.sin_addr.s_addr = INADDR_ANY;
    else
        inet_pton(AF_INET, host, &sa.sin_addr);

    if (bind(server_fd, (struct sockaddr*)&sa, sizeof(sa)) == SOCKET_ERROR) {
        printf("bind() failed: %d\n", WSAGetLastError());
        closesocket(server_fd); WSACleanup(); return;
    }

    if (listen(server_fd, SOMAXCONN) == SOCKET_ERROR) {
        printf("listen() failed: %d\n", WSAGetLastError());
        closesocket(server_fd); WSACleanup(); return;
    }

    s->listen_fd = server_fd;
    s->running = 1;
    printf("Server listening on %s:%d\n", host, port);

    int num_workers = 4;
    HANDLE* threads = (HANDLE*)malloc(num_workers * sizeof(HANDLE));
    for (int i = 0; i < num_workers; i++) {
        threads[i] = (HANDLE)_beginthreadex(NULL, 0, yk_worker_thread, s, 0, NULL);
    }

    // Wait for all workers (never actually completes — server runs until killed)
    WaitForMultipleObjects(num_workers, threads, TRUE, INFINITE);

    s->running = 0;
    for (int i = 0; i < num_workers; i++) CloseHandle(threads[i]);
    free(threads);
    closesocket(server_fd);
    WSACleanup();
}

#ifdef __cplusplus
}
#endif
"##;

pub struct LlvmCodegen {
    output: String,
    indent: usize,
    var_types: HashMap<String, String>,
    var_alloca: HashMap<String, String>,
    struct_defs: HashMap<String, Vec<(String, String)>>,
    class_defs: HashMap<String, Vec<(String, String)>>,
    class_vtables: HashMap<String, Vec<(String, String)>>,
    class_extends: HashMap<String, String>,
    class_method_ret_types: HashMap<(String, String), String>,
    tuple_type_names: HashMap<String, String>,
    tuple_types_output: Vec<String>,
    tuple_counter: usize,
    label_counter: usize,
    in_block: bool,
    string_constants: String,
    nullable_types: HashSet<String>,
    variant_tags: HashMap<String, i64>,
    variant_tag_counter: i64,
    deferred_fns: Vec<(String, Vec<Param>, Option<TypeNode>, ExprNode)>,
    closure_counter: usize,
    spawn_wrappers: Vec<(String, String, ExprNode)>,
    spawn_counter: usize,
    fn_name_map: HashMap<String, String>,
    current_module: String,
    ffi_modules: HashSet<String>,
    ffi_decls: HashSet<String>,
    current_fn_ret: String,
    fn_ret_types: HashMap<String, String>,
    handler_irs: Vec<String>,
    fn_defs: HashMap<String, crate::interpret::FnDef>,
}

impl LlvmCodegen {
    pub fn new() -> Self {
        Self {
            output: String::new(),
            indent: 0,
            var_types: HashMap::new(),
            var_alloca: HashMap::new(),
            struct_defs: HashMap::new(),
            class_defs: HashMap::new(),
            class_vtables: HashMap::new(),
            class_extends: HashMap::new(),
            class_method_ret_types: HashMap::new(),
            tuple_type_names: HashMap::new(),
            tuple_types_output: Vec::new(),
            tuple_counter: 0,
            label_counter: 0,
            in_block: false,
            string_constants: String::new(),
            nullable_types: HashSet::new(),
            variant_tags: HashMap::new(),
            variant_tag_counter: 0,
            deferred_fns: Vec::new(),
            closure_counter: 0,
            spawn_wrappers: Vec::new(),
            spawn_counter: 0,
            fn_name_map: HashMap::new(),
            current_module: String::new(),
            ffi_modules: HashSet::new(),
            ffi_decls: HashSet::new(),
            current_fn_ret: "void".into(),
            fn_ret_types: HashMap::new(),
            handler_irs: Vec::new(),
            fn_defs: HashMap::new(),
        }
    }

    fn e(&mut self, s: &str) {
        use std::fmt::Write;
        writeln!(self.output, "{}{}", "  ".repeat(self.indent), s).unwrap();
    }

    fn e_raw(&mut self, s: &str) {
        self.output.push_str(s);
        self.output.push('\n');
    }

    fn fresh_label(&mut self) -> String {
        let n = self.label_counter;
        self.label_counter += 1;
        format!("yk_{}", n)
    }

    fn mangle_name(&self, name: &str) -> String {
        if name == "main" {
            return name.to_string();
        }
        if let Some(mangled) = self.fn_name_map.get(name) {
            mangled.clone()
        } else {
            name.to_string()
        }
    }

    fn ssa(&self, raw: &str) -> String {
        format!("%{}", raw)
    }

    fn make_string_slot(&mut self, s: &str) -> String {
        let lbl = self.fresh_label();
        let escaped = s.replace('\\', "\\\\").replace('\n', "\\0A").replace('"', "\\22");
        use std::fmt::Write;
        writeln!(self.string_constants, "@{} = private unnamed_addr constant [{} x i8] c\"{}\\00\", align 1", lbl, s.len() + 1, escaped).unwrap();

        let ptr = self.fresh_label();
        self.e(&format!("%{} = getelementptr inbounds [{} x i8], ptr @{}, i64 0, i64 0", ptr, s.len() + 1, lbl));
        let tmp = self.fresh_label();
        self.e(&format!("%{} = insertvalue %yk_string undef, ptr %{}, 0", tmp, ptr));
        let tmp2 = self.fresh_label();
        self.e(&format!("%{} = insertvalue %yk_string %{}, i64 {}, 1", tmp2, tmp, s.len()));
        self.ssa(&tmp2)
    }

    fn string_to_ptr(&mut self, val: &str) -> String {
        let slot = self.fresh_label();
        self.e(&format!("%{} = alloca %yk_string, align 8", slot));
        self.e(&format!("store %yk_string {}, ptr %{}", val, slot));
        self.ssa(&slot)
    }

    fn to_i64(&mut self, val: String, typ: String) -> String {
        match typ.as_str() {
            "double" => {
                let b = self.fresh_label();
                self.e(&format!("%{} = bitcast double {} to i64", b, val));
                self.ssa(&b)
            }
            "i1" => {
                let b = self.fresh_label();
                self.e(&format!("%{} = zext i1 {} to i64", b, val));
                self.ssa(&b)
            }
            "%yk_string" => {
                let p = self.string_to_ptr(&val);
                let b = self.fresh_label();
                self.e(&format!("%{} = ptrtoint ptr {} to i64", b, p));
                self.ssa(&b)
            }
            "%yk_complex" => {
                let slot = self.fresh_label();
                self.e(&format!("%{} = alloca %yk_complex, align 8", slot));
                self.e(&format!("store %yk_complex {}, ptr %{}", val, slot));
                let b = self.fresh_label();
                self.e(&format!("%{} = ptrtoint ptr %{} to i64", b, slot));
                self.ssa(&b)
            }
            _ => val,
        }
    }

    fn compile_server_method(&mut self, handle: &str, field: &str, compiled_args: &[(String, String)], raw_args: &[ExprNode]) -> (String, String) {
        match field {
            "serve" => {
                let addr_ptr = compiled_args.first().map(|(av, at)| {
                    if at == "%yk_string" { self.string_to_ptr(av) } else { "null".into() }
                }).unwrap_or_else(|| "null".into());
                self.e(&format!("call void @yk_server_serve(i64 {}, ptr {})", handle, addr_ptr));
                ("0".into(), "void".into())
            }
            "get" | "post" => {
                let method_slot = self.make_string_slot(&field.to_uppercase());
                let method_ptr = self.string_to_ptr(&method_slot);
                let path_ptr = compiled_args.get(0).map(|(av, at)| {
                    if at == "%yk_string" { self.string_to_ptr(av) } else { "null".into() }
                }).unwrap_or_else(|| "null".into());
                let handler_ptr = if raw_args.len() > 1 {
                    match &raw_args[1].value {
                        Expr::LitStr(content) => {
                            let hname = format!("__yk_handler_lit_{}", self.fresh_label());
                            let ir = crate::codegen::llvm::generate_static_handler_ir(&hname, content, 200);
                            let cleaned: String = ir.lines()
                                .filter(|l| !l.starts_with("target triple") && !l.starts_with("; JIT"))
                                .collect::<Vec<_>>()
                                .join("\n");
                            self.handler_irs.push(cleaned);
                            let tmp = self.fresh_label();
                            self.e(&format!("%{} = ptrtoint ptr @{} to i64", tmp, hname));
                            self.ssa(&tmp)
                        }
                        Expr::Ident(fn_name) => {
                            // Named function handler
                            if let Some(fndef) = self.fn_defs.get(fn_name) {
                                let hname = format!("__yk_handler_fn_{}", fn_name);
                                if let Some(ir) = crate::codegen::llvm::generate_fn_handler_ir(&hname, fndef) {
                                    let cleaned: String = ir.lines()
                                        .filter(|l| !l.starts_with("target triple") && !l.starts_with("; JIT"))
                                        .collect::<Vec<_>>()
                                        .join("\n");
                                    self.handler_irs.push(cleaned);
                                    let tmp = self.fresh_label();
                                    self.e(&format!("%{} = ptrtoint ptr @{} to i64", tmp, hname));
                                    self.ssa(&tmp)
                                } else {
                                    "0".into()
                                }
                            } else {
                                if let Some((hv, _)) = compiled_args.get(1) {
                                    hv.clone()
                                } else {
                                    "0".into()
                                }
                            }
                        }
                        Expr::FnLit(params, _ret_type, body) => {
                            let fn_body = vec![Node::new(0, Span::new(0, 0), Stmt::Return(Some(*body.clone())))];
                            let fndef = FnDef::new(params.clone(), fn_body);
                            let hname = format!("__yk_handler_closure_{}", self.fresh_label());
                            if let Some(ir) = crate::codegen::llvm::generate_fn_handler_ir(&hname, &fndef) {
                                let cleaned: String = ir.lines()
                                    .filter(|l| !l.starts_with("target triple") && !l.starts_with("; JIT"))
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                self.handler_irs.push(cleaned);
                                let tmp = self.fresh_label();
                                self.e(&format!("%{} = ptrtoint ptr @{} to i64", tmp, hname));
                                self.ssa(&tmp)
                            } else {
                                "0".into()
                            }
                        }
                        Expr::Closure(params, body) => {
                            let fn_body = vec![Node::new(0, Span::new(0, 0), Stmt::Return(Some(*body.clone())))];
                            let fndef = FnDef::new(params.clone(), fn_body);
                            let hname = format!("__yk_handler_closure_{}", self.fresh_label());
                            if let Some(ir) = crate::codegen::llvm::generate_fn_handler_ir(&hname, &fndef) {
                                let cleaned: String = ir.lines()
                                    .filter(|l| !l.starts_with("target triple") && !l.starts_with("; JIT"))
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                self.handler_irs.push(cleaned);
                                let tmp = self.fresh_label();
                                self.e(&format!("%{} = ptrtoint ptr @{} to i64", tmp, hname));
                                self.ssa(&tmp)
                            } else {
                                "0".into()
                            }
                        }
                        _ => {
                            if let Some((hv, _)) = compiled_args.get(1) {
                                hv.clone()
                            } else {
                                "0".into()
                            }
                        }
                    }
                } else {
                    "0".into()
                };
                self.e(&format!("call void @yk_server_add_route(i64 {}, ptr {}, ptr {}, i64 {})",
                    handle, method_ptr, path_ptr, handler_ptr));
                ("0".into(), "void".into())
            }
            _ => ("0".into(), "i64".into()),
        }
    }

    fn get_variant_tag(&mut self, variant_name: &str) -> i64 {
        if let Some(&tag) = self.variant_tags.get(variant_name) {
            tag
        } else {
            let tag = self.variant_tag_counter;
            self.variant_tag_counter += 1;
            self.variant_tags.insert(variant_name.to_string(), tag);
            tag
        }
    }

    fn variant_name_tag(&self, variant_name: &str) -> i64 {
        self.variant_tags.get(variant_name).copied().unwrap_or(-1)
    }

    fn int_width(s: &str) -> usize {
        match s {
            "i8" => 8,
            "i16" => 16,
            "i32" => 32,
            "i64" => 64,
            _ => 64,
        }
    }

    fn is_nullable_ty(ty: &str) -> bool {
        ty.starts_with("%__nullable_")
    }

    /// Wrap a non-nullable value into a nullable struct `{ inner, i1 }`.
    /// Returns the SSA name of the wrapped value.
    fn wrap_in_nullable(&mut self, nullable_ty: &str, val: &str, val_ty: &str, is_null: bool) -> String {
        if is_null {
            // Use zeroinitializer for the whole struct, then set flag to 0
            let r1 = self.fresh_label();
            self.e(&format!("%{} = insertvalue {} zeroinitializer, i1 0, 1", r1, nullable_ty));
            self.ssa(&r1)
        } else {
            let r1 = self.fresh_label();
            self.e(&format!("%{} = insertvalue {} undef, {} {}, 0", r1, nullable_ty, val_ty, val));
            let r2 = self.fresh_label();
            self.e(&format!("%{} = insertvalue {} %{}, i1 1, 1", r2, nullable_ty, r1));
            self.ssa(&r2)
        }
    }

    fn type_to_llvm(&mut self, te: &TypeExpr) -> String {
        match te {
            TypeExpr::Int(w) | TypeExpr::Rint(w) => {
                match w {
                    8 => "i8".into(),
                    16 => "i16".into(),
                    32 => "i32".into(),
                    _ => "i64".into(), // 0 (generic) or 64
                }
            }
            TypeExpr::Real(w) => {
                match w {
                    32 => "float".into(),
                    _ => "double".into(), // 0 (generic) or 64
                }
            }
            TypeExpr::Complex(_, _) => "%yk_complex".into(),
            TypeExpr::Bool => "i1".into(),
            TypeExpr::Str => "%yk_string".into(),
            TypeExpr::Symbol => "%yk_string".into(),
            TypeExpr::Vector(inner) => {
                let inner_ty = self.type_to_llvm(inner);
                format!("<2 x {}>", inner_ty)
            }
            TypeExpr::Matrix(inner) => {
                let inner_ty = self.type_to_llvm(inner);
                format!("[<2 x {}> x 2]", inner_ty)
            }
            TypeExpr::Named(name) => {
                let lc = name.to_lowercase();
                match lc.as_str() {
                    "str" | "string" => "%yk_string".into(),
                    "int" | "int64" | "integer" => "i64".into(),
                    "int8" => "i8".into(),
                    "int16" => "i16".into(),
                    "int32" => "i32".into(),
                    "real" | "float" | "double" => "double".into(),
                    "bool" | "boolean" => "i1".into(),
                    "symbol" => "%yk_string".into(),
                    "complex" => "%yk_complex".into(),
                    _ => {
                        if self.struct_defs.contains_key(name) {
                            format!("%struct.{}", name)
                        } else if self.class_defs.contains_key(name) {
                            format!("%class.{}", name)
                        } else {
                            "i64".into()
                        }
                    }
                }
            }
            TypeExpr::Nullable(inner) => {
                let inner_ty = self.type_to_llvm(inner);
                let name = format!("%__nullable_{}", inner_ty.replace(|c: char| !c.is_alphanumeric(), "_"));
                if !self.nullable_types.contains(&name) {
                    self.nullable_types.insert(name.clone());
                    self.tuple_types_output.push(format!("{} = type {{ {}, i1 }}", name, inner_ty));
                }
                name
            }
            TypeExpr::Union(variants) => {
                if let Some(first) = variants.first() {
                    self.type_to_llvm(first)
                } else {
                    "i64".into()
                }
            }
            TypeExpr::Generic(name, _) => {
                if self.class_defs.contains_key(name) {
                    format!("%class.{}", name)
                } else if self.struct_defs.contains_key(name) {
                    format!("%struct.{}", name)
                } else if name == "Result" {
                    "%yk_result".into()
                } else {
                    "i64".into()
                }
            }
            TypeExpr::List(_) | TypeExpr::Set(_) | TypeExpr::Map(_, _) => "ptr".into(),
            _ => "i64".into(),
        }
    }

    fn get_or_create_tuple_type(&mut self, elem_types: &[String]) -> String {
        let sig = elem_types.join("_");
        if let Some(name) = self.tuple_type_names.get(&sig) {
            return name.clone();
        }
        let n = self.tuple_counter;
        self.tuple_counter += 1;
        let name = format!("%struct.__yk_t{}", n);
        self.tuple_type_names.insert(sig, name.clone());
        self.tuple_types_output.push(format!("{} = type {{ {} }}", name, elem_types.join(", ")));
        name
    }

    fn expr_type_str(&mut self, expr: &ExprNode) -> String {
        match &expr.value {
            Expr::LitInt(_) | Expr::LitHex(_) => "i64".into(),
            Expr::LitReal(_) => "double".into(),
            Expr::LitBool(_) => "i1".into(),
            Expr::LitStr(_) => "%yk_string".into(),
            Expr::LitSymbol(_) => "%yk_string".into(),
            Expr::Ident(name) => self.var_types.get(name).cloned().unwrap_or("i64".into()),
            Expr::BinOp(l, _op, r) => {
                let lt = self.expr_type_str(l);
                if lt == "%yk_string" { return "%yk_string".into(); }
                let rt = self.expr_type_str(r);
                if lt == "%yk_complex" || rt == "%yk_complex" { "%yk_complex".into() }
                else if lt == "double" || rt == "double" { "double".into() }
                else { lt }
            }
            Expr::UnOp(_, inner) => self.expr_type_str(inner),
            Expr::Call(_, _) => "i64".into(),
            Expr::StructLit(name, _) => {
                if self.class_defs.contains_key(name) {
                    format!("%class.{}", name)
                } else {
                    format!("%struct.{}", name)
                }
            }
            Expr::TupleLit(items) => {
                let elem_types: Vec<String> = items.iter().map(|i| self.expr_type_str(i)).collect();
                self.get_or_create_tuple_type(&elem_types)
            }
            Expr::LitComplex(_, _) => "%yk_complex".into(),
            Expr::Field(obj, field) => {
                let ot = self.expr_type_str(obj);
                if ot == "%yk_complex" {
                    if field == "conj" { "%yk_complex".into() } else { "double".into() }
                } else if ot == "%yk_string" {
                    "i64".into()
                } else {
                    "i64".into()
                }
            }
            Expr::PostInc(i) | Expr::PostDec(i) => self.expr_type_str(i),
            Expr::Try(_inner) => "i64".into(),
            Expr::ResultOk(_) | Expr::ResultErr(_) => "%yk_result".into(),
            Expr::Spawn(_) => "i64".into(),
            Expr::Await(inner) => self.expr_type_str(inner),
            Expr::As(_, target_type) => self.type_to_llvm(&target_type.value),
            Expr::Match(_, arms) => arms.first().map(|a| self.expr_type_str(&a.body)).unwrap_or("i64".into()),
            _ => "i64".into(),
        }
    }

    fn val_ty(&self, name: &str) -> String {
        self.var_types.get(name).cloned().unwrap_or("i64".into())
    }

    fn alloca_name(&self, var: &str) -> String {
        format!("%{}.ptr", var.replace('.', "_"))
    }

    fn find_alloca_for_expr(&self, expr: &ExprNode) -> String {
        match &expr.value {
            Expr::Ident(name) => self.var_alloca.get(name).cloned().unwrap_or_else(|| self.alloca_name(name)),
            Expr::Field(obj, field) => {
                let inner = self.find_alloca_for_expr(obj);
                format!("{}.{}", inner, field)
            }
            _ => "%0".into(),
        }
    }

    fn value_name(&mut self, var: &str) -> String {
        let n = self.label_counter;
        self.label_counter += 1;
        format!("%{}.val_{}", var.replace('.', "_"), n)
    }

    pub fn compile_module(&mut self, module: &Module) -> String {
        self.compile_modules(&[module])
    }

    pub fn compile_modules(&mut self, modules: &[&Module]) -> String {
        self.e_raw("; LLVM IR generated by yidi");
        self.e_raw("target datalayout = \"e-m:w-p270:32:32-p271:32:32-p272:64:64-i64:64-f80:128-n8:16:32:64-S128\"");
        self.e_raw("target triple = \"x86_64-pc-windows-msvc\"");
        self.e_raw("");

        self.e_raw("%yk_string = type { ptr, i64 }");
        self.e_raw("%yk_complex = type { double, double }");
        self.e_raw("%yk_variant = type { i64, i64 }");
        self.e_raw("%yk_result = type { i64, i1 }");
        self.e_raw("");

        // Build function name mapping (module-prefixed for multi-module)
        for module in modules {
            for item in &module.items {
                if let ItemKind::Fn { name, .. } = &item.value {
                    if name == "main" {
                        self.fn_name_map.insert(name.clone(), name.clone());
                    } else if module.name.is_empty() {
                        self.fn_name_map.insert(name.clone(), name.clone());
                    } else {
                        let mangled = format!("{}.{}", module.name, name);
                        self.fn_name_map.insert(name.clone(), mangled);
                    }
                }
            }
        }

        // Collect function definitions for handler compilation
        for module in modules {
            for item in &module.items {
                if let ItemKind::Fn { name, params, body, .. } = &item.value {
                    if name != "main" {
                        let fndef = FnDef::new(params.clone(), body.clone());
                        self.fn_defs.insert(name.clone(), fndef);
                    }
                }
            }
        }

        // Collect FFI module names from imports
        for module in modules {
            for import in &module.imports {
                if let Some(lang) = &import.lang {
                    if lang == "rust" || lang == "c++" {
                        for (name, _) in &import.names {
                            self.ffi_modules.insert(name.clone());
                        }
                    }
                }
            }
        }

        for module in modules {
            for item in &module.items {
                if let ItemKind::Struct { name, fields, .. } = &item.value {
                    let mut field_types = Vec::new();
                    let mut field_llvm = Vec::new();
                    for p in fields {
                        let ft = self.type_to_llvm(&p.type_expr.value);
                        field_llvm.push(ft.clone());
                        field_types.push((p.name.clone(), ft));
                    }
                    self.struct_defs.insert(name.clone(), field_types);
                    let align_attr = item.decorators.iter()
                        .find_map(|d| d.strip_prefix("align(").and_then(|s| s.strip_suffix(')')))
                        .map(|n| format!(", align {}", n))
                        .unwrap_or_default();
                    self.e_raw(&format!("%struct.{} = type {{ {} }}{}", name, field_llvm.join(", "), align_attr));
                }
            }
        }
        self.e_raw("");

        // First pass: collect class definitions and vtables
        for module in modules {
            for item in &module.items {
                if let ItemKind::Class { name, fields, methods: _, extends, .. } = &item.value {
                    // Collect field types
                    let mut class_field_types = Vec::new();
                    for p in fields {
                        let ft = self.type_to_llvm(&p.type_expr.value);
                        class_field_types.push((p.name.clone(), ft));
                    }
                    self.class_defs.insert(name.clone(), class_field_types);
                    if let Some(parent) = extends {
                        self.class_extends.insert(name.clone(), parent.clone());
                    }
                }
            }
        }
        // Build vtable info: walk inheritance chain to build full method list
        let mut class_vtable_methods: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for module in modules {
            for item in &module.items {
                if let ItemKind::Class { name, methods, extends, .. } = &item.value {
                    // Collect methods: walk parent chain
                    let mut vtable_entries: Vec<(String, String)> = Vec::new();
                    let mut seen_methods: HashSet<String> = HashSet::new();
                    // Walk up parent chain to collect inherited methods
                    if let Some(parent) = extends {
                        if let Some(parent_methods) = class_vtable_methods.get(parent.as_str()) {
                            for (mname, mfn) in parent_methods {
                                vtable_entries.push((mname.clone(), mfn.clone()));
                                seen_methods.insert(mname.clone());
                            }
                        }
                    }
                    // Add own methods (possibly overriding parent)
                    for m in methods {
                        if let ItemKind::Fn { name: mname, params: _, .. } = m {
                            let mangled = if module.name.is_empty() {
                                format!("__method_{}_{}", name, mname)
                            } else {
                                format!("__method_{}_{}_{}", module.name, name, mname)
                            };
                            if seen_methods.contains(mname.as_str()) {
                                // Override: replace the entry
                                if let Some(pos) = vtable_entries.iter().position(|(n, _)| n == mname) {
                                    vtable_entries[pos] = (mname.clone(), mangled);
                                }
                            } else {
                                vtable_entries.push((mname.clone(), mangled));
                                seen_methods.insert(mname.clone());
                            }
                        }
                    }
                    class_vtable_methods.insert(name.clone(), vtable_entries);
                }
            }
        }

        // Emit class types as LLVM structs (vtable ptr + fields)
        let class_names: Vec<String> = self.class_defs.keys().cloned().collect();
        for class_name in &class_names {
            if let Some(def_fields) = self.class_defs.get(class_name.as_str()) {
                let field_llvm: Vec<String> = std::iter::once("i64".to_string())  // vtable pointer
                    .chain(def_fields.iter().map(|(_, ft)| ft.clone()))
                    .collect();
                self.e_raw(&format!("%class.{} = type {{ {} }}", class_name, field_llvm.join(", ")));
            }
        }
        self.e_raw("");

        // Emit vtable globals
        let vtable_keys: Vec<String> = class_vtable_methods.keys().cloned().collect();
        for class_name in &vtable_keys {
            if let Some(vtable_methods) = class_vtable_methods.get(class_name.as_str()) {
                if !vtable_methods.is_empty() {
                    let ptr_entries: Vec<String> = (0..vtable_methods.len()).map(|_| "ptr".to_string()).collect();
                    self.e_raw(&format!("%class.{}.vtable = type {{ {} }}", class_name, ptr_entries.join(", ")));
                }
            }
        }
        for class_name in &vtable_keys {
            if let Some(vtable_methods) = class_vtable_methods.get(class_name.as_str()) {
                if !vtable_methods.is_empty() {
                    let init_entries: Vec<String> = vtable_methods.iter()
                        .map(|(_, mfn)| format!("ptr @{}", mfn))
                        .collect();
                    self.e_raw(&format!("@vtable.{} = global %class.{}.vtable {{ {} }}", class_name, class_name, init_entries.join(", ")));
                }
            }
        }
        self.e_raw("");
        self.class_vtables = class_vtable_methods;

        self.e_raw("declare void @yk_print_int(i64)");
        self.e_raw("declare void @yk_print_real(double)");
        self.e_raw("declare void @yk_print_bool(i1)");
        self.e_raw("declare void @yk_print_str_ptr(ptr)");
        self.e_raw("declare ptr @yk_string_from_int(i64)");
        self.e_raw("declare ptr @yk_string_from_real(double)");
        self.e_raw("declare ptr @yk_string_from_bool(i1)");
        self.e_raw("declare ptr @yk_string_from_complex(ptr)");
        self.e_raw("declare ptr @yk_string_concat_ptr(ptr, ptr)");
        self.e_raw("declare i64 @yk_string_len_ptr(ptr)");
        self.e_raw("declare void @yk_complex_set(ptr, double, double)");
        self.e_raw("declare double @yk_complex_real(ptr)");
        self.e_raw("declare double @yk_complex_imag(ptr)");
        self.e_raw("declare double @yk_complex_mod(ptr)");
        self.e_raw("declare double @yk_complex_arg(ptr)");
        self.e_raw("declare void @yk_complex_conj(ptr, ptr)");
        self.e_raw("declare void @yk_complex_add(ptr, ptr, ptr)");
        self.e_raw("declare void @yk_complex_sub(ptr, ptr, ptr)");
        self.e_raw("declare void @yk_complex_mul(ptr, ptr, ptr)");
        self.e_raw("declare void @yk_complex_div(ptr, ptr, ptr)");
        self.e_raw("declare void @yk_print_complex(ptr)");
        self.e_raw("declare i64 @yk_run_thread(ptr, ptr)");
        self.e_raw("declare i64 @yk_join_thread(i64)");
        self.e_raw("declare void @yk_task_set_result(i64, i64)");
        self.e_raw("declare i64 @yk_math_abs_i64(i64)");
        self.e_raw("declare double @yk_math_abs_real(double)");
        self.e_raw("declare double @yk_math_sqrt(double)");
        self.e_raw("declare double @yk_math_sin(double)");
        self.e_raw("declare double @yk_math_cos(double)");
        self.e_raw("declare double @yk_math_floor(double)");
        self.e_raw("declare double @yk_math_ceil(double)");
        self.e_raw("declare double @yk_math_round(double)");
        self.e_raw("declare double @yk_math_pow(double, double)");
        self.e_raw("declare double @yk_math_max(double, double)");
        self.e_raw("declare double @yk_math_min(double, double)");
        self.e_raw("declare i64 @yk_math_rand(i64)");
        self.e_raw("declare ptr @yk_time_now()");
        self.e_raw("declare void @yk_time_sleep(i64)");
        self.e_raw("declare i64 @yk_time_timestamp()");
        self.e_raw("declare i64 @yk_sys_pid()");
        self.e_raw("declare void @yk_sys_exit(i64)");
        self.e_raw("declare ptr @yk_sys_cwd()");
        self.e_raw("declare ptr @yk_sys_platform()");
        self.e_raw("declare ptr @yk_sys_env(ptr)");
        self.e_raw("declare ptr @yk_path_join(ptr, ptr)");
        self.e_raw("declare ptr @yk_path_dirname(ptr)");
        self.e_raw("declare ptr @yk_path_basename(ptr)");
        self.e_raw("declare ptr @yk_path_extension(ptr)");
        self.e_raw("declare i64 @yk_path_is_absolute(ptr)");
        self.e_raw("declare ptr @yk_fs_read(ptr)");
        self.e_raw("declare void @yk_fs_write(ptr, ptr)");
        self.e_raw("declare void @yk_fs_append(ptr, ptr)");
        self.e_raw("declare void @yk_fs_remove(ptr)");
        self.e_raw("declare i64 @yk_fs_exists(ptr)");
        self.e_raw("declare i64 @yk_fs_is_dir(ptr)");
        self.e_raw("declare i64 @yk_fs_is_file(ptr)");
        self.e_raw("declare ptr @yk_base64_encode(ptr)");
        self.e_raw("declare ptr @yk_base64_decode(ptr)");
        self.e_raw("declare ptr @yk_json_string(ptr)");
        self.e_raw("declare i64 @yk_re_match(ptr, ptr)");
        self.e_raw("declare ptr @yk_re_replace(ptr, ptr, ptr)");
        self.e_raw("declare ptr @yk_fetch(ptr, ptr, ptr)");
        self.e_raw("declare i64 @yk_server_new()");
        self.e_raw("declare void @yk_server_add_route(i64, ptr, ptr, i64)");
        self.e_raw("declare void @yk_server_serve(i64, ptr)");
        self.e_raw("declare ptr @yk_list_new()");
        self.e_raw("declare void @yk_list_push(ptr, i64)");
        self.e_raw("declare i64 @yk_list_get(ptr, i64)");
        self.e_raw("declare i64 @yk_list_len(ptr)");
        self.e_raw("declare i64 @yk_list_pop(ptr)");
        self.e_raw("declare void @yk_list_print(ptr)");
        self.e_raw("declare void @yk_list_sort(ptr)");
        self.e_raw("declare void @yk_list_reverse(ptr)");
        self.e_raw("declare void @yk_list_insert(ptr, i64, i64)");
        self.e_raw("declare void @yk_list_remove(ptr, i64)");
        self.e_raw("declare void @yk_list_clear(ptr)");
        self.e_raw("declare void @yk_print_result_val(i64, i1)");
        self.e_raw("declare i64 @yk_result_str_new(i64, i64)");
        self.e_raw("declare ptr @yk_datetime_now()");
        self.e_raw("declare ptr @yk_datetime_utc()");
        self.e_raw("declare i64 @yk_datetime_year(i64)");
        self.e_raw("declare i64 @yk_datetime_month(i64)");
        self.e_raw("declare i64 @yk_datetime_day(i64)");
        self.e_raw("declare i64 @yk_datetime_hour(i64)");
        self.e_raw("declare i64 @yk_datetime_minute(i64)");
        self.e_raw("declare i64 @yk_datetime_second(i64)");
        self.e_raw("declare ptr @yk_datetime_format(i64, ptr)");
        self.e_raw("");

        let has_main = modules.iter().any(|m| m.items.iter().any(|item| matches!(&item.value, ItemKind::Fn { name, .. } if name == "main")));

        // Pre-scan all function return types so calls can resolve them before the callee is compiled
        for module in modules.iter() {
            for item in &module.items {
                if let ItemKind::Fn { name, ret_type, .. } = &item.value {
                    let mangled = self.mangle_name(name);
            let ret = ret_type.as_ref().map(|t| self.type_to_llvm(&t.value)).unwrap_or_else(|| "void".into());
            self.current_fn_ret = ret.clone();
                    self.fn_ret_types.insert(mangled, ret);
                }
            }
        }

        for module in modules {
            self.current_module = module.name.clone();
            for item in &module.items {
                if let ItemKind::Fn { name, .. } = &item.value {
                    if name != "main" {
                        self.compile_fn(item);
                    }
                }
            }
            // Compile class methods
            for item in &module.items {
                if let ItemKind::Class { name, methods, .. } = &item.value {
                    for m in methods {
                        self.compile_class_method(name, m, &module.name);
                    }
                }
            }
        }
        self.current_module = String::new();

        if has_main {
            let saved_types = self.var_types.clone();
            let saved_alloca = self.var_alloca.clone();
            let main_item = modules.iter()
                .flat_map(|m| &m.items)
                .find(|item| matches!(&item.value, ItemKind::Fn { name, .. } if name == "main"))
                .unwrap();
            if let ItemKind::Fn { params, body, .. } = &main_item.value {
                self.current_fn_ret = "i32".into();
                self.e_raw("define i32 @main(i32 %argc, ptr %argv) {");
                self.indent += 1;
                let entry_ptr = self.fresh_label();
                self.e(&format!("%{} = alloca i32, align 4", entry_ptr));
                self.e(&format!("store i32 %argc, ptr %{}", entry_ptr));
                self.var_types.insert("argc".into(), "i32".into());
                self.var_alloca.insert("argc".into(), format!("%{}", entry_ptr));
                for p in params {
                    let ty = self.type_to_llvm(&p.type_expr.value);
                    let ptr = self.alloca_name(&p.name);
                    self.var_types.insert(p.name.clone(), ty);
                    self.var_alloca.insert(p.name.clone(), ptr.clone());
                }
                self.compile_fn_body(body);
                self.e("ret i32 0");
                self.indent -= 1;
                self.e_raw("}");
            }
            self.var_types = saved_types;
            self.var_alloca = saved_alloca;
        }

        // Emit deferred closures (anonymous functions)
        let deferred = std::mem::take(&mut self.deferred_fns);
        for (name, params, ret_type, body) in deferred {
            let ret = ret_type.as_ref().map(|t| self.type_to_llvm(&t.value)).unwrap_or_else(|| "i64".into());
            self.current_fn_ret = ret.clone();
            let param_types: Vec<String> = params.iter().map(|p| self.type_to_llvm(&p.type_expr.value)).collect();
            let param_list = param_types.join(", ");
            self.e_raw(&format!("define {} @{}({}) {{", ret, name, param_list));
            self.indent += 1;
            for (i, p) in params.iter().enumerate() {
                let ty = self.type_to_llvm(&p.type_expr.value);
                let ptr = self.alloca_name(&p.name);
                self.var_types.insert(p.name.clone(), ty.clone());
                self.var_alloca.insert(p.name.clone(), ptr.clone());
                self.e(&format!("{} = alloca {}, align 8", ptr, ty));
                self.e(&format!("store {} %{}, ptr {}", ty, i, ptr));
            }
            let body_stmts = vec![
                StmtNode::new(0, Span::new(0, 0), Stmt::Return(Some(body)))
            ];
            self.compile_fn_body(&body_stmts);
            if ret != "void" {
                if ret == "%yk_string" {
                    self.e("ret %yk_string zeroinitializer");
                } else if ret == "double" {
                    self.e("ret double 0.0");
                } else if ret == "i1" {
                    self.e("ret i1 false");
                } else {
                    self.e(&format!("ret {} 0", ret));
                }
            }
            self.indent -= 1;
            self.e_raw("}");
            self.e_raw("");
            self.var_types.clear();
            self.var_alloca.clear();
        }

        // Emit spawn wrappers
        let wrappers = std::mem::take(&mut self.spawn_wrappers);
        for (name, _slot, expr) in wrappers {
            self.e_raw(&format!("define void @{}(ptr %slot) {{", name));
            self.indent += 1;
            let (val, _ty) = self.compile_expr(&expr);
            self.e(&format!("store i64 {}, ptr %slot", val));
            self.e("ret void");
            self.indent -= 1;
            self.e_raw("}");
            self.e_raw("");
            self.var_types.clear();
            self.var_alloca.clear();
        }

        // Emit handler IRs (for net server handlers)
        for hir in &self.handler_irs {
            self.output.push_str(hir);
            self.output.push('\n');
        }

        let mut output = std::mem::take(&mut self.output);
        output.push_str(&self.string_constants);

        let type_defs = std::mem::take(&mut self.tuple_types_output);
        if !type_defs.is_empty() {
            let mut prefix = String::new();
            for td in &type_defs {
                prefix.push_str(td);
                prefix.push('\n');
            }
            prefix.push('\n');
            // Insert tuple type defs after %yk_string and before the rest
            if let Some(pos) = output.find("%yk_string = type { ptr, i64 }") {
                let after = pos + "%yk_string = type { ptr, i64 }".len();
                output.insert_str(after, &format!("\n{}", prefix));
            }
        }

        output
    }

    fn compile_fn(&mut self, item: &ItemNode) {
        let saved_types = self.var_types.clone();
        let saved_alloca = self.var_alloca.clone();
        if let ItemKind::Fn { name, params, ret_type, body, .. } = &item.value {
            let mangled = self.mangle_name(name);
            let ret = ret_type.as_ref().map(|t| self.type_to_llvm(&t.value)).unwrap_or_else(|| "void".into());
            self.current_fn_ret = ret.clone();
            self.fn_ret_types.insert(mangled.clone(), ret.clone());
            let param_types: Vec<String> = params.iter().map(|p| self.type_to_llvm(&p.type_expr.value)).collect();
            let param_list = param_types.join(", ");
            self.e_raw(&format!("define {} @{}({}) {{", ret, mangled, param_list));
            self.indent += 1;

            for (i, p) in params.iter().enumerate() {
                let ty = self.type_to_llvm(&p.type_expr.value);
                let ptr = self.alloca_name(&p.name);
                self.var_types.insert(p.name.clone(), ty.clone());
                self.var_alloca.insert(p.name.clone(), ptr.clone());
                self.e(&format!("{} = alloca {}, align 8", ptr, ty));
                self.e(&format!("store {} %{}, ptr {}", ty, i, ptr));
            }

            self.compile_fn_body(body);

            // Only emit default return if the body didn't already have one
            let body_ends_with_return = body.last().map_or(false, |s| matches!(&s.value, Stmt::Return(_)));
            if !body_ends_with_return {
                if ret == "void" {
                    self.e("ret void");
                } else if ret.starts_with('%') {
                    self.e(&format!("ret {} zeroinitializer", ret));
                } else {
                    self.e(&format!("ret {} 0", ret));
                }
            }

            self.indent -= 1;
            self.e_raw("}");
            self.e_raw("");
        }
        self.var_types = saved_types;
        self.var_alloca = saved_alloca;
    }

    fn compile_fn_body(&mut self, body: &[StmtNode]) {
        for stmt in body {
            self.compile_stmt(stmt);
        }
    }

    fn compile_class_method(&mut self, class_name: &str, method: &ItemKind, module_name: &str) {
        if let ItemKind::Fn { name: mname, params, ret_type, body, .. } = method {
            let mangled = if module_name.is_empty() {
                format!("__method_{}_{}", class_name, mname)
            } else {
                format!("__method_{}_{}_{}", module_name, class_name, mname)
            };
            let ret = ret_type.as_ref().map(|t| self.type_to_llvm(&t.value)).unwrap_or_else(|| "void".into());
            self.class_method_ret_types.insert((class_name.to_string(), mname.clone()), ret.clone());
            let saved_types = self.var_types.clone();
            let saved_alloca = self.var_alloca.clone();

            let has_self = params.first().map(|p| p.name == "self").unwrap_or(false);
            // Build param types: self is i64, others are as declared
            let mut param_types: Vec<String> = Vec::new();
            for (i, p) in params.iter().enumerate() {
                if i == 0 && has_self {
                    param_types.push("i64".into());
                } else {
                    param_types.push(self.type_to_llvm(&p.type_expr.value));
                }
            }
            let param_list = param_types.join(", ");
            self.e_raw(&format!("define {} @{}({}) {{", ret, mangled, param_list));
            self.indent += 1;

            for (i, p) in params.iter().enumerate() {
                if i == 0 && has_self {
                    let class_ty = format!("%class.{}", class_name);
                    let uptr = self.fresh_label();
                    self.e(&format!("%{} = inttoptr i64 %0 to ptr", uptr));
                    let loaded = self.fresh_label();
                    self.e(&format!("%{} = load {}, ptr %{}", loaded, class_ty, uptr));
                    let ptr = self.alloca_name(&p.name);
                    self.var_types.insert(p.name.clone(), class_ty.clone());
                    self.var_alloca.insert(p.name.clone(), ptr.clone());
                    self.e(&format!("{} = alloca {}, align 8", ptr, class_ty));
                    self.e(&format!("store {} %{}, ptr {}", class_ty, loaded, ptr));
                } else {
                    let ty = self.type_to_llvm(&p.type_expr.value);
                    let ptr = self.alloca_name(&p.name);
                    self.var_types.insert(p.name.clone(), ty.clone());
                    self.var_alloca.insert(p.name.clone(), ptr.clone());
                    self.e(&format!("{} = alloca {}, align 8", ptr, ty));
                    self.e(&format!("store {} %{}, ptr {}", ty, i, ptr));
                }
            }

            self.compile_fn_body(body);

            if ret == "void" {
                self.e("ret void");
            } else if ret == "%yk_string" {
                self.e("ret %yk_string zeroinitializer");
            } else if ret == "double" {
                self.e("ret double 0.0");
            } else if ret == "i1" {
                self.e("ret i1 false");
            } else {
                self.e(&format!("ret {} 0", ret));
            }

            self.var_types = saved_types;
            self.var_alloca = saved_alloca;
            self.indent -= 1;
            self.e_raw("}");
            self.e_raw("");
        }
    }

    fn compile_stmt(&mut self, stmt: &StmtNode) {
        match &stmt.value {
            Stmt::Decl { name, type_expr, value, .. } => {
                let ty = match type_expr {
                    Some(te) => self.type_to_llvm(&te.value),
                    None => self.expr_type_str(value),
                };
                let (val, val_ty) = self.compile_expr(value);
                let effective_ty = if ty == "i64" && val_ty != "i64" { val_ty.clone() } else { ty.clone() };
                let ptr = self.alloca_name(name);
                self.var_types.insert(name.clone(), effective_ty.clone());
                self.var_alloca.insert(name.clone(), ptr.clone());
                self.e(&format!("{} = alloca {}, align 8", ptr, effective_ty));

                if Self::is_nullable_ty(&effective_ty) && !Self::is_nullable_ty(&val_ty) {
                    let is_null = matches!(&value.value, Expr::LitNull | Expr::LitNone);
                    let wrapped = self.wrap_in_nullable(&effective_ty, &val, &val_ty, is_null);
                    self.e(&format!("store {} {}, ptr {}", effective_ty, wrapped, ptr));
                } else if ty == "i1" && val_ty == "i64" {
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = icmp ne i64 {}, 0", tmp, val));
                    self.e(&format!("store i1 %{}, ptr {}", tmp, ptr));
                } else if ty == "i64" && val_ty == "i1" {
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = zext i1 {} to i64", tmp, val));
                    self.e(&format!("store i64 %{}, ptr {}", tmp, ptr));
                } else if ty != val_ty && ty == "i64" && val_ty.starts_with("i") {
                    let bits: u32 = val_ty[1..].parse().unwrap_or(1);
                    let tmp = self.fresh_label();
                    if bits <= 1 {
                        self.e(&format!("%{} = zext {} {} to i64", tmp, val_ty, val));
                    } else {
                        self.e(&format!("%{} = sext {} {} to i64", tmp, val_ty, val));
                    }
                    self.e(&format!("store i64 %{}, ptr {}", tmp, ptr));
                } else {
                    self.e(&format!("store {} {}, ptr {}", val_ty, val, ptr));
                }
            }
            Stmt::Assign(name, expr) => {
                let val_ty = self.expr_type_str(expr);
                let ptr = match self.var_alloca.get(name) {
                    Some(p) => p.clone(),
                    None => {
                        let p = self.alloca_name(name);
                        self.e(&format!("{} = alloca {}, align 8", p, val_ty));
                        self.var_alloca.insert(name.clone(), p.clone());
                        self.var_types.insert(name.clone(), val_ty.clone());
                        p
                    }
                };
                let ty = self.val_ty(name);
                let (val, val_ty2) = self.compile_expr(expr);
                let val_ty = if val_ty2 != "i64" { val_ty2 } else { val_ty };
                if Self::is_nullable_ty(&ty) && !Self::is_nullable_ty(&val_ty) {
                    let is_null = matches!(&expr.value, Expr::LitNull | Expr::LitNone);
                    let wrapped = self.wrap_in_nullable(&ty, &val, &val_ty, is_null);
                    self.e(&format!("store {} {}, ptr {}", ty, wrapped, ptr));
                } else if !ty.is_empty() && ty != val_ty {
                    if ty == "i1" && val_ty == "i64" {
                        let tmp = self.fresh_label();
                        self.e(&format!("%{} = icmp ne i64 {}, 0", tmp, val));
                        self.e(&format!("store i1 %{}, ptr {}", tmp, ptr));
                    } else {
                        self.e(&format!("store {} {}, ptr {}", val_ty, val, ptr));
                    }
                } else {
                    self.e(&format!("store {} {}, ptr {}", val_ty, val, ptr));
                }
            }
            Stmt::Expr(e) => {
                self.compile_expr(e);
            }
            Stmt::Return(e) => {
                match e {
                    Some(ex) => {
                        let (val, val_ty) = self.compile_expr(ex);
                        if Self::is_nullable_ty(&self.current_fn_ret) && !Self::is_nullable_ty(&val_ty) {
                            let is_null = matches!(&ex.value, Expr::LitNull | Expr::LitNone);
                            let fn_ret = self.current_fn_ret.clone();
                            let wrapped = self.wrap_in_nullable(&fn_ret, &val, &val_ty, is_null);
                            self.e(&format!("ret {} {}", fn_ret, wrapped));
                        } else {
                            self.e(&format!("ret {} {}", val_ty, val));
                        }
                    }
                    None => self.e("ret void"),
                }
            }
            Stmt::If(cond, then_body, else_body) => {
                let then_label = self.fresh_label();
                let else_label = self.fresh_label();
                let merge_label = self.fresh_label();

                let (cond_val, _) = self.compile_expr(cond);

                self.e(&format!("br i1 {}, label %{}, label %{}", cond_val, then_label, else_label));
                self.e_raw(&format!("{}:", then_label));
                self.indent += 1;
                for s in then_body { self.compile_stmt(s); }
                self.e(&format!("br label %{}", merge_label));
                self.indent -= 1;

                self.e_raw(&format!("{}:", else_label));
                self.indent += 1;
                if let Some(eb) = else_body {
                    for s in eb { self.compile_stmt(s); }
                }
                self.e(&format!("br label %{}", merge_label));
                self.indent -= 1;

                self.e_raw(&format!("{}:", merge_label));
            }
            Stmt::While(cond, body) => {
                let head_label = self.fresh_label();
                let body_label = self.fresh_label();
                let exit_label = self.fresh_label();

                self.e(&format!("br label %{}", head_label));
                self.e_raw(&format!("{}:", head_label));
                let (cond_val, _) = self.compile_expr(cond);
                self.e(&format!("br i1 {}, label %{}, label %{}", cond_val, body_label, exit_label));

                self.e_raw(&format!("{}:", body_label));
                self.indent += 1;
                for s in body { self.compile_stmt(s); }
                self.e(&format!("br label %{}", head_label));
                self.indent -= 1;

                self.e_raw(&format!("{}:", exit_label));
            }
            Stmt::For(var, iter, body, _is_for_of) => {
                let iter_ty = self.expr_type_str(iter);
                if iter_ty == "%yk_string" {
                    // String iteration
                    let (str_val, _) = self.compile_expr(iter);
                    let head_label = self.fresh_label();
                    let body_label = self.fresh_label();
                    let exit_label = self.fresh_label();

                    let idx_ptr = self.alloca_name(&format!("{}_idx", var));
                    self.var_types.insert(format!("{}_idx", var), "i64".into());
                    self.var_alloca.insert(format!("{}_idx", var), idx_ptr.clone());
                    self.e(&format!("{} = alloca i64, align 8", idx_ptr));
                    self.e(&format!("store i64 0, ptr {}", idx_ptr));

                    let len = self.fresh_label();
                    self.e(&format!("%{} = extractvalue %yk_string {}, 1", len, str_val));
                    self.e(&format!("br label %{}", head_label));
                    self.e_raw(&format!("{}:", head_label));
                    let idx_val = self.value_name(&format!("{}_idx", var));
                    self.e(&format!("{} = load i64, ptr {}", idx_val, idx_ptr));
                    let cmp = self.fresh_label();
                    self.e(&format!("%{} = icmp slt i64 {}, %{}", cmp, idx_val, len));
                    self.e(&format!("br i1 %{}, label %{}, label %{}", cmp, body_label, exit_label));

                    self.e_raw(&format!("{}:", body_label));
                    self.indent += 1;
                    // Extract character at index
                    let data_ptr_label = self.fresh_label();
                    self.e(&format!("%{} = extractvalue %yk_string {}, 0", data_ptr_label, str_val));
                    let ch_ptr = self.fresh_label();
                    self.e(&format!("%{} = getelementptr inbounds i8, ptr %{}, i64 {}", ch_ptr, data_ptr_label, idx_val));
                    let ch_val = self.fresh_label();
                    self.e(&format!("%{} = load i8, ptr %{}", ch_val, ch_ptr));
                    // Store char in loop variable (as i64)
                    let ch_ext = self.fresh_label();
                    self.e(&format!("%{} = zext i8 %{} to i64", ch_ext, ch_val));
                    let ptr = self.alloca_name(var);
                    self.var_types.insert(var.clone(), "i64".into());
                    self.var_alloca.insert(var.clone(), ptr.clone());
                    self.e(&format!("{} = alloca i64, align 8", ptr));
                    self.e(&format!("store i64 %{}, ptr {}", ch_ext, ptr));
                    for s in body { self.compile_stmt(s); }
                    let next_idx = self.fresh_label();
                    self.e(&format!("%{} = add i64 {}, 1", next_idx, idx_val));
                    self.e(&format!("store i64 %{}, ptr {}", next_idx, idx_ptr));
                    self.e(&format!("br label %{}", head_label));
                    self.indent -= 1;
                    self.e_raw(&format!("{}:", exit_label));
                } else {
                    // Integer range iteration (existing behavior)
                    let init_label = self.fresh_label();
                    let cond_label = self.fresh_label();
                    let body_label = self.fresh_label();
                    let exit_label = self.fresh_label();

                    let (end_val, _) = self.compile_expr(iter);

                    self.e(&format!("br label %{}", init_label));
                    self.e_raw(&format!("{}:", init_label));

                    let ptr = self.alloca_name(var);
                    self.var_types.insert(var.clone(), "i64".into());
                    self.var_alloca.insert(var.clone(), ptr.clone());
                    self.e(&format!("{} = alloca i64, align 8", ptr));
                    self.e(&format!("store i64 0, ptr {}", ptr));

                    self.e(&format!("br label %{}", cond_label));
                    self.e_raw(&format!("{}:", cond_label));
                    let v = self.value_name(var);
                    self.e(&format!("{} = load i64, ptr {}", v, ptr));
                    self.e(&format!("%cmp_{} = icmp slt i64 {}, {}", var, v, end_val));
                    self.e(&format!("br i1 %cmp_{}, label %{}, label %{}", var, body_label, exit_label));

                    self.e_raw(&format!("{}:", body_label));
                    self.indent += 1;
                    for s in body { self.compile_stmt(s); }
                    let next_v = self.fresh_label();
                    self.e(&format!("%{} = add i64 {}, 1", next_v, v));
                    self.e(&format!("store i64 %{}, ptr {}", next_v, ptr));

                    self.e(&format!("br label %{}", cond_label));
                    self.indent -= 1;
                    self.e_raw(&format!("{}:", exit_label));
                }
            }
            Stmt::Loop(body) => {
                let loop_label = self.fresh_label();
                self.e(&format!("br label %{}", loop_label));
                self.e_raw(&format!("{}:", loop_label));
                self.indent += 1;
                for s in body { self.compile_stmt(s); }
                self.e(&format!("br label %{}", loop_label));
                self.indent -= 1;
            }
            Stmt::Destruct(_, expr) => {
                self.compile_expr(expr);
            }
        }
    }

    fn compile_expr(&mut self, expr: &ExprNode) -> (String, String) {
        match &expr.value {
            Expr::LitInt(n) => (n.to_string(), "i64".into()),
            Expr::LitHex(n) => (n.to_string(), "i64".into()),
            Expr::LitReal(n) => {
                let s = n.to_string();
                if s.contains('.') || s.contains('e') || s.contains('E') { (s, "double".into()) }
                else { (format!("{}.0", s), "double".into()) }
            }
            Expr::LitBool(true) => ("true".into(), "i1".into()),
            Expr::LitBool(false) => ("false".into(), "i1".into()),
            Expr::LitStr(s) => (self.make_string_slot(s), "%yk_string".into()),
            Expr::LitChar(c) => (format!("{}", *c as i64), "i64".into()),
            Expr::LitNull | Expr::LitNone => ("0".into(), "i64".into()),
            Expr::LitSymbol(s) => (self.make_string_slot(&format!(":{}", s)), "%yk_string".into()),
            Expr::Ident(name) => {
                let ptr_opt = self.var_alloca.get(name).cloned();
                let ty = self.val_ty(name);
                if let Some(ptr) = ptr_opt {
                    let val_name = self.value_name(name);
                    self.e(&format!("{} = load {}, ptr {}", val_name, ty, ptr));
                    (val_name, ty)
                } else {
                    (format!("%{}", name), ty)
                }
            }
            Expr::BinOp(l, op, r) => self.compile_binop(l, op, r),
            Expr::UnOp(op, inner) => {
                let (i, ty) = self.compile_expr(inner);
                let tmp = self.fresh_label();
                match op {
                    UnOp::Neg => {
                        if ty == "double" {
                            self.e(&format!("%{} = fsub double -0.0, {}", tmp, i));
                        } else {
                            self.e(&format!("%{} = sub {} 0, {}", tmp, ty, i));
                        }
                        (self.ssa(&tmp), ty)
                    }
                    UnOp::Not => {
                        if ty == "i1" {
                            self.e(&format!("%{} = xor i1 {}, true", tmp, i));
                        } else {
                            self.e(&format!("%{} = icmp eq i64 {}, 0", tmp, i));
                        }
                        (self.ssa(&tmp), "i1".into())
                    }
                }
            }
            Expr::Call(callee, args) => self.compile_call(callee, args),
            Expr::Field(obj, field) => {
                let (o, obj_ty) = self.compile_expr(obj);
                let tmp = self.fresh_label();
                if obj_ty == "%yk_string" {
                    let idx = if field == "data" { "0" } else { "1" };
                    self.e(&format!("%{} = extractvalue %yk_string {}, {}", tmp, o, idx));
                    if idx == "0" { (self.ssa(&tmp), "ptr".into()) } else { (self.ssa(&tmp), "i64".into()) }
                } else if let Some(struct_name) = obj_ty.strip_prefix("%struct.") {
                    let (index, field_ty): (Option<usize>, String) = if self.tuple_type_names.values().any(|n| n == &obj_ty) {
                        (field.parse().ok(), "i64".into())
                    } else {
                        let def = self.struct_defs.get(struct_name).and_then(|def_fields| {
                            def_fields.iter().enumerate().find(|(_, (n, _))| n == field)
                        });
                        match def {
                            Some((i, (_, ft))) => (Some(i), ft.clone()),
                            None => (None, "i64".into()),
                        }
                    };
                    if let Some(idx) = index {
                        self.e(&format!("%{} = extractvalue {} {}, {}", tmp, obj_ty, o, idx));
                        (self.ssa(&tmp), field_ty)
                    } else {
                        (self.ssa(&tmp), "i64".into())
                    }
                } else if let Some(class_name) = obj_ty.strip_prefix("%class.") {
                    let (index, field_ty) = self.class_defs.get(class_name).and_then(|def_fields| {
                        def_fields.iter().enumerate().find(|(_, (n, _))| n == field)
                            .map(|(i, (_, ft))| (Some(i + 1), ft.clone()))
                    }).unwrap_or((None, "i64".into()));
                    if let Some(idx) = index {
                        self.e(&format!("%{} = extractvalue {} {}, {}", tmp, obj_ty, o, idx));
                        (self.ssa(&tmp), field_ty)
                    } else {
                        (self.ssa(&tmp), "i64".into())
                    }
                } else if obj_ty == "%yk_complex" {
                    // Store complex value to alloca to get a pointer for runtime
                    let ca = self.fresh_label();
                    self.e(&format!("%{} = alloca %yk_complex, align 8", ca));
                    self.e(&format!("store %yk_complex {}, ptr %{}", o, ca));
                    if field == "conj" {
                        let r = self.fresh_label();
                        self.e(&format!("%{} = alloca %yk_complex, align 8", r));
                        self.e(&format!("call void @yk_complex_conj(ptr %{}, ptr %{})", r, ca));
                        let loaded = self.fresh_label();
                        self.e(&format!("%{} = load %yk_complex, ptr %{}", loaded, r));
                        return (self.ssa(&loaded), "%yk_complex".into());
                    }
                    let func = match field.as_str() {
                        "real" => "yk_complex_real",
                        "img" => "yk_complex_imag",
                        "mod" | "norm" => "yk_complex_mod",
                        "arg" => "yk_complex_arg",
                        _ => "yk_complex_real",
                    };
                    self.e(&format!("%{} = call double @{}(ptr %{})", tmp, func, ca));
                    (self.ssa(&tmp), "double".into())
                } else {
                    (self.ssa(&tmp), "i64".into())
                }
            }
            Expr::Index(obj, index) => {
                let (o, ot) = self.compile_expr(obj);
                let (i, _) = self.compile_expr(index);
                if ot == "ptr" {
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = call i64 @yk_list_get(ptr {}, i64 {})", tmp, o, i));
                    (self.ssa(&tmp), "i64".into())
                } else {
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = getelementptr inbounds i64, ptr {}, i64 {}", tmp, o, i));
                    let tmp2 = self.fresh_label();
                    self.e(&format!("%{} = load i64, ptr %{}", tmp2, tmp));
                    (self.ssa(&tmp2), "i64".into())
                }
            }
            Expr::Range(l, r) => {
                let (_lv, _) = self.compile_expr(l);
                let (rv, _) = self.compile_expr(r);
                (rv, "i64".into())
            }
            Expr::Block(stmts) => {
                let ret_ptr = self.fresh_label();
                self.e(&format!("%{} = alloca i64, align 8", ret_ptr));
                self.e(&format!("store i64 0, ptr %{}", ret_ptr));

                let old_in_block = self.in_block;
                self.in_block = true;

                for s in stmts {
                    match &s.value {
                        Stmt::Return(e) => {
                            let (val, ty) = match e {
                                Some(ex) => self.compile_expr(ex),
                                None => ("0".into(), "i64".into()),
                            };
                            self.e(&format!("store {} {}, ptr %{}", ty, val, ret_ptr));
                            let end_lbl = self.fresh_label();
                            self.e(&format!("br label %{}", end_lbl));
                            self.e_raw(&format!("{}:", end_lbl));
                        }
                        _ => self.compile_stmt(s),
                    }
                }

                self.in_block = old_in_block;

                let load_lbl = self.fresh_label();
                self.e(&format!("%{} = load i64, ptr %{}", load_lbl, ret_ptr));
                (self.ssa(&load_lbl), "i64".into())
            }
            Expr::AsConst(inner) => self.compile_expr(inner),
            Expr::If(cond, then_e, else_e) => {
                let then_label = self.fresh_label();
                let else_label = self.fresh_label();
                let merge_label = self.fresh_label();

                let (cond_val, _) = self.compile_expr(cond);
                self.e(&format!("br i1 {}, label %{}, label %{}", cond_val, then_label, else_label));

                self.e_raw(&format!("{}:", then_label));
                self.indent += 1;
                let (t_val, t_ty) = self.compile_expr(then_e);
                self.e(&format!("br label %{}", merge_label));
                self.indent -= 1;

                self.e_raw(&format!("{}:", else_label));
                self.indent += 1;
                let (e_val, _e_ty) = match else_e {
                    Some(ex) => self.compile_expr(ex),
                    None => ("0".into(), "i64".into()),
                };
                self.e(&format!("br label %{}", merge_label));
                self.indent -= 1;

                self.e_raw(&format!("{}:", merge_label));
                let result = self.fresh_label();
                self.e(&format!("%{} = phi {} [ {}, %{} ], [ {}, %{} ]", result, t_ty, t_val, then_label, e_val, else_label));
                (self.ssa(&result), t_ty)
            }
            Expr::ListLit(items) => {
                let list_ptr = self.fresh_label();
                self.e(&format!("%{} = call ptr @yk_list_new()", list_ptr));
                for item in items {
                    let (val, typ) = self.compile_expr(item);
                    let push_val = match typ.as_str() {
                        "double" => {
                            let b = self.fresh_label();
                            self.e(&format!("%{} = bitcast double {} to i64", b, val));
                            self.ssa(&b)
                        }
                        "i1" => {
                            let b = self.fresh_label();
                            self.e(&format!("%{} = zext i1 {} to i64", b, val));
                            self.ssa(&b)
                        }
                        "%yk_string" => {
                            let p = self.string_to_ptr(&val);
                            let b = self.fresh_label();
                            self.e(&format!("%{} = ptrtoint ptr {} to i64", b, p));
                            self.ssa(&b)
                        }
                        "%yk_complex" => {
                            let slot = self.fresh_label();
                            self.e(&format!("%{} = alloca %yk_complex, align 8", slot));
                            self.e(&format!("store %yk_complex {}, ptr %{}", val, slot));
                            let b = self.fresh_label();
                            self.e(&format!("%{} = ptrtoint ptr %{} to i64", b, slot));
                            self.ssa(&b)
                        }
                        _ => val,
                    };
                    self.e(&format!("call void @yk_list_push(ptr %{}, i64 {})", list_ptr, push_val));
                }
                (self.ssa(&list_ptr), "ptr".into())
            }
            Expr::StructLit(name, fields) => {
                let class_ty = format!("%class.{}", name);
                if self.class_defs.contains_key(name.as_str()) {
                    let mut agg = "undef".to_string();
                    // Set vtable pointer (index 0)
                    if let Some(vtable_methods) = self.class_vtables.get(name.as_str()) {
                        if !vtable_methods.is_empty() {
                            let vt = self.fresh_label();
                            self.e(&format!("%{} = ptrtoint %class.{}.vtable* @vtable.{} to i64", vt, name, name));
                            let tmp = self.fresh_label();
                            self.e(&format!("%{} = insertvalue {} {}, i64 %{}, 0", tmp, class_ty, agg, vt));
                            agg = self.ssa(&tmp);
                        }
                    }
                    let defs = self.class_defs.get(name.as_str()).cloned();
                    if let Some(def_fields) = defs {
                        for (idx, (fname, _fty)) in def_fields.iter().enumerate() {
                            if let Some((_, fexpr)) = fields.iter().find(|(n, _)| n == fname) {
                                let (fval, fty) = self.compile_expr(fexpr);
                                let tmp = self.fresh_label();
                                self.e(&format!("%{} = insertvalue {} {}, {} {}, {}", tmp, class_ty, agg, fty, fval, idx + 1));
                                agg = self.ssa(&tmp);
                            }
                        }
                    }
                    (agg, class_ty)
                } else {
                    let struct_ty = format!("%struct.{}", name);
                    let mut agg = "undef".to_string();
                    let defs = self.struct_defs.get(name.as_str()).cloned();
                    if let Some(def_fields) = defs {
                        for (idx, (fname, _fty)) in def_fields.iter().enumerate() {
                            if let Some((_, fexpr)) = fields.iter().find(|(n, _)| n == fname) {
                                let (fval, fty) = self.compile_expr(fexpr);
                                let tmp = self.fresh_label();
                                self.e(&format!("%{} = insertvalue {} {}, {} {}, {}", tmp, struct_ty, agg, fty, fval, idx));
                                agg = self.ssa(&tmp);
                            }
                        }
                    }
                    (agg, struct_ty)
                }
            }
            Expr::TupleLit(items) => {
                let elem_types: Vec<String> = items.iter().map(|i| self.expr_type_str(i)).collect();
                let ty = self.get_or_create_tuple_type(&elem_types);
                let mut agg = "undef".to_string();
                for (idx, item) in items.iter().enumerate() {
                    let (fval, fty) = self.compile_expr(item);
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = insertvalue {} {}, {} {}, {}", tmp, ty, agg, fty, fval, idx));
                    agg = self.ssa(&tmp);
                }
                (agg, ty)
            }
            Expr::MapLit(pairs) => {
                let list_ptr = self.fresh_label();
                self.e(&format!("%{} = call ptr @yk_list_new()", list_ptr));
                for (k, v) in pairs {
                    let (kv, kt) = self.compile_expr(k);
                    let kp = match kt.as_str() {
                        "double" => { let b=self.fresh_label(); self.e(&format!("%{b} = bitcast double {kv} to i64", b=b, kv=kv)); self.ssa(&b) }
                        "i1" => { let b=self.fresh_label(); self.e(&format!("%{b} = zext i1 {kv} to i64", b=b, kv=kv)); self.ssa(&b) }
                        "%yk_string" => { let p=self.string_to_ptr(&kv); let b=self.fresh_label(); self.e(&format!("%{b} = ptrtoint ptr {p} to i64", b=b, p=p)); self.ssa(&b) }
                        "%yk_complex" => { let s=self.fresh_label(); self.e(&format!("%{s} = alloca %yk_complex, align 8", s=s)); self.e(&format!("store %yk_complex {kv}, ptr %{s}", kv=kv, s=s)); let b=self.fresh_label(); self.e(&format!("%{b} = ptrtoint ptr %{s} to i64", b=b, s=s)); self.ssa(&b) }
                        _ => kv,
                    };
                    self.e(&format!("call void @yk_list_push(ptr %{}, i64 {})", list_ptr, kp));
                    let (vv, vt) = self.compile_expr(v);
                    let vp = match vt.as_str() {
                        "double" => { let b=self.fresh_label(); self.e(&format!("%{b} = bitcast double {vv} to i64", b=b, vv=vv)); self.ssa(&b) }
                        "i1" => { let b=self.fresh_label(); self.e(&format!("%{b} = zext i1 {vv} to i64", b=b, vv=vv)); self.ssa(&b) }
                        "%yk_string" => { let p=self.string_to_ptr(&vv); let b=self.fresh_label(); self.e(&format!("%{b} = ptrtoint ptr {p} to i64", b=b, p=p)); self.ssa(&b) }
                        "%yk_complex" => { let s=self.fresh_label(); self.e(&format!("%{s} = alloca %yk_complex, align 8", s=s)); self.e(&format!("store %yk_complex {vv}, ptr %{s}", vv=vv, s=s)); let b=self.fresh_label(); self.e(&format!("%{b} = ptrtoint ptr %{s} to i64", b=b, s=s)); self.ssa(&b) }
                        _ => vv,
                    };
                    self.e(&format!("call void @yk_list_push(ptr %{}, i64 {})", list_ptr, vp));
                }
                (self.ssa(&list_ptr), "ptr".into())
            }
            Expr::SetLit(items) => {
                let list_ptr = self.fresh_label();
                self.e(&format!("%{} = call ptr @yk_list_new()", list_ptr));
                for item in items {
                    let (val, typ) = self.compile_expr(item);
                    let push_val = match typ.as_str() {
                        "double" => {
                            let b = self.fresh_label();
                            self.e(&format!("%{} = bitcast double {} to i64", b, val));
                            self.ssa(&b)
                        }
                        "i1" => {
                            let b = self.fresh_label();
                            self.e(&format!("%{} = zext i1 {} to i64", b, val));
                            self.ssa(&b)
                        }
                        "%yk_string" => {
                            let p = self.string_to_ptr(&val);
                            let b = self.fresh_label();
                            self.e(&format!("%{} = ptrtoint ptr {} to i64", b, p));
                            self.ssa(&b)
                        }
                        "%yk_complex" => {
                            let slot = self.fresh_label();
                            self.e(&format!("%{} = alloca %yk_complex, align 8", slot));
                            self.e(&format!("store %yk_complex {}, ptr %{}", val, slot));
                            let b = self.fresh_label();
                            self.e(&format!("%{} = ptrtoint ptr %{} to i64", b, slot));
                            self.ssa(&b)
                        }
                        _ => val,
                    };
                    self.e(&format!("call void @yk_list_push(ptr %{}, i64 {})", list_ptr, push_val));
                }
                (self.ssa(&list_ptr), "ptr".into())
            }
            Expr::FnLit(params, ret_type, body) => {
                let name = format!("__closure_{}", self.closure_counter);
                self.closure_counter += 1;
                self.deferred_fns.push((name.clone(), params.clone(), ret_type.clone(), *body.clone()));
                let n = params.len();
                let param_tys = if n == 0 { String::new() } else { vec!["i64"; n].join(", ") };
                let tmp = self.fresh_label();
                self.e(&format!("%{} = ptrtoint i64 ({})* @{} to i64", tmp, param_tys, name));
                (self.ssa(&tmp), "i64".into())
            }
            Expr::Closure(params, body) => {
                let name = format!("__closure_{}", self.closure_counter);
                self.closure_counter += 1;
                self.deferred_fns.push((name.clone(), params.clone(), None, *body.clone()));
                let n = params.len();
                let param_tys = if n == 0 { String::new() } else { vec!["i64"; n].join(", ") };
                let tmp = self.fresh_label();
                self.e(&format!("%{} = ptrtoint i64 ({})* @{} to i64", tmp, param_tys, name));
                (self.ssa(&tmp), "i64".into())
            }
            Expr::Spawn(inner) => {
                let wrapper_name = format!("__spawn_wrapper_{}", self.spawn_counter);
                self.spawn_counter += 1;
                let result_ptr = self.fresh_label();
                self.e(&format!("%{} = alloca i64, align 8", result_ptr));
                self.e(&format!("store i64 0, ptr %{}", result_ptr));
                let handle = self.fresh_label();
                self.e(&format!("%{} = call i64 @yk_run_thread(ptr @{}, ptr %{})", handle, wrapper_name, result_ptr));
                self.spawn_wrappers.push((wrapper_name, self.ssa(&result_ptr), *inner.clone()));
                (self.ssa(&handle), "i64".into())
            }
            Expr::Await(inner) => {
                let (hv, _) = self.compile_expr(inner);
                let tmp = self.fresh_label();
                self.e(&format!("%{} = call i64 @yk_join_thread(i64 {})", tmp, hv));
                (self.ssa(&tmp), "i64".into())
            }
            Expr::ResultOk(inner) => {
                let (iv, it) = self.compile_expr(inner);
                let payload = if it == "%yk_string" {
                    let d = self.fresh_label();
                    let l = self.fresh_label();
                    self.e(&format!("%{} = extractvalue %yk_string {}, 0", d, iv));
                    self.e(&format!("%{} = extractvalue %yk_string {}, 1", l, iv));
                    let raw = self.fresh_label();
                    self.e(&format!("%{} = ptrtoint ptr %{} to i64", raw, d));
                    let p = self.fresh_label();
                    self.e(&format!("%{} = call i64 @yk_result_str_new(i64 %{}, i64 %{})", p, raw, l));
                    self.ssa(&p)
                } else {
                    self.to_i64(iv, it)
                };
                let tmp = self.fresh_label();
                self.e(&format!("%{} = insertvalue %yk_result undef, i64 {}, 0", tmp, payload));
                let tmp2 = self.fresh_label();
                self.e(&format!("%{} = insertvalue %yk_result %{}, i1 1, 1", tmp2, tmp));
                (self.ssa(&tmp2), "%yk_result".into())
            }
            Expr::ResultErr(inner) => {
                let (iv, it) = self.compile_expr(inner);
                let payload = if it == "%yk_string" {
                    let d = self.fresh_label();
                    let l = self.fresh_label();
                    self.e(&format!("%{} = extractvalue %yk_string {}, 0", d, iv));
                    self.e(&format!("%{} = extractvalue %yk_string {}, 1", l, iv));
                    let raw = self.fresh_label();
                    self.e(&format!("%{} = ptrtoint ptr %{} to i64", raw, d));
                    let p = self.fresh_label();
                    self.e(&format!("%{} = call i64 @yk_result_str_new(i64 %{}, i64 %{})", p, raw, l));
                    self.ssa(&p)
                } else {
                    self.to_i64(iv, it)
                };
                let tmp = self.fresh_label();
                self.e(&format!("%{} = insertvalue %yk_result undef, i64 {}, 0", tmp, payload));
                let tmp2 = self.fresh_label();
                self.e(&format!("%{} = insertvalue %yk_result %{}, i1 0, 1", tmp2, tmp));
                (self.ssa(&tmp2), "%yk_result".into())
            }
            Expr::Match(scrutinee, arms) => {
                let (sv, st) = self.compile_expr(scrutinee);
                let result_ty = arms.first().map(|a| self.expr_type_str(&a.body)).unwrap_or("i64".into());
                let result_ptr = self.fresh_label();
                self.e(&format!("%{} = alloca {}, align 8", result_ptr, result_ty));
                let merge_label = self.fresh_label();
                let arm_labels: Vec<String> = arms.iter().map(|_| self.fresh_label()).collect();
                for (idx, arm) in arms.iter().enumerate() {
                    let match_cond = self.compile_pattern_match(&arm.pattern, &sv, &st);
                    if let Some(ref cond) = match_cond {
                        let next_target = if idx + 1 < arm_labels.len() { &arm_labels[idx + 1] } else { &merge_label };
                        self.e(&format!("br i1 {}, label %{}, label %{}", cond, arm_labels[idx], next_target));
                    } else {
                        self.e(&format!("br label %{}", arm_labels[idx]));
                    }
                    self.e_raw(&format!("{}:", arm_labels[idx]));
                    self.compile_pattern_bind(&arm.pattern, &sv, &st);
                    let (body_val, body_ty) = self.compile_expr(&arm.body);
                    self.e(&format!("store {} {}, ptr %{}", body_ty, body_val, result_ptr));
                    self.e(&format!("br label %{}", merge_label));
                }
                self.e_raw(&format!("{}:", merge_label));
                let result_val = self.fresh_label();
                self.e(&format!("%{} = load {}, ptr %{}", result_val, result_ty, result_ptr));
                (self.ssa(&result_val), result_ty)
            }
            Expr::ForIn(_, _, _) | Expr::While(_, _) | Expr::Loop(_) => ("0".into(), "i64".into()),
            Expr::LitComplex(r, im) => {
                let cptr = self.fresh_label();
                self.e(&format!("%{} = alloca %yk_complex, align 8", cptr));
                let (rv, rt) = self.compile_expr(r);
                let (iv, it) = self.compile_expr(im);
                let rv_conv = if rt == "i64" {
                    let t = self.fresh_label();
                    self.e(&format!("%{} = sitofp i64 {} to double", t, rv));
                    self.ssa(&t)
                } else { rv.clone() };
                let iv_conv = if it == "i64" {
                    let t = self.fresh_label();
                    self.e(&format!("%{} = sitofp i64 {} to double", t, iv));
                    self.ssa(&t)
                } else { iv.clone() };
                self.e(&format!("call void @yk_complex_set(ptr %{}, double {}, double {})", cptr, rv_conv, iv_conv));
                let loaded = self.fresh_label();
                self.e(&format!("%{} = load %yk_complex, ptr %{}", loaded, cptr));
                (self.ssa(&loaded), "%yk_complex".into())
            }
            Expr::VectorLit(items) => {
                if items.is_empty() {
                    ("zeroinitializer".into(), "<2 x double>".into())
                } else {
                    let first = self.compile_expr(&items[0]);
                    let vty = format!("<{} x {}>", items.len(), first.1);
                    let mut result = format!("{} undef", vty);
                    for (i, item) in items.iter().enumerate() {
                        let (val, _) = self.compile_expr(item);
                        let lbl = self.fresh_label();
                        self.e(&format!("%{} = insertelement {} {}, {} {}", lbl, vty, self.ssa(&result), val, i));
                        result = format!("%{}", lbl);
                    }
                    (self.ssa(&result), vty)
                }
            }
            Expr::MatrixLit(rows) => {
                if rows.is_empty() || rows[0].is_empty() {
                    ("zeroinitializer".into(), "[<2 x double> x 0]".into())
                } else {
                    let first = self.compile_expr(&rows[0][0]);
                    let vty = format!("<{} x {}>", rows[0].len(), first.1);
                    let mty = format!("[{} x {}]", vty, rows.len());
                    let mut result = format!("{} undef", mty);
                    for (i, row) in rows.iter().enumerate() {
                        let mut row_val = format!("{} undef", vty);
                        for (j, item) in row.iter().enumerate() {
                            let (val, _) = self.compile_expr(item);
                            let lbl = self.fresh_label();
                            self.e(&format!("%{} = insertelement {} {}, {} {}", lbl, vty, self.ssa(&row_val), val, j));
                            row_val = format!("%{}", lbl);
                        }
                        let lbl2 = self.fresh_label();
                        self.e(&format!("%{} = insertvalue {} {}, {} {}", lbl2, mty, self.ssa(&result), self.ssa(&row_val), i));
                        result = format!("%{}", lbl2);
                    }
                    (self.ssa(&result), mty)
                }
            }
            Expr::PostInc(i) => {
                let (_val, ty) = self.compile_expr(i);
                let orig = self.fresh_label();
                self.e(&format!("%{} = load {}, ptr {}", orig, ty, self.find_alloca_for_expr(i)));
                let one = if ty == "double" { "1.0" } else { "1" };
                let new_val = self.fresh_label();
                if ty == "double" {
                    self.e(&format!("%{} = fadd double %{}, {}", new_val, orig, one));
                } else {
                    self.e(&format!("%{} = add nsw {} %{}, {}", new_val, ty, orig, one));
                }
                self.e(&format!("store {} %{}, ptr {}", ty, new_val, self.find_alloca_for_expr(i)));
                (self.ssa(&orig), ty)
            }
            Expr::PostDec(i) => {
                let (_val, ty) = self.compile_expr(i);
                let orig = self.fresh_label();
                self.e(&format!("%{} = load {}, ptr {}", orig, ty, self.find_alloca_for_expr(i)));
                let one = if ty == "double" { "1.0" } else { "1" };
                let new_val = self.fresh_label();
                if ty == "double" {
                    self.e(&format!("%{} = fsub double %{}, {}", new_val, orig, one));
                } else {
                    self.e(&format!("%{} = sub nsw {} %{}, {}", new_val, ty, orig, one));
                }
                self.e(&format!("store {} %{}, ptr {}", ty, new_val, self.find_alloca_for_expr(i)));
                (self.ssa(&orig), ty)
            }
            Expr::SafeCall(obj, field) => {
                let (obj_val, obj_ty) = self.compile_expr(obj);
                // Extract the null flag (i1, second element)
                let null_flag = self.fresh_label();
                self.e(&format!("%{} = extractvalue {} {}, 1", null_flag, obj_ty, obj_val));
                // Check if valid (1 = non-null)
                let is_valid = self.fresh_label();
                self.e(&format!("%{} = icmp eq i1 %{}, 1", is_valid, null_flag));
                let null_bb = self.fresh_label();
                let valid_bb = self.fresh_label();
                let merge_bb = self.fresh_label();
                self.e(&format!("br i1 %{}, label %{}, label %{}", is_valid, valid_bb, null_bb));
                // Null branch: return zero-initialized nullable
                self.e(&format!("{}:", null_bb));
                let null_result = self.fresh_label();
                // We need the field type - infer from struct definitions
                let inner_obj_ty = obj_ty.trim_start_matches("%__nullable_");
                let field_ty = self.guess_field_type(&inner_obj_ty, field);
                let nullable_field_ty = format!("%__nullable_{}", field_ty.replace(|c: char| !c.is_alphanumeric(), "_"));
                self.e(&format!("%{} = insertvalue {} undef, i1 0, 1", null_result, nullable_field_ty));
                let null_val = self.ssa(&null_result);
                self.e(&format!("br label %{}", merge_bb));
                // Valid branch: extract inner, access field, wrap in nullable
                self.e(&format!("{}:", valid_bb));
                let inner = self.fresh_label();
                self.e(&format!("%{} = extractvalue {} {}, 0", inner, obj_ty, obj_val));
                let inner_val = self.ssa(&inner);
                let (field_val, _) = self.compile_field_access(&inner_val, &inner_obj_ty, field);
                let valid_result = self.fresh_label();
                self.e(&format!("%{} = insertvalue {} undef, {} {}, 0", valid_result, nullable_field_ty, field_ty, field_val));
                let valid_result2 = self.fresh_label();
                self.e(&format!("%{} = insertvalue {} %{}, i1 1, 1", valid_result2, nullable_field_ty, valid_result));
                let valid_val = self.ssa(&valid_result2);
                self.e(&format!("br label %{}", merge_bb));
                // Merge
                self.e(&format!("{}:", merge_bb));
                let phi = self.fresh_label();
                self.e(&format!("%{} = phi {} [ {}, %{} ], [ {}, %{} ]", phi, nullable_field_ty, null_val, null_bb, valid_val, valid_bb));
                (self.ssa(&phi), nullable_field_ty)
            }
            Expr::Elvis(a, b) => {
                let (a_val, a_ty) = self.compile_expr(a);
                let null_flag = self.fresh_label();
                self.e(&format!("%{} = extractvalue {} {}, 1", null_flag, a_ty, a_val));
                let is_null = self.fresh_label();
                self.e(&format!("%{} = icmp eq i1 {}, 0", is_null, self.ssa(&null_flag)));
                let null_bb = self.fresh_label();
                let nonnull_bb = self.fresh_label();
                let merge_bb = self.fresh_label();
                self.e(&format!("br i1 {}, label %{}, label %{}", self.ssa(&is_null), null_bb, nonnull_bb));
                self.e(&format!("{}:", null_bb));
                let (b_val, b_ty) = self.compile_expr(b);
                self.e(&format!("br label %{}", merge_bb));
                self.e(&format!("{}:", nonnull_bb));
                let inner = self.fresh_label();
                self.e(&format!("%{} = extractvalue {} {}, 0", inner, a_ty, a_val));
                let inner_val = self.ssa(&inner);
                self.e(&format!("br label %{}", merge_bb));
                self.e(&format!("{}:", merge_bb));
                let phi = self.fresh_label();
                self.e(&format!("%{} = phi {} [ {}, %{} ], [ {}, %{} ]", phi, b_ty, b_val, null_bb, inner_val, nonnull_bb));
                (self.ssa(&phi), b_ty)
            }
            Expr::Variant(_enum_name, variant_name, args) => {
                let tag = self.get_variant_tag(variant_name);
                let payload: i64 = if args.len() == 1 {
                    let (_pv, _) = self.compile_expr(&args[0]);
                    0
                } else {
                    for arg in args { self.compile_expr(arg); }
                    0
                };
                let tmp = self.fresh_label();
                self.e(&format!("%{} = insertvalue %yk_variant undef, i64 {}, 0", tmp, tag));
                let tmp2 = self.fresh_label();
                self.e(&format!("%{} = insertvalue %yk_variant %{}, i64 {}, 1", tmp2, tmp, payload));
                (self.ssa(&tmp2), "%yk_variant".into())
            }
            Expr::Try(inner) => {
                let (rv, rt) = self.compile_expr(inner);
                if rt == "%yk_result" {
                    let flag = self.fresh_label();
                    self.e(&format!("%{} = extractvalue %yk_result {}, 1", flag, rv));
                    let is_err = self.fresh_label();
                    self.e(&format!("%{} = icmp eq i1 {}, 0", is_err, self.ssa(&flag)));
                    if self.current_fn_ret == "%yk_result" {
                        let ret_bb = self.fresh_label();
                        let cont_bb = self.fresh_label();
                        self.e(&format!("br i1 {}, label %{}, label %{}", self.ssa(&is_err), ret_bb, cont_bb));
                        self.e(&format!("{}:", ret_bb));
                        self.e(&format!("ret %yk_result {}", rv));
                        self.e(&format!("{}:", cont_bb));
                    }
                    let val = self.fresh_label();
                    self.e(&format!("%{} = extractvalue %yk_result {}, 0", val, rv));
                    (self.ssa(&val), "i64".into())
                } else {
                    (rv, rt)
                }
            }
            Expr::TryCatch(try_body, catch_var, catch_body) => {
                // Compile try body (if any Result error occurs inside, it will be
                // returned/propagated via the `?` operator above)
                for s in try_body { self.compile_stmt(s); }
                // Bind catch variable to 0 (default / no error) and compile catch body
                let ptr = self.fresh_label();
                self.e(&format!("%{} = alloca i64, align 8", ptr));
                self.e(&format!("store i64 0, ptr %{}", ptr));
                self.var_alloca.insert(catch_var.clone(), ptr.clone());
                self.var_types.insert(catch_var.clone(), "i64".into());
                for s in catch_body { self.compile_stmt(s); }
                ("0".into(), "i64".into())
            }
            Expr::As(inner, target_type) => {
                let (val, ty) = self.compile_expr(inner);
                let target_llvm_ty = self.type_to_llvm(&target_type.value);
                match (ty.as_str(), target_llvm_ty.as_str()) {
                    ("i64", "double") | ("i32", "double") | ("i16", "double") | ("i8", "double") => {
                        let r = self.fresh_label();
                        self.e(&format!("%{} = sitofp {} {} to double", r, ty, val));
                        (self.ssa(&r), "double".into())
                    }
                    ("double", "i64") | ("double", "i32") | ("double", "i16") | ("double", "i8") => {
                        let r = self.fresh_label();
                        self.e(&format!("%{} = fptosi double {} to {}", r, val, target_llvm_ty));
                        (self.ssa(&r), target_llvm_ty)
                    }
                    ("i64", "i1") | ("i32", "i1") | ("i16", "i1") | ("i8", "i1") => {
                        let r = self.fresh_label();
                        self.e(&format!("%{} = icmp ne {} {}, 0", r, ty, val));
                        (self.ssa(&r), "i1".into())
                    }
                    ("i1", "i64") | ("i1", "i32") | ("i1", "i16") | ("i1", "i8") => {
                        let r = self.fresh_label();
                        self.e(&format!("%{} = zext i1 {} to {}", r, val, target_llvm_ty));
                        (self.ssa(&r), target_llvm_ty)
                    }
                    ("double", "i1") => {
                        let cmp = self.fresh_label();
                        self.e(&format!("%{} = fcmp one double {}, 0.0", cmp, val));
                        (self.ssa(&cmp), "i1".into())
                    }
                    (src_ty, dst_ty) if src_ty.starts_with('i') && dst_ty.starts_with('i') => {
                        let src_w = Self::int_width(src_ty);
                        let dst_w = Self::int_width(dst_ty);
                        if src_w < dst_w {
                            let r = self.fresh_label();
                            self.e(&format!("%{} = sext {} {} to {}", r, src_ty, val, dst_ty));
                            (self.ssa(&r), dst_ty.to_string())
                        } else if src_w > dst_w {
                            let r = self.fresh_label();
                            self.e(&format!("%{} = trunc {} {} to {}", r, src_ty, val, dst_ty));
                            (self.ssa(&r), dst_ty.to_string())
                        } else {
                            (val, dst_ty.to_string())
                        }
                    }
                    (_, "i64") if target_llvm_ty == "i64" => (val, "i64".into()),
                    (_, ty) => (val, ty.into()),
                }
            }
        }
    }

    fn compile_field_access(&mut self, val: &str, ty: &str, field: &str) -> (String, String) {
        // Resolve the actual struct LLVM type from possibly-mangled name
        let (struct_name, llvm_ty) = if let Some(n) = ty.strip_prefix("%struct.") {
            (n, ty.to_string())
        } else if let Some(n) = ty.strip_prefix("_struct_") {
            (n, format!("%struct.{}", n))
        } else if let Some(n) = ty.strip_prefix("%class.") {
            (n, ty.to_string())
        } else if let Some(n) = ty.strip_prefix("_class_") {
            (n, format!("%class.{}", n))
        } else {
            ("", String::new())
        };
        if !struct_name.is_empty() {
            if let Some(defs) = self.struct_defs.get(struct_name) {
                if let Some(idx) = defs.iter().position(|(n, _)| n == field) {
                    let fty = defs[idx].1.clone();
                    let ext = self.fresh_label();
                    self.e(&format!("%{} = extractvalue {} {}, {}", ext, llvm_ty, val, idx));
                    return (self.ssa(&ext), fty);
                }
            }
        }
        // Fallback: return val as-is
        (val.to_string(), ty.to_string())
    }

    fn guess_field_type(&mut self, ty: &str, field: &str) -> String {
        let struct_name = ty.strip_prefix("%struct.")
            .or_else(|| ty.strip_prefix("_struct_"))
            .or_else(|| ty.strip_prefix("%class."))
            .or_else(|| ty.strip_prefix("_class_"))
            .unwrap_or("");
        if !struct_name.is_empty() {
            if let Some(defs) = self.struct_defs.get(struct_name) {
                if let Some(idx) = defs.iter().position(|(n, _)| n == field) {
                    return defs[idx].1.clone();
                }
            }
        }
        "i64".into()
    }

    fn compile_binop(&mut self, l: &ExprNode, op: &BinOp, r: &ExprNode) -> (String, String) {
        let lt = self.expr_type_str(l);
        let (mut lc, mut lt) = (self.compile_expr(l).0, lt);
        let rt = self.expr_type_str(r);
        let (mut rc, mut rt) = (self.compile_expr(r).0, rt);

        // Coerce integer types to match
        if lt != rt && !lt.starts_with('%') && !rt.starts_with('%') && lt != "double" && rt != "double" && lt != "float" && rt != "float" {
            if Self::int_width(&lt) < Self::int_width(&rt) {
                let ext = self.fresh_label();
                self.e(&format!("%{} = sext {} {} to {}", ext, lt, lc, rt));
                lc = self.ssa(&ext);
                lt = rt.clone();
            } else if Self::int_width(&rt) < Self::int_width(&lt) {
                let ext = self.fresh_label();
                self.e(&format!("%{} = sext {} {} to {}", ext, rt, rc, lt));
                rc = self.ssa(&ext);
                rt = lt.clone();
            }
        }
        let is_float = lt == "double";
        let is_complex = lt == "%yk_complex" || rt == "%yk_complex";
        let (arith_op, cmp_op, ofl_flag) = if is_float { ("f", "fcmp", "") } else { ("", "icmp", " nsw") };

        let tmp = self.fresh_label();
        if is_complex {
            let func = match op {
                BinOp::Add => "yk_complex_add",
                BinOp::Sub => "yk_complex_sub",
                BinOp::Mul => "yk_complex_mul",
                BinOp::Div => "yk_complex_div",
                _ => "yk_complex_add",
            };
            // Allocate temps on stack and store values to pass pointers to runtime
            let la = self.fresh_label();
            let ra = self.fresh_label();
            self.e(&format!("%{} = alloca %yk_complex, align 8", la));
            self.e(&format!("%{} = alloca %yk_complex, align 8", ra));
            // Left operand
            if lt == "%yk_complex" {
                self.e(&format!("store %yk_complex {}, ptr %{}", lc, la));
            } else {
                let lca = self.fresh_label();
                self.e(&format!("%{} = sitofp {} {} to double", lca, lt, lc));
                self.e(&format!("call void @yk_complex_set(ptr %{}, double %{}, double 0.0)", la, lca));
            }
            // Right operand (check right type)
            if rt == "%yk_complex" {
                self.e(&format!("store %yk_complex {}, ptr %{}", rc, ra));
            } else {
                let rca = self.fresh_label();
                self.e(&format!("%{} = sitofp {} {} to double", rca, rt, rc));
                self.e(&format!("call void @yk_complex_set(ptr %{}, double %{}, double 0.0)", ra, rca));
            }
            self.e(&format!("%{} = alloca %yk_complex, align 8", tmp));
            self.e(&format!("call void @{}(ptr %{}, ptr %{}, ptr %{})", func, tmp, la, ra));
            let loaded = self.fresh_label();
            self.e(&format!("%{} = load %yk_complex, ptr %{}", loaded, tmp));
            return (self.ssa(&loaded), "%yk_complex".into());
        }
        match op {
            BinOp::Add => {
                if lt == "%yk_string" {
                    let ptr_a = self.string_to_ptr(&lc);
                    let ptr_b = self.string_to_ptr(&rc);
                    self.e(&format!("%{} = call ptr @yk_string_concat_ptr(ptr {}, ptr {})", tmp, ptr_a, ptr_b));
                    let ptr_result = self.ssa(&tmp);
                    let loaded = self.fresh_label();
                    self.e(&format!("%{} = load %yk_string, ptr {}", loaded, ptr_result));
                    (self.ssa(&loaded), "%yk_string".into())
                } else {
                    self.e(&format!("%{} = {}add{} {} {}, {}", tmp, arith_op, ofl_flag, lt, lc, rc));
                    (self.ssa(&tmp), lt)
                }
            }
            BinOp::Sub => {
                self.e(&format!("%{} = {}sub{} {} {}, {}", tmp, arith_op, ofl_flag, lt, lc, rc));
                (self.ssa(&tmp), lt)
            }
            BinOp::Mul => {
                self.e(&format!("%{} = {}mul{} {} {}, {}", tmp, arith_op, ofl_flag, lt, lc, rc));
                (self.ssa(&tmp), lt)
            }
            BinOp::Div => {
                if is_float {
                    self.e(&format!("%{} = fdiv {} {}, {}", tmp, lt, lc, rc));
                } else {
                    self.e(&format!("%{} = sdiv {} {}, {}", tmp, lt, lc, rc));
                }
                (self.ssa(&tmp), lt)
            }
            BinOp::Eq => {
                self.e(&format!("%{} = {} oeq {} {}, {}", tmp, cmp_op, lt, lc, rc));
                (self.ssa(&tmp), "i1".into())
            }
            BinOp::Ne => {
                self.e(&format!("%{} = {} one {} {}, {}", tmp, cmp_op, lt, lc, rc));
                (self.ssa(&tmp), "i1".into())
            }
            BinOp::Lt => {
                let cond = if is_float { "olt" } else { "slt" };
                self.e(&format!("%{} = {} {} {} {}, {}", tmp, cmp_op, cond, lt, lc, rc));
                (self.ssa(&tmp), "i1".into())
            }
            BinOp::Gt => {
                let cond = if is_float { "ogt" } else { "sgt" };
                self.e(&format!("%{} = {} {} {} {}, {}", tmp, cmp_op, cond, lt, lc, rc));
                (self.ssa(&tmp), "i1".into())
            }
            BinOp::Le => {
                let cond = if is_float { "ole" } else { "sle" };
                self.e(&format!("%{} = {} {} {} {}, {}", tmp, cmp_op, cond, lt, lc, rc));
                (self.ssa(&tmp), "i1".into())
            }
            BinOp::Ge => {
                let cond = if is_float { "oge" } else { "sge" };
                self.e(&format!("%{} = {} {} {} {}, {}", tmp, cmp_op, cond, lt, lc, rc));
                (self.ssa(&tmp), "i1".into())
            }
            BinOp::And => {
                let z1 = self.fresh_label();
                let z2 = self.fresh_label();
                self.e(&format!("%{} = icmp ne {} {}, 0", z1, lt, lc));
                self.e(&format!("%{} = icmp ne {} {}, 0", z2, lt, rc));
                self.e(&format!("%{} = and i1 %{}, %{}", tmp, z1, z2));
                (self.ssa(&tmp), "i1".into())
            }
            BinOp::Or => {
                let z1 = self.fresh_label();
                let z2 = self.fresh_label();
                self.e(&format!("%{} = icmp ne {} {}, 0", z1, lt, lc));
                self.e(&format!("%{} = icmp ne {} {}, 0", z2, lt, rc));
                self.e(&format!("%{} = or i1 %{}, %{}", tmp, z1, z2));
                (self.ssa(&tmp), "i1".into())
            }
            BinOp::Assign => {
                // For compound assignment, emit store
                self.e(&format!("store {} {}, ptr {}", lt, rc, lc));
                (rc, lt)
            }
        }
    }

    fn compile_call(&mut self, callee: &ExprNode, args: &[ExprNode]) -> (String, String) {
        let arg_results: Vec<(String, String)> = args.iter().map(|a| self.compile_expr(a)).collect();

        match &callee.value {
            Expr::Ident(name) => match name.as_str() {
                "print" | "println" => {
                    if arg_results.is_empty() {
                        ("0".into(), "void".into())
                    } else {
                        for (av, at) in &arg_results {
                            if at.starts_with("%__nullable_") {
                                let flag = self.fresh_label();
                                let inner = self.fresh_label();
                                self.e(&format!("%{} = extractvalue {} {}, 1", flag, at, av));
                                self.e(&format!("%{} = extractvalue {} {}, 0", inner, at, av));
                                let isnull_bb = self.fresh_label();
                                let nonnull_bb = self.fresh_label();
                                let merge_bb = self.fresh_label();
                                self.e(&format!("br i1 %{}, label %{}, label %{}", flag, nonnull_bb, isnull_bb));
                                self.e(&format!("{}:", isnull_bb));
                                let null_str = self.make_string_slot("(null)");
                                let null_ptr = self.string_to_ptr(&null_str);
                                self.e(&format!("call void @yk_print_str_ptr(ptr {})", null_ptr));
                                self.e(&format!("br label %{}", merge_bb));
                                self.e(&format!("{}:", nonnull_bb));
                                self.e(&format!("call void @yk_print_int(i64 %{})", inner));
                                self.e(&format!("br label %{}", merge_bb));
                                self.e(&format!("{}:", merge_bb));
                            } else {
                                match at.as_str() {
                                    "i64" => self.e(&format!("call void @yk_print_int(i64 {})", av)),
                                    "double" => self.e(&format!("call void @yk_print_real(double {})", av)),
                                    "i1" => self.e(&format!("call void @yk_print_bool(i1 {})", av)),
                                    "%yk_string" => {
                                        let p = self.string_to_ptr(av);
                                        self.e(&format!("call void @yk_print_str_ptr(ptr {})", p));
                                    }
                                    "%yk_complex" => {
                                        self.e(&format!("call void @yk_print_complex(ptr {})", av));
                                    }
                                    "%yk_result" => {
                                        let payload = self.fresh_label();
                                        let flag = self.fresh_label();
                                        self.e(&format!("%{} = extractvalue %yk_result {}, 0", payload, av));
                                        self.e(&format!("%{} = extractvalue %yk_result {}, 1", flag, av));
                                        self.e(&format!("call void @yk_print_result_val(i64 %{}, i1 %{})", payload, flag));
                                    }
                                    "ptr" => {
                                        self.e(&format!("call void @yk_list_print(ptr {})", av));
                                    }
                                    _ => self.e(&format!("call void @yk_print_int(i64 {})", av)),
                                }
                            }
                        }
                        ("0".into(), "void".into())
                    }
                }
                "len" => {
                    if let Some((av, at)) = arg_results.first() {
                        if at == "%yk_string" {
                            let p = self.string_to_ptr(av);
                            let tmp = self.fresh_label();
                            self.e(&format!("%{} = call i64 @yk_string_len_ptr(ptr {})", tmp, p));
                            (self.ssa(&tmp), "i64".into())
                        } else if at == "ptr" {
                            let tmp = self.fresh_label();
                            self.e(&format!("%{} = call i64 @yk_list_len(ptr {})", tmp, av));
                            (self.ssa(&tmp), "i64".into())
                        } else {
                            ("0".into(), "i64".into())
                        }
                    } else { ("0".into(), "i64".into()) }
                }
                "str" => {
                    if let Some((av, at)) = arg_results.first() {
                        let ptr_ssa = if at.starts_with("%__nullable_") {
                            let flag = self.fresh_label();
                            let inner = self.fresh_label();
                            self.e(&format!("%{} = extractvalue {} {}, 1", flag, at, av));
                            self.e(&format!("%{} = extractvalue {} {}, 0", inner, at, av));
                            let isnull_bb = self.fresh_label();
                            let nonnull_bb = self.fresh_label();
                            let merge_bb = self.fresh_label();
                            self.e(&format!("br i1 %{}, label %{}, label %{}", flag, nonnull_bb, isnull_bb));
                            self.e(&format!("{}:", isnull_bb));
                            let null_str = self.make_string_slot("(null)");
                            let null_alloca = self.fresh_label();
                            self.e(&format!("%{} = alloca %yk_string, align 8", null_alloca));
                            self.e(&format!("store %yk_string {}, ptr %{}", null_str, null_alloca));
                            self.e(&format!("br label %{}", merge_bb));
                            self.e(&format!("{}:", nonnull_bb));
                            let t = self.fresh_label();
                            self.e(&format!("%{} = call ptr @yk_string_from_int(i64 %{})", t, inner));
                            self.e(&format!("br label %{}", merge_bb));
                            self.e(&format!("{}:", merge_bb));
                            let phi = self.fresh_label();
                            self.e(&format!("%{} = phi ptr [ %{}, %{} ], [ %{}, %{} ]", phi, null_alloca, isnull_bb, t, nonnull_bb));
                            phi
                        } else {
                            match at.as_str() {
                                "i64" => {
                                    let t = self.fresh_label();
                                    self.e(&format!("%{} = call ptr @yk_string_from_int(i64 {})", t, av));
                                    self.ssa(&t)
                                }
                                "double" => {
                                    let t = self.fresh_label();
                                    self.e(&format!("%{} = call ptr @yk_string_from_real(double {})", t, av));
                                    self.ssa(&t)
                                }
                                "i1" => {
                                    let t = self.fresh_label();
                                    self.e(&format!("%{} = call ptr @yk_string_from_bool(i1 {})", t, av));
                                    self.ssa(&t)
                                }
                                "%yk_complex" => {
                                    let ca = self.fresh_label();
                                    self.e(&format!("%{} = alloca %yk_complex, align 8", ca));
                                    self.e(&format!("store %yk_complex {}, ptr %{}", av, ca));
                                    let t = self.fresh_label();
                                    self.e(&format!("%{} = call ptr @yk_string_from_complex(ptr %{})", t, ca));
                                    self.ssa(&t)
                                }
                                "%yk_string" => {
                                    // Already a %yk_string value, alloca and get ptr
                                    let ca = self.fresh_label();
                                    self.e(&format!("%{} = alloca %yk_string, align 8", ca));
                                    self.e(&format!("store %yk_string {}, ptr %{}", av, ca));
                                    self.ssa(&ca)
                                }
                                _ => {
                                    let t = self.fresh_label();
                                    self.e(&format!("%{} = call ptr @yk_string_from_int(i64 {})", t, av));
                                    self.ssa(&t)
                                }
                            }
                        };
                        let loaded = self.fresh_label();
                        self.e(&format!("%{} = load %yk_string, ptr {}", loaded, ptr_ssa));
                        (self.ssa(&loaded), "%yk_string".into())
                    } else {
                        ("0".into(), "%yk_string".into())
                    }
                }
                "abs" => {
                    let tmp = self.fresh_label();
                    if let Some((av, at)) = arg_results.first() {
                        if at == "i64" {
                            self.e(&format!("%{} = call i64 @yk_math_abs_i64(i64 {})", tmp, av));
                            (self.ssa(&tmp), "i64".into())
                        } else {
                            self.e(&format!("%{} = call double @yk_math_abs_real(double {})", tmp, av));
                            (self.ssa(&tmp), "double".into())
                        }
                    } else {
                        ("0".into(), "i64".into())
                    }
                }
                "sqrt" | "sin" | "cos" | "floor" | "ceil" | "round" => {
                    let tmp = self.fresh_label();
                    if let Some((av, _)) = arg_results.first() {
                        self.e(&format!("%{} = call double @yk_math_{}(double {})", tmp, name, av));
                    }
                    (self.ssa(&tmp), "double".into())
                }
                "pow" => {
                    let tmp = self.fresh_label();
                    let (av1, _) = arg_results.first().cloned().unwrap_or_default();
                    let (av2, _) = arg_results.get(1).cloned().unwrap_or_default();
                    self.e(&format!("%{} = call double @yk_math_pow(double {}, double {})", tmp, av1, av2));
                    (self.ssa(&tmp), "double".into())
                }
                "max" | "min" => {
                    let tmp = self.fresh_label();
                    let (av1, _) = arg_results.first().cloned().unwrap_or_default();
                    let (av2, _) = arg_results.get(1).cloned().unwrap_or_default();
                    self.e(&format!("%{} = call double @yk_math_{}(double {}, double {})", tmp, name, av1, av2));
                    (self.ssa(&tmp), "double".into())
                }
                "rand" => {
                    let tmp = self.fresh_label();
                    let max = arg_results.first().map(|(v,_)| v.as_str()).unwrap_or("2147483647");
                    self.e(&format!("%{} = call i64 @yk_math_rand(i64 {})", tmp, max));
                    (self.ssa(&tmp), "i64".into())
                }
                "now" => {
                    let ptr_tmp = self.fresh_label();
                    self.e(&format!("%{} = call ptr @yk_time_now()", ptr_tmp));
                    let loaded = self.fresh_label();
                    self.e(&format!("%{} = load %yk_string, ptr %{}", loaded, ptr_tmp));
                    (self.ssa(&loaded), "%yk_string".into())
                }
                "sleep" => {
                    if let Some((av, _)) = arg_results.first() {
                        self.e(&format!("call void @yk_time_sleep(i64 {})", av));
                    }
                    ("0".into(), "void".into())
                }
                "timestamp" => {
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = call i64 @yk_time_timestamp()", tmp));
                    (self.ssa(&tmp), "i64".into())
                }
                "fetch" => {
                    let (url_av, _) = arg_results.first().cloned().unwrap_or(("0".into(), "i64".into()));
                    let url_ptr = if &url_av != "0" { self.string_to_ptr(&url_av) } else { "null".into() };
                    let method_ptr = if arg_results.len() > 1 {
                        let (av, _) = &arg_results[1];
                        self.string_to_ptr(av)
                    } else {
                        "null".into()
                    };
                    let body_ptr = if arg_results.len() > 2 {
                        let (av, _) = &arg_results[2];
                        self.string_to_ptr(av)
                    } else {
                        "null".into()
                    };
                    let ptr_tmp = self.fresh_label();
                    self.e(&format!("%{} = call ptr @yk_fetch(ptr {}, ptr {}, ptr {})", ptr_tmp, url_ptr, method_ptr, body_ptr));
                    let loaded = self.fresh_label();
                    self.e(&format!("%{} = load %yk_string, ptr %{}", loaded, ptr_tmp));
                    (self.ssa(&loaded), "%yk_string".into())
                }
                "Server" => {
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = call i64 @yk_server_new()", tmp));
                    (self.ssa(&tmp), "i64".into())
                }
                _ => {
                    let tmp = self.fresh_label();
                    let args_str: Vec<String> = arg_results.iter().map(|(v, t)| format!("{} {}", t, v)).collect();
                    let mangled = self.mangle_name(name);
                    let fn_ret = self.fn_ret_types.get(&mangled).cloned().unwrap_or_else(|| "i64".into());
                    self.e(&format!("%{} = call {} @{}({})", tmp, fn_ret, mangled, args_str.join(", ")));
                    (self.ssa(&tmp), fn_ret)
                }
            },
            Expr::Closure(params, body) => {
                let prefix = if self.current_module.is_empty() { String::new() } else { format!("{}.", self.current_module) };
                let name = format!("{}__closure_{}", prefix, self.closure_counter);
                self.closure_counter += 1;
                self.deferred_fns.push((name.clone(), params.clone(), None, *body.clone()));
                let tmp = self.fresh_label();
                let args_str: Vec<String> = arg_results.iter().map(|(v, t)| format!("{} {}", t, v)).collect();
                self.e(&format!("%{} = call i64 @{}({})", tmp, name, args_str.join(", ")));
                (self.ssa(&tmp), "i64".into())
            }
            Expr::FnLit(params, ret_type, body) => {
                let prefix = if self.current_module.is_empty() { String::new() } else { format!("{}.", self.current_module) };
                let name = format!("{}__closure_{}", prefix, self.closure_counter);
                self.closure_counter += 1;
                self.deferred_fns.push((name.clone(), params.clone(), ret_type.clone(), *body.clone()));
                let tmp = self.fresh_label();
                let args_str: Vec<String> = arg_results.iter().map(|(v, t)| format!("{} {}", t, v)).collect();
                self.e(&format!("%{} = call i64 @{}({})", tmp, name, args_str.join(", ")));
                (self.ssa(&tmp), "i64".into())
            }
            Expr::Field(obj, field) => {
                match (&obj.value, field.as_str()) {
                    (Expr::Ident(mod_name), func_name) if mod_name == "math" => {
                        let tmp = self.fresh_label();
                        match func_name {
                            "abs" => {
                                if let Some((av, at)) = arg_results.first() {
                                    if at == "i64" {
                                        self.e(&format!("%{} = call i64 @yk_math_abs_i64(i64 {})", tmp, av));
                                    } else {
                                        self.e(&format!("%{} = call double @yk_math_abs_real(double {})", tmp, av));
                                    }
                                }
                                (self.ssa(&tmp), "double".into())
                            }
                            "sqrt" | "sin" | "cos" | "floor" | "ceil" | "round" => {
                                if let Some((av, _)) = arg_results.first() {
                                    self.e(&format!("%{} = call double @yk_math_{}(double {})", tmp, func_name, av));
                                }
                                (self.ssa(&tmp), "double".into())
                            }
                            "pow" => {
                                let (av1, _) = arg_results.first().cloned().unwrap_or_default();
                                let (av2, _) = arg_results.get(1).cloned().unwrap_or_default();
                                self.e(&format!("%{} = call double @yk_math_pow(double {}, double {})", tmp, av1, av2));
                                (self.ssa(&tmp), "double".into())
                            }
                            "max" | "min" => {
                                let (av1, _) = arg_results.first().cloned().unwrap_or_default();
                                let (av2, _) = arg_results.get(1).cloned().unwrap_or_default();
                                self.e(&format!("%{} = call double @yk_math_{}(double {}, double {})", tmp, func_name, av1, av2));
                                (self.ssa(&tmp), "double".into())
                            }
                            "rand" => {
                                let max = arg_results.first().map(|(v,_)| v.as_str()).unwrap_or("2147483647");
                                self.e(&format!("%{} = call i64 @yk_math_rand(i64 {})", tmp, max));
                                (self.ssa(&tmp), "i64".into())
                            }
                            _ => ("0".into(), "i64".into()),
                        }
                    }
                    (Expr::Ident(mod_name), func_name) if mod_name == "time" => {
                        match func_name {
                            "now" => {
                                let ptr_tmp = self.fresh_label();
                                self.e(&format!("%{} = call ptr @yk_time_now()", ptr_tmp));
                                let loaded = self.fresh_label();
                                self.e(&format!("%{} = load %yk_string, ptr %{}", loaded, ptr_tmp));
                                (self.ssa(&loaded), "%yk_string".into())
                            }
                            "sleep" => {
                                if let Some((av, _)) = arg_results.first() {
                                    self.e(&format!("call void @yk_time_sleep(i64 {})", av));
                                }
                                ("0".into(), "void".into())
                            }
                            "timestamp" => {
                                let tmp = self.fresh_label();
                                self.e(&format!("%{} = call i64 @yk_time_timestamp()", tmp));
                                (self.ssa(&tmp), "i64".into())
                            }
                            _ => ("0".into(), "i64".into()),
                        }
                    }
                    (Expr::Ident(mod_name), func_name) if mod_name == "sys" => {
                        match func_name {
                            "pid" => {
                                let tmp = self.fresh_label();
                                self.e(&format!("%{} = call i64 @yk_sys_pid()", tmp));
                                (self.ssa(&tmp), "i64".into())
                            }
                            "exit" => {
                                if let Some((av, _)) = arg_results.first() {
                                    self.e(&format!("call void @yk_sys_exit(i64 {})", av));
                                }
                                ("0".into(), "void".into())
                            }
                            "cwd" => {
                                let ptr_tmp = self.fresh_label();
                                self.e(&format!("%{} = call ptr @yk_sys_cwd()", ptr_tmp));
                                let loaded = self.fresh_label();
                                self.e(&format!("%{} = load %yk_string, ptr %{}", loaded, ptr_tmp));
                                (self.ssa(&loaded), "%yk_string".into())
                            }
                            "platform" => {
                                let ptr_tmp = self.fresh_label();
                                self.e(&format!("%{} = call ptr @yk_sys_platform()", ptr_tmp));
                                let loaded = self.fresh_label();
                                self.e(&format!("%{} = load %yk_string, ptr %{}", loaded, ptr_tmp));
                                (self.ssa(&loaded), "%yk_string".into())
                            }
                            "env" => {
                                if let Some((av, _)) = arg_results.first() {
                                    let ptr_tmp = self.fresh_label();
                                    let p = self.string_to_ptr(av);
                                    self.e(&format!("%{} = call ptr @yk_sys_env(ptr {})", ptr_tmp, p));
                                    let loaded = self.fresh_label();
                                    self.e(&format!("%{} = load %yk_string, ptr %{}", loaded, ptr_tmp));
                                    (self.ssa(&loaded), "%yk_string".into())
                                } else {
                                    ("0".into(), "%yk_string".into())
                                }
                            }
                            _ => ("0".into(), "i64".into()),
                        }
                    }
                    (Expr::Ident(mod_name), func_name) if mod_name == "path" => {
                        match func_name {
                            "join" => {
                                if arg_results.len() >= 2 {
                                    let (av1, _) = &arg_results[0];
                                    let (av2, _) = &arg_results[1];
                                    let p1 = self.string_to_ptr(av1);
                                    let p2 = self.string_to_ptr(av2);
                                    let ptr_tmp = self.fresh_label();
                                    self.e(&format!("%{} = call ptr @yk_path_join(ptr {}, ptr {})", ptr_tmp, p1, p2));
                                    let loaded = self.fresh_label();
                                    self.e(&format!("%{} = load %yk_string, ptr %{}", loaded, ptr_tmp));
                                    (self.ssa(&loaded), "%yk_string".into())
                                } else {
                                    ("0".into(), "%yk_string".into())
                                }
                            }
                            "dirname" | "basename" | "extension" => {
                                if let Some((av, _)) = arg_results.first() {
                                    let p = self.string_to_ptr(av);
                                    let ptr_tmp = self.fresh_label();
                                    self.e(&format!("%{} = call ptr @yk_path_{}(ptr {})", ptr_tmp, func_name, p));
                                    let loaded = self.fresh_label();
                                    self.e(&format!("%{} = load %yk_string, ptr %{}", loaded, ptr_tmp));
                                    (self.ssa(&loaded), "%yk_string".into())
                                } else {
                                    ("0".into(), "%yk_string".into())
                                }
                            }
                            "is_absolute" => {
                                if let Some((av, _)) = arg_results.first() {
                                    let p = self.string_to_ptr(av);
                                    let tmp = self.fresh_label();
                                    self.e(&format!("%{} = call i64 @yk_path_is_absolute(ptr {})", tmp, p));
                                    let b = self.fresh_label();
                                    self.e(&format!("%{} = icmp ne i64 %{}, 0", b, tmp));
                                    (self.ssa(&b), "i1".into())
                                } else {
                                    ("0".into(), "i1".into())
                                }
                            }
                            _ => ("0".into(), "i64".into()),
                        }
                    }
                    (Expr::Ident(mod_name), func_name) if mod_name == "fs" || mod_name == "io" => {
                        match func_name {
                            "read" => {
                                if let Some((av, _)) = arg_results.first() {
                                    let p = self.string_to_ptr(av);
                                    let ptr_tmp = self.fresh_label();
                                    self.e(&format!("%{} = call ptr @yk_fs_read(ptr {})", ptr_tmp, p));
                                    let loaded = self.fresh_label();
                                    self.e(&format!("%{} = load %yk_string, ptr %{}", loaded, ptr_tmp));
                                    (self.ssa(&loaded), "%yk_string".into())
                                } else {
                                    ("0".into(), "%yk_string".into())
                                }
                            }
                            "write" | "append" => {
                                if arg_results.len() >= 2 {
                                    let (p, _) = &arg_results[0];
                                    let (c, _) = &arg_results[1];
                                    let pp = self.string_to_ptr(p);
                                    let pc = self.string_to_ptr(c);
                                    self.e(&format!("call void @yk_fs_{}(ptr {}, ptr {})", func_name, pp, pc));
                                }
                                ("0".into(), "void".into())
                            }
                            "remove" => {
                                if let Some((av, _)) = arg_results.first() {
                                    let p = self.string_to_ptr(av);
                                    self.e(&format!("call void @yk_fs_remove(ptr {})", p));
                                }
                                ("0".into(), "void".into())
                            }
                            "exists" | "is_dir" | "is_file" => {
                                if let Some((av, _)) = arg_results.first() {
                                    let p = self.string_to_ptr(av);
                                    let tmp = self.fresh_label();
                                    self.e(&format!("%{} = call i64 @yk_fs_{}(ptr {})", tmp, func_name, p));
                                    let b = self.fresh_label();
                                    self.e(&format!("%{} = icmp ne i64 %{}, 0", b, tmp));
                                    (self.ssa(&b), "i1".into())
                                } else {
                                    ("0".into(), "i1".into())
                                }
                            }
                            _ => ("0".into(), "i64".into()),
                        }
                    }
                    (Expr::Ident(mod_name), func_name) if mod_name == "base64" => {
                        match func_name {
                            "encode" | "decode" => {
                                if let Some((av, _)) = arg_results.first() {
                                    let p = self.string_to_ptr(av);
                                    let ptr_tmp = self.fresh_label();
                                    self.e(&format!("%{} = call ptr @yk_base64_{}(ptr {})", ptr_tmp, func_name, p));
                                    let loaded = self.fresh_label();
                                    self.e(&format!("%{} = load %yk_string, ptr %{}", loaded, ptr_tmp));
                                    (self.ssa(&loaded), "%yk_string".into())
                                } else {
                                    ("0".into(), "%yk_string".into())
                                }
                            }
                            _ => ("0".into(), "i64".into()),
                        }
                    }
                    (Expr::Ident(mod_name), func_name) if mod_name == "json" => {
                        match func_name {
                            "stringify" => {
                                if let Some((av, at)) = arg_results.first() {
                                    match at.as_str() {
                                        "i64" => {
                                            let ptr_tmp = self.fresh_label();
                                            self.e(&format!("%{} = call ptr @yk_string_from_int(i64 {})", ptr_tmp, av));
                                            let loaded = self.fresh_label();
                                            self.e(&format!("%{} = load %yk_string, ptr %{}", loaded, ptr_tmp));
                                            (self.ssa(&loaded), "%yk_string".into())
                                        }
                                        "double" => {
                                            let ptr_tmp = self.fresh_label();
                                            self.e(&format!("%{} = call ptr @yk_string_from_real(double {})", ptr_tmp, av));
                                            let loaded = self.fresh_label();
                                            self.e(&format!("%{} = load %yk_string, ptr %{}", loaded, ptr_tmp));
                                            (self.ssa(&loaded), "%yk_string".into())
                                        }
                                        "i1" => {
                                            let ptr_tmp = self.fresh_label();
                                            self.e(&format!("%{} = call ptr @yk_string_from_bool(i1 {})", ptr_tmp, av));
                                            let loaded = self.fresh_label();
                                            self.e(&format!("%{} = load %yk_string, ptr %{}", loaded, ptr_tmp));
                                            (self.ssa(&loaded), "%yk_string".into())
                                        }
                                        "%yk_string" => {
                                            let p = self.string_to_ptr(av);
                                            let ptr_tmp = self.fresh_label();
                                            self.e(&format!("%{} = call ptr @yk_json_string(ptr {})", ptr_tmp, p));
                                            let loaded = self.fresh_label();
                                            self.e(&format!("%{} = load %yk_string, ptr %{}", loaded, ptr_tmp));
                                            (self.ssa(&loaded), "%yk_string".into())
                                        }
                                        _ => {
                                            let ptr_tmp = self.fresh_label();
                                            self.e(&format!("%{} = call ptr @yk_string_from_int(i64 {})", ptr_tmp, av));
                                            let loaded = self.fresh_label();
                                            self.e(&format!("%{} = load %yk_string, ptr %{}", loaded, ptr_tmp));
                                            (self.ssa(&loaded), "%yk_string".into())
                                        }
                                    }
                                } else {
                                    ("0".into(), "%yk_string".into())
                                }
                            }
                            _ => ("0".into(), "i64".into()),
                        }
                    }
                    (Expr::Ident(mod_name), func_name) if mod_name == "re" => {
                        match func_name {
                            "match" => {
                                if arg_results.len() >= 2 {
                                    let (p, _) = &arg_results[0];
                                    let (t, _) = &arg_results[1];
                                    let pp = self.string_to_ptr(p);
                                    let pt = self.string_to_ptr(t);
                                    let tmp = self.fresh_label();
                                    self.e(&format!("%{} = call i64 @yk_re_match(ptr {}, ptr {})", tmp, pp, pt));
                                    let b = self.fresh_label();
                                    self.e(&format!("%{} = icmp ne i64 %{}, 0", b, tmp));
                                    (self.ssa(&b), "i1".into())
                                } else {
                                    ("0".into(), "i1".into())
                                }
                            }
                            "replace" => {
                                if arg_results.len() >= 3 {
                                    let (p, _) = &arg_results[0];
                                    let (t, _) = &arg_results[1];
                                    let (r, _) = &arg_results[2];
                                    let pp = self.string_to_ptr(p);
                                    let pt = self.string_to_ptr(t);
                                    let pr = self.string_to_ptr(r);
                                    let ptr_tmp = self.fresh_label();
                                    self.e(&format!("%{} = call ptr @yk_re_replace(ptr {}, ptr {}, ptr {})", ptr_tmp, pp, pt, pr));
                                    let loaded = self.fresh_label();
                                    self.e(&format!("%{} = load %yk_string, ptr %{}", loaded, ptr_tmp));
                                    (self.ssa(&loaded), "%yk_string".into())
                                } else {
                                    ("0".into(), "%yk_string".into())
                                }
                            }
                            _ => ("0".into(), "i64".into()),
                        }
                    }
                    (Expr::Ident(mod_name), func_name) if mod_name == "datetime" => {
                        match func_name {
                            "now" => {
                                let ptr_tmp = self.fresh_label();
                                self.e(&format!("%{} = call ptr @yk_datetime_now()", ptr_tmp));
                                let loaded = self.fresh_label();
                                self.e(&format!("%{} = load %yk_string, ptr %{}", loaded, ptr_tmp));
                                (self.ssa(&loaded), "%yk_string".into())
                            }
                            "utc" => {
                                let ptr_tmp = self.fresh_label();
                                self.e(&format!("%{} = call ptr @yk_datetime_utc()", ptr_tmp));
                                let loaded = self.fresh_label();
                                self.e(&format!("%{} = load %yk_string, ptr %{}", loaded, ptr_tmp));
                                (self.ssa(&loaded), "%yk_string".into())
                            }
                            "timestamp" => {
                                let tmp = self.fresh_label();
                                self.e(&format!("%{} = call i64 @yk_time_timestamp()", tmp));
                                (self.ssa(&tmp), "i64".into())
                            }
                            "year" | "month" | "day" | "hour" | "minute" | "second" => {
                                let tmp = self.fresh_label();
                                if let Some((av, _)) = arg_results.first() {
                                    self.e(&format!("%{} = call i64 @yk_datetime_{}(i64 {})", tmp, func_name, av));
                                }
                                (self.ssa(&tmp), "i64".into())
                            }
                            "format" => {
                                if arg_results.len() >= 2 {
                                    let ts = &arg_results[0].0;
                                    let fmt_p = self.string_to_ptr(&arg_results[1].0);
                                    let ptr_tmp = self.fresh_label();
                                    self.e(&format!("%{} = call ptr @yk_datetime_format(i64 {}, ptr {})", ptr_tmp, ts, fmt_p));
                                    let loaded = self.fresh_label();
                                    self.e(&format!("%{} = load %yk_string, ptr %{}", loaded, ptr_tmp));
                                    (self.ssa(&loaded), "%yk_string".into())
                                } else {
                                    ("0".into(), "%yk_string".into())
                                }
                            }
                            _ => ("0".into(), "i64".into()),
                        }
                    }
                    (Expr::Ident(mod_name), func_name) if self.ffi_modules.contains(mod_name.as_str()) => {
                        let mangled = format!("yk_{}_{}", mod_name, func_name);
                        if !self.ffi_decls.contains(&mangled) {
                            self.ffi_decls.insert(mangled.clone());
                            let args_decl: Vec<String> = arg_results.iter().map(|(_, t)| t.clone()).collect();
                            if args_decl.is_empty() {
                                self.e_raw(&format!("declare i64 @{}()", mangled));
                            } else {
                                self.e_raw(&format!("declare i64 @{}({})", mangled, args_decl.join(", ")));
                            }
                        }
                        let tmp = self.fresh_label();
                        let args_str: Vec<String> = arg_results.iter().map(|(v, t)| format!("{} {}", t, v)).collect();
                        if args_str.is_empty() {
                            self.e(&format!("%{} = call i64 @{}()", tmp, mangled));
                        } else {
                            self.e(&format!("%{} = call i64 @{}({})", tmp, mangled, args_str.join(", ")));
                        }
                        (self.ssa(&tmp), "i64".into())
                    }
                    _ => {
                        // Check if this is a class method call: obj.method(args)
                        let (o_val, o_ty) = self.compile_expr(obj);
                        // Server method dispatch (get, post, serve)
                        if o_ty == "i64" && matches!(field.as_str(), "get" | "post" | "serve") {
                            return self.compile_server_method(&o_val, field, &arg_results, args);
                        }
                        // List method dispatch (push, pop, len)
                        if o_ty == "ptr" {
                            match field.as_str() {
                                "push" => {
                                    if let Some((pv, pt)) = arg_results.first() {
                                        let push_val = match pt.as_str() {
                                            "double" => { let t = self.fresh_label(); self.e(&format!("%{} = bitcast double {} to i64", t, pv)); self.ssa(&t) }
                                            "i1" => { let t = self.fresh_label(); self.e(&format!("%{} = zext i1 {} to i64", t, pv)); self.ssa(&t) }
                                            "%yk_string" => { let t = self.fresh_label(); self.e(&format!("%{} = ptrtoint ptr {} to i64", t, pv)); self.ssa(&t) }
                                            _ => pv.clone()
                                        };
                                        self.e(&format!("call void @yk_list_push(ptr {}, i64 {})", o_val, push_val));
                                    }
                                    return ("0".into(), "i64".into());
                                }
                                "pop" => {
                                    let tmp = self.fresh_label();
                                    self.e(&format!("%{} = call i64 @yk_list_pop(ptr {})", tmp, o_val));
                                    return (self.ssa(&tmp), "i64".into());
                                }
                                "len" => {
                                    let tmp = self.fresh_label();
                                    self.e(&format!("%{} = call i64 @yk_list_len(ptr {})", tmp, o_val));
                                    return (self.ssa(&tmp), "i64".into());
                                }
                                "sort" => {
                                    self.e(&format!("call void @yk_list_sort(ptr {})", o_val));
                                    return ("0".into(), "i64".into());
                                }
                                "reverse" => {
                                    self.e(&format!("call void @yk_list_reverse(ptr {})", o_val));
                                    return ("0".into(), "i64".into());
                                }
                                "insert" => {
                                    if let Some((iv, it)) = arg_results.get(0) {
                                        let idx = if it == "i64" { iv.clone() } else { "0".into() };
                                        let val = if let Some((vv, vt)) = arg_results.get(1) {
                                            self.to_i64(vv.clone(), vt.clone())
                                        } else { "0".into() };
                                        self.e(&format!("call void @yk_list_insert(ptr {}, i64 {}, i64 {})", o_val, idx, val));
                                    }
                                    return ("0".into(), "i64".into());
                                }
                                "remove" => {
                                    if let Some((iv, it)) = arg_results.first() {
                                        let idx = if it == "i64" { iv.clone() } else { "0".into() };
                                        self.e(&format!("call void @yk_list_remove(ptr {}, i64 {})", o_val, idx));
                                    }
                                    return ("0".into(), "i64".into());
                                }
                                "clear" => {
                                    self.e(&format!("call void @yk_list_clear(ptr {})", o_val));
                                    return ("0".into(), "i64".into());
                                }
                                _ => {}
                            }
                        }
                        if let Some(cls_name) = o_ty.strip_prefix("%class.") {
                            if let Some(vtable_methods) = self.class_vtables.get(cls_name) {
                                if let Some(method_idx) = vtable_methods.iter().position(|(n, _)| n == field) {
                                    // Alloca the struct value to get a pointer for self
                                    let alloca_ptr = self.fresh_label();
                                    self.e(&format!("%{} = alloca {}, align 8", alloca_ptr, o_ty));
                                    self.e(&format!("store {} {}, ptr %{}", o_ty, o_val, alloca_ptr));
                                    // Load vtable pointer
                                    let vp = self.fresh_label();
                                    self.e(&format!("%{} = getelementptr inbounds {}, ptr %{}, i32 0, i32 0", vp, o_ty, alloca_ptr));
                                    let vt = self.fresh_label();
                                    self.e(&format!("%{} = load i64, ptr %{}", vt, vp));
                                    // Get vtable pointer
                                    let vtable_ptr = self.fresh_label();
                                    self.e(&format!("%{} = inttoptr i64 %{} to ptr", vtable_ptr, vt));
                                    // Get method function pointer
                                    let method_ptr = self.fresh_label();
                                    self.e(&format!("%{} = getelementptr inbounds %class.{}.vtable, ptr %{}, i32 0, i32 {}", method_ptr, cls_name, vtable_ptr, method_idx));
                                    let fn_ptr = self.fresh_label();
                                    self.e(&format!("%{} = load ptr, ptr %{}", fn_ptr, method_ptr));
                                    // Cast self to i64
                                    let self_int = self.fresh_label();
                                    self.e(&format!("%{} = ptrtoint ptr %{} to i64", self_int, alloca_ptr));
                                    // Build args: self + all original args
                                    let result_tmp = self.fresh_label();
                                    let call_ret = self.class_method_ret_types.get(&(cls_name.to_string(), field.clone())).cloned().unwrap_or_else(|| "i64".into());
                                    self.e(&format!("%{} = call {} (i64) %{}(i64 %{})", result_tmp, call_ret, fn_ptr, self_int));
                                    (self.ssa(&result_tmp), call_ret)
                                } else {
                                    let tmp = self.fresh_label();
                                    let args_str: Vec<String> = arg_results.iter().map(|(v, t)| format!("{} {}", t, v)).collect();
                                    self.e(&format!("%{} = call i64 @{}({})", tmp, o_val, args_str.join(", ")));
                                    (self.ssa(&tmp), "i64".into())
                                }
                            } else {
                                let tmp = self.fresh_label();
                                let args_str: Vec<String> = arg_results.iter().map(|(v, t)| format!("{} {}", t, v)).collect();
                                self.e(&format!("%{} = call i64 @{}({})", tmp, o_val, args_str.join(", ")));
                                (self.ssa(&tmp), "i64".into())
                            }
                        } else {
                            let tmp = self.fresh_label();
                            let args_str: Vec<String> = arg_results.iter().map(|(v, t)| format!("{} {}", t, v)).collect();
                            self.e(&format!("%{} = call i64 @{}({})", tmp, o_val, args_str.join(", ")));
                            (self.ssa(&tmp), "i64".into())
                        }
                    }
                }
            }
            _ => ("0".into(), "i64".into()),
        }
    }

    fn compile_pattern_match(&mut self, pattern: &Pattern, scrutinee_val: &str, scrutinee_ty: &str) -> Option<String> {
        match pattern {
            Pattern::Ignore => None, // always matches
            Pattern::Ident(_) | Pattern::Rest(_) => None, // always matches (bind)
            Pattern::LitInt(n) => {
                if scrutinee_ty == "i64" {
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = icmp eq i64 {}, {}", tmp, scrutinee_val, n));
                    Some(self.ssa(&tmp))
                } else {
                    Some("true".into())
                }
            }
            Pattern::LitReal(n) => {
                if scrutinee_ty == "double" {
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = fcmp oeq double {}, {:.10}", tmp, scrutinee_val, n));
                    Some(self.ssa(&tmp))
                } else {
                    Some("true".into())
                }
            }
            Pattern::LitBool(b) => {
                let v = if *b { "true" } else { "false" };
                if scrutinee_ty == "i1" {
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = icmp eq i1 {}, {}", tmp, scrutinee_val, v));
                    Some(self.ssa(&tmp))
                } else {
                    Some(v.into())
                }
            }
            Pattern::LitStr(s) => {
                if scrutinee_ty == "%yk_string" {
                    let len_check = self.fresh_label();
                    let target_len = s.len() as i64;
                    self.e(&format!("%{} = extractvalue %yk_string {}, 1", len_check, scrutinee_val));
                    let len_match = self.fresh_label();
                    self.e(&format!("%{} = icmp eq i64 %{}, {}", len_match, len_check, target_len));
                    let memcmp_result = self.fresh_label();
                    let data_ptr = self.fresh_label();
                    self.e(&format!("%{} = extractvalue %yk_string {}, 0", data_ptr, scrutinee_val));
                    let str_val = self.make_string_slot(s);
                    let str_slot = self.fresh_label();
                    self.e(&format!("%{} = alloca %yk_string, align 8", str_slot));
                    self.e(&format!("store %yk_string {}, ptr %{}", str_val, str_slot));
                    let str_data = self.fresh_label();
                    self.e(&format!("%{} = getelementptr inbounds %yk_string, ptr %{}, i32 0, i32 0", str_data, str_slot));
                    let str_data_ptr = self.fresh_label();
                    self.e(&format!("%{} = load ptr, ptr %{}", str_data_ptr, str_data));
                    self.e(&format!("%{} = call i32 @memcmp(ptr %{}, ptr %{}, i64 {})", memcmp_result, data_ptr, str_data_ptr, target_len));
                    let eq_result = self.fresh_label();
                    self.e(&format!("%{} = icmp eq i32 %{}, 0", eq_result, memcmp_result));
                    let final_result = self.fresh_label();
                    self.e(&format!("%{} = and i1 %{}, %{}", final_result, len_match, eq_result));
                    Some(self.ssa(&final_result))
                } else {
                    Some("true".into())
                }
            }
            Pattern::Variant(variant_name, _subpatterns) => {
                if scrutinee_ty == "%yk_variant" {
                    let expected_tag = self.variant_name_tag(variant_name);
                    let tag_val = self.fresh_label();
                    self.e(&format!("%{} = extractvalue %yk_variant {}, 0", tag_val, scrutinee_val));
                    let tag_match = self.fresh_label();
                    self.e(&format!("%{} = icmp eq i64 %{}, {}", tag_match, tag_val, expected_tag));
                    Some(self.ssa(&tag_match))
                } else {
                    Some("true".into())
                }
            }
            Pattern::Destruct(_) => Some("true".into()),
            Pattern::ListDestruct(_) => Some("true".into()),
        }
    }

    fn compile_pattern_bind(&mut self, pattern: &Pattern, scrutinee_val: &str, scrutinee_ty: &str) {
        match pattern {
            Pattern::Ident(name) => {
                let ptr = self.alloca_name(name);
                self.var_alloca.insert(name.clone(), ptr.clone());
                self.var_types.insert(name.clone(), scrutinee_ty.to_string());
                self.e(&format!("{} = alloca {}, align 8", ptr, scrutinee_ty));
                self.e(&format!("store {} {}, ptr {}", scrutinee_ty, scrutinee_val, ptr));
            }
            Pattern::Rest(name) => {
                let ptr = self.alloca_name(name);
                self.var_alloca.insert(name.clone(), ptr.clone());
                self.var_types.insert(name.clone(), scrutinee_ty.to_string());
                self.e(&format!("{} = alloca {}, align 8", ptr, scrutinee_ty));
                self.e(&format!("store {} {}, ptr {}", scrutinee_ty, scrutinee_val, ptr));
            }
            Pattern::ListDestruct(patterns) => {
                for (idx, pat) in patterns.iter().enumerate() {
                    match pat {
                        Pattern::Ident(name) => {
                            let ptr = self.alloca_name(name);
                            self.var_alloca.insert(name.clone(), ptr.clone());
                            self.var_types.insert(name.clone(), "i64".into());
                            self.e(&format!("{} = alloca i64, align 8", ptr));
                            // Try extractvalue for tuples
                            if scrutinee_ty.starts_with("%tuple.") {
                                let field = self.fresh_label();
                                self.e(&format!("%{} = extractvalue {} {}, {}", field, scrutinee_ty, scrutinee_val, idx));
                                self.e(&format!("store i64 %{}, ptr {}", field, ptr));
                            } else {
                                self.e(&format!("store i64 0, ptr {}", ptr));
                            }
                        }
                        Pattern::Rest(name) => {
                            let ptr = self.alloca_name(name);
                            self.var_alloca.insert(name.clone(), ptr.clone());
                            self.var_types.insert(name.clone(), "i64".into());
                            self.e(&format!("{} = alloca i64, align 8", ptr));
                            self.e(&format!("store i64 0, ptr {}", ptr));
                        }
                        _ => self.compile_pattern_bind(pat, scrutinee_val, scrutinee_ty),
                    }
                }
            }
            Pattern::Destruct(fields) => {
                let struct_name = scrutinee_ty.strip_prefix("%struct.").unwrap_or("").to_string();
                let field_map: Vec<(String, String, usize)> = if !struct_name.is_empty() {
                    if let Some(defs) = self.struct_defs.get(struct_name.as_str()) {
                        fields.iter().filter_map(|(fname, _pat)| {
                            defs.iter().position(|(n, _)| n == fname)
                                .map(|idx| (fname.clone(), defs[idx].1.clone(), idx))
                        }).collect()
                    } else { Vec::new() }
                } else { Vec::new() };
                if !field_map.is_empty() {
                    for (fname, fty, idx) in &field_map {
                        let field_val = self.fresh_label();
                        self.e(&format!("%{} = extractvalue {} {}, {}", field_val, scrutinee_ty, scrutinee_val, idx));
                        let field_str = self.ssa(&field_val);
                        let loaded_str = field_str.clone();
                        let pat = fields.iter().find(|(n, _)| n == fname).map(|(_, p)| p).unwrap();
                        self.compile_pattern_bind(pat, &loaded_str, fty);
                    }
                } else {
                    for (_fname, pat) in fields {
                        self.compile_pattern_bind(pat, scrutinee_val, scrutinee_ty);
                    }
                }
            }
            Pattern::Variant(_variant_name, subpatterns) => {
                if scrutinee_ty == "%yk_variant" {
                    let payload = self.fresh_label();
                    self.e(&format!("%{} = extractvalue %yk_variant {}, 1", payload, scrutinee_val));
                    let payload_str = self.ssa(&payload);
                    for pat in subpatterns {
                        self.compile_pattern_bind(pat, &payload_str, "i64");
                    }
                } else {
                    for pat in subpatterns {
                        self.compile_pattern_bind(pat, scrutinee_val, scrutinee_ty);
                    }
                }
            }
            _ => {}
        }
    }
}

pub fn compile_to_llvm(module: &Module) -> String {
    compile_to_llvm_modules(&[module])
}

pub fn compile_to_llvm_modules(modules: &[&Module]) -> String {
    let mut codegen = LlvmCodegen::new();
    codegen.compile_modules(modules)
}

pub fn validate_modules(modules: &[&Module], file_paths: &[String]) -> Vec<String> {
    let unsupported_stds: [&str; 0] = [];
    let mut errors = Vec::new();
    for (mod_idx, module) in modules.iter().enumerate() {
        let fname = file_paths.get(mod_idx).map(|s| s.as_str()).unwrap_or("?");
        for import in &module.imports {
            if import.lang.is_none() && unsupported_stds.contains(&import.source.as_str()) {
                errors.push(format!("{}: std module '{}' not supported in AOT compilation (use `run` instead)", fname, import.source));
            }
        }
        for item in &module.items {
            match &item.value {
                ItemKind::Object { .. } => {
                    errors.push(format!("{}: objects not supported in AOT", fname));
                }
                ItemKind::Interface { .. } => {
                    errors.push(format!("{}: interfaces not supported in AOT", fname));
                }
                _ => {}
            }
        }
    }
    errors
}

fn detect_clang() -> Option<String> {
    // Check common LLVM installation paths
    let candidates = [
        r"C:\Program Files\LLVM\bin\clang.exe",
        r"C:\Program Files (x86)\LLVM\bin\clang.exe",
    ];
    for p in &candidates {
        if std::path::Path::new(p).exists() { return Some(p.to_string()); }
    }
    // Check PATH
    std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p).find_map(|d| {
            let c = d.join("clang.exe");
            if c.exists() { Some(c.to_string_lossy().to_string()) } else { None }
        })
    })
}

pub fn detect_vcvars() -> Option<String> {
    // Try vswhere
    let vswhere = r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe";
    if std::path::Path::new(vswhere).exists() {
        if let Ok(out) = std::process::Command::new(vswhere)
            .args(["-latest", "-property", "installationPath"])
            .output()
        {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let bat = format!(r"{}\VC\Auxiliary\Build\vcvars64.bat", path);
            if std::path::Path::new(&bat).exists() { return Some(bat); }
        }
    }
    // Fallback: common paths
    let candidates = [
        r"C:\Program Files\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat",
        r"C:\Program Files (x86)\Microsoft Visual Studio\2019\Community\VC\Auxiliary\Build\vcvars64.bat",
        r"C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat",
    ];
    for p in &candidates {
        if std::path::Path::new(p).exists() { return Some(p.to_string()); }
    }
    None
}

pub fn compile_to_exe(llvm_ir: &str, output_path: &Path, hw: &HardwareInfo) -> Result<()> {
    // Fallback: batch script with clang + MSVC (use instead of in-process LLVM-C due to LLVM 18+ API incompatibility)
    compile_to_exe_batch(llvm_ir, output_path, hw, &[])
}

pub fn compile_to_exe_with_extra_libs(llvm_ir: &str, output_path: &Path, hw: &HardwareInfo, extra_libs: &[String]) -> Result<()> {
    compile_to_exe_batch(llvm_ir, output_path, hw, extra_libs)
}

/// Fast compilation path using in-process LLVM API (no batch script, no clang).
/// Falls back silently if LLVM-C DLL is not available.
#[allow(dead_code)]
fn compile_to_exe_fast(llvm_ir: &str, output_path: &Path, hw: &HardwareInfo) -> Result<()> {
    let obj_path = output_path.with_extension("obj");
    let exe_path = output_path.with_extension("exe");

    // Try to load LLVM-C and emit object in-process
    let api_path = match crate::codegen::llvm_api::find_llvm_lib() {
        Some(p) => p,
        None => return Err(error::err(ErrorKind::Internal, Span::new(0, 0), "LLVM-C not found")),
    };
    let api = crate::codegen::llvm_api::LlvmApi::load(&api_path)?;
    emit_obj_in_memory(&api, llvm_ir, &obj_path, hw)?;

    // Ensure runtime object is cached
    let runtime_obj = cache_runtime_obj(&obj_path)?;

    // Single link command (no batch script, no clang)
    let vcvars = detect_vcvars()
        .ok_or_else(|| error::err(ErrorKind::Internal, Span::new(0, 0),
            "Visual Studio not found"))?;
    let status = std::process::Command::new("cmd.exe")
        .args(["/c", &format!(r#""{}" x64 >nul 2>&1 && link.exe /nologo "{}" "{}" /OUT:"{}" /defaultlib:libcmt.lib"#,
            vcvars, obj_path.to_string_lossy(), runtime_obj.to_string_lossy(), exe_path.to_string_lossy())])
        .status()
        .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0),
            format!("link.exe failed: {}", e)))?;
    if !status.success() {
        return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
            format!("link.exe exited with code {:?}", status.code())));
    }

    // Cleanup obj files
    let _ = std::fs::remove_file(&obj_path);
    let _ = std::fs::remove_file(&runtime_obj);
    Ok(())
}

pub fn emit_obj_in_memory(api: &crate::codegen::llvm_api::LlvmApi, llvm_ir: &str, obj_path: &Path, hw: &HardwareInfo) -> Result<()> {
    unsafe {
        if let Some(f) = api.LLVMInitializeX86TargetInfo { f(); }
        if let Some(f) = api.LLVMInitializeX86Target { f(); }
        if let Some(f) = api.LLVMInitializeX86TargetMC { f(); }
        if let Some(f) = api.LLVMInitializeX86AsmPrinter { f(); }

        let ctx = (api.LLVMContextCreate)();
        let ir_str = std::ffi::CString::new(llvm_ir)
            .map_err(|_| error::err(ErrorKind::Internal, Span::new(0, 0), "LLVM IR contains null byte"))?;
        let name = std::ffi::CString::new("yk").unwrap();
        let membuf = (api.LLVMCreateMemoryBufferWithMemoryRange)(ir_str.as_ptr(), llvm_ir.len(), name.as_ptr(), 1);
        let mut module: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut err: *mut i8 = std::ptr::null_mut();
        let parse_rc = (api.LLVMParseIRInContext)(ctx, membuf, &mut module, &mut err);
        if parse_rc != 0 {
            let err_str = api.get_error(err);
            (api.LLVMDisposeMemoryBuffer)(membuf);
            (api.LLVMContextDispose)(ctx);
            return Err(error::err(ErrorKind::Internal, Span::new(0, 0), format!("LLVM IR parse failed: {}", err_str)));
        }
        (api.LLVMDisposeMemoryBuffer)(membuf);

        // Run optimization passes
        let pm = (api.LLVMCreatePassManager)();
        if let Some(f) = api.LLVMAddConstantPropagationPass { f(pm); }
        if let Some(f) = api.LLVMAddInstructionCombiningPass { f(pm); }
        if let Some(f) = api.LLVMAddGVNPass { f(pm); }
        if let Some(f) = api.LLVMAddAggressiveDCEPass { f(pm); }
        (api.LLVMRunPassManager)(pm, module);
        (api.LLVMDisposePassManager)(pm);

        // Use hardware-adaptive optimization level
        let opt_level: i32 = if crate::hardware::memory::is_low_memory(&hw.memory) { 2 }
            else { 3 };
        let triple_c = std::ffi::CString::new(hw.os.triple.as_str()).unwrap();
        let cpu_c = std::ffi::CString::new(hw.cpu.name.as_str()).unwrap();
        let features = hw.cpu.simd.to_llvm_features().join(",");
        let features_c = std::ffi::CString::new(features).unwrap();
        (api.LLVMSetTarget)(module, triple_c.as_ptr());

        let mut target_ref = (api.LLVMGetFirstTarget)();
        if target_ref.is_null() {
            let mut err_target: *mut i8 = std::ptr::null_mut();
            let rc = (api.LLVMGetTargetFromTriple)(triple_c.as_ptr(), &mut target_ref, &mut err_target);
            if rc != 0 || target_ref.is_null() {
                let err_str = if !err_target.is_null() { api.get_error(err_target) } else { "unknown".into() };
                (api.LLVMDisposeModule)(module);
                (api.LLVMContextDispose)(ctx);
                return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                    format!("LLVM: no target for triple '{}' ({})", hw.os.triple, err_str)));
            }
        }
        // Hardware-adaptive compilation
        let tm = (api.LLVMCreateTargetMachine)(target_ref, triple_c.as_ptr(), cpu_c.as_ptr(), features_c.as_ptr(), opt_level, 2, 0);
        if tm.is_null() {
            (api.LLVMDisposeModule)(module);
            (api.LLVMContextDispose)(ctx);
            return Err(error::err(ErrorKind::Internal, Span::new(0, 0), "LLVM: failed to create target machine"));
        }

        let td = (api.LLVMCreateTargetDataLayout)(tm);
        (api.LLVMSetModuleDataLayout)(module, td);

        // Emit to memory buffer
        let mut membuf_out: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut err2: *mut i8 = std::ptr::null_mut();
        let emit_result = (api.LLVMTargetMachineEmitToMemoryBuffer)(tm, module, 1, &mut err2, &mut membuf_out);
        if emit_result != 0 {
            let err_str = api.get_error(err2);
            (api.LLVMDisposeTargetMachine)(tm);
            (api.LLVMDisposeModule)(module);
            (api.LLVMContextDispose)(ctx);
            return Err(error::err(ErrorKind::Internal, Span::new(0, 0), format!("LLVM emit failed: {}", err_str)));
        }

        // Write memory buffer to file
        let buf_ptr = (api.LLVMGetBufferStart)(membuf_out);
        let buf_size = (api.LLVMGetBufferSize)(membuf_out);
        let obj_data = std::slice::from_raw_parts(buf_ptr as *const u8, buf_size);
        std::fs::write(obj_path, obj_data)
            .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0), format!("Failed to write obj: {}", e)))?;

        (api.LLVMDisposeMemoryBuffer)(membuf_out);
        (api.LLVMDisposeTargetMachine)(tm);
        (api.LLVMDisposeModule)(module);
        (api.LLVMContextDispose)(ctx);
        Ok(())
    }
}

/// Cache the pre-compiled runtime C object file to avoid recompiling every time
#[allow(dead_code)]
fn cache_runtime_obj(_output_path: &Path) -> Result<std::path::PathBuf> {
    let cache_dir = std::env::temp_dir().join("yk_cache");
    let _ = std::fs::create_dir_all(&cache_dir);
    let cached_obj = cache_dir.join("yk_rt.obj");

    if !cached_obj.exists() {
        let runtime_c_path = cache_dir.join("yk_rt.c");
        std::fs::write(&runtime_c_path, RUNTIME_C)
            .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0),
                format!("Failed to write runtime C: {}", e)))?;

        let vcvars = detect_vcvars()
            .ok_or_else(|| error::err(ErrorKind::Internal, Span::new(0, 0), "Visual Studio not found"))?;
        let rt_c_str = runtime_c_path.to_string_lossy();
        let rt_obj_str = cached_obj.to_string_lossy();
        let status = std::process::Command::new("cmd.exe")
            .args(["/c", &format!(r#""{}" x64 >nul 2>&1 && cl.exe /nologo /TP /EHsc /std:c++17 /c "{}" /Fo:"{}" /utf-8"#,
                vcvars, rt_c_str, rt_obj_str)])

            .status()
            .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0),
                format!("cl.exe failed: {}", e)))?;
        if !status.success() {
            return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
                format!("cl.exe exited with code {:?}", status.code())));
        }
    }
    Ok(cached_obj)
}

fn compile_to_exe_batch(llvm_ir: &str, output_path: &Path, hw: &HardwareInfo, extra_libs: &[String]) -> Result<()> {
    let ll_path = output_path.with_extension("ll");
    std::fs::write(&ll_path, llvm_ir)
        .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0),
            format!("Failed to write {}: {}", ll_path.display(), e)))?;

    let obj_path = output_path.with_extension("obj");
    let exe_path = output_path.with_extension("exe");

    let runtime_dir = output_path.parent().unwrap_or(Path::new("."));
    let runtime_c_path = runtime_dir.join("yk_rt.c");
    std::fs::write(&runtime_c_path, RUNTIME_C)
        .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0),
            format!("Failed to write {}: {}", runtime_c_path.display(), e)))?;
    let runtime_obj_path = runtime_dir.join("yk_rt.obj");

    let vcvars = detect_vcvars()
        .ok_or_else(|| error::err(ErrorKind::Internal, Span::new(0, 0),
            "Visual Studio 2019/2022/2025 not found. Install Build Tools or set PATH manually.".to_string()))?;
    let clang = detect_clang()
        .ok_or_else(|| error::err(ErrorKind::Internal, Span::new(0, 0),
            "clang.exe not found. Install LLVM from https://llvm.org or add it to PATH.".to_string()))?;

    let opt_level = if crate::hardware::memory::is_low_memory(&hw.memory) { "O2" }
        else { "O3" };
    let target = &hw.os.triple;
    let march = hw.cpu.name.replace("x86_64", "x86-64");
    let simd_features = hw.cpu.simd.to_llvm_features().join(",");
    let target_features = if simd_features.is_empty() { String::new() }
        else { format!(" -Xclang -target-feature -Xclang {}", simd_features) };

    let bat_dir = std::env::temp_dir();
    let bat_name = format!("yk_build_{:x}.bat", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
    let bat_path = bat_dir.join(&bat_name);
    let exe_str = exe_path.to_string_lossy();
    let ll_str = ll_path.to_string_lossy();
    let obj_str = obj_path.to_string_lossy();
    let rt_c_str = runtime_c_path.to_string_lossy();
    let rt_obj_str = runtime_obj_path.to_string_lossy();

    let extra_libs_str = if extra_libs.is_empty() {
        String::new()
    } else {
        format!(" {}", extra_libs.iter().map(|l| format!("\"{}\"", l)).collect::<Vec<_>>().join(" "))
    };

    let bat_content = format!(
        "@echo off\r\n\
         call \"{vcvars}\" x64 >nul 2>&1\r\n\
         if errorlevel 1 exit /b 1\r\n\
         \r\n\
         :: Compile LLVM IR to object file (hardware-adaptive)\r\n\
         \"{clang}\" -c \"{ll}\" -o \"{obj}\" -target {target} -{opt} -march={march}{features}\r\n\
         if errorlevel 1 exit /b 1\r\n\
         \r\n\
         :: Compile runtime C to object file\r\n\
         cl.exe /nologo /TP /EHsc /std:c++17 /c \"{rtc}\" /Fo:\"{rto}\" /utf-8\r\n\
         if errorlevel 1 exit /b 1\r\n\
         \r\n\
         :: Link objects into executable\r\n\
         link.exe /nologo \"{obj}\" \"{rto}\"{extra} /OUT:\"{exe}\" /defaultlib:libcmt.lib\r\n\
         exit /b %errorlevel%\r\n",
        vcvars=vcvars, clang=clang, ll=ll_str, obj=obj_str, target=target,
        opt=opt_level, march=march, features=target_features,
        rtc=rt_c_str, rto=rt_obj_str, extra=extra_libs_str, exe=exe_str
    );

    std::fs::write(&bat_path, bat_content)
        .map_err(|e| error::err(ErrorKind::Io, Span::new(0, 0),
            format!("Failed to write build script: {}", e)))?;

    let result = Command::new("cmd.exe")
        .args(["/c", &bat_path.to_string_lossy()])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| {
            let _ = std::fs::remove_file(&bat_path);
            error::err(ErrorKind::Io, Span::new(0, 0),
                format!("Failed to invoke build: {}", e))
        })?;

    let _ = std::fs::remove_file(&bat_path);

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let stdout = String::from_utf8_lossy(&result.stdout);
        let exit_code = result.status.code().unwrap_or(-1);
        return Err(error::err(ErrorKind::Internal, Span::new(0, 0),
            format!("Compilation failed (exit={}):\nSTDOUT:\n{}\nSTDERR:\n{}",
                exit_code, stdout, stderr)));
    }

    let _ = std::fs::remove_file(&ll_path);
    let _ = std::fs::remove_file(&runtime_c_path);
    let _ = std::fs::remove_file(&runtime_obj_path);
    let _ = std::fs::remove_file(&obj_path);
    Ok(())
}

/// Generate LLVM IR for a handler function that returns a static string.
///
/// The generated function has C ABI: `void handler_name(struct YkResponse* resp)`
/// and writes the response body, length, and status code into the struct.
///
/// # Arguments
/// * `handler_name` - The name of the LLVM function (also used as JIT symbol)
/// * `response_body` - The static response body string
/// * `status_code` - HTTP status code (e.g. 200)
pub fn generate_static_handler_ir(handler_name: &str, response_body: &str, status_code: i32) -> String {
    let body_len = response_body.len();
    let escaped_body = response_body
        .replace('\\', "\\\\")
        .replace('"', "\\22")
        .replace('\n', "\\0A")
        .replace('\r', "\\0D")
        .replace('\t', "\\09");
    let cstr_name = format!("@__yk_cstr_{}", handler_name);

    format!(
        r#"; JIT-compiled handler for '{handler_name}'
target triple = "x86_64-pc-windows-msvc"

{global_str} = private unnamed_addr constant [{array_len} x i8] c"{escaped_body}\00"

define void @{handler_name}(ptr %resp) {{
entry:
  ; Write body pointer at byte offset 0
  %bp = getelementptr i8, ptr %resp, i32 0
  store ptr {global_str}, ptr %bp
  ; Write body_len at byte offset 8
  %lp = getelementptr i8, ptr %resp, i32 8
  store i64 {body_len}, ptr %lp
  ; Write status_code at byte offset 16
  %sp = getelementptr i8, ptr %resp, i32 16
  store i32 {status_code}, ptr %sp
  ret void
}}
"#,
        handler_name = handler_name,
        global_str = cstr_name,
        escaped_body = escaped_body,
        array_len = body_len + 1,
        body_len = body_len,
        status_code = status_code,
    )
}

/// Generate LLVM IR for a handler function from an AST FnDef body.
///
/// The generated function has C ABI:
///   void @handler_name(ptr %resp, ptr %req, ptr %buf, i64 %buf_len)
///
/// It evaluates the FnDef body (supporting string literals, req field access,
/// and string concatenation of literals + req fields) and writes the result
/// into the caller-provided %buf buffer, updating %resp.
///
/// Handler parameter `req` maps to the %req struct pointer. The struct layout:
///   offsets: method(0 ptr, 8 len), path(16 ptr, 24 len), body(32 ptr, 40 len)
pub fn generate_fn_handler_ir(handler_name: &str, fndef: &crate::interpret::FnDef) -> Option<String> {
    let body = &fndef.body;
    let ret_expr = find_return_expr(body)?;
    let mut gen = FnIrGen::new();
    Some(gen.gen_fn(handler_name, ret_expr))
}

fn find_return_expr(body: &[StmtNode]) -> Option<&ExprNode> {
    for stmt in body.iter().rev() {
        match &stmt.value {
            Stmt::Return(Some(e)) => return Some(e),
            Stmt::Return(None) => return None,
            Stmt::Expr(e) => return Some(e),
            _ => {}
        }
    }
    None
}

struct FnIrGen {
    label: usize,
    output: String,
    string_constants: String,
}

impl FnIrGen {
    fn new() -> Self {
        FnIrGen { label: 0, output: String::new(), string_constants: String::new() }
    }

    fn fresh(&mut self) -> String {
        let n = self.label;
        self.label += 1;
        format!("%l{}", n)
    }

    fn e(&mut self, s: &str) {
        use std::fmt::Write;
        writeln!(self.output, "  {}", s).unwrap();
    }

    fn gen_fn(&mut self, handler_name: &str, ret_expr: &ExprNode) -> String {
        self.output.clear();
        self.string_constants.clear();

        // Build the full IR in correct order: target triple, types, constants, function
        let mut ir = String::new();
        ir.push_str(&format!("; JIT-compiled FnDef handler for '{}'\n", handler_name));
        ir.push_str("target triple = \"x86_64-pc-windows-msvc\"\n");
        ir.push_str("\n");
        ir.push_str("%YkResponse = type { ptr, i64, i32 }\n");
        ir.push_str("\n");

        // Generate function body into self.output & string constants
        self.output.push_str(&format!(
            "define void @{}(ptr %resp, ptr %req, ptr %buf, i64 %buf_len) {{\n",
            handler_name
        ));
        self.output.push_str("entry:\n");

        let (ptr_val, len_val) = self.gen_expr(ret_expr);

        self.e(&format!("store ptr {}, ptr %resp", ptr_val));
        self.e(&format!("%lp = getelementptr i8, ptr %resp, i32 8"));
        self.e(&format!("store i64 {}, ptr %lp", len_val));
        self.e(&format!("%sp = getelementptr i8, ptr %resp, i32 16"));
        self.e("store i32 200, ptr %sp");
        self.e("ret void");
        self.output.push_str("}\n");

        // Assemble: constants then function body
        ir.push_str(&self.string_constants);
        ir.push('\n');
        ir.push_str(&self.output);
        ir
    }

    fn gen_str_constant(&mut self, s: &str) -> (String, String) {
        let idx = self.label;
        self.label += 1;
        let cstr_name = format!("@__yk_cstr_{}", idx);
        let escaped = s.replace('\\', "\\\\").replace('"', "\\22").replace('\n', "\\0A").replace('\r', "\\0D");
        let arr_len = s.len() + 1;
        use std::fmt::Write;
        writeln!(self.string_constants, "{cstr_name} = private unnamed_addr constant [{arr_len} x i8] c\"{escaped}\\00\"").unwrap();
        let ptr = self.fresh();
        self.e(&format!("{ptr} = getelementptr inbounds [{arr_len} x i8], ptr {cstr_name}, i64 0, i64 0"));
        let len_val = s.len().to_string();
        (ptr, len_val)
    }

    fn gen_int64(&mut self, expr: &ExprNode) -> String {
        match &expr.value {
            Expr::LitInt(n) => n.to_string(),
            _ => "0".into(),
        }
    }

    fn gen_condition(&mut self, expr: &ExprNode) -> String {
        match &expr.value {
            Expr::LitBool(b) => {
                if *b { "true".into() } else { "false".into() }
            }
            Expr::BinOp(l, BinOp::Eq, r) => {
                let lhs = self.gen_int64(l);
                let rhs = self.gen_int64(r);
                let r = self.fresh();
                self.e(&format!("{r} = icmp eq i64 {lhs}, {rhs}"));
                r
            }
            Expr::BinOp(l, BinOp::Ne, r) => {
                let lhs = self.gen_int64(l);
                let rhs = self.gen_int64(r);
                let r = self.fresh();
                self.e(&format!("{r} = icmp ne i64 {lhs}, {rhs}"));
                r
            }
            _ => "true".into(),
        }
    }

    fn gen_expr(&mut self, expr: &ExprNode) -> (String, String) {
        match &expr.value {
            Expr::LitStr(s) => self.gen_str_constant(s),
            Expr::LitInt(n) => self.gen_str_constant(&n.to_string()),
            Expr::LitBool(b) => self.gen_str_constant(if *b { "true" } else { "false" }),
            Expr::LitChar(c) => self.gen_str_constant(&c.to_string()),
            Expr::LitReal(v) => self.gen_str_constant(&format!("{}", v)),
            Expr::LitHex(n) => self.gen_str_constant(&format!("{}", n)),
            Expr::If(cond, then_expr, else_expr) => {
                let cond_i1 = self.gen_condition(cond);
                let (then_ptr, then_len) = self.gen_expr(then_expr);
                let (else_ptr, else_len) = if let Some(ee) = else_expr {
                    self.gen_expr(ee)
                } else {
                    let empty = self.fresh();
                    self.e(&format!("{empty} = getelementptr i8, ptr %buf, i64 0"));
                    (empty, "0".into())
                };
                let ptr = self.fresh();
                self.e(&format!("{ptr} = select i1 {cond_i1}, ptr {then_ptr}, ptr {else_ptr}"));
                let len = self.fresh();
                self.e(&format!("{len} = select i1 {cond_i1}, i64 {then_len}, i64 {else_len}"));
                (ptr, len)
            }
            Expr::Field(obj, field) => {
                if let Expr::Ident(name) = &obj.value {
                    if name == "req" {
                        let (field_offset_ptr, field_offset_len) = match field.as_str() {
                            "method" => (0i32, 8i32),
                            "path" => (16, 24),
                            "body" => (32, 40),
                            _ => (32, 40),
                        };
                        let ptr_label = self.fresh();
                        let len_label = self.fresh();
                        self.e(&format!("; Load req.{}", field));
                        self.e(&format!("{ptr_label} = getelementptr i8, ptr %req, i32 {field_offset_ptr}"));
                        let ptr_v = self.fresh();
                        self.e(&format!("{ptr_v} = load ptr, ptr {ptr_label}"));
                        self.e(&format!("{len_label} = getelementptr i8, ptr %req, i32 {field_offset_len}"));
                        let len_v = self.fresh();
                        self.e(&format!("{len_v} = load i64, ptr {len_label}"));
                        (ptr_v, len_v)
                    } else {
                        let empty_ptr = self.fresh();
                        self.e(&format!("{empty_ptr} = getelementptr i8, ptr %buf, i64 0"));
                        (empty_ptr, "0".into())
                    }
                } else {
                    let empty_ptr = self.fresh();
                    self.e(&format!("{empty_ptr} = getelementptr i8, ptr %buf, i64 0"));
                    (empty_ptr, "0".into())
                }
            }
            Expr::Ident(name) => {
                if name == "req" {
                    self.gen_expr(
                        &ExprNode::new(0, Span::new(0, 0),
                            Expr::Field(Box::new(ExprNode::new(0, Span::new(0, 0), Expr::Ident("req".into()))), "body".into())),
                    )
                } else {
                    let empty_ptr = self.fresh();
                    self.e(&format!("{empty_ptr} = getelementptr i8, ptr %buf, i64 0"));
                    (empty_ptr, "0".into())
                }
            }
            Expr::BinOp(l, op, r) if *op == BinOp::Add => {
                let (l_ptr, l_len) = self.gen_expr(l);
                let (r_ptr, r_len) = self.gen_expr(r);

                let total = self.fresh();
                self.e(&format!("{total} = add i64 {l_len}, {r_len}")); // unused, replaced by clamped

                // Clamp left to buf_len
                let l_clamped = self.fresh();
                self.e(&format!("{l_clamped} = icmp ugt i64 {l_len}, %buf_len"));
                let l_actual = self.fresh();
                self.e(&format!("{l_actual} = select i1 {l_clamped}, i64 %buf_len, i64 {l_len}"));
                self.e(&format!("call void @llvm.memcpy.p0.p0.i64(ptr %buf, ptr {l_ptr}, i64 {l_actual}, i1 false)"));

                let r_offset = self.fresh();
                self.e(&format!("{r_offset} = getelementptr i8, ptr %buf, i64 {l_actual}"));
                let r_remaining = self.fresh();
                self.e(&format!("{r_remaining} = sub i64 %buf_len, {l_actual}"));
                let r_clamped = self.fresh();
                self.e(&format!("{r_clamped} = icmp ugt i64 {r_len}, {r_remaining}"));
                let r_actual = self.fresh();
                self.e(&format!("{r_actual} = select i1 {r_clamped}, i64 {r_remaining}, i64 {r_len}"));
                self.e(&format!("call void @llvm.memcpy.p0.p0.i64(ptr {r_offset}, ptr {r_ptr}, i64 {r_actual}, i1 false)"));

                let actual_len = self.fresh();
                self.e(&format!("{actual_len} = add i64 {l_actual}, {r_actual}"));

                ("%buf".into(), actual_len)
            }
            Expr::Block(stmts) => {
                if let Some(ret) = find_return_expr(stmts) {
                    self.gen_expr(ret)
                } else {
                    let empty_ptr = self.fresh();
                    self.e(&format!("{empty_ptr} = getelementptr i8, ptr %buf, i64 0"));
                    (empty_ptr, "0".into())
                }
            }
            _ => {
                let empty_ptr = self.fresh();
                self.e(&format!("{empty_ptr} = getelementptr i8, ptr %buf, i64 0"));
                (empty_ptr, "0".into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::parser::Parser;
    use crate::syntax::ast;

    #[test]
    fn test_struct_lit_and_field() {
        ast::reset_ids();
        let module = Parser::parse("struct Point { x: int, y: int } fn main() { p: Point = Point { x: 1, y: 2 }; print(p.x); }").unwrap();
        let llvm = compile_to_llvm(&module);
        assert!(llvm.contains("%struct.Point = type { i64, i64 }"));
        assert!(llvm.contains("insertvalue %struct.Point undef, i64 1, 0"));
        assert!(llvm.contains("insertvalue %struct.Point %"));
        assert!(llvm.contains("extractvalue %struct.Point"));
    }

    #[test]
    fn test_tuple_lit() {
        ast::reset_ids();
        let module = Parser::parse("fn main() { print((10, 20, 30).0); }").unwrap();
        let llvm = compile_to_llvm(&module);
        eprintln!("LLVM OUTPUT:\n{}", llvm);
        assert!(llvm.contains("i64, i64, i64"));
        assert!(llvm.contains("insertvalue"));
    }

    #[test]
    fn test_generate_fn_handler_ir_literal() {
        // Manually construct a FnDef that returns a string literal
        use crate::syntax::ast::{Param, TypeNode, Stmt};
        use crate::interpret::FnDef;
        let param = Param {
            name: "req".into(),
            type_expr: TypeNode::new(fresh_id(), Span::new(0, 0), TypeExpr::Infer),
            is_ref: false,
        };
        let body = vec![
            StmtNode::new(fresh_id(), Span::new(0, 0), Stmt::Return(Some(
                ExprNode::new(fresh_id(), Span::new(0, 0), Expr::LitStr("Hello World".into()))
            ))),
        ];
        let fndef = FnDef::new(vec![param], body);
        let ir = generate_fn_handler_ir("test_fn_handler", &fndef)
            .expect("Should generate IR");
        assert!(ir.contains("define void @test_fn_handler"));
        assert!(ir.contains("target triple = \"x86_64-pc-windows-msvc\""));
        assert!(ir.contains("%YkResponse = type { ptr, i64, i32 }"));
        assert!(ir.contains("store i32 200"));
        assert!(ir.contains("Hello World"));
    }

    #[test]
    fn test_generate_fn_handler_ir_req_body() {
        use crate::syntax::ast::{Param, TypeNode, Stmt};
        use crate::interpret::FnDef;

        // Build: return req.body
        let param = Param {
            name: "req".into(),
            type_expr: TypeNode::new(fresh_id(), Span::new(0, 0), TypeExpr::Infer),
            is_ref: false,
        };
        let req_ident = ExprNode::new(fresh_id(), Span::new(0, 0), Expr::Ident("req".into()));
        let body_field = ExprNode::new(fresh_id(), Span::new(0, 0),
            Expr::Field(Box::new(req_ident), "body".into()));
        let body = vec![
            StmtNode::new(fresh_id(), Span::new(0, 0), Stmt::Return(Some(body_field))),
        ];
        let fndef = FnDef::new(vec![param], body);
        let ir = generate_fn_handler_ir("test_req_body", &fndef)
            .expect("Should generate IR");
        assert!(ir.contains("define void @test_req_body"));
        assert!(ir.contains("Load req.body"));
        assert!(ir.contains("getelementptr i8, ptr %req, i32 32")); // body ptr offset
        assert!(ir.contains("getelementptr i8, ptr %req, i32 40")); // body len offset
    }

    #[test]
    fn test_generate_fn_handler_ir_concat() {
        use crate::syntax::ast::{Param, TypeNode, Stmt};
        use crate::interpret::FnDef;

        // Build: return "Prefix: " + req.body
        let param = Param {
            name: "req".into(),
            type_expr: TypeNode::new(fresh_id(), Span::new(0, 0), TypeExpr::Infer),
            is_ref: false,
        };
        let prefix = ExprNode::new(fresh_id(), Span::new(0, 0), Expr::LitStr("Prefix: ".into()));
        let req_ident = ExprNode::new(fresh_id(), Span::new(0, 0), Expr::Ident("req".into()));
        let req_body = ExprNode::new(fresh_id(), Span::new(0, 0),
            Expr::Field(Box::new(req_ident), "body".into()));
        let concat = ExprNode::new(fresh_id(), Span::new(0, 0),
            Expr::BinOp(Box::new(prefix), BinOp::Add, Box::new(req_body)));
        let body = vec![
            StmtNode::new(fresh_id(), Span::new(0, 0), Stmt::Return(Some(concat))),
        ];
        let fndef = FnDef::new(vec![param], body);
        let ir = generate_fn_handler_ir("test_concat", &fndef)
            .expect("Should generate IR");
        assert!(ir.contains("define void @test_concat"));
        assert!(ir.contains("call void @llvm.memcpy.p0.p0.i64"));
        assert!(ir.contains("Prefix:"));
    }

    #[test]
    fn test_closure_codegen() {
        ast::reset_ids();
        // Manually construct: fn main() { print(42); } with a deferred closure to test codegen
        let closure_body = ExprNode::new(fresh_id(), Span::new(0, 0), Expr::LitInt(42));
        let closure_param = Param {
            name: "x".into(),
            type_expr: TypeNode::new(fresh_id(), Span::new(0, 0), TypeExpr::Infer),
            is_ref: false,
        };
        let closure = ExprNode::new(fresh_id(), Span::new(0, 0), Expr::Closure(vec![closure_param], Box::new(closure_body)));
        let print_call = ExprNode::new(fresh_id(), Span::new(0, 0), Expr::Call(
            Box::new(ExprNode::new(fresh_id(), Span::new(0, 0), Expr::Ident("print".into()))),
            vec![closure],
        ));
        let main_body = StmtNode::new(fresh_id(), Span::new(0, 0), Stmt::Expr(print_call));
        let main_fn = ItemNode::new(fresh_id(), Span::new(0, 0), ItemKind::Fn {
            name: "main".into(),
            params: vec![],
            ret_type: None,
            body: vec![main_body],
            is_async: false,
            generics: vec![],
            is_open: false,
            is_override: false,
            is_final: false,
            is_abstract_method: false,
        });
        let module = Module { name: String::new(), span: Span::new(0, 0), imports: vec![], exports: vec![], items: vec![main_fn] };
        let llvm = compile_to_llvm(&module);
        eprintln!("LLVM:\n{}", llvm);
        assert!(llvm.contains("__closure_0"));
        assert!(llvm.contains("ptrtoint"));
    }
}
