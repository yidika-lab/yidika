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
#ifdef __AVX2__
#include <immintrin.h>
#endif
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <math.h>
#include <time.h>
#include <regex>

// ── Platform Abstraction ────────────────────────────────────────
#ifdef _WIN32
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <winhttp.h>
#pragma comment(lib, "winhttp.lib")
#pragma comment(lib, "ws2_32.lib")
#include <winsock2.h>
#include <ws2tcpip.h>
#include <direct.h>
#include <process.h>
typedef SOCKET yk_socket_t;
#define YK_INVALID_SOCKET INVALID_SOCKET
#define yk_closesocket(fd) closesocket(fd)
#define YK_TLS __declspec(thread)
#define YK_GET_ERR() WSAGetLastError()
#elif __linux__
#include <sys/socket.h>
#include <netinet/in.h>
#include <netdb.h>
#include <arpa/inet.h>
#include <unistd.h>
#include <fcntl.h>
#include <pthread.h>
#include <liburing.h>
#include <errno.h>
typedef int yk_socket_t;
typedef int SOCKET;
#define YK_INVALID_SOCKET (-1)
#define yk_closesocket(fd) close(fd)
#define YK_TLS __thread
#define YK_GET_ERR() errno
#ifndef SOCKET_ERROR
#define SOCKET_ERROR (-1)
#endif
#ifndef INFINITE
#define INFINITE ((unsigned int)-1)
#endif
typedef unsigned int DWORD;
typedef int BOOL;
typedef unsigned long ULONG;
typedef long LONG;
#define TRUE 1
#define FALSE 0
#ifndef MAX_PATH
#define MAX_PATH 4096
#endif
// Windows compat stubs for Linux
#define WSAStartup(a,b) 0
#define WSACleanup()
#define WSAIoctl(a,b,c,d,e,f,g,h,i) (-1)
#define MAKEWORD(a,b) 0
#define SD_BOTH 2
#define SO_UPDATE_ACCEPT_CONTEXT ((int)0x700B)
// Windows types compat (not used on Linux, just for compilation)
typedef struct { int dummy; } OVERLAPPED;
typedef void* LPOVERLAPPED;
#define WAIT_OBJECT_0 0
#define WAIT_TIMEOUT 258
#define WAIT_FAILED ((unsigned int)-1)
#define WSAGetLastError() errno
#endif

// ── Debug Logger (env YK_DEBUG=1 active) ─────────────────────
// Zéro overhead en production : un seul check + sprintf évité.
// Utilise vfprintf pour être portable MSVC/GCC/Clang.
#include <stdarg.h>
static int yk_debug_on = -1;
static void yk_log(const char* fmt, ...) {
    if (yk_debug_on == -1) {
        const char* e = getenv("YK_DEBUG");
        yk_debug_on = (e && e[0] == '1') ? 1 : 0;
    }
    if (!yk_debug_on) return;
    va_list ap;
    va_start(ap, fmt);
    fprintf(stderr, "[yk] ");
    vfprintf(stderr, fmt, ap);
    fprintf(stderr, "\n");
    va_end(ap);
}

// ── TLS Memory Pool ───────────────────────────────────────────
#define YK_POOL_BLOCK_SIZE (64 * 1024)
#define YK_POOL_ALIGNMENT 16

typedef struct yk_pool_block { struct yk_pool_block* next; char data[]; } yk_pool_block;
typedef struct { yk_pool_block* head; char* free_ptr; char* end_ptr; } yk_mem_pool;

#ifdef _MSC_VER
YK_TLS static yk_mem_pool* yk_tls_pool = NULL;
YK_TLS static char* yk_tls_handler_buf = NULL;
YK_TLS static int yk_tls_handler_buf_size = 0;
#else
static __thread yk_mem_pool* yk_tls_pool = NULL;
static __thread char* yk_tls_handler_buf = NULL;
static __thread int yk_tls_handler_buf_size = 0;
#endif

static void yk_pool_init(void) {
    if (!yk_tls_pool) {
        yk_tls_pool = (yk_mem_pool*)malloc(sizeof(yk_mem_pool));
        yk_pool_block* block = (yk_pool_block*)malloc(sizeof(yk_pool_block) + YK_POOL_BLOCK_SIZE);
        block->next = NULL;
        yk_tls_pool->head = block;
        yk_tls_pool->free_ptr = block->data;
        yk_tls_pool->end_ptr = block->data + YK_POOL_BLOCK_SIZE;
    }
}

static void* yk_pool_alloc(size_t size) {
    yk_pool_init();
    size = (size + YK_POOL_ALIGNMENT - 1) & ~(YK_POOL_ALIGNMENT - 1);
    if (yk_tls_pool->free_ptr + (int64_t)size > yk_tls_pool->end_ptr) {
        size_t block_size = size > YK_POOL_BLOCK_SIZE ? size : YK_POOL_BLOCK_SIZE;
        yk_pool_block* block = (yk_pool_block*)malloc(sizeof(yk_pool_block) + block_size);
        block->next = yk_tls_pool->head;
        yk_tls_pool->head = block;
        yk_tls_pool->free_ptr = block->data;
        yk_tls_pool->end_ptr = block->data + block_size;
    }
    void* ptr = yk_tls_pool->free_ptr;
    yk_tls_pool->free_ptr += (int64_t)size;
    return ptr;
}

static void yk_pool_reset(void) {
    if (yk_tls_pool && yk_tls_pool->head) {
        yk_tls_pool->free_ptr = yk_tls_pool->head->data;
        yk_tls_pool->end_ptr = yk_tls_pool->head->data + YK_POOL_BLOCK_SIZE;
    }
}

static char* yk_get_handler_buf(int min_size) {
    if (min_size > yk_tls_handler_buf_size) {
        if (yk_tls_handler_buf) free(yk_tls_handler_buf);
        yk_tls_handler_buf_size = min_size < 16384 ? 16384 : min_size;
        yk_tls_handler_buf = (char*)malloc(yk_tls_handler_buf_size);
    }
    return yk_tls_handler_buf;
}

// ── Virtual Memory Pool (lock-free Treiber stack) ──────────────
// Reserves virtual address space, commits pages on demand.
// Slot size is fixed per pool instance.
#ifdef _MSC_VER
#define YK_CAS_PTR(dest, comp, exch) _InterlockedCompareExchangePointer((void*volatile*)(dest), (void*)(exch), (void*)(comp))
#else
#define YK_CAS_PTR(dest, comp, exch) __sync_val_compare_and_swap((void*volatile*)(dest), (void*)(comp), (void*)(exch))
#endif

typedef struct yk_vmem_slot { struct yk_vmem_slot* volatile next; } yk_vmem_slot;

typedef struct {
    char* base;
    char* commit_end;
    size_t total_size;
    size_t page_size;
    int slot_size;
    yk_vmem_slot* volatile free_head;  // Treiber stack head
} yk_vmem_pool;

#define YK_CONN_POOL_SIZE (512ULL * 1024 * 1024)  // 512 MB → ~2M conns

static yk_vmem_pool yk_conn_pool = {0};

static int yk_vmem_pool_init(size_t total_size, int slot_size) {
    yk_vmem_pool* pool = &yk_conn_pool;
    if (pool->base) return 0;  // already initialized
#ifdef _WIN32
    SYSTEM_INFO si;
    GetSystemInfo(&si);
    pool->page_size = si.dwPageSize;
#else
    pool->page_size = 4096;
#endif
    pool->total_size = total_size;
    pool->slot_size = slot_size;
    pool->free_head = NULL;
#ifdef _WIN32
    pool->base = (char*)VirtualAlloc(NULL, total_size, MEM_RESERVE, PAGE_NOACCESS);
    if (!pool->base) return -1;
#else
    pool->base = (char*)mmap(NULL, total_size, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (pool->base == MAP_FAILED) return -1;
#endif
    pool->commit_end = pool->base;
    return 0;
}

static void* yk_vmem_pool_alloc(void) {
    yk_vmem_pool* pool = &yk_conn_pool;
    int slot_size = pool->slot_size;

    // Try lock-free free list first
    for (;;) {
        yk_vmem_slot* head = pool->free_head;
        if (!head) break;
        if (YK_CAS_PTR(&pool->free_head, head, head->next) == head) {
            memset(head, 0, slot_size);
            return head;
        }
    }

    // Commit enough pages to hold at least one slot
    int page_size = (int)pool->page_size;
    int commit_size = ((slot_size + page_size - 1) / page_size) * page_size;
    char* page = pool->commit_end;
    if (page + commit_size > pool->base + pool->total_size) return NULL;

#ifdef _WIN32
    VirtualAlloc(page, commit_size, MEM_COMMIT, PAGE_READWRITE);
#else
    mmap(page, commit_size, PROT_READ | PROT_WRITE,
         MAP_FIXED | MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
#endif
    pool->commit_end += commit_size;

    // If multiple slots fit in the committed region, push extras onto free list
    if (slot_size > 0 && commit_size / slot_size > 1) {
        int slots = commit_size / slot_size;
        for (int i = 1; i < slots; i++) {
            yk_vmem_slot* s = (yk_vmem_slot*)(page + i * slot_size);
            for (;;) {
                yk_vmem_slot* head = pool->free_head;
                s->next = head;
                if (YK_CAS_PTR(&pool->free_head, head, s) == head) break;
            }
        }
    }

    memset(page, 0, slot_size);
    return page;
}

static void yk_vmem_pool_free(void* ptr) {
    if (!ptr) return;
    yk_vmem_pool* pool = &yk_conn_pool;
    yk_vmem_slot* s = (yk_vmem_slot*)ptr;
    for (;;) {
        yk_vmem_slot* head = pool->free_head;
        s->next = head;
        if (YK_CAS_PTR(&pool->free_head, head, s) == head) return;
    }
}

static void yk_vmem_pool_destroy(void) {
    yk_vmem_pool* pool = &yk_conn_pool;
    if (!pool->base) return;
#ifdef _WIN32
    VirtualFree(pool->base, 0, MEM_RELEASE);
#else
    munmap(pool->base, pool->total_size);
#endif
    memset(pool, 0, sizeof(yk_vmem_pool));
}

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

int64_t yk_pow_int(int64_t a, int64_t b) { int64_t r = 1; for (int64_t i = 0; i < b; i++) r *= a; return r; }
double yk_pow_real(double a, double b) { return pow(a, b); }

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

yk_string* yk_list_to_string(yk_list* l) {
    // Approximate: return a string like "[1, 2, 3]"
    // For simplicity, return first element as string
    if (l->len == 0) {
        yk_string* s = (yk_string*)malloc(sizeof(yk_string));
        s->data = strdup("[]");
        s->len = 2;
        return s;
    }
    // Build a string of all elements
    char buf[4096];
    int pos = 0;
    buf[pos++] = '[';
    for (int64_t i = 0; i < l->len && pos < 4090; i++) {
        if (i > 0) { buf[pos++] = ','; buf[pos++] = ' '; }
        int n = snprintf(buf + pos, 4096 - pos, "%lld", (long long)l->data[i]);
        if (n > 0) pos += n;
    }
    buf[pos++] = ']';
    buf[pos] = '\0';
    yk_string* s = (yk_string*)malloc(sizeof(yk_string));
    s->data = strdup(buf);
    s->len = pos;
    return s;
}

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

// ── Radix Tree Route Trie ──────────────────────────────────────

typedef struct yk_route_node {
    char* segment;                // static segment label, or NULL for root
    int segment_len;
    int is_param;                 // 1 if this is a {param} placeholder
    int param_is_int;             // 1 if {param:int}
    struct yk_route_node** children;
    int child_count;
    int child_cap;
    int is_leaf;                  // has a handler registered here
    char method[8];               // "GET", "POST", etc.
    void (*handler)(void*,void*,char*,int64_t);
    int is_ws;
} yk_route_node;

static yk_route_node* yk_route_node_new(void) {
    yk_route_node* n = (yk_route_node*)malloc(sizeof(yk_route_node));
    memset(n, 0, sizeof(yk_route_node));
    return n;
}

static void yk_route_add_child(yk_route_node* parent, yk_route_node* child) {
    if (parent->child_count >= parent->child_cap) {
        parent->child_cap = parent->child_cap ? parent->child_cap * 2 : 4;
        parent->children = (yk_route_node**)realloc(parent->children,
            parent->child_cap * sizeof(yk_route_node*));
    }
    parent->children[parent->child_count++] = child;
}

// Check if segment is a param placeholder like {name} or {name:int}
static int yk_is_param_seg(const char* seg, int len, int* out_is_int) {
    if (len < 3 || seg[0] != '{') return 0;
    const char* end = seg + len - 1;
    if (*end != '}') return 0;
    // Check for {name:int}
    const char* colon = (const char*)memchr(seg, ':', len);
    if (colon && colon < end) {
        if (strncmp(colon + 1, "int", 3) == 0 && colon + 4 == end) {
            *out_is_int = 1;
            return 1;
        }
    }
    *out_is_int = 0;
    return 1;
}

// Split path into segments, return count (max 64)
static int yk_split_path(const char* path, const char** segs, int* lens, int max_segs) {
    if (!path || !*path) return 0;
    if (*path == '/') path++;
    int count = 0;
    while (*path && count < max_segs) {
        segs[count] = path;
        while (*path && *path != '/') path++;
        lens[count] = (int)(path - segs[count]);
        count++;
        if (*path == '/') path++;
    }
    return count;
}

// Insert a route into the radix tree
static void yk_route_insert(yk_route_node* root, const char* pattern, const char* method,
    void (*handler)(void*,void*,char*,int64_t), int is_ws) {
    const char* segs[64];
    int lens[64];
    int nsegs = yk_split_path(pattern, segs, lens, 64);
    yk_route_node* cur = root;
    for (int i = 0; i < nsegs; i++) {
        int is_int = 0;
        int is_param = yk_is_param_seg(segs[i], lens[i], &is_int);
        // Try to find existing child
        yk_route_node* match = NULL;
        for (int j = 0; j < cur->child_count; j++) {
            yk_route_node* child = cur->children[j];
            if (child->segment_len == lens[i] &&
                memcmp(child->segment, segs[i], lens[i]) == 0) {
                match = child;
                break;
            }
        }
        if (!match) {
            // Also check if any child has same param status (avoid duplicating params)
            if (is_param) {
                for (int j = 0; j < cur->child_count; j++) {
                    if (cur->children[j]->is_param) {
                        match = cur->children[j];
                        break;
                    }
                }
            }
        }
        if (!match) {
            match = yk_route_node_new();
            match->segment = (char*)malloc(lens[i] + 1);
            memcpy(match->segment, segs[i], lens[i]);
            match->segment[lens[i]] = '\0';
            match->segment_len = lens[i];
            match->is_param = is_param;
            match->param_is_int = is_int;
            yk_route_add_child(cur, match);
        }
        cur = match;
    }
    // Set handler at leaf
    cur->is_leaf = 1;
    cur->handler = handler;
    cur->is_ws = is_ws;
    memcpy(cur->method, method, 7);
    cur->method[7] = '\0';
}

// Match a path against the radix tree, return matching node
static yk_route_node* yk_route_match_node(yk_route_node* root, const char* path, const char* method) {
    const char* segs[64];
    int lens[64];
    int nsegs = yk_split_path(path, segs, lens, 64);
    yk_route_node* cur = root;
    for (int i = 0; i < nsegs; i++) {
        yk_route_node* exact = NULL;
        yk_route_node* param = NULL;
        for (int j = 0; j < cur->child_count; j++) {
            yk_route_node* child = cur->children[j];
            if (child->is_param) {
                // Validate {param:int} matches integer segment
                if (child->param_is_int) {
                    int all_digits = 1;
                    for (int k = 0; k < lens[i]; k++) {
                        char c = segs[i][k];
                        if (k == 0 && c == '-') continue;
                        if (c < '0' || c > '9') { all_digits = 0; break; }
                    }
                    if (!all_digits) continue;
                }
                param = child;
            } else if (child->segment_len == lens[i] &&
                       memcmp(child->segment, segs[i], lens[i]) == 0) {
                exact = child;
            }
        }
        // Prefer exact match over param
        cur = exact ? exact : param;
        if (!cur) return NULL;
    }
    // Check method match
    if (cur && cur->is_leaf) {
        if (strcmp(cur->method, method) == 0 || strcmp(cur->method, "*") == 0) {
            return cur;
        }
        // For WebSocket, client sends GET but route is WS
        if (cur->is_ws && strcmp(method, "GET") == 0) return cur;
    }
    return NULL;
}

// ── Platform-specific Server Definitions ───────────────────────

#ifdef _WIN32

typedef BOOL (WINAPI *lpfn_AcceptEx)(SOCKET, SOCKET, PVOID, DWORD, DWORD, DWORD, LPDWORD, LPOVERLAPPED);
static const GUID yk_acceptex_guid = {0xb5367df1,0xcbac,0x11cf,{0x95,0xca,0x00,0x80,0x5f,0x48,0xa1,0x92}};
#ifndef SO_UPDATE_ACCEPT_CONTEXT
#define SO_UPDATE_ACCEPT_CONTEXT ((int)0x700B)
#endif

// Accept operation (Windows IOCP)
typedef struct {
    OVERLAPPED ov;
    SOCKET client_fd;
    char addrbuf[(sizeof(struct sockaddr_in) + 16) * 2];
} yk_accept_op;

#endif // _WIN32

#ifdef __linux__

// io_uring accept state (Linux)
typedef struct {
    struct sockaddr_storage addr;
    socklen_t addrlen;
} yk_accept_op;

#endif // __linux__

// ── Connection (cross-platform) ──────────────────────────────
typedef struct yk_conn {
    // 8-byte pointers first (no padding)
    char* buf;
    char* method;
    char* path;
    char* resp_buf;
    char* body_ptr;
    void* h2;          // yk_h2_session* when H2 is active (non-blocking async)
    void* ws;          // yk_ws_state* when WS is active (non-blocking async)
#ifdef _WIN32
    SOCKET socket;     // 8 bytes on Win64
    OVERLAPPED ov;     // 32 bytes
#endif
    // 4-byte ints packed at end (no alignment gaps)
#ifdef __linux__
    SOCKET socket;     // int, 4 bytes on Linux
#endif
    int buf_cap;
    int buf_used;
    int method_len;
    int path_len;
    int http_major;
    int http_minor;
    int keep_alive;
    int op_type;       // 1=recv, 2=send
    int resp_buf_cap;
    int resp_len;
    int resp_sent;
    int body_len;
    int header_len;
} yk_conn;

// ── TLS connection cache (per-worker thread pool, after yk_conn defined) ──
YK_TLS static yk_conn* yk_tls_conn_cache[64] = {NULL};
YK_TLS static int yk_tls_conn_count = 0;

// ── Dynamic buffer helpers ───────────────────────────────────
static int yk_conn_grow_buf(yk_conn* c, int min_cap) {
    int new_cap = c->buf_cap ? c->buf_cap : 4096;
    while (new_cap < min_cap) new_cap *= 2;
    if (new_cap > 16*1024*1024) return -1;
    char* new_buf = (char*)realloc(c->buf, new_cap);
    if (!new_buf) return -1;
    c->buf = new_buf;
    c->buf_cap = new_cap;
    c->buf[c->buf_used] = '\0';
    return 0;
}

static int yk_conn_grow_resp_buf(yk_conn* c, int min_cap) {
    int new_cap = c->resp_buf_cap ? c->resp_buf_cap : 4096;
    while (new_cap < min_cap) new_cap *= 2;
    if (new_cap > 16*1024*1024) return -1;
    char* new_buf = (char*)realloc(c->resp_buf, new_cap);
    if (!new_buf) return -1;
    c->resp_buf = new_buf;
    c->resp_buf_cap = new_cap;
    return 0;
}

static yk_conn* yk_conn_alloc(void) {
    if (yk_tls_conn_count > 0) {
        yk_conn* c = yk_tls_conn_cache[--yk_tls_conn_count];
        // Save buffer pointers, zero everything else, restore buffers
        char* save_buf = c->buf;
        int save_buf_cap = c->buf_cap;
        char* save_resp_buf = c->resp_buf;
        int save_resp_buf_cap = c->resp_buf_cap;
        memset(c, 0, sizeof(yk_conn));
        c->buf = save_buf;
        c->buf_cap = save_buf_cap;
        c->resp_buf = save_resp_buf;
        c->resp_buf_cap = save_resp_buf_cap;
        return c;
    }
    yk_conn* c = (yk_conn*)yk_vmem_pool_alloc();
    if (!c) {
        // Fallback to malloc if pool is exhausted
        c = (yk_conn*)malloc(sizeof(yk_conn));
        if (!c) return NULL;
    }
    memset(c, 0, sizeof(yk_conn));
    return c;
}

static void yk_conn_free(yk_conn* c) {
    if (!c) return;
    if (yk_tls_conn_count < 64) {
        yk_tls_conn_cache[yk_tls_conn_count++] = c;
    } else {
        free(c->buf);
        free(c->resp_buf);
        yk_vmem_pool_free(c);
    }
}

// ── Server (platform-specific fields) ────────────────────────
typedef struct {
    yk_route_node* trie_root;
    int64_t count;
    yk_socket_t listen_fd;
    volatile int running;
    int thread_count;
#ifdef _WIN32
    HANDLE iocp;
    lpfn_AcceptEx accept_fn;
#elif __linux__
    struct io_uring ring;
#endif
    void* handler_pool; // yk_handler_pool* for offloading handler execution
} yk_server;

// Forward declarations (struct yk_handler_pool defined later)
struct yk_handler_pool;
static struct yk_handler_pool* yk_handler_pool_new(int thread_count);
static void yk_handler_pool_destroy(struct yk_handler_pool* pool);
static int yk_process_request(yk_conn* c, yk_server* s);

int64_t yk_server_new(void) {
    yk_server* s = (yk_server*)malloc(sizeof(yk_server));
    memset(s, 0, sizeof(yk_server));
    s->listen_fd = YK_INVALID_SOCKET;
    s->thread_count = 8;
    s->trie_root = yk_route_node_new();
    // Initialize virtual memory pool for yk_conn structs (safe to call multiple times)
    yk_vmem_pool_init(YK_CONN_POOL_SIZE, sizeof(yk_conn));
    // Handler offloading pool activé par défaut (fusion async + parallel)
    s->handler_pool = NULL;  // créé par yk_ensure_handler_pool dans yk_server_serve
    yk_log("server new (threads=%d)", s->thread_count);
    return (int64_t)(intptr_t)s;
}

// Activer le handler offloading pool (appelé depuis yk_server_serve si nécessaire)
static void yk_ensure_handler_pool(yk_server* s) {
    if (!s->handler_pool) {
        s->handler_pool = yk_handler_pool_new(4);
    }
}

void yk_server_add_route(int64_t handle, yk_string* method, yk_string* path, void* fn_ptr) {
    yk_server* s = (yk_server*)(intptr_t)handle;
    // Extract method string, convert to uppercase
    char method_buf[8];
    int ml = method->len < 7 ? method->len : 7;
    memcpy(method_buf, method->data, ml);
    method_buf[ml] = '\0';
    for (int i = 0; i < ml; i++) if (method_buf[i] >= 'a' && method_buf[i] <= 'z') method_buf[i] -= 32;
    // Extract path (null-terminated)
    char path_buf[4096];
    int pl = path->len < 4095 ? path->len : 4095;
    memcpy(path_buf, path->data, pl);
    path_buf[pl] = '\0';
    // Insert into radix tree
    yk_route_insert(s->trie_root, path_buf, method_buf,
        (void (*)(void*,void*,char*,int64_t))(intptr_t)fn_ptr,
        (strcmp(method_buf, "WS") == 0));
    s->count++;
}

// ── TcpStream (TCP client) ──
#ifdef _WIN32
int64_t yk_tcp_connect(yk_string* addr_str) {
    WSADATA wsaData;
    WSAStartup(MAKEWORD(2,2), &wsaData);  // safe to call multiple times (refcounted)
    char addr_buf[512];
    int al = addr_str->len < 511 ? addr_str->len : 511;
    memcpy(addr_buf, addr_str->data, al);
    addr_buf[al] = '\0';
    char* colon = strrchr(addr_buf, ':');
    if (!colon) return -2;
    *colon = '\0';
    char* host = addr_buf;
    char* port_str = colon + 1;
    struct addrinfo hints, *res;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    hints.ai_protocol = IPPROTO_TCP;
    if (getaddrinfo(host, port_str, &hints, &res) != 0) return -3;
    SOCKET sock = socket(res->ai_family, res->ai_socktype, res->ai_protocol);
    if (sock == INVALID_SOCKET) { freeaddrinfo(res); return -4; }
    if (connect(sock, res->ai_addr, res->ai_addrlen) == SOCKET_ERROR) {
        closesocket(sock); freeaddrinfo(res); return -5;
    }
    freeaddrinfo(res);
    int one = 1;
    setsockopt(sock, IPPROTO_TCP, TCP_NODELAY, (char*)&one, sizeof(one));
    return (int64_t)sock;
}
#else
int64_t yk_tcp_connect(yk_string* addr_str) {
    char addr_buf[512];
    int al = addr_str->len < 511 ? addr_str->len : 511;
    memcpy(addr_buf, addr_str->data, al);
    addr_buf[al] = '\0';
    char* colon = strrchr(addr_buf, ':');
    if (!colon) return -2;
    *colon = '\0';
    char* host = addr_buf;
    char* port_str = colon + 1;
    struct addrinfo hints, *res;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    hints.ai_protocol = IPPROTO_TCP;
    if (getaddrinfo(host, port_str, &hints, &res) != 0) return -3;
    int sock = socket(res->ai_family, res->ai_socktype, res->ai_protocol);
    if (sock < 0) { freeaddrinfo(res); return -4; }
    if (connect(sock, res->ai_addr, res->ai_addrlen) < 0) {
        close(sock); freeaddrinfo(res); return -5;
    }
    freeaddrinfo(res);
    int one = 1;
    setsockopt(sock, IPPROTO_TCP, TCP_NODELAY, &one, sizeof(one));
    return (int64_t)sock;
}
#endif

int64_t yk_tcp_send(int64_t fd, yk_string* data) {
    if (fd <= 0) return -1;
#ifdef _WIN32
    SOCKET s = (SOCKET)fd;
    int sent = send(s, data->data, (int)data->len, 0);
    if (sent == SOCKET_ERROR) return -2;
    return (int64_t)sent;
#else
    int sent = send((int)fd, data->data, data->len, 0);
    if (sent < 0) return -2;
    return (int64_t)sent;
#endif
}

yk_string* yk_tcp_recv(int64_t fd, int64_t n) {
    yk_string* result = (yk_string*)malloc(sizeof(yk_string));
    if (!result) return NULL;
    result->data = NULL;
    result->len = 0;
    if (fd <= 0 || n <= 0) return result;
    char* buf = (char*)malloc((size_t)n);
    if (!buf) return result;
#ifdef _WIN32
    SOCKET s = (SOCKET)fd;
    int received = recv(s, buf, (int)n, 0);
    if (received == SOCKET_ERROR) { free(buf); free(result); return NULL; }
#else
    int received = recv((int)fd, buf, (size_t)n, 0);
    if (received < 0) { free(buf); free(result); return NULL; }
#endif
    result->data = buf;
    result->len = received;
    return result;
}

void yk_tcp_close(int64_t fd) {
    if (fd <= 0) return;
#ifdef _WIN32
    shutdown((SOCKET)fd, SD_BOTH);
    closesocket((SOCKET)fd);
#else
    shutdown((int)fd, SHUT_RDWR);
    close((int)fd);
#endif
}

// ── UdpSocket ──
#ifdef _WIN32
int64_t yk_udp_bind(yk_string* addr_str) {
    WSADATA wsaData;
    WSAStartup(MAKEWORD(2,2), &wsaData);
    char addr_buf[512];
    int al = addr_str->len < 511 ? addr_str->len : 511;
    memcpy(addr_buf, addr_str->data, al);
    addr_buf[al] = '\0';
    char* colon = strrchr(addr_buf, ':');
    if (!colon) return -2;
    *colon = '\0';
    char* host = addr_buf;
    char* port_str = colon + 1;
    struct addrinfo hints, *res;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_DGRAM;
    hints.ai_protocol = IPPROTO_UDP;
    if (getaddrinfo(host, port_str, &hints, &res) != 0) return -3;
    SOCKET sock = socket(res->ai_family, res->ai_socktype, res->ai_protocol);
    if (sock == INVALID_SOCKET) { freeaddrinfo(res); return -4; }
    if (bind(sock, res->ai_addr, (int)res->ai_addrlen) == SOCKET_ERROR) {
        closesocket(sock); freeaddrinfo(res); return -5;
    }
    freeaddrinfo(res);
    return (int64_t)sock;
}
#else
int64_t yk_udp_bind(yk_string* addr_str) {
    char addr_buf[512];
    int al = addr_str->len < 511 ? addr_str->len : 511;
    memcpy(addr_buf, addr_str->data, al);
    addr_buf[al] = '\0';
    char* colon = strrchr(addr_buf, ':');
    if (!colon) return -2;
    *colon = '\0';
    char* host = addr_buf;
    char* port_str = colon + 1;
    struct addrinfo hints, *res;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_DGRAM;
    hints.ai_protocol = IPPROTO_UDP;
    if (getaddrinfo(host, port_str, &hints, &res) != 0) return -3;
    int sock = socket(res->ai_family, res->ai_socktype, res->ai_protocol);
    if (sock < 0) { freeaddrinfo(res); return -4; }
    if (bind(sock, res->ai_addr, res->ai_addrlen) < 0) {
        close(sock); freeaddrinfo(res); return -5;
    }
    freeaddrinfo(res);
    return (int64_t)sock;
}
#endif

int64_t yk_udp_send_to(int64_t fd, yk_string* data, yk_string* addr_str) {
    if (fd <= 0) return -1;
    char addr_buf[512];
    int al = addr_str->len < 511 ? addr_str->len : 511;
    memcpy(addr_buf, addr_str->data, al);
    addr_buf[al] = '\0';
    char* colon = strrchr(addr_buf, ':');
    if (!colon) return -2;
    *colon = '\0';
    struct addrinfo hints, *res;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_DGRAM;
    hints.ai_protocol = IPPROTO_UDP;
    if (getaddrinfo(addr_buf, colon+1, &hints, &res) != 0) return -3;
#ifdef _WIN32
    int sent = sendto((SOCKET)fd, data->data, (int)data->len, 0, res->ai_addr, (int)res->ai_addrlen);
    freeaddrinfo(res);
    if (sent == SOCKET_ERROR) return -4;
    return (int64_t)sent;
#else
    int sent = sendto((int)fd, data->data, data->len, 0, res->ai_addr, res->ai_addrlen);
    freeaddrinfo(res);
    if (sent < 0) return -4;
    return (int64_t)sent;
#endif
}

yk_string* yk_udp_recv_from(int64_t fd, int64_t n) {
    yk_string* result = (yk_string*)malloc(sizeof(yk_string));
    if (!result) return NULL;
    result->data = NULL; result->len = 0;
    if (fd <= 0 || n <= 0) return result;
    char* buf = (char*)malloc((size_t)n);
    if (!buf) return result;
#ifdef _WIN32
    struct sockaddr_in src_addr;
    int addr_len = sizeof(src_addr);
    int received = recvfrom((SOCKET)fd, buf, (int)n, 0, (struct sockaddr*)&src_addr, &addr_len);
    if (received == SOCKET_ERROR) { free(buf); free(result); return NULL; }
#else
    struct sockaddr_in src_addr;
    socklen_t addr_len = sizeof(src_addr);
    int received = recvfrom((int)fd, buf, (size_t)n, 0, (struct sockaddr*)&src_addr, &addr_len);
    if (received < 0) { free(buf); free(result); return NULL; }
#endif
    result->data = buf;
    result->len = received;
    return result;
}

// ── TcpListener ──
int64_t yk_tcp_listen(yk_string* addr_str) {
#ifdef _WIN32
    WSADATA wsaData;
    WSAStartup(MAKEWORD(2,2), &wsaData);
#endif
    char addr_buf[512];
    int al = addr_str->len < 511 ? addr_str->len : 511;
    memcpy(addr_buf, addr_str->data, al);
    addr_buf[al] = '\0';
    char* colon = strrchr(addr_buf, ':');
    if (!colon) return -2;
    *colon = '\0';
    char* host = addr_buf;
    char* port_str = colon + 1;
    struct addrinfo hints, *res;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    hints.ai_protocol = IPPROTO_TCP;
    hints.ai_flags = AI_PASSIVE;
    if (getaddrinfo(host, port_str, &hints, &res) != 0) return -3;
#ifdef _WIN32
    SOCKET sock = socket(res->ai_family, res->ai_socktype, res->ai_protocol);
    if (sock == INVALID_SOCKET) { freeaddrinfo(res); return -4; }
    if (bind(sock, res->ai_addr, (int)res->ai_addrlen) == SOCKET_ERROR) { closesocket(sock); freeaddrinfo(res); return -5; }
    if (listen(sock, SOMAXCONN) == SOCKET_ERROR) { closesocket(sock); freeaddrinfo(res); return -6; }
#else
    int sock = socket(res->ai_family, res->ai_socktype, res->ai_protocol);
    if (sock < 0) { freeaddrinfo(res); return -4; }
    int one = 1;
    setsockopt(sock, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one));
    if (bind(sock, res->ai_addr, res->ai_addrlen) < 0) { close(sock); freeaddrinfo(res); return -5; }
    if (listen(sock, SOMAXCONN) < 0) { close(sock); freeaddrinfo(res); return -6; }
#endif
    freeaddrinfo(res);
    return (int64_t)sock;
}

int64_t yk_tcp_accept(int64_t fd) {
    if (fd <= 0) return -1;
#ifdef _WIN32
    SOCKET client = accept((SOCKET)fd, NULL, NULL);
    if (client == INVALID_SOCKET) return -2;
    int one = 1;
    setsockopt(client, IPPROTO_TCP, TCP_NODELAY, (char*)&one, sizeof(one));
    return (int64_t)client;
#else
    int client = accept((int)fd, NULL, NULL);
    if (client < 0) return -2;
    int one = 1;
    setsockopt(client, IPPROTO_TCP, TCP_NODELAY, &one, sizeof(one));
    return (int64_t)client;
#endif
}

// ── DNS lookup ──
yk_string* yk_dns_lookup(yk_string* host) {
    yk_string* result = (yk_string*)malloc(sizeof(yk_string));
    if (!result) return NULL;
    result->data = NULL; result->len = 0;
    char host_buf[512];
    int hl = host->len < 511 ? host->len : 511;
    memcpy(host_buf, host->data, hl);
    host_buf[hl] = '\0';
    struct addrinfo hints, *res;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    if (getaddrinfo(host_buf, NULL, &hints, &res) != 0) return result;
    void* addr;
    if (res->ai_family == AF_INET) {
        addr = &((struct sockaddr_in*)res->ai_addr)->sin_addr;
    } else {
        addr = &((struct sockaddr_in6*)res->ai_addr)->sin6_addr;
    }
    char ip_buf[INET6_ADDRSTRLEN];
    const char* ip = inet_ntop(res->ai_family, addr, ip_buf, sizeof(ip_buf));
    freeaddrinfo(res);
    if (!ip) return result;
    int iplen = (int)strlen(ip);
    char* dup = (char*)malloc(iplen);
    if (!dup) return result;
    memcpy(dup, ip, iplen);
    result->data = dup;
    result->len = iplen;
    return result;
}

// ── HTTP class (instance-based HTTP client) ──
typedef struct { int32_t status; char* body; int64_t body_len; } yk_http_inst;

int64_t yk_http_new() {
    yk_http_inst* inst = (yk_http_inst*)calloc(1, sizeof(yk_http_inst));
    return (int64_t)(intptr_t)inst;
}

void yk_http_request(int64_t handle, yk_string* url, yk_string* method, yk_string* body) {
    yk_http_inst* inst = (yk_http_inst*)(intptr_t)handle;
    if (inst->body) { free(inst->body); inst->body = NULL; }
    inst->status = 0; inst->body_len = 0;
#ifdef _WIN32
    if (!url || url->len == 0) return;
    HINTERNET hSession = WinHttpOpen(L"Yidika/1.0", WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, NULL, NULL, 0);
    if (!hSession) return;
    URL_COMPONENTSW uc = {0};
    uc.dwStructSize = sizeof(uc);
    uc.dwHostNameLength = (DWORD)-1;
    uc.dwUrlPathLength = (DWORD)-1;
    int url_len = MultiByteToWideChar(CP_UTF8, 0, url->data, (int)url->len, NULL, 0);
    wchar_t* wurl = (wchar_t*)malloc((url_len + 1) * sizeof(wchar_t));
    MultiByteToWideChar(CP_UTF8, 0, url->data, (int)url->len, wurl, url_len);
    wurl[url_len] = L'\0';
    if (!WinHttpCrackUrl(wurl, url_len, 0, &uc)) { free(wurl); WinHttpCloseHandle(hSession); return; }
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
    if (!hConnect) { free(wurl); WinHttpCloseHandle(hSession); return; }
    const wchar_t* wmethod = L"GET";
    wchar_t wm_buf[16] = {0};
    if (method && method->len > 0) {
        int mlen = MultiByteToWideChar(CP_UTF8, 0, method->data, (int)method->len, NULL, 0);
        if (mlen < 16) {
            MultiByteToWideChar(CP_UTF8, 0, method->data, (int)method->len, wm_buf, mlen);
            wm_buf[mlen] = L'\0';
            wmethod = wm_buf;
        }
    }
    HINTERNET hRequest = WinHttpOpenRequest(hConnect, wmethod, path, NULL, NULL, NULL, flags);
    if (!hRequest) { free(wurl); WinHttpCloseHandle(hConnect); WinHttpCloseHandle(hSession); return; }
    LPCWSTR headers = L"Content-Type: application/octet-stream\r\n";
    void* body_data = NULL;
    DWORD body_len = 0;
    if (body && body->len > 0) { body_data = body->data; body_len = (DWORD)body->len; }
    BOOL sent = WinHttpSendRequest(hRequest, headers, -1, body_data, body_len, body_len, 0);
    if (!sent) { free(wurl); WinHttpCloseHandle(hRequest); WinHttpCloseHandle(hConnect); WinHttpCloseHandle(hSession); return; }
    if (!WinHttpReceiveResponse(hRequest, NULL)) { free(wurl); WinHttpCloseHandle(hRequest); WinHttpCloseHandle(hConnect); WinHttpCloseHandle(hSession); return; }
    DWORD status_code = 0;
    DWORD sc_size = sizeof(status_code);
    WinHttpQueryHeaders(hRequest, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, NULL, &status_code, &sc_size, NULL);
    inst->status = (int32_t)status_code;
    DWORD total = 0, cap = 4096;
    char* buf = (char*)malloc(cap);
    DWORD read_bytes = 0;
    while (WinHttpReadData(hRequest, buf + total, cap - total - 1, &read_bytes) && read_bytes > 0) {
        total += read_bytes;
        if (total + 1024 >= cap) { cap *= 2; buf = (char*)realloc(buf, cap); }
        read_bytes = 0;
    }
    buf[total] = '\0';
    free(wurl);
    WinHttpCloseHandle(hRequest);
    WinHttpCloseHandle(hConnect);
    WinHttpCloseHandle(hSession);
    inst->body = buf;
    inst->body_len = total;
#else
    (void)url; (void)method; (void)body;
#endif
}

int32_t yk_http_status(int64_t handle) {
    yk_http_inst* inst = (yk_http_inst*)(intptr_t)handle;
    return inst->status;
}

yk_string* yk_http_body(int64_t handle) {
    yk_http_inst* inst = (yk_http_inst*)(intptr_t)handle;
    yk_string* s = (yk_string*)malloc(sizeof(yk_string));
    if (!s) return NULL;
    s->data = inst->body;
    s->len = inst->body_len;
    return s;
}

void yk_http_free(int64_t handle) {
    yk_http_inst* inst = (yk_http_inst*)(intptr_t)handle;
    if (inst->body) free(inst->body);
    free(inst);
}

// ── Handler offloading pool (lock-free MPMC Treiber stack + work stealing) ──

typedef struct { void* body; int64_t body_len; int32_t status; } yk_hw_resp;
typedef struct { void* m; int64_t ml; void* p; int64_t pl; void* b; int64_t bl; } yk_hw_req;
typedef struct {
    void (*handler)(void*,void*,char*,int64_t);
    yk_hw_resp resp;
    yk_hw_req req;
    char _method[64];       // owned copy (method_buf stack de yk_process_request)
    char _path[4096];       // owned copy (path_buf stack de yk_process_request)
    yk_conn* c;
    yk_server* s;
} yk_hw_item;

typedef struct yk_hw_node {
    yk_hw_item item;
    struct yk_hw_node* next;
} yk_hw_node;

// TLS node cache — amortise malloc/free per-thread
YK_TLS static yk_hw_node* yk_tls_hw_node_free = NULL;

static yk_hw_node* yk_hw_node_alloc(yk_hw_item* item) {
    yk_hw_node* node = yk_tls_hw_node_free;
    if (node) {
        yk_tls_hw_node_free = node->next;
    } else {
        node = (yk_hw_node*)malloc(sizeof(yk_hw_node));
        if (!node) return NULL;
    }
    node->item = *item;
    node->next = NULL;
    return node;
}

static void yk_hw_node_free(yk_hw_node* node) {
    node->next = yk_tls_hw_node_free;
    yk_tls_hw_node_free = node;
}

typedef struct yk_handler_pool {
    yk_hw_node* head;  // lock-free Treiber stack head
    volatile int running;
    int thread_count;
#ifdef _WIN32
    CRITICAL_SECTION wake_lock;  // only for condition variable signaling
    CONDITION_VARIABLE wake_cv;
    HANDLE* threads;
#elif __linux__
    pthread_mutex_t wake_mutex;
    pthread_cond_t wake_cond;
    pthread_t* threads;
#endif
} yk_handler_pool;

// Lock-free enqueue (Treiber stack push) + signal one worker
static void yk_hw_enqueue(yk_handler_pool* pool, yk_hw_item* item) {
    yk_hw_node* node = yk_hw_node_alloc(item);
    if (!node) return;
    for (;;) {
        yk_hw_node* old_head = pool->head;
        node->next = old_head;
        if (YK_CAS_PTR(&pool->head, old_head, node) == old_head) break;
    }
#ifdef _WIN32
    EnterCriticalSection(&pool->wake_lock);
    WakeConditionVariable(&pool->wake_cv);
    LeaveCriticalSection(&pool->wake_lock);
#else
    pthread_mutex_lock(&pool->wake_mutex);
    pthread_cond_signal(&pool->wake_cond);
    pthread_mutex_unlock(&pool->wake_mutex);
#endif
}

// Lock-free dequeue (Treiber stack pop) with CV wait when empty
static int yk_hw_dequeue(yk_handler_pool* pool, yk_hw_item* out, int timeout_ms) {
    // Fast path: try lock-free pop without waiting
    yk_hw_node* node;
    for (;;) {
        node = pool->head;
        if (!node) break;
        if (YK_CAS_PTR(&pool->head, node, node->next) == node) {
            *out = node->item;
            yk_hw_node_free(node);
            return 1;
        }
    }

    // Slow path: wait for work
#ifdef _WIN32
    EnterCriticalSection(&pool->wake_lock);
    while (pool->running) {
        node = pool->head;
        if (node) {
            LeaveCriticalSection(&pool->wake_lock);
            // Retry pop — another thread might have stolen it
            for (;;) {
                node = pool->head;
                if (!node) break;
                if (YK_CAS_PTR(&pool->head, node, node->next) == node) {
                    *out = node->item;
                    yk_hw_node_free(node);
                    return 1;
                }
            }
            // Stolen, go back to wait
            EnterCriticalSection(&pool->wake_lock);
            continue;
        }
        if (!SleepConditionVariableCS(&pool->wake_cv, &pool->wake_lock, timeout_ms)) break;
    }
    LeaveCriticalSection(&pool->wake_lock);
#else
    pthread_mutex_lock(&pool->wake_mutex);
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    ts.tv_sec += timeout_ms / 1000;
    ts.tv_nsec += (timeout_ms % 1000) * 1000000;
    if (ts.tv_nsec >= 1000000000) { ts.tv_sec++; ts.tv_nsec -= 1000000000; }
    while (pool->running) {
        node = pool->head;
        if (node) {
            pthread_mutex_unlock(&pool->wake_mutex);
            for (;;) {
                node = pool->head;
                if (!node) break;
                if (YK_CAS_PTR(&pool->head, node, node->next) == node) {
                    *out = node->item;
                    yk_hw_node_free(node);
                    return 1;
                }
            }
            pthread_mutex_lock(&pool->wake_mutex);
            continue;
        }
        if (pthread_cond_timedwait(&pool->wake_cond, &pool->wake_mutex, &ts) != 0) break;
    }
    pthread_mutex_unlock(&pool->wake_mutex);
#endif
    return 0;
}

// Handler worker thread: executes handlers, builds response, posts async I/O
// NO blocking I/O — all I/O goes through IOCP (Win) or io_uring (Linux)
#ifdef _WIN32
static unsigned __stdcall yk_hw_worker(void* arg) {
#else
static void* yk_hw_worker(void* arg) {
#endif
    yk_handler_pool* pool = (yk_handler_pool*)arg;
    while (pool->running) {
        yk_hw_item item;
        if (!yk_hw_dequeue(pool, &item, 1000)) continue;

        yk_conn* c = item.c;
        yk_server* s = item.s;

        // Execute handler (compute only — no I/O)
        char* handler_buf = yk_get_handler_buf(16384);
        item.handler(&item.resp, &item.req, handler_buf, yk_tls_handler_buf_size);

        int status = item.resp.status;
        char* body_ptr = (char*)item.resp.body;
        int64_t body_len = item.resp.body_len;
        int keep_alive = (c->keep_alive && s->running);

        // Build response header into c->resp_buf
        const char* conn_hdr = keep_alive ? "keep-alive" : "close";
        if (yk_conn_grow_resp_buf(c, 4096) != 0) { yk_closesocket(c->socket); yk_conn_free(c); continue; }
        int n = snprintf(c->resp_buf, c->resp_buf_cap,
            "HTTP/1.1 %d OK\r\nContent-Length: %lld\r\nContent-Type: text/plain\r\nConnection: %s\r\n\r\n",
            status, (long long)(body_ptr ? body_len : 0), conn_hdr);
        c->header_len = n;
        c->body_ptr = body_ptr;
        c->body_len = (int)(body_ptr ? (body_len > 0 ? body_len : 0) : 0);
        c->resp_len = n + c->body_len;
        c->resp_sent = 0;

        // Post async send — IOCP/io_uring worker handles completion & keep-alive
        c->op_type = 2;
#ifdef _WIN32
        DWORD bytes, flags = 0;
        WSABUF wbuf[2];
        int nbufs = 1;
        wbuf[0].buf = c->resp_buf;
        wbuf[0].len = c->header_len;
        if (c->body_ptr && c->body_len > 0) {
            wbuf[1].buf = c->body_ptr;
            wbuf[1].len = c->body_len;
            nbufs = 2;
        }
        if (WSASend(c->socket, wbuf, nbufs, &bytes, flags, &c->ov, NULL) == SOCKET_ERROR &&
            WSAGetLastError() != WSA_IO_PENDING) {
            closesocket(c->socket); yk_conn_free(c);
        }
#else
        pthread_mutex_lock(&yk_ring_mutex);
        struct io_uring_sqe* sqe = io_uring_get_sqe(&s->ring);
        if (sqe) {
            struct iovec* iov = (struct iovec*)malloc(2 * sizeof(struct iovec));
            iov[0].iov_base = c->resp_buf;
            iov[0].iov_len = c->header_len;
            int niov = 1;
            if (c->body_ptr && c->body_len > 0) {
                iov[1].iov_base = c->body_ptr;
                iov[1].iov_len = c->body_len;
                niov = 2;
            }
            io_uring_prep_writev(sqe, c->socket, iov, niov, 0);
            io_uring_sqe_set_data(sqe, (void*)(uintptr_t)c);
        }
        pthread_mutex_unlock(&yk_ring_mutex);
        io_uring_submit(&s->ring);
#endif
    }
#ifdef _WIN32
    return 0;
#else
    return NULL;
#endif
}

// Initialize handler pool with N threads
static yk_handler_pool* yk_handler_pool_new(int thread_count) {
    yk_handler_pool* pool = (yk_handler_pool*)malloc(sizeof(yk_handler_pool));
    memset(pool, 0, sizeof(yk_handler_pool));
    pool->thread_count = thread_count;
    pool->running = 1;
#ifdef _WIN32
    InitializeCriticalSection(&pool->wake_lock);
    InitializeConditionVariable(&pool->wake_cv);
    pool->threads = (HANDLE*)malloc(thread_count * sizeof(HANDLE));
    for (int i = 0; i < thread_count; i++)
        pool->threads[i] = (HANDLE)_beginthreadex(NULL, 0, yk_hw_worker, pool, 0, NULL);
#else
    pthread_mutex_init(&pool->wake_mutex, NULL);
    pthread_cond_init(&pool->wake_cond, NULL);
    pool->threads = (pthread_t*)malloc(thread_count * sizeof(pthread_t));
    for (int i = 0; i < thread_count; i++)
        pthread_create(&pool->threads[i], NULL, yk_hw_worker, pool);
#endif
    return pool;
}

static void yk_handler_pool_destroy(yk_handler_pool* pool) {
    if (!pool) return;
    pool->running = 0;
#ifdef _WIN32
    WakeAllConditionVariable(&pool->wake_cv);
    for (int i = 0; i < pool->thread_count; i++)
        WaitForSingleObject(pool->threads[i], INFINITE);
    DeleteCriticalSection(&pool->wake_lock);
    free(pool->threads);
#else
    pthread_cond_broadcast(&pool->wake_cond);
    for (int i = 0; i < pool->thread_count; i++)
        pthread_join(pool->threads[i], NULL);
    pthread_mutex_destroy(&pool->wake_mutex);
    pthread_cond_destroy(&pool->wake_cond);
    free(pool->threads);
#endif
    free(pool);
}

// Check if "Connection: keep-alive" (case-insensitive) appears in headers
// Forward declarations for WebSocket (async state machine)
static void yk_ws_init(yk_conn* c, yk_server* s);
static int yk_ws_process_data(yk_conn* c);
static void yk_ws_cleanup(yk_conn* c);
static void yk_ws_handle(yk_conn* c, yk_server* s); // legacy blocking path for interp compat
static void yk_h2_init(yk_conn* c, yk_server* s);
static int yk_h2_process_data(yk_conn* c);
static void yk_h2_cleanup(yk_conn* c);
static void yk_h2_run(yk_conn* c, yk_server* s);

// Post an AcceptEx (uses malloc, freed in yk_iocp_worker)
#ifdef _WIN32
static void yk_post_accept(yk_server* s) {
    yk_accept_op* op = (yk_accept_op*)malloc(sizeof(yk_accept_op));
    memset(op, 0, sizeof(yk_accept_op));
    op->client_fd = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if (op->client_fd == INVALID_SOCKET) { free(op); return; }
    DWORD bytes = 0;
    BOOL ret = s->accept_fn(s->listen_fd, op->client_fd, op->addrbuf, 0,
        sizeof(struct sockaddr_in) + 16, sizeof(struct sockaddr_in) + 16, &bytes, &op->ov);
    if (!ret && WSAGetLastError() != WSA_IO_PENDING) {
        closesocket(op->client_fd);
        free(op);
    }
}
#endif

// SIMD-accelerated end-of-headers finder: locate \r\n\r\n in a buffer
// Returns offset of the first \r in \r\n\r\n, or -1 if incomplete
// Only uses AVX2 when available, otherwise scalar loop
static int yk_find_header_end(const char* buf, int buf_used) {
#ifdef __AVX2__
    __m256i cr = _mm256_set1_epi8(0x0D);
    __m256i lf = _mm256_set1_epi8(0x0A);
    int pos = 0;
    while (pos + 32 <= buf_used) {
        __m256i chunk = _mm256_loadu_si256((const __m256i*)(buf + pos));
        int cr_mask = _mm256_movemask_epi8(_mm256_cmpeq_epi8(chunk, cr));
        int lf_mask = _mm256_movemask_epi8(_mm256_cmpeq_epi8(chunk, lf));
        // Find \r\n sequences: shift cr_mask left 1, check lf_mask for overlap
        int crlf_mask = (cr_mask >> 1) & lf_mask;
        if (crlf_mask) {
            // We have at least one \r\n in this chunk; check for \r\n\r\n
            // The \r\n\r\n pattern means: a \r\n at position p, and another \r\n at p+2
            int dbl_crlf = 0;
            unsigned int m = (unsigned int)crlf_mask;
            while (m) {
                unsigned long bit;
#ifdef _MSC_VER
                _BitScanForward(&bit, m);
#else
                bit = (unsigned long)__builtin_ctz(m);
#endif
                int p = (int)bit;
                m &= m - 1; // clear lowest bit
                // Check if there's another \r\n two bytes later
                int next_pos = pos + p + 2;
                if (next_pos + 1 >= buf_used) return -1; // truncated
                int byte0 = (unsigned char)buf[next_pos];
                int byte1 = (unsigned char)buf[next_pos + 1];
                if (byte0 == 0x0D && byte1 == 0x0A) {
                    return pos + p; // found \r\n\r\n, return first \r
                }
            }
        }
        pos += 32;
    }
#endif
    // Scalar fallback: find sequential \r\n\r\n
    for (int i = 0; i + 3 < buf_used; i++) {
        if (buf[i] == 0x0D && buf[i+1] == 0x0A && buf[i+2] == 0x0D && buf[i+3] == 0x0A)
            return i;
    }
    return -1;
}

// SIMD-accelerated character search (AVX2), with scalar fallback
static int yk_find_char(const char* buf, int pos, int buf_used, char c) {
#ifdef __AVX2__
    __m256i target = _mm256_set1_epi8(c);
    while (pos + 32 <= buf_used) {
        __m256i chunk = _mm256_loadu_si256((const __m256i*)(buf + pos));
        __m256i cmp = _mm256_cmpeq_epi8(chunk, target);
        int mask = _mm256_movemask_epi8(cmp);
        if (mask) {
#ifdef _MSC_VER
            unsigned long idx; _BitScanForward(&idx, (unsigned long)mask);
            return pos + (int)idx;
#else
            return pos + __builtin_ctz(mask);
#endif
        }
        pos += 32;
    }
#endif
    // Scalar fallback
    while (pos < buf_used && buf[pos] != c) pos++;
    return pos;
}

// Optimized HTTP header scanner using SIMD \r\n\r\n detection
// Returns offset past \r\n\r\n (body start), or -1 if incomplete
static int yk_scan_http(char* buf, int buf_used, int* out_method_len, int* out_path_len,
    int* out_major, int* out_minor, int* out_keep_alive, int* out_ws_upgrade) {
    *out_keep_alive = 0;
    *out_ws_upgrade = 0;
    // Fast-path: locate \r\n\r\n via SIMD
    int hdr_end = yk_find_header_end(buf, buf_used);
    if (hdr_end < 0) { yk_log("scan: no \\r\\n\\r\\n, buf_used=%d", buf_used); return -1; }
    // Parse request line: method SP path SP HTTP/version
    int i = 0;
    int meth_start = i;
    i = yk_find_char(buf, i, buf_used, ' ');
    if (i >= buf_used || i >= hdr_end) { yk_log("scan: no space (method)"); return -1; }
    *out_method_len = i - meth_start;
    i++;
    int path_start = i;
    i = yk_find_char(buf, i, buf_used, ' ');
    if (i >= buf_used || i >= hdr_end) { yk_log("scan: no space (path)"); return -1; }
    *out_path_len = i - path_start;
    i++;
    int ver_start = i;
    i = yk_find_char(buf, i, buf_used, '\r');
    if (i >= buf_used || i >= hdr_end || buf[i] != '\r' || i+1 >= buf_used || buf[i+1] != '\n') {
        yk_log("scan: no CRLF after version"); return -1;
    }
    *out_major = 1; *out_minor = 1;
    if (i - ver_start > 7 && strncmp(buf + ver_start, "HTTP/", 5) == 0) {
        *out_major = buf[ver_start + 5] - '0';
        *out_minor = buf[ver_start + 7] - '0';
    }
    // Skip request line to start of headers
    i += 2;
    // Scan headers between request line and \r\n\r\n (hdr_end)
    int has_conn_close = 0;
    int has_conn_ka = 0;
    while (i < hdr_end) {
        int line_start = i;
        // Find end of this header line
        i = yk_find_char(buf, i, hdr_end, '\n');
        if (i >= hdr_end) break;
        i++; // skip \n
        int line_len = i - line_start;
        // Check for Connection header (case-insensitive via SWAR-like check)
        if (line_len > 12) {
            // Quick check first two chars: 'C'/'c' and 'o' (or 'U'/'u' for Upgrade)
            unsigned char c0 = (unsigned char)buf[line_start];
            // Connection header check
            if ((c0 == 'C' || c0 == 'c') && (line_start + 10 <= hdr_end)) {
                if (strncmp(buf + line_start, "Connection:", 11) == 0 || strncmp(buf + line_start, "connection:", 11) == 0) {
                    int val_pos = line_start + 11;
                    // Skip leading spaces
                    while (val_pos < hdr_end && buf[val_pos] == ' ') val_pos++;
                    // Check for known values
                    if (hdr_end - val_pos >= 10 && strncmp(buf + val_pos, "keep-alive", 10) == 0) has_conn_ka = 1;
                    if (hdr_end - val_pos >= 5 && strncmp(buf + val_pos, "close", 5) == 0) has_conn_close = 1;
                }
            }
            // Upgrade header check
            if ((c0 == 'U' || c0 == 'u') && (line_start + 7 <= hdr_end)) {
                if (strncmp(buf + line_start, "Upgrade:", 8) == 0 || strncmp(buf + line_start, "upgrade:", 8) == 0) {
                    int val_pos = line_start + 8;
                    while (val_pos < hdr_end && buf[val_pos] == ' ') val_pos++;
                    if (hdr_end - val_pos >= 9 && strncmp(buf + val_pos, "websocket", 9) == 0) {
                        *out_ws_upgrade = 1;
                    }
                }
            }
        }
    }
    // Keep-alive logic
    if (*out_major > 1 || (*out_major == 1 && *out_minor >= 1)) {
        *out_keep_alive = !has_conn_close;
    } else {
        *out_keep_alive = has_conn_ka;
    }
    return hdr_end + 4; // return body start
}

// Cross-platform HTTP request processing: parse headers, match route, call handler, build response
// Returns 1 if complete (response in c->resp_buf), 0 if need more data
// Returns -1 if WS upgrade handled (caller should return immediately)
static int yk_process_request(yk_conn* c, yk_server* s) {
    // H2 session already active: process frames directly
    if (c->h2) {
        int h2ret = yk_h2_process_data(c);
        if (h2ret < 0) { yk_log("H2 closed"); yk_h2_cleanup(c); return -1; }
        if (h2ret == 1) { yk_log("H2 GOAWAY"); yk_h2_cleanup(c); return -1; }
        return 0;
    }
    // Check for HTTP/2 preface (24 bytes: "PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n")
    if (c->buf_used >= 24 && memcmp(c->buf, "PRI * HTTP/2.0\r\n", 16) == 0) {
        yk_log("H2 preface detected");
        yk_h2_init(c, s);
        c->buf_used -= 24;
        memmove(c->buf, c->buf + 24, c->buf_used);
        int h2ret = yk_h2_process_data(c);
        if (h2ret < 0) { yk_log("H2 init: closed"); yk_h2_cleanup(c); return -1; }
        if (h2ret == 1) { yk_log("H2 init: GOAWAY"); yk_h2_cleanup(c); return -1; }
        return 0;
    }
    
    int method_len, path_len, http_major, http_minor, keep_alive, ws_upgrade;
    int hdr_end = yk_scan_http(c->buf, c->buf_used, &method_len, &path_len,
        &http_major, &http_minor, &keep_alive, &ws_upgrade);
    if (hdr_end < 0) return 0;

    // Copy method and path into local buffers (saves 4KB per idle connection)
    char method_buf[64];
    char path_buf[4096];
    c->method_len = method_len < 63 ? method_len : 63;
    path_len = path_len < 4095 ? path_len : 4095;
    if (c->method_len > 0) {
        memcpy(method_buf, c->buf, c->method_len);
        method_buf[c->method_len] = '\0';
        c->method = method_buf;
    } else {
        c->method = "";
    }
    if (path_len > 0) {
        memcpy(path_buf, c->buf + method_len + 1, path_len);
        path_buf[path_len] = '\0';
        c->path = path_buf;
    } else {
        c->path = "";
    }
    c->path_len = path_len;
    c->http_major = http_major;
    c->http_minor = http_minor;
    c->keep_alive = keep_alive;

    // Radix tree route matching
    yk_route_node* matched = yk_route_match_node(s->trie_root, c->path, c->method);

    // Check for WebSocket upgrade (client sends GET, not WS)
    if (!matched && ws_upgrade && strcmp(c->method, "GET") == 0) {
        matched = yk_route_match_node(s->trie_root, c->path, "WS");
        if (matched && matched->is_ws) {
            yk_ws_init(c, s);           // async: sends 101, creates ws state
            return -1;                  // caller re-posts recv (c->buf consumed)
        }
        matched = NULL;
    }

    int status;
    char* body_ptr = NULL;
    int64_t body_len = 0;

    if (matched) {
        yk_log("route: %s %s -> %s", c->method, c->path, matched->segment ? matched->segment : "/");
        yk_hw_req req;
        req.m = c->method; req.ml = c->method_len;
        req.p = c->path; req.pl = path_len;
        req.b = NULL; req.bl = 0;

        yk_hw_resp resp;
        resp.body = NULL; resp.body_len = 0; resp.status = 200;

        // Handler offloading: if pool active, enqueue and return 2
        if (s->handler_pool) {
            yk_hw_item item;
            item.handler = matched->handler;
            item.resp = resp;
            // Copy method/path into owned buffers (stack de yk_process_request sera libéré)
            memcpy(item._method, method_buf, c->method_len + 1);
            memcpy(item._path, path_buf, path_len + 1);
            item.req.m = item._method; item.req.ml = c->method_len;
            item.req.p = item._path; item.req.pl = path_len;
            item.req.b = NULL; item.req.bl = 0;
            item.c = c;
            item.s = s;
            yk_hw_enqueue((yk_handler_pool*)s->handler_pool, &item);
            yk_log("handler offloaded");
            return 2;
        }

        char* handler_buf = yk_get_handler_buf(16384);
        matched->handler(&resp, &req, handler_buf, yk_tls_handler_buf_size);

        status = resp.status;
        body_ptr = (char*)resp.body;
        body_len = resp.body_len;
    } else {
        status = 404;
        yk_log("no route: %s %s (404)", c->method, c->path);
    }

    // Build response: header goes into resp_buf, body is a separate pointer (global constant)
    const char* conn_hdr = keep_alive ? "keep-alive" : "close";
    // Ensure resp_buf is large enough for the header
    if (yk_conn_grow_resp_buf(c, 4096) != 0) return -1;
    int n = snprintf(c->resp_buf, c->resp_buf_cap,
        "HTTP/1.1 %d OK\r\nContent-Length: %lld\r\nContent-Type: text/plain\r\nConnection: %s\r\n\r\n",
        status, (long long)(body_ptr ? body_len : 0), conn_hdr);
    yk_log("response: %s %s -> %d (%d+%d bytes, %s)", c->method, c->path, status, n, c->body_len, conn_hdr);
    
    // Zero-copy: header in resp_buf, body is wherever the handler put it (usually a global constant)
    c->header_len = n;
    c->body_ptr = body_ptr;
    c->body_len = (int)(body_ptr ? (body_len > 0 ? body_len : 0) : 0);
    // body is zero-copy (separate pointer), not constrained by resp_buf
    c->resp_len = n + c->body_len;
    c->resp_sent = 0;
    return 1;
}

// ── Windows IOCP Implementation ────────────────────────────────
#ifdef _WIN32

// Handle recv completion: parse HTTP, dispatch handler, post WSASend
static void yk_on_recv(yk_conn* c, yk_server* s, DWORD bytes) {
    if (bytes == 0) {
        if (c->ws) { yk_log("WS closed"); yk_ws_cleanup(c); }
        else if (c->h2) { yk_log("H2 recv closed"); yk_h2_cleanup(c); }
        else { yk_log("recv: 0 bytes (closed)"); closesocket(c->socket); yk_conn_free(c); }
        return;
    }
    yk_log("recv: %d bytes (buf_used=%d, ws=%d, h2=%d)", bytes, c->buf_used + bytes, !!c->ws, !!c->h2);
    c->buf_used += bytes;
    c->buf[c->buf_used] = '\0';

    // WS async: process frames directly (session already initialized)
    if (c->ws) {
        int ws_ret = yk_ws_process_data(c);
        if (ws_ret == 1 || ws_ret < 0) { yk_ws_cleanup(c); return; }
        // ws_ret == 0: need more data
        if (yk_conn_grow_buf(c, c->buf_used + 4096) != 0) { yk_ws_cleanup(c); return; }
        c->op_type = 1;
        DWORD flags = 0;
        WSABUF wbuf;
        wbuf.buf = c->buf + c->buf_used;
        wbuf.len = (ULONG)(c->buf_cap - c->buf_used);
        if (WSARecv(c->socket, &wbuf, 1, &bytes, &flags, &c->ov, NULL) == SOCKET_ERROR &&
            WSAGetLastError() != WSA_IO_PENDING) {
            yk_ws_cleanup(c);
        }
        return;
    }

    // H2 async: process frames directly (session already initialized)
    if (c->h2) {
        int h2ret = yk_h2_process_data(c);
        if (h2ret == 1 || h2ret < 0) { yk_h2_cleanup(c); return; }
        // Need more data: post async recv
        if (yk_conn_grow_buf(c, c->buf_used + 4096) != 0) { yk_h2_cleanup(c); return; }
        c->op_type = 1;
        DWORD flags = 0;
        WSABUF wbuf;
        wbuf.buf = c->buf + c->buf_used;
        wbuf.len = (ULONG)(c->buf_cap - c->buf_used);
        if (WSARecv(c->socket, &wbuf, 1, &bytes, &flags, &c->ov, NULL) == SOCKET_ERROR &&
            WSAGetLastError() != WSA_IO_PENDING) {
            yk_h2_cleanup(c);
        }
        return;
    }

    int ret = yk_process_request(c, s);
    if (ret < 0) {
        // WS handled by yk_ws_init (sends 101, creates ws state)
        // Post async recv for WS frames
        if (c->ws) {
            if (yk_conn_grow_buf(c, c->buf_used + 4096) != 0) { yk_ws_cleanup(c); return; }
            c->op_type = 1;
            DWORD flags = 0;
            WSABUF wbuf;
            wbuf.buf = c->buf + c->buf_used;
            wbuf.len = (ULONG)(c->buf_cap - c->buf_used);
            if (WSARecv(c->socket, &wbuf, 1, &bytes, &flags, &c->ov, NULL) == SOCKET_ERROR &&
                WSAGetLastError() != WSA_IO_PENDING) {
                yk_ws_cleanup(c);
            }
        }
        return;
    }
    if (ret == 2) {
        // Handler offloaded to dedicated thread pool — connection owned by handler worker
        return;
    }
    if (ret == 0) {
        // Need more data
        if (yk_conn_grow_buf(c, c->buf_used + 4096) != 0) { closesocket(c->socket); yk_conn_free(c); return; }
        c->op_type = 1;
        DWORD flags = 0;
        WSABUF wbuf;
        wbuf.buf = c->buf + c->buf_used;
        wbuf.len = (ULONG)(c->buf_cap - c->buf_used);
        if (WSARecv(c->socket, &wbuf, 1, &bytes, &flags, &c->ov, NULL) == SOCKET_ERROR &&
            WSAGetLastError() != WSA_IO_PENDING) {
            closesocket(c->socket); yk_conn_free(c);
        }
        return;
    }

    // Post WSASend with vectored I/O: header from resp_buf, body from body_ptr
    c->op_type = 2;
    DWORD flags = 0;
    WSABUF wbuf[2];
    int nbufs = 1;
    wbuf[0].buf = c->resp_buf;
    wbuf[0].len = c->header_len;
    if (c->body_ptr && c->body_len > 0) {
        wbuf[1].buf = c->body_ptr;
        wbuf[1].len = c->body_len;
        nbufs = 2;
    }
    if (WSASend(c->socket, wbuf, nbufs, &bytes, flags, &c->ov, NULL) == SOCKET_ERROR &&
        WSAGetLastError() != WSA_IO_PENDING) {
        closesocket(c->socket); yk_conn_free(c);
    }
}

// Handle send completion: support partial sends (vectored I/O), then handle keep-alive
static void yk_on_send(yk_conn* c, yk_server* s, DWORD bytes) {
    c->resp_sent += bytes;
    yk_log("send: %d/%d bytes (keep=%d)", c->resp_sent, c->resp_len, c->keep_alive);
    if (c->resp_sent < c->resp_len) {
        c->op_type = 2;
        WSABUF wbuf[2];
        int nbufs;
        if (c->resp_sent < c->header_len) {
            // Header partially sent
            wbuf[0].buf = c->resp_buf + c->resp_sent;
            wbuf[0].len = c->header_len - c->resp_sent;
            nbufs = 1;
            if (c->body_ptr && c->body_len > 0) {
                wbuf[1].buf = c->body_ptr;
                wbuf[1].len = c->body_len;
                nbufs = 2;
            }
        } else {
            // Header fully sent, body partially sent
            int body_sent = c->resp_sent - c->header_len;
            wbuf[0].buf = c->body_ptr + body_sent;
            wbuf[0].len = c->body_len - body_sent;
            nbufs = 1;
        }
        DWORD flags = 0;
        if (WSASend(c->socket, wbuf, nbufs, &bytes, flags, &c->ov, NULL) == SOCKET_ERROR &&
            WSAGetLastError() != WSA_IO_PENDING) {
            closesocket(c->socket); yk_conn_free(c);
        }
        return;
    }
    if (c->keep_alive && s->running) {
        yk_log("keep-alive recv posted");
        c->buf_used = 0;
        c->op_type = 1;
        DWORD flags = 0;
        WSABUF wbuf;
        wbuf.buf = c->buf;
        wbuf.len = (ULONG)c->buf_cap;
        if (WSARecv(c->socket, &wbuf, 1, &bytes, &flags, &c->ov, NULL) == SOCKET_ERROR &&
            WSAGetLastError() != WSA_IO_PENDING) {
            closesocket(c->socket); yk_conn_free(c);
        }
    } else {
        yk_log("connection closed (keep_alive=%d)", c->keep_alive);
        closesocket(c->socket);
        yk_conn_free(c);
    }
}

// IOCP worker thread
static unsigned __stdcall yk_iocp_worker(void* arg) {
    yk_server* s = (yk_server*)arg;
#ifdef _WIN32
    // NUMA-aware CPU pinning: distribute workers across available cores
    {
        static LONG volatile next_cpu = 0;
        int cpu_idx = (int)InterlockedIncrement(&next_cpu) - 1;
        SYSTEM_INFO sysinfo;
        GetSystemInfo(&sysinfo);
        int ncpu = (int)sysinfo.dwNumberOfProcessors;
        SetThreadAffinityMask(GetCurrentThread(), (DWORD_PTR)1 << (cpu_idx % ncpu));
    }
#endif
    while (s->running) {
        DWORD bytes = 0;
        ULONG_PTR key = 0;
        OVERLAPPED* ov = NULL;
        BOOL ok = GetQueuedCompletionStatus(s->iocp, &bytes, &key, &ov, INFINITE);
        if (!ok) {
            DWORD err = GetLastError();
            if (err == WAIT_TIMEOUT || err == ERROR_ABANDONED_WAIT_0) break;
            continue;
        }
        if (!ov) continue;

        // key == (ULONG_PTR)1 means accept completion (listen socket marker)
        if (key == (ULONG_PTR)1) {
            yk_accept_op* aop = (yk_accept_op*)((char*)ov - offsetof(yk_accept_op, ov));
            setsockopt(aop->client_fd, SOL_SOCKET, SO_UPDATE_ACCEPT_CONTEXT,
                (const char*)&s->listen_fd, sizeof(s->listen_fd));

            yk_conn* c = yk_conn_alloc();
            c->socket = aop->client_fd;
            c->op_type = 1;
            CreateIoCompletionPort((HANDLE)c->socket, s->iocp, (ULONG_PTR)c, 0);
            yk_log("accept: fd=%d slot=%d", c->socket, yk_conn_pool.slot_size);

            // Lazy buffer allocation: allocate on first use
            if (!c->buf) { c->buf = (char*)malloc(4096); c->buf_cap = 4096; }
            if (!c->resp_buf) { c->resp_buf = (char*)malloc(4096); c->resp_buf_cap = 4096; }

            free(aop);
            for (int _i = 0; _i < 32; _i++) yk_post_accept(s);

            DWORD flags = 0;
            WSABUF wbuf;
            wbuf.buf = c->buf;
            wbuf.len = (ULONG)c->buf_cap;
            if (WSARecv(c->socket, &wbuf, 1, &bytes, &flags, &c->ov, NULL) == SOCKET_ERROR &&
                WSAGetLastError() != WSA_IO_PENDING) {
                closesocket(c->socket); yk_conn_free(c);
            }
        } else {
            yk_conn* c = (yk_conn*)key;
            if (!ok) {
                yk_log("iocp error on conn");
                closesocket(c->socket); yk_conn_free(c);
            } else if (c->op_type == 1) {
                yk_on_recv(c, s, bytes);
            } else if (c->op_type == 2) {
                yk_on_send(c, s, bytes);
            }
        }
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
    if (!port_str) { printf("Invalid address format\n"); return; }
    *port_str++ = '\0';
    char* host = addr_buf;
    int port = atoi(port_str);
    if (port <= 0) port = 8080;

    WSADATA wsa;
    if (WSAStartup(MAKEWORD(2,2), &wsa) != 0) { printf("WSAStartup failed\n"); return; }

    s->listen_fd = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if (s->listen_fd == INVALID_SOCKET) { printf("socket failed\n"); WSACleanup(); return; }

    int opt = 1;
    setsockopt(s->listen_fd, SOL_SOCKET, SO_REUSEADDR, (const char*)&opt, sizeof(opt));

    struct sockaddr_in sa;
    sa.sin_family = AF_INET;
    sa.sin_port = htons((unsigned short)port);
    if (strcmp(host, "*") == 0 || strcmp(host, "0.0.0.0") == 0)
        sa.sin_addr.s_addr = INADDR_ANY;
    else
        inet_pton(AF_INET, host, &sa.sin_addr);

    if (bind(s->listen_fd, (struct sockaddr*)&sa, sizeof(sa)) == SOCKET_ERROR) {
        printf("bind failed\n"); closesocket(s->listen_fd); WSACleanup(); return;
    }
    if (listen(s->listen_fd, SOMAXCONN) == SOCKET_ERROR) {
        printf("listen failed\n"); closesocket(s->listen_fd); WSACleanup(); return;
    }

    // Load AcceptEx function
    DWORD bytes;
    if (WSAIoctl(s->listen_fd, SIO_GET_EXTENSION_FUNCTION_POINTER,
                 (void*)&yk_acceptex_guid, sizeof(yk_acceptex_guid),
                 &s->accept_fn, sizeof(s->accept_fn),
                 &bytes, NULL, NULL) != 0) {
        printf("WSAIoctl AcceptEx failed\n"); closesocket(s->listen_fd); WSACleanup(); return;
    }

    // Create IOCP
    s->iocp = CreateIoCompletionPort(INVALID_HANDLE_VALUE, NULL, 0, 0);
    if (!s->iocp) { printf("CreateIoCompletionPort failed\n"); closesocket(s->listen_fd); WSACleanup(); return; }

    // Associate listen socket with IOCP (key = 1 for accept completions)
    CreateIoCompletionPort((HANDLE)s->listen_fd, s->iocp, (ULONG_PTR)1, 0);

    s->running = 1;
    printf("Server listening on %s:%d (IOCP, %d threads)\n", host, port, s->thread_count);
    yk_log("serve start %s:%d (IOCP, %d routes, %d threads)", host, port, s->count, s->thread_count);

    // Start worker threads
    HANDLE* threads = (HANDLE*)malloc(s->thread_count * sizeof(HANDLE));
    for (int i = 0; i < s->thread_count; i++) {
        threads[i] = (HANDLE)_beginthreadex(NULL, 0, yk_iocp_worker, s, 0, NULL);
    }

    // Enable handler offloading pool (fusion async + parallel, 4 threads)
    yk_ensure_handler_pool(s);

    // Post initial batch of AcceptEx operations (pipeline of 128)
    for (int i = 0; i < 128; i++) {
        yk_post_accept(s);
    }

    // Wait for workers (blocks until process killed)
    WaitForMultipleObjects(s->thread_count, threads, TRUE, INFINITE);

    // Cleanup (never reached under normal operation)
    yk_log("serve cleanup IOCP");
    s->running = 0;
    yk_handler_pool_destroy((yk_handler_pool*)s->handler_pool);
    s->handler_pool = NULL;
    for (int i = 0; i < s->thread_count; i++) CloseHandle(threads[i]);
    free(threads);
    CloseHandle(s->iocp);
    closesocket(s->listen_fd);
    yk_vmem_pool_destroy();
    free(s);
    WSACleanup();
}

// ── Linux io_uring Implementation ──────────────────────────────
#elif __linux__

#include <sys/epoll.h>

// io_uring worker thread
#ifdef __linux__
static pthread_mutex_t yk_ring_mutex = PTHREAD_MUTEX_INITIALIZER;
#endif

static void* yk_io_uring_worker(void* arg) {
    yk_server* s = (yk_server*)arg;
#ifdef __linux__
    // NUMA-aware CPU pinning
    {
        static int next_cpu = 0;
        int cpu_idx = __sync_fetch_and_add(&next_cpu, 1);
        int ncpu = (int)sysconf(_SC_NPROCESSORS_ONLN);
        if (ncpu > 0) {
            cpu_set_t cpuset;
            CPU_ZERO(&cpuset);
            CPU_SET(cpu_idx % ncpu, &cpuset);
            pthread_setaffinity_np(pthread_self(), sizeof(cpu_set_t), &cpuset);
        }
    }
#endif
    struct io_uring_cqe* cqe;
    while (s->running) {
        int ret = io_uring_wait_cqe(&s->ring, &cqe);
        if (ret < 0) break;
        void* user_data = (void*)(uintptr_t)cqe->user_data;
        int res = cqe->res;
        io_uring_cqe_seen(&s->ring, cqe);

        if (user_data == NULL) continue;
        if ((uintptr_t)user_data == (uintptr_t)1) {
            // Accept completion
            yk_log("accept: fd=%d", res);
            yk_conn* c = yk_conn_alloc();
            c->socket = res;
            c->op_type = 1;
            // Lazy buffer allocation: allocate on first use
            if (!c->buf) { c->buf = (char*)malloc(4096); c->buf_cap = 4096; }
            if (!c->resp_buf) { c->resp_buf = (char*)malloc(4096); c->resp_buf_cap = 4096; }
            // Post next batch of accepts
            pthread_mutex_lock(&yk_ring_mutex);
            for (int _i = 0; _i < 32; _i++) {
                struct io_uring_sqe* sqe = io_uring_get_sqe(&s->ring);
                if (!sqe) break;
                io_uring_prep_accept(sqe, s->listen_fd, NULL, NULL, SOCK_CLOEXEC);
                io_uring_sqe_set_data(sqe, (void*)(uintptr_t)1);
            }
            {
                struct io_uring_sqe* sqe = io_uring_get_sqe(&s->ring);
                if (sqe) {
                    struct iovec* iov = (struct iovec*)malloc(sizeof(struct iovec));
                    iov->iov_base = c->buf;
                    iov->iov_len = c->buf_cap;
                    io_uring_prep_readv(sqe, c->socket, iov, 1, 0);
                    io_uring_sqe_set_data(sqe, (void*)(uintptr_t)c);
                }
            }
            pthread_mutex_unlock(&yk_ring_mutex);
            io_uring_submit(&s->ring);
        } else {
            yk_conn* c = (yk_conn*)user_data;
            if (res <= 0) {
                if (c->ws) { yk_log("WS closed"); yk_ws_cleanup(c); }
                else if (c->h2) { yk_log("H2 closed"); yk_h2_cleanup(c); }
                else { yk_log("conn closed (res=%d)", res); yk_closesocket(c->socket); yk_conn_free(c); }
                continue;
            }
            if (c->op_type == 1) {
                // Recv completion
                c->buf_used += res;
                yk_log("recv: %d bytes (buf_used=%d, ws=%d, h2=%d)", res, c->buf_used, !!c->ws, !!c->h2);
                c->buf[c->buf_used] = '\0';

                // WS async: process frames directly (session already initialized)
                if (c->ws) {
                    int ws_ret = yk_ws_process_data(c);
                    if (ws_ret == 1 || ws_ret < 0) { yk_ws_cleanup(c); continue; }
                    if (yk_conn_grow_buf(c, c->buf_used + 4096) != 0) { yk_ws_cleanup(c); continue; }
                    pthread_mutex_lock(&yk_ring_mutex);
                    struct io_uring_sqe* sqe = io_uring_get_sqe(&s->ring);
                    if (sqe) {
                        struct iovec* iov = (struct iovec*)malloc(sizeof(struct iovec));
                        iov->iov_base = c->buf + c->buf_used;
                        iov->iov_len = c->buf_cap - c->buf_used;
                        io_uring_prep_readv(sqe, c->socket, iov, 1, 0);
                        io_uring_sqe_set_data(sqe, (void*)(uintptr_t)c);
                        c->op_type = 1;
                    }
                    pthread_mutex_unlock(&yk_ring_mutex);
                    io_uring_submit(&s->ring);
                    continue;
                }

                // H2 async: process frames directly
                if (c->h2) {
                    int h2ret = yk_h2_process_data(c);
                    if (h2ret == 1 || h2ret < 0) { yk_h2_cleanup(c); continue; }
                    // Need more data
                    if (yk_conn_grow_buf(c, c->buf_used + 4096) != 0) { yk_h2_cleanup(c); continue; }
                    pthread_mutex_lock(&yk_ring_mutex);
                    struct io_uring_sqe* sqe = io_uring_get_sqe(&s->ring);
                    if (sqe) {
                        struct iovec* iov = (struct iovec*)malloc(sizeof(struct iovec));
                        iov->iov_base = c->buf + c->buf_used;
                        iov->iov_len = c->buf_cap - c->buf_used;
                        io_uring_prep_readv(sqe, c->socket, iov, 1, 0);
                        io_uring_sqe_set_data(sqe, (void*)(uintptr_t)c);
                        c->op_type = 1;
                    }
                    pthread_mutex_unlock(&yk_ring_mutex);
                    io_uring_submit(&s->ring);
                    continue;
                }
                int ret2 = yk_process_request(c, s);
                if (ret2 < 0) {
                    // WS handled by yk_ws_init
                    if (c->ws) {
                        if (yk_conn_grow_buf(c, c->buf_used + 4096) != 0) { yk_ws_cleanup(c); continue; }
                        pthread_mutex_lock(&yk_ring_mutex);
                        struct io_uring_sqe* sqe = io_uring_get_sqe(&s->ring);
                        if (sqe) {
                            struct iovec* iov = (struct iovec*)malloc(sizeof(struct iovec));
                            iov->iov_base = c->buf + c->buf_used;
                            iov->iov_len = c->buf_cap - c->buf_used;
                            io_uring_prep_readv(sqe, c->socket, iov, 1, 0);
                            io_uring_sqe_set_data(sqe, (void*)(uintptr_t)c);
                            c->op_type = 1;
                        }
                        pthread_mutex_unlock(&yk_ring_mutex);
                        io_uring_submit(&s->ring);
                    }
                    continue;
                }
                if (ret2 == 2) {
                    // Handler offloaded to dedicated thread pool — connection owned by handler worker
                    continue;
                }
                if (ret2 == 0) {
                    // Need more data - post another recv
                    pthread_mutex_lock(&yk_ring_mutex);
                    struct io_uring_sqe* sqe = io_uring_get_sqe(&s->ring);
                    if (sqe) {
                        struct iovec* iov = (struct iovec*)malloc(sizeof(struct iovec));
                        iov->iov_base = c->buf + c->buf_used;
                        iov->iov_len = c->buf_cap - c->buf_used;
                        io_uring_prep_readv(sqe, c->socket, iov, 1, 0);
                        io_uring_sqe_set_data(sqe, (void*)(uintptr_t)c);
                        c->op_type = 1;
                    }
                    pthread_mutex_unlock(&yk_ring_mutex);
                    io_uring_submit(&s->ring);
                    continue;
                // Response ready, post send with vectored I/O
                c->op_type = 2;
                pthread_mutex_lock(&yk_ring_mutex);
                struct io_uring_sqe* sqe = io_uring_get_sqe(&s->ring);
                if (sqe) {
                    struct iovec* iov = (struct iovec*)malloc(2 * sizeof(struct iovec));
                    iov[0].iov_base = c->resp_buf;
                    iov[0].iov_len = c->header_len;
                    int niov = 1;
                    if (c->body_ptr && c->body_len > 0) {
                        iov[1].iov_base = c->body_ptr;
                        iov[1].iov_len = c->body_len;
                        niov = 2;
                    }
                    io_uring_prep_writev(sqe, c->socket, iov, niov, 0);
                    io_uring_sqe_set_data(sqe, (void*)(uintptr_t)c);
                }
                pthread_mutex_unlock(&yk_ring_mutex);
                io_uring_submit(&s->ring);
            } else if (c->op_type == 2) {
                // Send completion
                c->resp_sent += res;
                yk_log("send: %d/%d bytes (keep=%d)", c->resp_sent, c->resp_len, c->keep_alive);
                if (c->resp_sent < c->resp_len) {
                    // Partial send, continue with vectored I/O
                    pthread_mutex_lock(&yk_ring_mutex);
                    struct io_uring_sqe* sqe = io_uring_get_sqe(&s->ring);
                    if (sqe) {
                        struct iovec* iov = (struct iovec*)malloc(2 * sizeof(struct iovec));
                        int niov;
                        if (c->resp_sent < c->header_len) {
                            iov[0].iov_base = c->resp_buf + c->resp_sent;
                            iov[0].iov_len = c->header_len - c->resp_sent;
                            niov = 1;
                            if (c->body_ptr && c->body_len > 0) {
                                iov[1].iov_base = c->body_ptr;
                                iov[1].iov_len = c->body_len;
                                niov = 2;
                            }
                        } else {
                            int body_sent = c->resp_sent - c->header_len;
                            iov[0].iov_base = c->body_ptr + body_sent;
                            iov[0].iov_len = c->body_len - body_sent;
                            niov = 1;
                        }
                        io_uring_prep_writev(sqe, c->socket, iov, niov, 0);
                        io_uring_sqe_set_data(sqe, (void*)(uintptr_t)c);
                    }
                    pthread_mutex_unlock(&yk_ring_mutex);
                    io_uring_submit(&s->ring);
                } else if (c->keep_alive && s->running) {
                    yk_log("keep-alive recv posted");
                    c->buf_used = 0;
                    c->op_type = 1;
                    pthread_mutex_lock(&yk_ring_mutex);
                    struct io_uring_sqe* sqe = io_uring_get_sqe(&s->ring);
                    if (sqe) {
                        struct iovec* iov = (struct iovec*)malloc(sizeof(struct iovec));
                        iov->iov_base = c->buf;
                        iov->iov_len = c->buf_cap;
                        io_uring_prep_readv(sqe, c->socket, iov, 1, 0);
                        io_uring_sqe_set_data(sqe, (void*)(uintptr_t)c);
                    }
                    pthread_mutex_unlock(&yk_ring_mutex);
                    io_uring_submit(&s->ring);
                } else {
                    yk_log("connection closed (keep_alive=%d)", c->keep_alive);
                    yk_closesocket(c->socket);
                    yk_conn_free(c);
                }
            }
        }
    }
    return NULL;
}

// Linux kernel tuning for high-throughput networking (silently ignores permission errors)
#ifdef __linux__
static void yk_linux_sysctl_tune() {
#define YK_SYSCTL(path, val) do { FILE* f = fopen(path, "w"); if (f) { fprintf(f, "%s", val); fclose(f); } } while(0)
    YK_SYSCTL("/proc/sys/net/core/somaxconn", "65535");
    YK_SYSCTL("/proc/sys/net/core/netdev_max_backlog", "100000");
    YK_SYSCTL("/proc/sys/net/core/rmem_max", "16777216");
    YK_SYSCTL("/proc/sys/net/core/wmem_max", "16777216");
    YK_SYSCTL("/proc/sys/net/ipv4/tcp_rmem", "4096 87380 16777216");
    YK_SYSCTL("/proc/sys/net/ipv4/tcp_wmem", "4096 65536 16777216");
    YK_SYSCTL("/proc/sys/net/ipv4/tcp_congestion_control", "bbr");
    YK_SYSCTL("/proc/sys/net/ipv4/tcp_fastopen", "3");
    YK_SYSCTL("/proc/sys/net/ipv4/tcp_tw_reuse", "1");
    YK_SYSCTL("/proc/sys/net/ipv4/tcp_max_syn_backlog", "65535");
    YK_SYSCTL("/proc/sys/net/ipv4/tcp_fin_timeout", "15");
#undef YK_SYSCTL
}

// Configure NIC for multiqueue + RSS via ethtool (best-effort, non-root-friendly fallback)
static void yk_linux_nic_tune(const char* ifname) {
    if (!ifname || !*ifname) return;
    char cmd[512];
    // Set combined channels (rx+tx) to CPU count
    int ncpu = (int)sysconf(_SC_NPROCESSORS_ONLN);
    snprintf(cmd, sizeof(cmd), "ethtool -L %s combined %d 2>/dev/null", ifname, ncpu);
    system(cmd);
    // Enable symmetric RSS hash for TCP
    snprintf(cmd, sizeof(cmd), "ethtool -X %s equal %d 2>/dev/null", ifname, ncpu);
    system(cmd);
}
#endif

void yk_server_serve(int64_t handle, yk_string* addr) {
    yk_server* s = (yk_server*)(intptr_t)handle;
    if (!s || s->count == 0) { printf("No routes registered\n"); return; }

#ifdef __linux__
    // Kernel tuning for high-throughput networking
    yk_linux_sysctl_tune();
    // Auto-detect NIC for multiqueue tuning
    {
        FILE* rt = fopen("/proc/net/route", "r");
        if (rt) {
            char line[256], ifname[32];
            while (fgets(line, sizeof(line), rt)) {
                if (sscanf(line, "%31s", ifname) == 1 && strcmp(ifname, "Iface") != 0) {
                    if (strcmp(ifname, "lo") != 0) { yk_linux_nic_tune(ifname); break; }
                }
            }
            fclose(rt);
        }
    }
#endif

    // Parse addr: "host:port"
    char addr_buf[256];
    int addr_len = (int)(addr->len < 255 ? addr->len : 255);
    memcpy(addr_buf, addr->data, addr_len);
    addr_buf[addr_len] = '\0';
    char* port_str = strrchr(addr_buf, ':');
    if (!port_str) { printf("Invalid address format\n"); return; }
    *port_str++ = '\0';
    char* host = addr_buf;
    int port = atoi(port_str);
    if (port <= 0) port = 8080;

    s->listen_fd = socket(AF_INET, SOCK_STREAM | SOCK_NONBLOCK, IPPROTO_TCP);
    if (s->listen_fd < 0) { printf("socket failed\n"); return; }

    int opt = 1;
    setsockopt(s->listen_fd, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof(opt));
#ifdef SO_REUSEPORT
    setsockopt(s->listen_fd, SOL_SOCKET, SO_REUSEPORT, &opt, sizeof(opt));
#endif

    struct sockaddr_in sa;
    sa.sin_family = AF_INET;
    sa.sin_port = htons((unsigned short)port);
    if (strcmp(host, "*") == 0 || strcmp(host, "0.0.0.0") == 0)
        sa.sin_addr.s_addr = INADDR_ANY;
    else
        inet_pton(AF_INET, host, &sa.sin_addr);

    if (bind(s->listen_fd, (struct sockaddr*)&sa, sizeof(sa)) < 0) {
        printf("bind failed\n"); yk_closesocket(s->listen_fd); return;
    }
    if (listen(s->listen_fd, SOMAXCONN) < 0) {
        printf("listen failed\n"); yk_closesocket(s->listen_fd); return;
    }

    // Initialize io_uring with SQPOLL for kernel-side polling
    struct io_uring_params params;
    memset(&params, 0, sizeof(params));
    params.flags = IORING_SETUP_SQPOLL | IORING_SETUP_COOP_TASKRUN;
    params.sq_thread_idle = 2000; // 2 second idle before SQPOLL thread sleeps

    int ret = io_uring_queue_init_params(4096, &s->ring, &params);
    if (ret != 0) {
        // Fallback: try without SQPOLL
        memset(&params, 0, sizeof(params));
        params.flags = IORING_SETUP_COOP_TASKRUN;
        ret = io_uring_queue_init_params(4096, &s->ring, &params);
        if (ret != 0) {
            printf("io_uring_queue_init failed\n");
            yk_closesocket(s->listen_fd);
            return;
        }
    }

    s->running = 1;
    printf("Server listening on %s:%d (io_uring, %d threads)\n", host, port, s->thread_count);
    yk_log("serve start %s:%d (io_uring, %d routes, %d threads)", host, port, s->count, s->thread_count);

    // Start worker threads (share the same ring)
    pthread_t* workers = (pthread_t*)malloc(s->thread_count * sizeof(pthread_t));
    for (int i = 0; i < s->thread_count; i++) {
        pthread_create(&workers[i], NULL, yk_io_uring_worker, s);
    }

    // Enable handler offloading pool (fusion async + parallel, 4 threads)
    yk_ensure_handler_pool(s);

    // Post initial accept operations (pipeline of 128)
    struct io_uring_sqe* sqe;
    for (int i = 0; i < 128; i++) {
        sqe = io_uring_get_sqe(&s->ring);
        if (!sqe) break;
        io_uring_prep_accept(sqe, s->listen_fd, NULL, NULL, SOCK_CLOEXEC);
        io_uring_sqe_set_data(sqe, (void*)(uintptr_t)1);
    }
    io_uring_submit(&s->ring);

    // Wait for all worker threads (blocks until process killed)
    for (int i = 0; i < s->thread_count; i++) {
        pthread_join(workers[i], NULL);
    }

    // Cleanup (never reached under normal operation)
    yk_log("serve cleanup io_uring");
    s->running = 0;
    yk_handler_pool_destroy((yk_handler_pool*)s->handler_pool);
    s->handler_pool = NULL;
    free(workers);
    io_uring_queue_exit(&s->ring);
    yk_closesocket(s->listen_fd);
    yk_vmem_pool_destroy();
    free(s);
}

#endif // _WIN32 / __linux__

// ── WebSocket helpers ───

// Minimal SHA-1 implementation for WebSocket accept key
typedef struct { uint32_t state[5]; uint64_t count; unsigned char buffer[64]; } yk_sha1_ctx;

static void yk_sha1_init(yk_sha1_ctx* ctx) {
    ctx->state[0] = 0x67452301; ctx->state[1] = 0xEFCDAB89;
    ctx->state[2] = 0x98BADCFE; ctx->state[3] = 0x10325476;
    ctx->state[4] = 0xC3D2E1F0; ctx->count = 0;
}

#define YK_SHA1_ROTL(x, n) (((x) << (n)) | ((x) >> (32 - (n))))
static void yk_sha1_process(yk_sha1_ctx* ctx, const unsigned char data[64]) {
    uint32_t w[80]; for (int i = 0; i < 16; i++) w[i] = ((uint32_t)data[i*4]<<24)|(data[i*4+1]<<16)|(data[i*4+2]<<8)|data[i*4+3];
    for (int i = 16; i < 80; i++) w[i] = YK_SHA1_ROTL(w[i-3] ^ w[i-8] ^ w[i-14] ^ w[i-16], 1);
    uint32_t a = ctx->state[0], b = ctx->state[1], c = ctx->state[2], d = ctx->state[3], e = ctx->state[4], f, k, temp;
    for (int i = 0; i < 80; i++) {
        if (i < 20) { f = (b & c) | (~b & d); k = 0x5A827999; }
        else if (i < 40) { f = b ^ c ^ d; k = 0x6ED9EBA1; }
        else if (i < 60) { f = (b & c) | (b & d) | (c & d); k = 0x8F1BBCDC; }
        else { f = b ^ c ^ d; k = 0xCA62C1D6; }
        temp = YK_SHA1_ROTL(a, 5) + f + e + k + w[i]; e = d; d = c; c = YK_SHA1_ROTL(b, 30); b = a; a = temp;
    }
    ctx->state[0] += a; ctx->state[1] += b; ctx->state[2] += c; ctx->state[3] += d; ctx->state[4] += e;
}

static void yk_sha1_update(yk_sha1_ctx* ctx, const unsigned char* data, size_t len) {
    size_t idx = ctx->count & 0x3F; ctx->count += len;
    size_t part = 64 - idx; ctx->count += 0;
    if (len >= part) { memcpy(ctx->buffer + idx, data, part); yk_sha1_process(ctx, ctx->buffer);
    for (size_t i = part; i + 63 < len; i += 64) yk_sha1_process(ctx, data + i); idx = 0; }
    else { memcpy(ctx->buffer + idx, data, len); return; }
    memcpy(ctx->buffer, data + len - (len - idx - (len - idx) / 64 * 64), (len - idx) % 64);
}

static void yk_sha1_final(yk_sha1_ctx* ctx, unsigned char hash[20]) {
    size_t idx = ctx->count & 0x3F; ctx->buffer[idx++] = 0x80;
    if (idx > 56) { memset(ctx->buffer + idx, 0, 64 - idx); yk_sha1_process(ctx, ctx->buffer); idx = 0; }
    memset(ctx->buffer + idx, 0, 56 - idx);
    uint64_t bits = ctx->count * 8;
    ctx->buffer[56] = (unsigned char)(bits >> 56); ctx->buffer[57] = (unsigned char)(bits >> 48);
    ctx->buffer[58] = (unsigned char)(bits >> 40); ctx->buffer[59] = (unsigned char)(bits >> 32);
    ctx->buffer[60] = (unsigned char)(bits >> 24); ctx->buffer[61] = (unsigned char)(bits >> 16);
    ctx->buffer[62] = (unsigned char)(bits >> 8);  ctx->buffer[63] = (unsigned char)(bits);
    yk_sha1_process(ctx, ctx->buffer);
    for (int i = 0; i < 5; i++) { hash[i*4] = ctx->state[i] >> 24; hash[i*4+1] = ctx->state[i] >> 16; hash[i*4+2] = ctx->state[i] >> 8; hash[i*4+3] = ctx->state[i]; }
}

static const char yk_b64_table[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
static void yk_ws_base64_encode(const unsigned char* in, size_t in_len, char* out) {
    for (size_t i = 0; i < in_len; i += 3) {
        int n = in_len - i < 3 ? (int)(in_len - i) : 3;
        uint32_t v = ((uint32_t)in[i] << 16) | (n > 1 ? (uint32_t)in[i+1] << 8 : 0) | (n > 2 ? in[i+2] : 0);
        *out++ = yk_b64_table[(v >> 18) & 0x3F];
        *out++ = yk_b64_table[(v >> 12) & 0x3F];
        *out++ = n > 1 ? yk_b64_table[(v >> 6) & 0x3F] : '=';
        *out++ = n > 2 ? yk_b64_table[v & 0x3F] : '=';
    }
    *out = '\0';
}

// WebSocket accept key: SHA-1(key + magic GUID) → base64
static void yk_ws_accept_key(const char* client_key, char* out, int out_len) {
    const char* magic = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    yk_sha1_ctx ctx; yk_sha1_init(&ctx);
    yk_sha1_update(&ctx, (const unsigned char*)client_key, strlen(client_key));
    yk_sha1_update(&ctx, (const unsigned char*)magic, 36);
    unsigned char hash[20]; yk_sha1_final(&ctx, hash);
    yk_ws_base64_encode(hash, 20, out);
}

// WebSocket frame send (unmasked server → client, cross-platform)
static int yk_ws_send(SOCKET fd, int opcode, const char* data, int64_t len) {
    char header[10]; int hdr_len;
    if (len < 126) { header[0] = 0x80 | (opcode & 0x0F); header[1] = (char)len; hdr_len = 2; }
    else if (len < 65536) { header[0] = 0x80 | (opcode & 0x0F); header[1] = 126;
        header[2] = (char)(len >> 8); header[3] = (char)(len); hdr_len = 4; }
    else { header[0] = 0x80 | (opcode & 0x0F); header[1] = 127;
        for (int i = 0; i < 8; i++) header[2+i] = (char)(len >> (56 - i*8)); hdr_len = 10; }
    if (send(fd, header, hdr_len, 0) < 0) return -1;
    if (data && len > 0) {
        if (send(fd, data, (int)len, 0) < 0) return -1;
    }
    return 0;
}

// ── WS async state machine (non-blocking IOCP/io_uring) ─────────

#define YK_WS_STATE_FRAME_HEADER 0
#define YK_WS_STATE_EXT_LEN_16 1
#define YK_WS_STATE_EXT_LEN_64 2
#define YK_WS_STATE_MASK_KEY 3
#define YK_WS_STATE_PAYLOAD 4

typedef struct {
    int state;
    int fin;
    int opcode;
    int masked;
    int64_t payload_len;
    int64_t payload_read;
    char mask_key[4];
    char* payload;
} yk_ws_state;

// Initialize WS async session: parse key from HTTP buffer, send 101, create ws state
static void yk_ws_init(yk_conn* c, yk_server* s) {
    char key_buf[256];
    char* key_start = strstr(c->buf, "Sec-WebSocket-Key:");
    if (!key_start) key_start = strstr(c->buf, "sec-websocket-key:");
    if (!key_start) { yk_closesocket(c->socket); yk_conn_free(c); return; }
    key_start += 18;
    while (*key_start == ' ') key_start++;
    char* key_end = strstr(key_start, "\r\n");
    if (!key_end) { yk_closesocket(c->socket); yk_conn_free(c); return; }
    int key_len = (int)(key_end - key_start);
    if (key_len > 255) key_len = 255;
    memcpy(key_buf, key_start, key_len);
    key_buf[key_len] = '\0';
    char accept_buf[64];
    yk_ws_accept_key(key_buf, accept_buf, sizeof(accept_buf));
    int n = snprintf(c->resp_buf, c->resp_buf_cap,
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: %s\r\n\r\n", accept_buf);
    send(c->socket, c->resp_buf, n, 0);

    // Create WS state, consume HTTP request buffer
    yk_ws_state* ws = (yk_ws_state*)malloc(sizeof(yk_ws_state));
    memset(ws, 0, sizeof(yk_ws_state));
    ws->state = YK_WS_STATE_FRAME_HEADER;
    c->ws = ws;
    c->keep_alive = 1; // WS connections stay alive until close frame
    c->buf_used = 0;
}

// Process WS frames from buffer (non-blocking, called from IOCP/io_uring recv completion)
// Returns: 0=need more data, 1=done(close/error), -1=error
static int yk_ws_process_data(yk_conn* c) {
    yk_ws_state* ws = (yk_ws_state*)c->ws;
    if (!ws) return -1;

    uint8_t* raw = (uint8_t*)c->buf;
    int raw_len = c->buf_used;
    int pos = 0;
    int frame_complete = 0;

    while (pos < raw_len && !frame_complete) {
        switch (ws->state) {
            case YK_WS_STATE_FRAME_HEADER: {
                if (raw_len - pos < 2) goto done;
                ws->fin = (raw[pos] >> 7) & 1;
                ws->opcode = raw[pos] & 0x0F;
                ws->masked = (raw[pos + 1] >> 7) & 1;
                int64_t len7 = raw[pos + 1] & 0x7F;
                pos += 2;
                if (len7 < 126) {
                    ws->payload_len = len7;
                    ws->state = YK_WS_STATE_MASK_KEY;
                } else if (len7 == 126) {
                    ws->state = YK_WS_STATE_EXT_LEN_16;
                } else {
                    ws->state = YK_WS_STATE_EXT_LEN_64;
                }
                break;
            }
            case YK_WS_STATE_EXT_LEN_16: {
                if (raw_len - pos < 2) goto done;
                ws->payload_len = ((int64_t)raw[pos] << 8) | raw[pos + 1];
                pos += 2;
                ws->state = YK_WS_STATE_MASK_KEY;
                break;
            }
            case YK_WS_STATE_EXT_LEN_64: {
                if (raw_len - pos < 8) goto done;
                ws->payload_len = 0;
                for (int i = 0; i < 8; i++)
                    ws->payload_len = (ws->payload_len << 8) | raw[pos + i];
                pos += 8;
                ws->state = YK_WS_STATE_MASK_KEY;
                break;
            }
            case YK_WS_STATE_MASK_KEY: {
                if (ws->masked) {
                    if (raw_len - pos < 4) goto done;
                    memcpy(ws->mask_key, raw + pos, 4);
                    pos += 4;
                }
                if (ws->payload_len > 65536) return -1;
                ws->payload_read = 0;
                if (ws->payload_len > 0) {
                    ws->payload = (char*)malloc((size_t)(ws->payload_len + 1));
                    if (!ws->payload) return -1;
                }
                ws->state = YK_WS_STATE_PAYLOAD;
                if (ws->payload_len == 0) frame_complete = 1;
                break;
            }
            case YK_WS_STATE_PAYLOAD: {
                int64_t avail = raw_len - pos;
                int64_t need = ws->payload_len - ws->payload_read;
                int64_t copy = (avail < need) ? avail : need;
                if (copy > 0) {
                    memcpy(ws->payload + ws->payload_read, raw + pos, (size_t)copy);
                    pos += (int)copy;
                    ws->payload_read += copy;
                }
                if (ws->payload_read >= ws->payload_len) frame_complete = 1;
                break;
            }
            default:
                return -1;
        }

        if (frame_complete) {
            // Unmask payload
            if (ws->masked && ws->payload)
                for (int64_t i = 0; i < ws->payload_len; i++)
                    ws->payload[i] ^= ws->mask_key[i & 3];
            if (ws->payload) ws->payload[ws->payload_len] = '\0';

            switch (ws->opcode) {
                case 8: // close
                    free(ws->payload); ws->payload = NULL;
                    return 1;
                case 9: // ping → pong
                    yk_ws_send(c->socket, 10, ws->payload, ws->payload_len);
                    break;
                case 1: // text frame → echo back
                    yk_ws_send(c->socket, 1, ws->payload, ws->payload_len);
                    break;
                default:
                    break;
            }
            free(ws->payload);
            ws->payload = NULL;
            ws->state = YK_WS_STATE_FRAME_HEADER;
            frame_complete = 0;
        }
    }

done:
    if (pos < raw_len && pos > 0) {
        memmove(c->buf, c->buf + pos, raw_len - pos);
        c->buf_used = raw_len - pos;
    } else {
        c->buf_used = 0;
    }
    return 0; // need more data
}

// Cleanup WS async session
static void yk_ws_cleanup(yk_conn* c) {
    yk_ws_state* ws = (yk_ws_state*)c->ws;
    if (!ws) return;
    if (ws->payload) free(ws->payload);
    free(ws);
    c->ws = NULL;
    yk_closesocket(c->socket);
    yk_conn_free(c);
}

// Handle WebSocket connection: upgrade + frame loop (legacy blocking, for interp compat)
static void yk_ws_handle(yk_conn* c, yk_server* s) {
    // Upgrade: 101 Switching Protocols
    char key_buf[256]; char* key_start = strstr(c->buf, "Sec-WebSocket-Key:");
    if (!key_start) key_start = strstr(c->buf, "sec-websocket-key:");
    if (!key_start) { closesocket(c->socket); yk_conn_free(c); return; }
    key_start += 18; while (*key_start == ' ') key_start++;
    char* key_end = strstr(key_start, "\r\n"); if (!key_end) { closesocket(c->socket); yk_conn_free(c); return; }
    int key_len = (int)(key_end - key_start); if (key_len > 255) key_len = 255;
    memcpy(key_buf, key_start, key_len); key_buf[key_len] = '\0';
    char accept_buf[64]; yk_ws_accept_key(key_buf, accept_buf, sizeof(accept_buf));
    int n = snprintf(c->resp_buf, c->resp_buf_cap,
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: %s\r\n\r\n", accept_buf);
    send(c->socket, c->resp_buf, n, 0);
    // Frame loop
    while (s->running) {
        char frame_hdr[2]; int r = recv(c->socket, frame_hdr, 2, 0);
        if (r <= 0) break;
        int fin = (frame_hdr[0] >> 7) & 1;
        int opcode = frame_hdr[0] & 0x0F;
        int masked = (frame_hdr[1] >> 7) & 1;
        int64_t payload_len = frame_hdr[1] & 0x7F;
        if (payload_len == 126) { char ext[2]; if (recv(c->socket, ext, 2, 0) <= 0) break; payload_len = ((int64_t)ext[0] << 8) | ext[1]; }
        else if (payload_len == 127) { char ext[8]; if (recv(c->socket, ext, 8, 0) <= 0) break; payload_len = 0; for (int i = 0; i < 8; i++) payload_len = (payload_len << 8) | (unsigned char)ext[i]; }
        char mask_key[4]; if (masked) { if (recv(c->socket, mask_key, 4, 0) <= 0) break; }
        if (payload_len > 65536) break; // sanity
        // Allocate and read payload
        char* payload = (char*)malloc((size_t)(payload_len + 1));
        int64_t to_read = payload_len;
        int64_t offset = 0;
        while (to_read > 0) { r = recv(c->socket, payload + offset, (int)to_read, 0); if (r <= 0) { free(payload); goto done; } offset += r; to_read -= r; }
        if (masked) for (int64_t i = 0; i < payload_len; i++) payload[i] ^= mask_key[i & 3];
        payload[payload_len] = '\0';
        if (opcode == 8) { free(payload); break; } // close
        if (opcode == 9) { yk_ws_send(c->socket, 10, NULL, 0); free(payload); continue; } // ping → pong
        if (opcode == 1) { // text frame → echo back
            yk_ws_send(c->socket, 1, payload, payload_len);
        }
        free(payload);
    }
    done:
    closesocket(c->socket);
    yk_conn_free(c);
}

// ═══════════════════════════════════════════════════════════════
// HTTP/2 — RFC 7540
// ═══════════════════════════════════════════════════════════════

// ── Frame types ─────────────────────────────────────────────
#define YK_H2_DATA         0x00
#define YK_H2_HEADERS      0x01
#define YK_H2_PRIORITY     0x02
#define YK_H2_RST_STREAM   0x03
#define YK_H2_SETTINGS     0x04
#define YK_H2_PUSH_PROMISE 0x05
#define YK_H2_PING         0x06
#define YK_H2_GOAWAY       0x07
#define YK_H2_WINDOW_UPDATE 0x08
#define YK_H2_CONTINUATION 0x09

#define YK_H2_FLAG_END_STREAM  0x01
#define YK_H2_FLAG_END_HEADERS 0x04
#define YK_H2_FLAG_PADDED      0x08
#define YK_H2_FLAG_PRIORITY    0x20

#define YK_H2_NO_ERROR           0x00
#define YK_H2_PROTOCOL_ERROR     0x01
#define YK_H2_INTERNAL_ERROR     0x02
#define YK_H2_FLOW_CONTROL_ERROR 0x03
#define YK_H2_STREAM_CLOSED      0x05
#define YK_H2_FRAME_SIZE_ERROR   0x06
#define YK_H2_REFUSED_STREAM     0x07
#define YK_H2_COMPRESSION_ERROR  0x09

#define YK_H2_SETTINGS_HEADER_TABLE_SIZE     0x01
#define YK_H2_SETTINGS_ENABLE_PUSH           0x02
#define YK_H2_SETTINGS_MAX_CONCURRENT_STREAMS 0x03
#define YK_H2_SETTINGS_INITIAL_WINDOW_SIZE   0x04
#define YK_H2_SETTINGS_MAX_FRAME_SIZE        0x05

#define YK_H2_INITIAL_WINDOW_SIZE 65535
#define YK_H2_MAX_FRAME_SIZE      16384
#define YK_H2_MAX_STREAMS         128
#define YK_H2_HPACK_MAX_TABLE     4096

// ── Huffman codes (RFC 7541 Appendix B) ─────────────────────
static const uint32_t yk_h2_huff_code[256] = {
    0x1ff8,0x7fffd8,0xfffffe2,0xfffffe3,0xfffffe4,0xfffffe5,0xfffffe6,0xfffffe7,
    0xfffffe8,0xffffea,0x3ffffffc,0xfffffe9,0xfffffea,0x3ffffffd,0xfffffeb,0xfffffec,
    0xfffffed,0xfffffee,0xfffffef,0xffffff0,0xffffff1,0xffffff2,0x3ffffffe,0xffffff3,
    0xffffff4,0xffffff5,0xffffff6,0xffffff7,0xffffff8,0xffffff9,0xffffffa,0xffffffb,
    0x14,0x3f8,0x3f9,0xffa,0x1ff9,0x15,0xf8,0x7fa,0x3fa,0x3fb,0xf9,0xfb7,
    0xfb8,0xfb9,0xfba,0xfbb,0xfbc,0xfbd,0xfbe,0xfbf,0xfc0,0xfc1,0xfc2,0xfc3,
    0xfc4,0xfc5,0xfc6,0xfc7,0xfc8,0xfc9,0xfca,0xfcb,0xfcc,0xfcd,0xfce,0xfcf,
    0xfd0,0xfd1,0xfd2,0xfd3,0xfd4,0xfd5,0xfd6,0xfd7,0xfd8,0xfd9,0xfda,0xfdb,
    0xfdc,0xfdd,0xfde,0xfdf,0xfe0,0xfe1,0xfe2,0xfe3,0xfe4,0xfe5,0xfe6,0xfe7,
    0xfe8,0xfe9,0xfea,0xfeb,0xfec,0xfed,0xfee,0xfef,0xff0,0xff1,0xff2,0xff3,
    0xff4,0xff5,0xff6,0xff7,0xff8,0xff9,0xffa,0xffb,0xffc,0xffd,0xffe,0xfff,
    0x100,0x101,0x102,0x103,0x104,0x105,0x106,0x107,0x108,0x109,0x10a,0x10b,
    0x10c,0x10d,0x10e,0x10f,0x110,0x111,0x112,0x113,0x114,0x115,0x116,0x117,
    0x118,0x119,0x11a,0x11b,0x11c,0x11d,0x11e,0x11f,0x120,0x121,0x122,0x123,
    0x124,0x125,0x126,0x127,0x128,0x129,0x12a,0x12b,0x12c,0x12d,0x12e,0x12f,
    0x130,0x131,0x132,0x133,0x134,0x135,0x136,0x137,0x138,0x139,0x13a,0x13b,
    0x13c,0x13d,0x13e,0x13f,0x140,0x141,0x142,0x143,0x144,0x145,0x146,0x147,
    0x148,0x149,0x14a,0x14b,0x14c,0x14d,0x14e,0x14f,0x150,0x151,0x152,0x153,
    0x154,0x155,0x156,0x157,0x158,0x159,0x15a,0x15b,0x15c,0x15d,0x15e,0x15f,
    0x160,0x161,0x162,0x163,0x164,0x165,0x166,0x167,0x168,0x169,0x16a,0x16b,
    0x16c,0x16d,0x16e,0x16f,0x170,0x171,0x172,0x173,0x174,0x175,0x176,0x177,
    0x178,0x179,0x17a,0x17b,0x17c,0x17d,0x17e,0x17f,
};
static const uint8_t yk_h2_huff_len[256] = {
    13,23,28,28,28,28,28,28,28,24,30,28,28,30,28,28,
    28,28,28,28,28,28,30,28,28,28,28,28,28,28,28,28,
     6,10,10,12,13, 6, 8,11,10,10, 8,11,11,11,11,11,
    11,11,11,11,11,11,11,11,11,11,11,11,11,11,11,11,
    11,11,11,11,11,11,11,11,11,11,11,11,11,11,11,11,
    11,11,11,11,11,11,11,11,11,11,11,11,11,11,11,11,
    11,11,11,11,11,11,11,11,11,11,11,11,11,11,11,11,
    12,12,12,12,12,12,12,12,12,12,12,12,12,12,12,12,
    12,12,12,12,12,12,12,12,12,12,12,12,12,12,12,12,
    12,12,12,12,12,12,12,12,12,12,12,12,12,12,12,12,
    12,12,12,12,12,12,12,12,12,12,12,12,12,12,12,12,
    12,12,12,12,12,12,12,12,12,12,12,12,12,12,12,12,
    12,12,12,12,12,12,12,12,12,12,12,12,12,12,12,12,
    12,12,12,12,12,12,12,12,12,12,12,12,12,12,12,12,
    12,12,12,12,12,12,12,12,12,12,12,12,12,12,12,12,
    12,12,12,12,12,12,12,12,12,12,12,12,12,12,12,12,
};

// ── HPACK Static Table (RFC 7541 Appendix A, first 61 entries) ──
static const char* yk_h2_static_name[61] = {
    ":authority",":method",":method",":path",":path",":scheme",":scheme",
    ":status",":status",":status",":status",":status",":status",":status",
    "accept-charset","accept-encoding","accept-language","accept-ranges","age","allow",
    "authorization","cache-control","content-disposition","content-encoding","content-language","content-length","content-location","content-range",
    "content-type","cookie","date","etag","expect","expires","from","host",
    "if-match","if-modified-since","if-none-match","if-range","if-unmodified-since","last-modified","link","location",
    "max-forwards","proxy-authenticate","proxy-authorization","range","referer","retry-after","server","set-cookie",
    "strict-transport-security","transfer-encoding","user-agent","vary","via","www-authenticate",
    "content-encoding","content-type","content-type",
};
static const char* yk_h2_static_value[61] = {
    "","GET","POST","/","/index.html","http","https",
    "200","204","206","304","400","404","500",
    "","","","","","",
    "","","","","","","","",
    "","","","","","","","",
    "","","","","","","","",
    "","","","","","","",
    "","","","","","","",
    "gzip","application/x-www-form-urlencoded","text/html; charset=utf-8",
};

// ── HPACK Decoder ───────────────────────────────────────────
typedef struct {
    uint8_t* data;
    int pos;
    int len;
    uint32_t dyn_table[YK_H2_HPACK_MAX_TABLE / 32]; // dynamic table (simplified: name+value pairs stored as flat strings)
} yk_hpack_decoder;

static uint32_t yk_h2_read_byte(uint8_t* buf, int* pos, int len) {
    if (*pos >= len) return 0;
    return buf[(*pos)++];
}

// Decode variable-length integer (RFC 7541 §5.1)
static uint64_t yk_h2_decode_int(uint8_t* buf, int* pos, int len, int prefix_bits) {
    uint64_t val = buf[(*pos)++] & ((1 << prefix_bits) - 1);
    if (val < (uint64_t)((1 << prefix_bits) - 1)) return val;
    uint64_t m = 0;
    uint64_t b;
    do {
        b = yk_h2_read_byte(buf, pos, len);
        val += (b & 0x7f) << m;
        m += 7;
    } while (b & 0x80);
    return val;
}

// Decode a Huffman-coded string
static int yk_h2_decode_huff(uint8_t* buf, int* pos, int len, uint8_t* out, int out_max) {
    uint64_t bits = 0;
    int bits_left = 0;
    int out_pos = 0;
    int start = *pos;
    int end = start + (int)yk_h2_decode_int(buf, pos, len, 7);
    if (end > len) end = len;
    int i;
    for (i = start; i < end; i++) {
        bits = (bits << 8) | buf[i];
        bits_left += 8;
        while (bits_left >= 16) {
            uint16_t top = (uint16_t)(bits >> (bits_left - 16));
            int found = 0;
            for (int s = 0; s < 256; s++) {
                int clen = yk_h2_huff_len[s];
                if (clen > bits_left) continue;
                uint32_t mask = (clen < 16) ? ((1 << clen) - 1) << (16 - clen) : 0xFFFF;
                uint32_t code_shifted = yk_h2_huff_code[s];
                if (clen < 16) code_shifted <<= (16 - clen);
                if ((top & mask) == (code_shifted & mask)) {
                    if (out_pos < out_max) out[out_pos++] = (uint8_t)s;
                    bits_left -= clen;
                    found = 1;
                    break;
                }
            }
            if (!found) break; // padding
        }
    }
    // EOS symbol (0xFF) padding check — skip remaining bits
    *pos = end;
    return out_pos;
}

// Decode a plain (non-Huffman) string
static int yk_h2_decode_plain(uint8_t* buf, int* pos, int len, uint8_t* out, int out_max) {
    int str_len = (int)yk_h2_decode_int(buf, pos, len, 7);
    if (*pos + str_len > len) str_len = len - *pos;
    int copy = str_len < out_max ? str_len : out_max;
    memcpy(out, buf + *pos, copy);
    *pos += str_len;
    return copy;
}

// Decode a string literal (RFC 7541 §5.2)
static int yk_h2_decode_string(uint8_t* buf, int* pos, int len, uint8_t* out, int out_max) {
    if (*pos >= len) return 0;
    int huff = (buf[*pos] >> 7) & 1;
    if (huff) return yk_h2_decode_huff(buf, pos, len, out, out_max);
    return yk_h2_decode_plain(buf, pos, len, out, out_max);
}

// ── H2 Stream ───────────────────────────────────────────────
typedef enum {
    YK_H2_SS_IDLE,
    YK_H2_SS_OPEN,
    YK_H2_SS_HALF_CLOSED_REMOTE,
    YK_H2_SS_HALF_CLOSED_LOCAL,
    YK_H2_SS_CLOSED
} yk_h2_stream_state;

typedef struct {
    uint32_t id;
    yk_h2_stream_state state;
    int32_t window_size;
    uint8_t* header_data;
    int header_data_len;
    int headers_complete;
    uint8_t* body;
    int body_len;
    int body_cap;
} yk_h2_stream;

// ── H2 Session ──────────────────────────────────────────────
typedef struct {
    yk_conn* conn;
    yk_server* server;
    uint32_t last_stream_id;
    int32_t window_size;
    yk_h2_stream streams[YK_H2_MAX_STREAMS];
    int stream_count;
    int goaway_sent;
    // SETTINGS
    uint32_t settings_max_concurrent;
    uint32_t settings_initial_window;
    uint32_t settings_max_frame;
    // HPACK decoder state (dynamic table — simplified: reuse static only)
    uint8_t* decoder_buf;
    int decoder_len;
} yk_h2_session;

static yk_h2_stream* yk_h2_find_stream(yk_h2_session* sess, uint32_t id) {
    for (int i = 0; i < sess->stream_count; i++)
        if (sess->streams[i].id == id) return &sess->streams[i];
    return NULL;
}

static yk_h2_stream* yk_h2_new_stream(yk_h2_session* sess, uint32_t id) {
    if (sess->stream_count >= YK_H2_MAX_STREAMS) return NULL;
    yk_h2_stream* s = &sess->streams[sess->stream_count++];
    memset(s, 0, sizeof(yk_h2_stream));
    s->id = id;
    s->state = YK_H2_SS_OPEN;
    s->window_size = sess->settings_initial_window;
    return s;
}

// ── Frame I/O ───────────────────────────────────────────────
static int yk_h2_read_frame(uint8_t* buf, int* pos, int len,
    uint32_t* out_length, uint8_t* out_type, uint8_t* out_flags, uint32_t* out_stream_id) {
    if (*pos + 9 > len) return 0;
    *out_length = ((uint32_t)buf[*pos] << 16) | ((uint32_t)buf[*pos+1] << 8) | buf[*pos+2];
    *out_type = buf[*pos+3];
    *out_flags = buf[*pos+4];
    *out_stream_id = ((uint32_t)buf[*pos+5] << 24) | ((uint32_t)buf[*pos+6] << 16) |
                     ((uint32_t)buf[*pos+7] << 8) | buf[*pos+8];
    *out_stream_id &= 0x7FFFFFFF;
    *pos += 9;
    return 1;
}

static int yk_h2_write_frame_header(uint8_t* buf, uint32_t length, uint8_t type, uint8_t flags, uint32_t stream_id) {
    buf[0] = (uint8_t)(length >> 16);
    buf[1] = (uint8_t)(length >> 8);
    buf[2] = (uint8_t)(length);
    buf[3] = type;
    buf[4] = flags;
    buf[5] = (uint8_t)(stream_id >> 24);
    buf[6] = (uint8_t)(stream_id >> 16);
    buf[7] = (uint8_t)(stream_id >> 8);
    buf[8] = (uint8_t)(stream_id);
    return 9;
}

// ── Send H2 frame on socket ─────────────────────────────────
static void yk_h2_send_frame(yk_conn* c, uint8_t type, uint8_t flags, uint32_t stream_id,
    uint8_t* payload, uint32_t payload_len) {
    uint8_t hdr[9];
    yk_h2_write_frame_header(hdr, payload_len, type, flags, stream_id);
#ifdef _WIN32
    WSABUF bufs[2];
    bufs[0].buf = (char*)hdr; bufs[0].len = 9;
    bufs[1].buf = (char*)payload; bufs[1].len = payload_len;
    DWORD sent;
    WSASend(c->socket, bufs, 2, &sent, 0, NULL, NULL);
#else
    struct iovec iov[2];
    iov[0].iov_base = hdr; iov[0].iov_len = 9;
    iov[1].iov_base = payload; iov[1].iov_len = payload_len;
    writev(c->socket, iov, 2);
#endif
}

// ── Send SETTINGS frame ─────────────────────────────────────
static void yk_h2_send_settings(yk_conn* c) {
    uint8_t payload[18]; // 3 settings × 6 bytes each
    int pos = 0;
    // SETTINGS_MAX_CONCURRENT_STREAMS = 128 (ID=0x0003)
    payload[pos++] = 0; payload[pos++] = 0x03;
    payload[pos++] = 0; payload[pos++] = 0; payload[pos++] = 0; payload[pos++] = 128;
    // SETTINGS_INITIAL_WINDOW_SIZE = 65535 (ID=0x0004)
    payload[pos++] = 0; payload[pos++] = 0x04;
    payload[pos++] = 0; payload[pos++] = 0; payload[pos++] = 0xFF; payload[pos++] = 0xFF;
    // SETTINGS_MAX_FRAME_SIZE = 16384 (ID=0x0005)
    payload[pos++] = 0; payload[pos++] = 0x05;
    payload[pos++] = 0; payload[pos++] = 0; payload[pos++] = 0x40; payload[pos++] = 0;
    yk_h2_send_frame(c, YK_H2_SETTINGS, 0, 0, payload, pos);
}

// ── Send HEADERS + DATA response ────────────────────────────
static void yk_h2_send_response(yk_conn* c, uint32_t stream_id, int status, const char* body, int64_t body_len) {
    uint8_t hdr_buf[32];
    uint8_t* p = hdr_buf;
    switch (status) {
        case 200: *p++ = 0x88; break;  // Index 8 = :status 200
        case 204: *p++ = 0x89; break;  // Index 9 = :status 204
        case 206: *p++ = 0x8A; break;  // Index 10 = :status 206
        case 304: *p++ = 0x8B; break;  // Index 11 = :status 304
        case 400: *p++ = 0x8C; break;  // Index 12 = :status 400
        case 404: *p++ = 0x8D; break;  // Index 13 = :status 404
        case 500: *p++ = 0x8E; break;  // Index 14 = :status 500
        default:  *p++ = 0x8E; break;  // Fallback to 500
    }
    int hdr_len = (int)(p - hdr_buf);
    yk_h2_send_frame(c, YK_H2_HEADERS, YK_H2_FLAG_END_HEADERS, stream_id, hdr_buf, hdr_len);
    
    if (body && body_len > 0) {
        yk_h2_send_frame(c, YK_H2_DATA, YK_H2_FLAG_END_STREAM, stream_id, (uint8_t*)body, (uint32_t)body_len);
    } else {
        // Send empty DATA frame with END_STREAM
        yk_h2_send_frame(c, YK_H2_DATA, YK_H2_FLAG_END_STREAM, stream_id, NULL, 0);
    }
}

// ── Process H2 request from stream headers ─────────────────
static void yk_h2_process_stream(yk_h2_session* sess, yk_h2_stream* stream) {
    yk_conn* c = sess->conn;
    yk_server* s = sess->server;
    (void)c; (void)s;
    
    // Parse method and path from headers
    // We stored raw headers in stream->header_data; find :method and :path
    char method[64] = "GET";
    char path[4096] = "/";
    if (stream->header_data && stream->header_data_len > 0) {
        // Simple linear scan for :method and :path pseudo-headers
        char* hdrs = (char*)stream->header_data;
        int hlen = stream->header_data_len;
        char* method_start = strstr(hdrs, ":method");
        if (method_start && method_start < hdrs + hlen - 8) {
            method_start += 8; // skip ":method "
            char* end = strchr(method_start, '\0');
            if (!end) end = method_start + 32;
            int ml = (int)(end - method_start);
            if (ml > 63) ml = 63;
            memcpy(method, method_start, ml); method[ml] = '\0';
        }
        char* path_start = strstr(hdrs, ":path");
        if (path_start && path_start < hdrs + hlen - 6) {
            path_start += 6; // skip ":path "
            char* end = strchr(path_start, '\0');
            if (!end) end = path_start + 4095;
            int pl = (int)(end - path_start);
            if (pl > 4095) pl = 4095;
            memcpy(path, path_start, pl); path[pl] = '\0';
        }
    }
    (void)method; (void)path;
    
    // Match route and call handler
    yk_route_node* matched = yk_route_match_node(s->trie_root, path, method);
    
    if (matched) {
        struct { void* m; int64_t ml; void* p; int64_t pl; void* b; int64_t bl; } req;
        req.m = method; req.ml = strlen(method);
        req.p = path; req.pl = strlen(path);
        req.b = stream->body; req.bl = stream->body_len;
        
        struct { void* body; int64_t body_len; int32_t status; } resp;
        resp.body = NULL; resp.body_len = 0; resp.status = 200;
        
        char* handler_buf = yk_get_handler_buf(16384);
        matched->handler(&resp, &req, handler_buf, yk_tls_handler_buf_size);
        
        yk_h2_send_response(c, stream->id, resp.status, (char*)resp.body, resp.body_len);
    } else {
        yk_h2_send_response(c, stream->id, 404, "Not Found", 9);
    }
    
    stream->state = YK_H2_SS_CLOSED;
}

// ── Handle SETTINGS frame ──────────────────────────────────
static void yk_h2_handle_settings(yk_h2_session* sess, uint8_t* payload, uint32_t len) {
    for (uint32_t i = 0; i + 5 < len; i += 6) {
        uint16_t id = (uint16_t)((payload[i] << 8) | payload[i+1]);
        uint32_t val = ((uint32_t)payload[i+2] << 24) | ((uint32_t)payload[i+3] << 16) |
                       ((uint32_t)payload[i+4] << 8) | payload[i+5];
        switch (id) {
            case YK_H2_SETTINGS_MAX_CONCURRENT_STREAMS:
                if (val < YK_H2_MAX_STREAMS) sess->settings_max_concurrent = val;
                break;
            case YK_H2_SETTINGS_INITIAL_WINDOW_SIZE:
                sess->settings_initial_window = val;
                break;
            case YK_H2_SETTINGS_MAX_FRAME_SIZE:
                if (val >= 16384 && val <= 16777215) sess->settings_max_frame = val;
                break;
        }
    }
    // Send SETTINGS ACK
    yk_h2_send_frame(sess->conn, YK_H2_SETTINGS, 0x01, 0, NULL, 0);
}

// ── Handle HEADERS frame ────────────────────────────────────
static void yk_h2_handle_headers(yk_h2_session* sess, uint8_t* payload, uint32_t len, uint8_t flags, uint32_t stream_id) {
    yk_h2_stream* stream = yk_h2_find_stream(sess, stream_id);
    if (!stream) {
        stream = yk_h2_new_stream(sess, stream_id);
        if (!stream) {
            uint8_t refused_frame[4] = {0, 0, 0, YK_H2_REFUSED_STREAM};
            yk_h2_send_frame(sess->conn, YK_H2_RST_STREAM, 0, stream_id,
                refused_frame, 4);
            return;
        }
    }
    
    // Parse HPACK-encoded headers from payload
    int pos = 0;
    int padded = flags & YK_H2_FLAG_PADDED;
    int priority = flags & YK_H2_FLAG_PRIORITY;
    if (padded && pos < (int)len) { int pad = payload[pos++]; if ((int)pad <= (int)len - pos) len -= pad; else return; }
    if (priority && pos + 4 < (int)len) pos += 5; // E + stream dep + weight
    
    // Decode headers into a flat buffer
    uint8_t hdr_buf[4096];
    int hdr_pos = 0;
    while (pos < (int)len) {
        uint8_t first = payload[pos];
        if (first & 0x80) {
            // Indexed header field
            uint64_t idx = yk_h2_decode_int(payload, &pos, len, 7);
            int sidx = (int)idx - 1;
            if (sidx >= 0 && sidx < 61) {
                int nl = (int)strlen(yk_h2_static_name[sidx]);
                int vl = (int)strlen(yk_h2_static_value[sidx]);
                if (hdr_pos + nl + vl + 2 < 4096) {
                    memcpy(hdr_buf + hdr_pos, yk_h2_static_name[sidx], nl);
                    hdr_pos += nl; hdr_buf[hdr_pos++] = ' ';
                    memcpy(hdr_buf + hdr_pos, yk_h2_static_value[sidx], vl);
                    hdr_pos += vl; hdr_buf[hdr_pos++] = '\0';
                }
            }
        } else if ((first & 0xC0) == 0x40) {
            // Literal with indexing — name indexed or literal
            uint64_t idx = 0;
            if ((first & 0x3F) > 0) {
                idx = yk_h2_decode_int(payload, &pos, len, 6);
            }
            uint8_t name_buf[256]; int name_len = 0;
            uint8_t val_buf[1024]; int val_len = 0;
            
            if (idx > 0 && (int)idx - 1 < 61) {
                name_len = (int)strlen(yk_h2_static_name[(int)idx - 1]);
                memcpy(name_buf, yk_h2_static_name[(int)idx - 1], name_len);
            } else {
                name_len = yk_h2_decode_string(payload, &pos, (int)len, name_buf, 256);
            }
            val_len = yk_h2_decode_string(payload, &pos, (int)len, val_buf, 1024);
            
            if (hdr_pos + name_len + val_len + 2 < 4096) {
                memcpy(hdr_buf + hdr_pos, name_buf, name_len);
                hdr_pos += name_len; hdr_buf[hdr_pos++] = ' ';
                memcpy(hdr_buf + hdr_pos, val_buf, val_len);
                hdr_pos += val_len; hdr_buf[hdr_pos++] = '\0';
            }
        } else if ((first & 0xC0) == 0x00) {
            // Literal without indexing / never indexed
            uint64_t idx = 0;
            if ((first & 0x0F) > 0) {
                idx = yk_h2_decode_int(payload, &pos, len, 4);
            }
            uint8_t name_buf[256]; int name_len = 0;
            uint8_t val_buf[1024]; int val_len = 0;
            
            if (idx > 0 && (int)idx - 1 < 61) {
                name_len = (int)strlen(yk_h2_static_name[(int)idx - 1]);
                memcpy(name_buf, yk_h2_static_name[(int)idx - 1], name_len);
            } else {
                name_len = yk_h2_decode_string(payload, &pos, (int)len, name_buf, 256);
            }
            val_len = yk_h2_decode_string(payload, &pos, (int)len, val_buf, 1024);
            
            if (hdr_pos + name_len + val_len + 2 < 4096) {
                memcpy(hdr_buf + hdr_pos, name_buf, name_len);
                hdr_pos += name_len; hdr_buf[hdr_pos++] = ' ';
                memcpy(hdr_buf + hdr_pos, val_buf, val_len);
                hdr_pos += val_len; hdr_buf[hdr_pos++] = '\0';
            }
        } else {
            // Literal without indexing, never indexed (0x10-0x1F)
            break;
        }
    }
    
    // Store headers
    if (stream->header_data) free(stream->header_data);
    stream->header_data = (uint8_t*)malloc(hdr_pos + 1);
    memcpy(stream->header_data, hdr_buf, hdr_pos);
    stream->header_data[hdr_pos] = '\0';
    stream->header_data_len = hdr_pos;
    
    if (flags & YK_H2_FLAG_END_STREAM) {
        stream->headers_complete = 1;
        yk_h2_process_stream(sess, stream);
    }
}

// ── Handle DATA frame ───────────────────────────────────────
static void yk_h2_handle_data(yk_h2_session* sess, uint8_t* payload, uint32_t len, uint8_t flags, uint32_t stream_id) {
    yk_h2_stream* stream = yk_h2_find_stream(sess, stream_id);
    if (!stream) return;
    
    // Append body data
    if (len > 0) {
        int new_len = stream->body_len + (int)len;
        if (new_len > stream->body_cap) {
            stream->body_cap = new_len + 1024;
            stream->body = (uint8_t*)realloc(stream->body, stream->body_cap);
        }
        memcpy(stream->body + stream->body_len, payload, len);
        stream->body_len = new_len;
        
        // WINDOW_UPDATE: return window capacity to sender (stream + connection)
        uint8_t wu[4];
        wu[0] = (uint8_t)(len >> 24); wu[1] = (uint8_t)(len >> 16);
        wu[2] = (uint8_t)(len >> 8); wu[3] = (uint8_t)(len);
        yk_h2_send_frame(sess->conn, YK_H2_WINDOW_UPDATE, 0, stream_id, wu, 4);
        yk_h2_send_frame(sess->conn, YK_H2_WINDOW_UPDATE, 0, 0, wu, 4);
    }
    if (flags & YK_H2_FLAG_END_STREAM) {
        stream->headers_complete = 1;
        yk_h2_process_stream(sess, stream);
    }
}

// ── Main H2 session loop (blocking read loop) ──────────────
// H2 async helpers: init once, then process data as it arrives via IOCP/io_uring
static void yk_h2_init(yk_conn* c, yk_server* s) {
    yk_h2_session* sess = (yk_h2_session*)malloc(sizeof(yk_h2_session));
    memset(sess, 0, sizeof(yk_h2_session));
    sess->conn = c;
    sess->server = s;
    sess->settings_max_concurrent = YK_H2_MAX_STREAMS;
    sess->settings_initial_window = YK_H2_INITIAL_WINDOW_SIZE;
    sess->settings_max_frame = YK_H2_MAX_FRAME_SIZE;
    sess->window_size = YK_H2_INITIAL_WINDOW_SIZE;
    c->h2 = sess;
    // Send initial SETTINGS (synchronous, small frame)
    yk_h2_send_settings(c);
}

// Process available H2 data in buffer. Returns 0=need more, 1=done(GOAWAY/error), -1=closed
static int yk_h2_process_data(yk_conn* c) {
    yk_h2_session* sess = (yk_h2_session*)c->h2;
    if (!sess) return -1;

    uint8_t* raw = (uint8_t*)c->buf;
    int raw_len = c->buf_used;
    int pos = 0;

    // Process all complete frames in the buffer
    while (pos + 9 <= raw_len && !sess->goaway_sent) {
        uint32_t frame_len;
        uint8_t frame_type, flags;
        uint32_t stream_id;

        if (!yk_h2_read_frame(raw, &pos, raw_len, &frame_len, &frame_type, &flags, &stream_id))
            break;
        if (pos + (int)frame_len > raw_len) { pos -= 9; break; }

        uint8_t* payload = raw + pos;

        switch (frame_type) {
            case YK_H2_SETTINGS:
                if (stream_id == 0) {
                    if (!(flags & 0x01)) yk_h2_handle_settings(sess, payload, frame_len);
                }
                break;
            case YK_H2_HEADERS:
                yk_h2_handle_headers(sess, payload, frame_len, flags, stream_id);
                break;
            case YK_H2_DATA:
                yk_h2_handle_data(sess, payload, frame_len, flags, stream_id);
                break;
            case YK_H2_RST_STREAM: {
                yk_h2_stream* st = yk_h2_find_stream(sess, stream_id);
                if (st) st->state = YK_H2_SS_CLOSED;
                break;
            }
            case YK_H2_PING:
                yk_h2_send_frame(c, YK_H2_PING, 0x01, 0, payload, frame_len);
                break;
            case YK_H2_GOAWAY:
                sess->goaway_sent = 1;
                break;
            case YK_H2_WINDOW_UPDATE: {
                uint32_t inc = ((uint32_t)payload[0] << 24) | ((uint32_t)payload[1] << 16) |
                               ((uint32_t)payload[2] << 8) | payload[3];
                inc &= 0x7FFFFFFF;
                sess->window_size += inc;
                break;
            }
        }
        pos += (int)frame_len;
    }

    // Compact remaining partial frame data
    if (pos < raw_len) {
        memmove(c->buf, c->buf + pos, raw_len - pos);
        c->buf_used = raw_len - pos;
    } else {
        c->buf_used = 0;
    }

    if (sess->goaway_sent) return 1;
    return 0; // need more data
}

static void yk_h2_cleanup(yk_conn* c) {
    yk_h2_session* sess = (yk_h2_session*)c->h2;
    if (!sess) return;
    for (int i = 0; i < sess->stream_count; i++) {
        if (sess->streams[i].header_data) free(sess->streams[i].header_data);
        if (sess->streams[i].body) free(sess->streams[i].body);
    }
    free(sess);
    c->h2 = NULL;
    yk_closesocket(c->socket);
    yk_conn_free(c);
}

// Legacy blocking H2 entry point (used by interpreter/JIT, kept for compatibility)
static void yk_h2_run(yk_conn* c, yk_server* s) {
    yk_h2_init(c, s);
    int skip = (c->buf_used >= 24) ? 24 : 0;
    if (skip > 0) {
        memmove(c->buf, c->buf + skip, c->buf_used - skip);
        c->buf_used -= skip;
    }
    while (s->running) {
        int ret = yk_h2_process_data(c);
        if (ret == 1) { yk_h2_cleanup(c); return; }
        if (ret < 0) { yk_h2_cleanup(c); return; }
        if (c->buf_used >= (int)c->buf_cap - 1) {
            if (yk_conn_grow_buf(c, c->buf_used + 4096) != 0) break;
        }
        int n = recv(c->socket, c->buf + c->buf_used,
                     (int)c->buf_cap - c->buf_used - 1, 0);
        if (n <= 0) break;
        c->buf_used += n;
        c->buf[c->buf_used] = '\0';
    }
    yk_h2_cleanup(c);
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
    /// Maps variable name -> the variant type expressions for its union type
    var_union_variants: HashMap<String, Vec<TypeExpr>>,
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
    fn_param_types: HashMap<String, Vec<String>>,
    fn_param_union_variants: HashMap<String, Vec<Option<Vec<TypeExpr>>>>,
    handler_irs: Vec<String>,
    fn_defs: HashMap<String, crate::interpret::FnDef>,
    interface_methods: HashMap<String, Vec<String>>,
    interface_method_ret_types: HashMap<(String, String), String>,
    class_impls: HashMap<String, Vec<String>>,
    class_modules: HashMap<String, String>,
    object_defs: HashMap<String, Vec<(String, String)>>,
    object_method_ret_types: HashMap<(String, String), String>,
    object_method_has_self: HashSet<(String, String)>,
    object_modules: HashMap<String, String>,
    http_vars: HashSet<String>,
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
            var_union_variants: HashMap::new(),
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
            fn_param_types: HashMap::new(),
            fn_param_union_variants: HashMap::new(),
            handler_irs: Vec::new(),
            fn_defs: HashMap::new(),
            interface_methods: HashMap::new(),
            interface_method_ret_types: HashMap::new(),
            class_impls: HashMap::new(),
            class_modules: HashMap::new(),
            object_defs: HashMap::new(),
            object_method_ret_types: HashMap::new(),
            object_method_has_self: HashSet::new(),
            object_modules: HashMap::new(),
            http_vars: HashSet::new(),
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
            "get" | "post" | "put" | "delete" | "patch" | "ws" => {
                let method_name = if field == "ws" { "WS".to_string() } else { field.to_uppercase() };
                let method_slot = self.make_string_slot(&method_name);
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
                                .filter(|l| !l.starts_with("target triple") && !l.starts_with("; JIT") && !l.starts_with("%YkResponse"))
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
                                        .filter(|l| !l.starts_with("target triple") && !l.starts_with("; JIT") && !l.starts_with("%YkResponse"))
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
                                    .filter(|l| !l.starts_with("target triple") && !l.starts_with("; JIT") && !l.starts_with("%YkResponse"))
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
                                    .filter(|l| !l.starts_with("target triple") && !l.starts_with("; JIT") && !l.starts_with("%YkResponse"))
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

    fn is_union_ty(ty: &str) -> bool {
        ty == "%yk_variant"
    }

    fn wrap_in_variant(&mut self, variant_te: &TypeExpr, val: &str, val_ty: &str) -> String {
        let tag = self.variant_name_tag(&variant_te.to_string());
        let payload = if val_ty == "%yk_string" || val_ty == "ptr" {
            let a = self.fresh_label();
            self.e(&format!("%{} = alloca {}, align 8", a, val_ty));
            self.e(&format!("store {} {}, ptr %{}", val_ty, val, a));
            let p = self.fresh_label();
            self.e(&format!("%{} = ptrtoint ptr %{} to i64", p, a));
            self.ssa(&p)
        } else if val_ty == "i1" {
            let p = self.fresh_label();
            self.e(&format!("%{} = zext i1 {} to i64", p, val));
            self.ssa(&p)
        } else {
            val.to_string()
        };
        let r1 = self.fresh_label();
        self.e(&format!("%{} = insertvalue %yk_variant undef, i64 {}, 0", r1, tag));
        let r2 = self.fresh_label();
        self.e(&format!("%{} = insertvalue %yk_variant %{}, i64 {}, 1", r2, r1, payload));
        self.ssa(&r2)
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
                        } else if self.interface_methods.contains_key(name) {
                            format!("%iface.{}", name)
                        } else if self.object_defs.contains_key(name) {
                            format!("%object.{}", name)
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
                for v in variants {
                    self.get_variant_tag(&v.to_string());
                }
                "%yk_variant".into()
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
        let sig = elem_types.join("||");
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

    fn is_http_var(&self, obj: &ExprNode) -> bool {
        if let Expr::Ident(var_name) = &obj.value {
            self.http_vars.contains(var_name)
        } else { false }
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
        self.e_raw("%YkResponse = type { ptr, i64, i32 }");
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

        // Pre-scan: collect interface methods and class implements
        for module in modules {
            for item in &module.items {
                if let ItemKind::Interface { name, methods } = &item.value {
                    let method_names: Vec<String> = methods.iter()
                        .map(|m| m.name.clone())
                        .collect();
                    self.interface_methods.insert(name.clone(), method_names);
                    for m in methods {
                        let ret_ty = m.ret_type.as_ref()
                            .map(|rt| self.type_to_llvm(&rt.value))
                            .unwrap_or_else(|| "void".into());
                        self.interface_method_ret_types.insert((name.clone(), m.name.clone()), ret_ty);
                    }
                }
            }
        }
        for module in modules {
            for item in &module.items {
                if let ItemKind::Class { name, implements, .. } = &item.value {
                    for iface in implements {
                        self.class_impls.entry(iface.clone()).or_default().push(name.clone());
                    }
                }
            }
        }
        // Pre-scan object definitions
        for module in modules {
            for item in &module.items {
                if let ItemKind::Object { name, methods, fields, .. } = &item.value {
                    let mut field_types = Vec::new();
                    for p in fields {
                        let ft = self.type_to_llvm(&p.type_expr.value);
                        field_types.push((p.name.clone(), ft));
                    }
                    self.object_defs.insert(name.clone(), field_types);
                    self.object_modules.insert(name.clone(), module.name.clone());
                    for m in methods {
                        if let ItemKind::Fn { name: mname, ret_type, .. } = m {
                            let ret = ret_type.as_ref().map(|t| self.type_to_llvm(&t.value)).unwrap_or_else(|| "i64".into());
                            self.object_method_ret_types.insert((name.clone(), mname.clone()), ret);
                        }
                    }
                }
            }
        }

        // First pass: collect class definitions and vtables
        for module in modules {
            for item in &module.items {
                if let ItemKind::Class { name, fields, constructor, methods: _, extends, .. } = &item.value {
                    // Collect field types (including constructor params)
                    let mut class_field_types = Vec::new();
                    for p in constructor {
                        let ft = self.type_to_llvm(&p.type_expr.value);
                        class_field_types.push((p.name.clone(), ft));
                    }
                    for p in fields {
                        let ft = self.type_to_llvm(&p.type_expr.value);
                        class_field_types.push((p.name.clone(), ft));
                    }
                    self.class_defs.insert(name.clone(), class_field_types);
                    self.class_modules.insert(name.clone(), module.name.clone());
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

        // Emit interface types (%iface.Name = type { ptr, ptr })
        let iface_names: Vec<String> = self.interface_methods.keys().cloned().collect();
        for iface_name in &iface_names {
            self.e_raw(&format!("%iface.{} = type {{ ptr, ptr }}", iface_name));
        }
        if !iface_names.is_empty() {
            self.e_raw("");
        }

        // Build and emit interface vtables for each (class, interface) pair
        let iface_methods_snapshot: Vec<(String, Vec<String>)> = iface_names.iter()
            .filter_map(|name| Some((name.clone(), self.interface_methods.get(name.as_str())?.clone())))
            .collect();
        let class_impls_snapshot: Vec<(String, Vec<String>)> = self.class_impls.iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let class_vtables_snapshot: Vec<(String, Vec<(String, String)>)> = self.class_vtables.iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (iface_name, iface_methods) in &iface_methods_snapshot {
            if let Some(impl_classes) = class_impls_snapshot.iter().find(|(n, _)| n == iface_name).map(|(_, v)| v) {
                for class_name in impl_classes {
                    if let Some(vtable_methods) = class_vtables_snapshot.iter().find(|(n, _)| n == class_name).map(|(_, v)| v) {
                        let matching: Vec<String> = iface_methods.iter()
                            .filter_map(|mname| vtable_methods.iter().find(|(vm, _)| vm == mname).map(|(_, mfn)| mfn.clone()))
                            .collect();
                        if !matching.is_empty() {
                            let iface_vtable_ty = format!("%{}.{}.iface_vtable", iface_name, class_name);
                            let ptrs: String = (0..matching.len()).map(|_| "ptr").collect::<Vec<_>>().join(", ");
                            self.e_raw(&format!("{} = type {{ {} }}", iface_vtable_ty, ptrs));
                            let vals: String = matching.iter().map(|mfn| format!("ptr @{}", mfn)).collect::<Vec<_>>().join(", ");
                            self.e_raw(&format!("@iface_vtable.{}.{} = global {} {{ {} }}", iface_name, class_name, iface_vtable_ty, vals));
                        }
                    }
                }
            }
        }
        if !iface_names.is_empty() {
            self.e_raw("");
        }

        // Emit object types and globals
        let object_names: Vec<String> = self.object_defs.keys().cloned().collect();
        for obj_name in &object_names {
            if let Some(fields) = self.object_defs.get(obj_name.as_str()) {
                let field_llvm: Vec<String> = fields.iter().map(|(_, ft)| ft.clone()).collect();
                let body = if field_llvm.is_empty() { "i8".to_string() } else { field_llvm.join(", ") };
                self.e_raw(&format!("%object.{} = type {{ {} }}", obj_name, body));
            }
        }
        for obj_name in &object_names {
            self.e_raw(&format!("@object.{} = global %object.{} zeroinitializer", obj_name, obj_name));
        }
        if !object_names.is_empty() {
            self.e_raw("");
        }

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
        self.e_raw("declare i64 @yk_tcp_connect(ptr)");
        self.e_raw("declare i64 @yk_tcp_send(i64, ptr)");
        self.e_raw("declare ptr @yk_tcp_recv(i64, i64)");
        self.e_raw("declare void @yk_tcp_close(i64)");
        self.e_raw("declare ptr @yk_dns_lookup(ptr)");
        self.e_raw("declare i64 @yk_tcp_listen(ptr)");
        self.e_raw("declare i64 @yk_tcp_accept(i64)");
        self.e_raw("declare i64 @yk_http_new()");
        self.e_raw("declare void @yk_http_request(i64, ptr, ptr, ptr)");
        self.e_raw("declare i32 @yk_http_status(i64)");
        self.e_raw("declare ptr @yk_http_body(i64)");
        self.e_raw("declare void @yk_http_free(i64)");
        self.e_raw("declare i64 @yk_udp_bind(ptr)");
        self.e_raw("declare i64 @yk_udp_send_to(i64, ptr, ptr)");
        self.e_raw("declare ptr @yk_udp_recv_from(i64, i64)");
        self.e_raw("declare void @yk_server_serve(i64, ptr)");
        self.e_raw("declare ptr @yk_list_new()");
        self.e_raw("declare void @yk_list_push(ptr, i64)");
        self.e_raw("declare i64 @yk_list_get(ptr, i64)");
        self.e_raw("declare i64 @yk_list_len(ptr)");
        self.e_raw("declare i64 @yk_list_pop(ptr)");
        self.e_raw("declare void @yk_list_print(ptr)");
        self.e_raw("declare ptr @yk_list_to_string(ptr)");
        self.e_raw("declare void @yk_list_sort(ptr)");
        self.e_raw("declare void @yk_list_reverse(ptr)");
        self.e_raw("declare void @yk_list_insert(ptr, i64, i64)");
        self.e_raw("declare void @yk_list_remove(ptr, i64)");
        self.e_raw("declare void @yk_list_clear(ptr)");
        self.e_raw("declare void @yk_print_result_val(i64, i1)");
        self.e_raw("declare i64 @yk_result_str_new(i64, i64)");
        self.e_raw("declare i64 @yk_pow_int(i64, i64)");
        self.e_raw("declare double @yk_pow_real(double, double)");
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
                if let ItemKind::Fn { name, ret_type, params, .. } = &item.value {
                    let mangled = self.mangle_name(name);
                    let ret = ret_type.as_ref().map(|t| self.type_to_llvm(&t.value)).unwrap_or_else(|| "void".into());
                    self.current_fn_ret = ret.clone();
                    self.fn_ret_types.insert(mangled.clone(), ret);
                    let param_types: Vec<String> = params.iter().map(|p| self.type_to_llvm(&p.type_expr.value)).collect();
                    self.fn_param_types.insert(mangled.clone(), param_types);
                    let union_variants: Vec<Option<Vec<TypeExpr>>> = params.iter().map(|p| {
                        if let TypeExpr::Union(ts) = &p.type_expr.value {
                            Some(ts.clone())
                        } else {
                            None
                        }
                    }).collect();
                    self.fn_param_union_variants.insert(mangled, union_variants);
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
            // Compile class init functions
            for item in &module.items {
                if let ItemKind::Class { name, init_body, .. } = &item.value {
                    self.compile_class_init(name, init_body, &module.name);
                }
                if let ItemKind::Object { name, .. } = &item.value {
                    self.compile_object_init(name, &module.name);
                }
            }
            // Compile object methods
            for item in &module.items {
                if let ItemKind::Object { name, methods, .. } = &item.value {
                    for m in methods {
                        self.compile_object_method(name, m, &module.name);
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
                // Call object init functions
                let object_names: Vec<String> = self.object_defs.keys().cloned().collect();
                for obj_name in &object_names {
                    let module_name = self.object_modules.get(obj_name.as_str()).cloned().unwrap_or_default();
                    let mangled_init = if module_name.is_empty() {
                        format!("__obj_init_{}", obj_name)
                    } else {
                        format!("__obj_init_{}_{}", module_name, obj_name)
                    };
                    self.e(&format!("call void @{}()", mangled_init));
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
            self.fn_param_types.insert(mangled.clone(), param_types.clone());
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

    fn compile_class_init(&mut self, class_name: &str, init_body: &[StmtNode], module_name: &str) {
        let mangled = if module_name.is_empty() {
            format!("__class_init_{}", class_name)
        } else {
            format!("__class_init_{}_{}", module_name, class_name)
        };
        let class_ty = format!("%class.{}", class_name);
        let saved_types = self.var_types.clone();
        let saved_alloca = self.var_alloca.clone();
        let saved_fn_ret = self.current_fn_ret.clone();
        self.current_fn_ret = "void".into();
        self.e_raw(&format!("define void @{}(i64 %self_int) {{", mangled));
        self.indent += 1;
        let uptr = self.fresh_label();
        self.e(&format!("%{} = inttoptr i64 %self_int to ptr", uptr));
        let loaded = self.fresh_label();
        self.e(&format!("%{} = load {}, ptr %{}", loaded, class_ty, uptr));
        let ptr = self.alloca_name("self");
        self.var_types.insert("self".to_string(), class_ty.clone());
        self.var_alloca.insert("self".to_string(), ptr.clone());
        self.e(&format!("{} = alloca {}, align 8", ptr, class_ty));
        self.e(&format!("store {} %{}, ptr {}", class_ty, loaded, ptr));
        self.compile_fn_body(init_body);
        self.e("ret void");
        self.indent -= 1;
        self.e_raw("}");
        self.e_raw("");
        self.var_types = saved_types;
        self.var_alloca = saved_alloca;
        self.current_fn_ret = saved_fn_ret;
    }

    fn compile_object_method(&mut self, obj_name: &str, method: &ItemKind, module_name: &str) {
        if let ItemKind::Fn { name: mname, params, ret_type, body, .. } = method {
            let mangled = if module_name.is_empty() {
                format!("__obj_method_{}_{}", obj_name, mname)
            } else {
                format!("__obj_method_{}_{}_{}", module_name, obj_name, mname)
            };
            let ret = ret_type.as_ref().map(|t| self.type_to_llvm(&t.value)).unwrap_or_else(|| "void".into());
            self.object_method_ret_types.insert((obj_name.to_string(), mname.clone()), ret.clone());
            let has_self = params.first().map(|p| p.name == "self").unwrap_or(false);
            if has_self {
                self.object_method_has_self.insert((obj_name.to_string(), mname.clone()));
            }
            let saved_types = self.var_types.clone();
            let saved_alloca = self.var_alloca.clone();

            let has_self = params.first().map(|p| p.name == "self").unwrap_or(false);
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
                    let obj_ty = format!("%object.{}", obj_name);
                    let uptr = self.fresh_label();
                    self.e(&format!("%{} = inttoptr i64 %0 to ptr", uptr));
                    let loaded = self.fresh_label();
                    self.e(&format!("%{} = load {}, ptr %{}", loaded, obj_ty, uptr));
                    let ptr = self.alloca_name(&p.name);
                    self.var_types.insert(p.name.clone(), obj_ty.clone());
                    self.var_alloca.insert(p.name.clone(), ptr.clone());
                    self.e(&format!("{} = alloca {}, align 8", ptr, obj_ty));
                    self.e(&format!("store {} %{}, ptr {}", obj_ty, loaded, ptr));
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

    fn compile_object_init(&mut self, obj_name: &str, module_name: &str) {
        let mangled = if module_name.is_empty() {
            format!("__obj_init_{}", obj_name)
        } else {
            format!("__obj_init_{}_{}", module_name, obj_name)
        };
        let obj_ty = format!("%object.{}", obj_name);
        self.e_raw(&format!("define void @{}() {{", mangled));
        self.indent += 1;
        let fields = self.object_defs.get(obj_name).cloned().unwrap_or_default();
        if !fields.is_empty() {
            let global_name = format!("@object.{}", obj_name);
            for (_fname, fty) in &fields {
                let zero = match fty.as_str() {
                    "i64" => "0".to_string(),
                    "double" => "0.0".to_string(),
                    "i1" => "false".to_string(),
                    "%yk_string" => format!("{} zeroinitializer", fty),
                    t if t.starts_with('%') => format!("{} zeroinitializer", t),
                    _ => "0".to_string(),
                };
                let ptr = self.fresh_label();
                self.e(&format!("%{} = getelementptr inbounds {}, ptr {}, i32 0, i32 {}", ptr, obj_ty, global_name, 0));
                self.e(&format!("store {} {}, ptr %{}", fty, zero, ptr));
            }
        }
        self.e("ret void");
        self.indent -= 1;
        self.e_raw("}");
        self.e_raw("");
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

                // Store union variant definitions for later use in Assign
                if effective_ty == "%yk_variant" {
                    if let Some(te) = type_expr {
                        if let TypeExpr::Union(variants) = &te.value {
                            self.var_union_variants.insert(name.clone(), variants.clone());
                        }
                    }
                }

                if Self::is_nullable_ty(&effective_ty) && !Self::is_nullable_ty(&val_ty) {
                    let is_null = matches!(&value.value, Expr::LitNull | Expr::LitNone);
                    let wrapped = self.wrap_in_nullable(&effective_ty, &val, &val_ty, is_null);
                    self.e(&format!("store {} {}, ptr {}", effective_ty, wrapped, ptr));
                } else if Self::is_union_ty(&effective_ty) && !Self::is_union_ty(&val_ty) {
                    if let Some(te) = type_expr {
                        if let TypeExpr::Union(variants) = &te.value {
                            for variant in variants {
                                let v_llvm = self.type_to_llvm(variant);
                                if v_llvm == val_ty {
                                    let wrapped = self.wrap_in_variant(variant, &val, &val_ty);
                                    self.e(&format!("store {} {}, ptr {}", effective_ty, wrapped, ptr));
                                    break;
                                }
                            }
                        }
                    }
                } else if ty.starts_with("%iface.") && val_ty.starts_with("%class.") {
                    let iface_name = ty.strip_prefix("%iface.").unwrap();
                    let class_name = val_ty.strip_prefix("%class.").unwrap();
                    let obj_alloca = self.fresh_label();
                    self.e(&format!("%{} = alloca {}, align 8", obj_alloca, val_ty));
                    self.e(&format!("store {} {}, ptr %{}", val_ty, val, obj_alloca));
                    let obj_ptr = self.fresh_label();
                    self.e(&format!("%{} = ptrtoint ptr %{} to i64", obj_ptr, obj_alloca));
                    let obj_data_ptr = self.fresh_label();
                    self.e(&format!("%{} = inttoptr i64 %{} to ptr", obj_data_ptr, obj_ptr));
                    let vtable_global = format!("@iface_vtable.{}.{}", iface_name, class_name);
                    let vtable_int = self.fresh_label();
                    self.e(&format!("%{} = ptrtoint ptr {} to i64", vtable_int, vtable_global));
                    let vtable_ptr = self.fresh_label();
                    self.e(&format!("%{} = inttoptr i64 %{} to ptr", vtable_ptr, vtable_int));
                    let iface_val = self.fresh_label();
                    self.e(&format!("%{} = insertvalue {} undef, ptr %{}, 0", iface_val, ty, obj_data_ptr));
                    let iface_val2 = self.fresh_label();
                    self.e(&format!("%{} = insertvalue {} %{}, ptr %{}, 1", iface_val2, ty, iface_val, vtable_ptr));
                    self.e(&format!("store {} %{}, ptr {}", ty, iface_val2, ptr));
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
                let (val, val_ty) = self.compile_expr(expr);
                let mut is_http_ctor = false;
                if let Expr::Call(callee, _) = &expr.value {
                    if let Expr::Ident(callee_name) = &callee.value {
                        if callee_name == "HTTP" { is_http_ctor = true; }
                    }
                }
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
                if is_http_ctor { self.http_vars.insert(name.clone()); }
                let ty = self.val_ty(name);
                if Self::is_nullable_ty(&ty) && !Self::is_nullable_ty(&val_ty) {
                    let is_null = matches!(&expr.value, Expr::LitNull | Expr::LitNone);
                    let wrapped = self.wrap_in_nullable(&ty, &val, &val_ty, is_null);
                    self.e(&format!("store {} {}, ptr {}", ty, wrapped, ptr));
                } else if Self::is_union_ty(&ty) && !Self::is_union_ty(&val_ty) {
                    let variants = self.var_union_variants.get(name).cloned();
                    if let Some(variants) = variants {
                        for variant in &variants {
                            let v_llvm = self.type_to_llvm(variant);
                            if v_llvm == val_ty {
                                let wrapped = self.wrap_in_variant(variant, &val, &val_ty);
                                self.e(&format!("store {} {}, ptr {}", ty, wrapped, ptr));
                                break;
                            }
                        }
                    } else {
                        self.e(&format!("store {} {}, ptr {}", val_ty, val, ptr));
                    }
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
                        let fn_ret = self.current_fn_ret.clone();
                        if Self::is_nullable_ty(&fn_ret) && !Self::is_nullable_ty(&val_ty) {
                            let is_null = matches!(&ex.value, Expr::LitNull | Expr::LitNone);
                            let wrapped = self.wrap_in_nullable(&fn_ret, &val, &val_ty, is_null);
                            self.e(&format!("ret {} {}", fn_ret, wrapped));
                        } else if fn_ret == "i64" && val_ty == "ptr" {
                            let c = self.fresh_label();
                            self.e(&format!("%{c} = ptrtoint ptr {val} to i64"));
                            self.e(&format!("ret i64 %{c}"));
                        } else if fn_ret == "ptr" && val_ty == "i64" {
                            let c = self.fresh_label();
                            self.e(&format!("%{c} = inttoptr i64 {val} to ptr"));
                            self.e(&format!("ret ptr %{c}"));
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
                    self.e(&format!("%cmp_{} = icmp sle i64 {}, {}", var, v, end_val));
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
                if self.object_defs.contains_key(name.as_str()) {
                    let global_name = format!("@object.{}", name);
                    let obj_ty = format!("%object.{}", name);
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = load {}, ptr {}", tmp, obj_ty, global_name));
                    (self.ssa(&tmp), obj_ty)
                } else {
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
                    UnOp::BitNot => {
                        self.e(&format!("%{} = xor {} {}, -1", tmp, ty, i));
                        (self.ssa(&tmp), ty)
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
                    let (index, field_ty): (Option<usize>, String) = if let Some(sig) = self.tuple_type_names.iter().find(|(_, n)| *n == &obj_ty).map(|(k, _)| k) {
                        let idx: usize = field.parse().unwrap_or(0);
                        let elem_tys: Vec<&str> = sig.split("||").collect();
                        let ft = elem_tys.get(idx).copied().unwrap_or("i64").to_string();
                        (Some(idx), ft)
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
                } else if let Some(obj_name) = obj_ty.strip_prefix("%object.") {
                    let (index, field_ty) = self.object_defs.get(obj_name).and_then(|def_fields| {
                        def_fields.iter().enumerate().find(|(_, (n, _))| n == field)
                            .map(|(i, (_, ft))| (Some(i), ft.clone()))
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
                } else if obj_ty == "i64" && matches!(field.as_str(), "status") && self.is_http_var(obj) {
                    let tmp2 = self.fresh_label();
                    self.e(&format!("%{} = call i32 @yk_http_status(i64 {})", tmp2, o));
                    let extended = self.fresh_label();
                    self.e(&format!("%{} = sext i32 %{} to i64", extended, tmp2));
                    (self.ssa(&extended), "i64".into())
                } else if obj_ty == "i64" && matches!(field.as_str(), "body") && self.is_http_var(obj) {
                    let ptr_tmp = self.fresh_label();
                    self.e(&format!("%{} = call ptr @yk_http_body(i64 {})", ptr_tmp, o));
                    let loaded = self.fresh_label();
                    self.e(&format!("%{} = load %yk_string, ptr %{}", loaded, ptr_tmp));
                    (self.ssa(&loaded), "%yk_string".into())
                } else if obj_ty == "i64" && matches!(field.as_str(), "method") && self.is_http_var(obj) {
                    let slot = self.make_string_slot("GET");
                    let ptr = self.string_to_ptr(&slot);
                    let loaded = self.fresh_label();
                    self.e(&format!("%{} = load %yk_string, ptr {}", loaded, ptr));
                    (self.ssa(&loaded), "%yk_string".into())
                } else if obj_ty == "i64" && matches!(field.as_str(), "method" | "path" | "body") {
                    let req_ptr = self.fresh_label();
                    self.e(&format!("%{} = inttoptr i64 {} to ptr", req_ptr, o));
                    let (field_offset_ptr, field_offset_len) = match field.as_str() {
                        "method" => (0i32, 8i32),
                        "path" => (16, 24),
                        "body" => (32, 40),
                        _ => (32, 40),
                    };
                    let ptr_gep = self.fresh_label();
                    self.e(&format!("%{} = getelementptr i8, ptr %{}, i32 {}", ptr_gep, req_ptr, field_offset_ptr));
                    let ptr_v = self.fresh_label();
                    self.e(&format!("%{} = load ptr, ptr %{}", ptr_v, ptr_gep));
                    let len_gep = self.fresh_label();
                    self.e(&format!("%{} = getelementptr i8, ptr %{}, i32 {}", len_gep, req_ptr, field_offset_len));
                    let len_v = self.fresh_label();
                    self.e(&format!("%{} = load i64, ptr %{}", len_v, len_gep));
                    let sv1 = self.fresh_label();
                    self.e(&format!("%{} = insertvalue %yk_string undef, ptr %{}, 0", sv1, ptr_v));
                    let sv2 = self.fresh_label();
                    self.e(&format!("%{} = insertvalue %yk_string %{}, i64 %{}, 1", sv2, sv1, len_v));
                    (self.ssa(&sv2), "%yk_string".into())
                } else if obj_ty == "ptr" && field == "length" {
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = call i64 @yk_list_len(ptr {})", tmp, o));
                    (self.ssa(&tmp), "i64".into())
                } else if obj_ty == "%yk_string" && field == "length" {
                    let p = self.string_to_ptr(&o);
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = call i64 @yk_string_len_ptr(ptr {})", tmp, p));
                    (self.ssa(&tmp), "i64".into())
                } else {
                    self.e(&format!("%{} = add i64 0, 0", tmp));
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
                let (payload_val, payload_ty) = if args.len() == 1 {
                    self.compile_expr(&args[0])
                } else {
                    for arg in args { self.compile_expr(arg); }
                    ("0".into(), "i64".into())
                };
                let payload = if payload_ty == "%yk_string" || payload_ty == "ptr" {
                    let a = self.fresh_label();
                    self.e(&format!("%{} = alloca {}, align 8", a, payload_ty));
                    self.e(&format!("store {} {}, ptr %{}", payload_ty, payload_val, a));
                    let p = self.fresh_label();
                    self.e(&format!("%{} = ptrtoint ptr %{} to i64", p, a));
                    self.ssa(&p)
                } else if payload_ty == "i1" {
                    let p = self.fresh_label();
                    self.e(&format!("%{} = zext i1 {} to i64", p, payload_val));
                    self.ssa(&p)
                } else {
                    payload_val
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
        // Coerce int → double when mixed with float
        if lt == "double" && !rt.starts_with('%') && rt != "double" && rt != "float" {
            let ext = self.fresh_label();
            self.e(&format!("%{} = sitofp {} {} to double", ext, rt, rc));
            rc = self.ssa(&ext);
            rt = "double".to_string();
        } else if rt == "double" && !lt.starts_with('%') && lt != "double" && lt != "float" {
            let ext = self.fresh_label();
            self.e(&format!("%{} = sitofp {} {} to double", ext, lt, lc));
            lc = self.ssa(&ext);
            lt = "double".to_string();
        } else if lt == "float" && !rt.starts_with('%') && rt != "double" && rt != "float" {
            let ext = self.fresh_label();
            self.e(&format!("%{} = sitofp {} {} to float", ext, rt, rc));
            rc = self.ssa(&ext);
            rt = "float".to_string();
        } else if rt == "float" && !lt.starts_with('%') && lt != "double" && lt != "float" {
            let ext = self.fresh_label();
            self.e(&format!("%{} = sitofp {} {} to float", ext, lt, lc));
            lc = self.ssa(&ext);
            lt = "float".to_string();
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
            BinOp::Mod => {
                if is_float {
                    self.e(&format!("%{} = frem {} {}, {}", tmp, lt, lc, rc));
                } else {
                    self.e(&format!("%{} = srem {} {}, {}", tmp, lt, lc, rc));
                }
                (self.ssa(&tmp), lt)
            }
            BinOp::Pow => {
                if is_float {
                    self.e(&format!("%{} = call double @yk_pow_real(double {}, double {})", tmp, lc, rc));
                } else {
                    self.e(&format!("%{} = call i64 @yk_pow_int(i64 {}, i64 {})", tmp, lc, rc));
                }
                (self.ssa(&tmp), lt)
            }
            BinOp::BitAnd => {
                self.e(&format!("%{} = and {} {}, {}", tmp, lt, lc, rc));
                (self.ssa(&tmp), lt)
            }
            BinOp::BitOr => {
                self.e(&format!("%{} = or {} {}, {}", tmp, lt, lc, rc));
                (self.ssa(&tmp), lt)
            }
            BinOp::BitXor => {
                self.e(&format!("%{} = xor {} {}, {}", tmp, lt, lc, rc));
                (self.ssa(&tmp), lt)
            }
            BinOp::Shl => {
                self.e(&format!("%{} = shl {} {}, {}", tmp, lt, lc, rc));
                (self.ssa(&tmp), lt)
            }
            BinOp::Shr => {
                self.e(&format!("%{} = ashr {} {}, {}", tmp, lt, lc, rc));
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
                let ptr = self.find_alloca_for_expr(l);
                self.e(&format!("store {} {}, ptr {}", lt, rc, ptr));
                (rc, lt)
            }
        }
    }

    fn compile_call(&mut self, callee: &ExprNode, args: &[ExprNode]) -> (String, String) {
        // Don't compile FnLit/Closure args as standalone i64 functions when calling server methods
        let is_srv = matches!(&callee.value, Expr::Field(obj, field) if matches!(obj.value, Expr::Ident(_)) &&
            matches!(field.as_str(), "get" | "post" | "put" | "delete" | "patch" | "ws" | "serve"));
        let arg_results: Vec<(String, String)> = args.iter().map(|a| {
            if is_srv && matches!(&a.value, Expr::FnLit(..) | Expr::Closure(..)) {
                ("0".into(), "i64".into())
            } else {
                self.compile_expr(a)
            }
        }).collect();

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
                                "%yk_variant" => {
                                    let tag = self.fresh_label();
                                    let payload = self.fresh_label();
                                    self.e(&format!("%{} = extractvalue %yk_variant {}, 0", tag, av));
                                    self.e(&format!("%{} = extractvalue %yk_variant {}, 1", payload, av));
                                    // Match int(0) and str variants by tag
                                    let int_name = "int(0)";
                                    let str_name = "str";
                                    let int_tag = self.variant_name_tag(int_name);
                                    let str_tag = self.variant_name_tag(str_name);
                                    if int_tag >= 0 && str_tag >= 0 {
                                        let is_int = self.fresh_label();
                                        self.e(&format!("%{} = icmp eq i64 %{}, {}", is_int, tag, int_tag));
                                        let int_bb = self.fresh_label();
                                        let str_bb = self.fresh_label();
                                        let done_bb = self.fresh_label();
                                        self.e(&format!("br i1 %{}, label %{}, label %{}", is_int, int_bb, str_bb));
                                        self.e(&format!("{}:", int_bb));
                                        self.e(&format!("call void @yk_print_int(i64 %{})", payload));
                                        self.e(&format!("br label %{}", done_bb));
                                        self.e(&format!("{}:", str_bb));
                                        let str_ptr = self.fresh_label();
                                        self.e(&format!("%{} = inttoptr i64 %{} to ptr", str_ptr, payload));
                                        self.e(&format!("call void @yk_print_str_ptr(ptr %{})", str_ptr));
                                        self.e(&format!("br label %{}", done_bb));
                                        self.e(&format!("{}:", done_bb));
                                    } else if int_tag >= 0 {
                                        self.e(&format!("call void @yk_print_int(i64 %{})", payload));
                                    } else if str_tag >= 0 {
                                        let str_ptr = self.fresh_label();
                                        self.e(&format!("%{} = inttoptr i64 %{} to ptr", str_ptr, payload));
                                        self.e(&format!("call void @yk_print_str_ptr(ptr %{})", str_ptr));
                                    } else {
                                        self.e(&format!("call void @yk_print_int(i64 %{})", payload));
                                    }
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
                                "%yk_variant" => {
                                    let tag = self.fresh_label();
                                    let payload = self.fresh_label();
                                    self.e(&format!("%{} = extractvalue %yk_variant {}, 0", tag, av));
                                    self.e(&format!("%{} = extractvalue %yk_variant {}, 1", payload, av));
                                    let int_name = "int(0)";
                                    let str_name = "str";
                                    let int_tag = self.variant_name_tag(int_name);
                                    let str_tag = self.variant_name_tag(str_name);
                                    if int_tag >= 0 && str_tag >= 0 {
                                        let is_int = self.fresh_label();
                                        self.e(&format!("%{} = icmp eq i64 %{}, {}", is_int, tag, int_tag));
                                        let int_bb = self.fresh_label();
                                        let str_bb = self.fresh_label();
                                        let done_bb = self.fresh_label();
                                        let result = self.fresh_label();
                                        self.e(&format!("br i1 %{}, label %{}, label %{}", is_int, int_bb, str_bb));
                                        self.e(&format!("{}:", int_bb));
                                        let t1 = self.fresh_label();
                                        self.e(&format!("%{} = call ptr @yk_string_from_int(i64 %{})", t1, payload));
                                        self.e(&format!("br label %{}", done_bb));
                                        self.e(&format!("{}:", str_bb));
                                        let str_ptr = self.fresh_label();
                                        self.e(&format!("%{} = inttoptr i64 %{} to ptr", str_ptr, payload));
                                        self.e(&format!("br label %{}", done_bb));
                                        self.e(&format!("{}:", done_bb));
                                        self.e(&format!("%{} = phi ptr [ %{}, %{} ], [ %{}, %{} ]", result, t1, int_bb, str_ptr, str_bb));
                                        self.ssa(&result)
                                    } else {
                                        let t = self.fresh_label();
                                        self.e(&format!("%{} = call ptr @yk_string_from_int(i64 %{})", t, payload));
                                        self.ssa(&t)
                                    }
                                }
                                "ptr" => {
                                    let t = self.fresh_label();
                                    self.e(&format!("%{} = call ptr @yk_list_to_string(ptr {})", t, av));
                                    self.ssa(&t)
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
                "HTTP" => {
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = call i64 @yk_http_new()", tmp));
                    (self.ssa(&tmp), "i64".into())
                }
                "lookup" => {
                    let host_ptr = arg_results.first().map(|(av, at)| {
                        if at == "%yk_string" { self.string_to_ptr(av) } else { "null".into() }
                    }).unwrap_or_else(|| "null".into());
                    let ptr_tmp = self.fresh_label();
                    self.e(&format!("%{} = call ptr @yk_dns_lookup(ptr {})", ptr_tmp, host_ptr));
                    let loaded = self.fresh_label();
                    self.e(&format!("%{} = load %yk_string, ptr %{}", loaded, ptr_tmp));
                    (self.ssa(&loaded), "%yk_string".into())
                }
                "Server" => {
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = call i64 @yk_server_new()", tmp));
                    (self.ssa(&tmp), "i64".into())
                }
                "TcpStream" => {
                    let addr_ptr = arg_results.first().map(|(av, at)| {
                        if at == "%yk_string" { self.string_to_ptr(av) } else { "null".into() }
                    }).unwrap_or_else(|| "null".into());
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = call i64 @yk_tcp_connect(ptr {})", tmp, addr_ptr));
                    (self.ssa(&tmp), "i64".into())
                }
                "UdpSocket" => {
                    let addr_ptr = arg_results.first().map(|(av, at)| {
                        if at == "%yk_string" { self.string_to_ptr(av) } else { "null".into() }
                    }).unwrap_or_else(|| "null".into());
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = call i64 @yk_udp_bind(ptr {})", tmp, addr_ptr));
                    (self.ssa(&tmp), "i64".into())
                }
                "TcpListener" => {
                    let addr_ptr = arg_results.first().map(|(av, at)| {
                        if at == "%yk_string" { self.string_to_ptr(av) } else { "null".into() }
                    }).unwrap_or_else(|| "null".into());
                    let tmp = self.fresh_label();
                    self.e(&format!("%{} = call i64 @yk_tcp_listen(ptr {})", tmp, addr_ptr));
                    (self.ssa(&tmp), "i64".into())
                }
                _ => {
                    // Check if this is a class constructor (Foo(args) → create Foo instance)
                    if self.class_defs.contains_key(name) {
                        let obj_ty = format!("%class.{}", name);
                        let alloca_ptr = self.fresh_label();
                        self.e(&format!("%{} = alloca {}, align 8", alloca_ptr, obj_ty));
                        // Zero-init the struct
                        self.e(&format!("store {} zeroinitializer, ptr %{}", obj_ty, alloca_ptr));
                        // Store vtable pointer (index 0)
                        let vtable_name = format!("@vtable.{}", name);
                        let vptr = self.fresh_label();
                        self.e(&format!("%{} = getelementptr inbounds {}, ptr %{}, i32 0, i32 0", vptr, obj_ty, alloca_ptr));
                        let virt = self.fresh_label();
                        self.e(&format!("%{} = ptrtoint ptr {} to i64", virt, vtable_name));
                        self.e(&format!("store i64 %{}, ptr %{}", virt, vptr));
                        // Store constructor params as fields
                        let def_fields = self.class_defs.get(name).cloned().unwrap_or_default();
                        for (i, (_fname, fty)) in def_fields.iter().enumerate() {
                            if let Some((val, _)) = arg_results.get(i) {
                                let fptr = self.fresh_label();
                                self.e(&format!("%{} = getelementptr inbounds {}, ptr %{}, i32 0, i32 {}", fptr, obj_ty, alloca_ptr, i + 1));
                                self.e(&format!("store {} {}, ptr %{}", fty, val, fptr));
                            }
                        }
                        let loaded = self.fresh_label();
                        self.e(&format!("%{} = load {}, ptr %{}", loaded, obj_ty, alloca_ptr));
                        // Call init function
                        let class_module = self.class_modules.get(name).cloned().unwrap_or_default();
                        let mangled_init = if class_module.is_empty() {
                            format!("__class_init_{}", name)
                        } else {
                            format!("__class_init_{}_{}", class_module, name)
                        };
                        let init_ptr = self.fresh_label();
                        self.e(&format!("%{} = ptrtoint ptr %{} to i64", init_ptr, alloca_ptr));
                        self.e(&format!("call void @{}(i64 %{})", mangled_init, init_ptr));
                        (self.ssa(&loaded), obj_ty)
                    } else {
                        let tmp = self.fresh_label();
                        let mangled = self.mangle_name(name);
                        let fn_ret = self.fn_ret_types.get(&mangled).cloned().unwrap_or_else(|| "i64".into());
                        // Wrap class args into interface fat pointers if needed
                        let param_types_opt = self.fn_param_types.get(&mangled).cloned();
                        let wrapped_args: Vec<String> = if let Some(param_types) = param_types_opt {
                            let mut result = Vec::new();
                            for (i, (v, t)) in arg_results.iter().enumerate() {
                                if let Some(pt) = param_types.get(i) {
                                    if pt.starts_with("%iface.") && t.starts_with("%class.") {
                                        let class_name = t.strip_prefix("%class.").unwrap();
                                        let iface_name = pt.strip_prefix("%iface.").unwrap();
                                        let alloca_p = self.fresh_label();
                                        self.e(&format!("%{} = alloca {}, align 8", alloca_p, t));
                                        self.e(&format!("store {} {}, ptr %{}", t, v, alloca_p));
                                        let obj_ptr = self.fresh_label();
                                        self.e(&format!("%{} = ptrtoint ptr %{} to i64", obj_ptr, alloca_p));
                                        let obj_data_ptr = self.fresh_label();
                                        self.e(&format!("%{} = inttoptr i64 %{} to ptr", obj_data_ptr, obj_ptr));
                                        let vtable_global = format!("@iface_vtable.{}.{}", iface_name, class_name);
                                        let vtable_int = self.fresh_label();
                                        self.e(&format!("%{} = ptrtoint ptr {} to i64", vtable_int, vtable_global));
                                        let vtable_ptr = self.fresh_label();
                                        self.e(&format!("%{} = inttoptr i64 %{} to ptr", vtable_ptr, vtable_int));
                                        let iface_val = self.fresh_label();
                                        self.e(&format!("%{} = insertvalue {} undef, ptr %{}, 0", iface_val, pt, obj_data_ptr));
                                        let iface_val2 = self.fresh_label();
                                        self.e(&format!("%{} = insertvalue {} %{}, ptr %{}, 1", iface_val2, pt, iface_val, vtable_ptr));
                                        result.push(format!("{} {}", pt, self.ssa(&iface_val2)));
                                    } else if pt == "%yk_variant" && t != pt {
                                        let variants_opt = self.fn_param_union_variants.get(&mangled).and_then(|uvs| uvs.get(i));
                                        let union_variants: Vec<TypeExpr> = variants_opt.and_then(|o| o.clone()).unwrap_or_default();
                                        let mut wrapped = false;
                                        for variant_te in &union_variants {
                                            let v_llvm = self.type_to_llvm(variant_te);
                                            if v_llvm == *t {
                                                let vv = self.wrap_in_variant(variant_te, v, t);
                                                result.push(format!("{} {}", pt, vv));
                                                wrapped = true;
                                                break;
                                            }
                                        }
                                        if !wrapped {
                                            result.push(format!("{} {}", t, v));
                                        }
                                    } else {
                                        result.push(format!("{} {}", t, v));
                                    }
                                } else {
                                    result.push(format!("{} {}", t, v));
                                }
                            }
                            result
                        } else {
                            arg_results.iter().map(|(v, t)| format!("{} {}", t, v)).collect()
                        };
                        if fn_ret == "void" {
                            self.e(&format!("call void @{}({})", mangled, wrapped_args.join(", ")));
                            ("0".into(), "void".into())
                        } else {
                            self.e(&format!("%{} = call {} @{}({})", tmp, fn_ret, mangled, wrapped_args.join(", ")));
                            (self.ssa(&tmp), fn_ret)
                        }
                    }
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
                    (Expr::Ident(mod_name), func_name) if mod_name == "re" || mod_name == "regex" => {
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
                        // HTTP method dispatch (BEFORE Server to avoid hijacking HTTP requests)
                        if o_ty == "i64" && self.is_http_var(obj) && matches!(field.as_str(), "get" | "post" | "put" | "delete" | "head" | "options" | "patch") {
                            let method_str = field.as_str().to_uppercase();
                            let method_slot = self.make_string_slot(&method_str);
                            let method_ptr = self.string_to_ptr(&method_slot);
                            let url_ptr = arg_results.first().map(|(av, at)| {
                                if at == "%yk_string" { self.string_to_ptr(av) } else { "null".into() }
                            }).unwrap_or_else(|| "null".into());
                            let body_ptr = if matches!(field.as_str(), "post" | "put" | "patch") {
                                arg_results.get(1).map(|(av, at)| {
                                    if at == "%yk_string" { self.string_to_ptr(av) } else { "null".into() }
                                }).unwrap_or_else(|| "null".into())
                            } else { "null".into() };
                            self.e(&format!("call void @yk_http_request(i64 {}, ptr {}, ptr {}, ptr {})", o_val, url_ptr, method_ptr, body_ptr));
                            let tmp = self.fresh_label();
                            self.e(&format!("%{} = call i32 @yk_http_status(i64 {})", tmp, o_val));
                            let extended = self.fresh_label();
                            self.e(&format!("%{} = sext i32 %{} to i64", extended, tmp));
                            return (self.ssa(&extended), "i64".into());
                        }
                        // Server method dispatch (get, post, serve)
                        if o_ty == "i64" && matches!(field.as_str(), "get" | "post" | "put" | "delete" | "patch" | "ws" | "serve") {
                            return self.compile_server_method(&o_val, field, &arg_results, args);
                        }
                        // TcpStream method dispatch (send, recv, close)
                        if o_ty == "i64" && matches!(field.as_str(), "send" | "recv" | "close") {
                            match field.as_str() {
                                "send" => {
                                    if let Some((av, at)) = arg_results.first() {
                                        let data_ptr = if at == "%yk_string" { self.string_to_ptr(av) } else { "null".into() };
                                        let tmp = self.fresh_label();
                                        self.e(&format!("%{} = call i64 @yk_tcp_send(i64 {}, ptr {})", tmp, o_val, data_ptr));
                                        return (self.ssa(&tmp), "i64".into());
                                    }
                                    return ("0".into(), "i64".into());
                                }
                                "recv" => {
                                    let n = arg_results.first().map(|(av, _)| av.as_str()).unwrap_or("4096");
                                    let ptr_tmp = self.fresh_label();
                                    self.e(&format!("%{} = call ptr @yk_tcp_recv(i64 {}, i64 {})", ptr_tmp, o_val, n));
                                    let loaded = self.fresh_label();
                                    self.e(&format!("%{} = load %yk_string, ptr %{}", loaded, ptr_tmp));
                                    return (self.ssa(&loaded), "%yk_string".into());
                                }
                                "close" => {
                                    self.e(&format!("call void @yk_tcp_close(i64 {})", o_val));
                                    return ("0".into(), "void".into());
                                }
                                _ => return ("0".into(), "i64".into()),
                            }
                        }
                        // UdpSocket method dispatch (send_to, recv_from, close)
                        if o_ty == "i64" && matches!(field.as_str(), "send_to" | "recv_from" | "close") {
                            match field.as_str() {
                                "send_to" => {
                                    let data_ptr = arg_results.get(0).map(|(av, at)| {
                                        if at == "%yk_string" { self.string_to_ptr(av) } else { "null".into() }
                                    }).unwrap_or_else(|| "null".into());
                                    let addr_ptr = arg_results.get(1).map(|(av, at)| {
                                        if at == "%yk_string" { self.string_to_ptr(av) } else { "null".into() }
                                    }).unwrap_or_else(|| "null".into());
                                    let tmp = self.fresh_label();
                                    self.e(&format!("%{} = call i64 @yk_udp_send_to(i64 {}, ptr {}, ptr {})", tmp, o_val, data_ptr, addr_ptr));
                                    return (self.ssa(&tmp), "i64".into());
                                }
                                "recv_from" => {
                                    let n = arg_results.first().map(|(av, _)| av.as_str()).unwrap_or("4096");
                                    let ptr_tmp = self.fresh_label();
                                    self.e(&format!("%{} = call ptr @yk_udp_recv_from(i64 {}, i64 {})", ptr_tmp, o_val, n));
                                    let loaded = self.fresh_label();
                                    self.e(&format!("%{} = load %yk_string, ptr %{}", loaded, ptr_tmp));
                                    return (self.ssa(&loaded), "%yk_string".into());
                                }
                                "close" => {
                                    self.e(&format!("call void @yk_tcp_close(i64 {})", o_val));
                                    return ("0".into(), "void".into());
                                }
                                _ => return ("0".into(), "i64".into()),
                            }
                        }
                        // TcpListener method dispatch (accept, close)
                        if o_ty == "i64" && matches!(field.as_str(), "accept" | "close") {
                            match field.as_str() {
                                "accept" => {
                                    let tmp = self.fresh_label();
                                    self.e(&format!("%{} = call i64 @yk_tcp_accept(i64 {})", tmp, o_val));
                                    return (self.ssa(&tmp), "i64".into());
                                }
                                "close" => {
                                    self.e(&format!("call void @yk_tcp_close(i64 {})", o_val));
                                    return ("0".into(), "void".into());
                                }
                                _ => return ("0".into(), "i64".into()),
                            }
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
                        // .toString() method dispatch
                        if field == "toString" {
                            let ptr_ssa = match o_ty.as_str() {
                                "i64" => {
                                    let t = self.fresh_label();
                                    self.e(&format!("%{} = call ptr @yk_string_from_int(i64 {})", t, o_val));
                                    self.ssa(&t)
                                }
                                "double" => {
                                    let t = self.fresh_label();
                                    self.e(&format!("%{} = call ptr @yk_string_from_real(double {})", t, o_val));
                                    self.ssa(&t)
                                }
                                "i1" => {
                                    let t = self.fresh_label();
                                    self.e(&format!("%{} = call ptr @yk_string_from_bool(i1 {})", t, o_val));
                                    self.ssa(&t)
                                }
                                "%yk_complex" => {
                                    let ca = self.fresh_label();
                                    self.e(&format!("%{} = alloca %yk_complex, align 8", ca));
                                    self.e(&format!("store %yk_complex {}, ptr %{}", o_val, ca));
                                    let t = self.fresh_label();
                                    self.e(&format!("%{} = call ptr @yk_string_from_complex(ptr %{})", t, ca));
                                    self.ssa(&t)
                                }
                                "%yk_string" => {
                                    let ca = self.fresh_label();
                                    self.e(&format!("%{} = alloca %yk_string, align 8", ca));
                                    self.e(&format!("store %yk_string {}, ptr %{}", o_val, ca));
                                    self.ssa(&ca)
                                }
                                "ptr" => {
                                    let t = self.fresh_label();
                                    self.e(&format!("%{} = call ptr @yk_list_to_string(ptr {})", t, o_val));
                                    self.ssa(&t)
                                }
                                _ => {
                                    let t = self.fresh_label();
                                    self.e(&format!("%{} = call ptr @yk_string_from_int(i64 {})", t, o_val));
                                    self.ssa(&t)
                                }
                            };
                            let loaded = self.fresh_label();
                            self.e(&format!("%{} = load %yk_string, ptr {}", loaded, ptr_ssa));
                            return (self.ssa(&loaded), "%yk_string".into());
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
                                    let self_ssa = self.ssa(&self_int);
                                    let mut all_args = vec![format!("i64 {}", self_ssa)];
                                    all_args.extend(arg_results.iter().map(|(v, t)| format!("{} {}", t, v)));
                                    let call_ret = self.class_method_ret_types.get(&(cls_name.to_string(), field.clone())).cloned().unwrap_or_else(|| "i64".into());
                                    let all_args_str = all_args.join(", ");
                                    let arg_types_str = all_args.iter().map(|s| s.split_whitespace().next().unwrap_or("i64")).collect::<Vec<_>>().join(", ");
                                    if call_ret == "void" {
                                        self.e(&format!("call void ({}) %{}({})", arg_types_str, fn_ptr, all_args_str));
                                        ("0".into(), "void".into())
                                    } else {
                                        let result_tmp = self.fresh_label();
                                        self.e(&format!("%{} = call {} ({}) %{}({})", result_tmp, call_ret, arg_types_str, fn_ptr, all_args_str));
                                        (self.ssa(&result_tmp), call_ret)
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
                        } else if let Some(iface_name) = o_ty.strip_prefix("%iface.") {
                            // Interface dispatch: extract vtable ptr from fat pointer
                            let vtable_ptr = self.fresh_label();
                            self.e(&format!("%{} = extractvalue {} {}, 1", vtable_ptr, o_ty, o_val));
                            let data_val = self.fresh_label();
                            self.e(&format!("%{} = extractvalue {} {}, 0", data_val, o_ty, o_val));
                            let self_int = self.fresh_label();
                            self.e(&format!("%{} = ptrtoint ptr %{} to i64", self_int, data_val));
                            if let Some(iface_methods) = self.interface_methods.get(iface_name) {
                                if let Some(method_idx) = iface_methods.iter().position(|n| n == field) {
                                    let method_ptr = self.fresh_label();
                                    self.e(&format!("%{} = getelementptr inbounds ptr, ptr %{}, i64 {}", method_ptr, vtable_ptr, method_idx));
                                    let fn_ptr = self.fresh_label();
                                    self.e(&format!("%{} = load ptr, ptr %{}", fn_ptr, method_ptr));
                                    let call_ret = self.interface_method_ret_types.get(&(iface_name.to_string(), field.clone())).cloned().unwrap_or_else(|| "i64".into());
                                    if call_ret == "void" {
                                        self.e(&format!("call void (i64) %{}(i64 %{})", fn_ptr, self_int));
                                        ("0".into(), "void".into())
                                    } else {
                                        let result_tmp = self.fresh_label();
                                        self.e(&format!("%{} = call {} (i64) %{}(i64 %{})", result_tmp, call_ret, fn_ptr, self_int));
                                        (self.ssa(&result_tmp), call_ret)
                                    }
                                } else {
                                    ("0".into(), "i64".into())
                                }
                            } else {
                                ("0".into(), "i64".into())
                            }
                        } else if let Some(obj_name) = o_ty.strip_prefix("%object.") {
                            // Object method dispatch
                            let alloca_ptr = self.fresh_label();
                            self.e(&format!("%{} = alloca {}, align 8", alloca_ptr, o_ty));
                            self.e(&format!("store {} {}, ptr %{}", o_ty, o_val, alloca_ptr));
                            let self_int = self.fresh_label();
                            self.e(&format!("%{} = ptrtoint ptr %{} to i64", self_int, alloca_ptr));
                            let mangled_method = if let Some(omod) = self.object_modules.get(obj_name) {
                                if omod.is_empty() {
                                    format!("__obj_method_{}_{}", obj_name, field)
                                } else {
                                    format!("__obj_method_{}_{}_{}", omod, obj_name, field)
                                }
                            } else {
                                format!("__obj_method_{}_{}", obj_name, field)
                            };
                            let call_ret = self.object_method_ret_types.get(&(obj_name.to_string(), field.clone())).cloned().unwrap_or_else(|| "i64".into());
                            let has_self = self.object_method_has_self.contains(&(obj_name.to_string(), field.clone()));
                            let self_ssa = self.ssa(&self_int);
                            let (all_args_str, arg_types_str) = if has_self {
                                let all_args: Vec<String> = std::iter::once(format!("i64 {}", self_ssa))
                                    .chain(arg_results.iter().map(|(v, t)| format!("{} {}", t, v)))
                                    .collect();
                                let a = all_args.join(", ");
                                let t = all_args.iter().map(|s| s.split_whitespace().next().unwrap_or("i64")).collect::<Vec<_>>().join(", ");
                                (a, t)
                            } else {
                                let a = arg_results.iter().map(|(v, t)| format!("{} {}", t, v)).collect::<Vec<_>>().join(", ");
                                let t = arg_results.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(", ");
                                (a, t)
                            };
                            if call_ret == "void" {
                                self.e(&format!("call void ({}) @{}({})", arg_types_str, mangled_method, all_args_str));
                                ("0".into(), "void".into())
                            } else {
                                let result_tmp = self.fresh_label();
                                self.e(&format!("%{} = call {} ({}) @{}({})", result_tmp, call_ret, arg_types_str, mangled_method, all_args_str));
                                (self.ssa(&result_tmp), call_ret)
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
            .args(["/c", &format!(r#""{}" x64 >nul 2>&1 && cl.exe /nologo /TP /EHsc /std:c++17 /arch:AVX2 /c "{}" /Fo:"{}" /utf-8"#,
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
         cl.exe /nologo /TP /EHsc /std:c++17 /arch:AVX2 /c \"{rtc}\" /Fo:\"{rto}\" /utf-8\r\n\
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
    let mut gen = FnIrGen::new(handler_name);
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
    prefix: String,
    output: String,
    string_constants: String,
}

impl FnIrGen {
    fn new(prefix: &str) -> Self {
        FnIrGen { label: 0, prefix: prefix.to_string(), output: String::new(), string_constants: String::new() }
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
        let cstr_name = format!("@__yk_cstr_{}_{}", self.prefix, idx);
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
            Expr::ListLit(items) => {
                let parts: Vec<String> = items.iter().map(|e| self.lit_to_str(e)).collect();
                let result = format!("[{}]", parts.join(", "));
                self.gen_str_constant(&result)
            }
            Expr::SetLit(items) => {
                let parts: Vec<String> = items.iter().map(|e| self.lit_to_str(e)).collect();
                let result = format!("{{{}}}", parts.join(", "));
                self.gen_str_constant(&result)
            }
            Expr::MapLit(pairs) => {
                let parts: Vec<String> = pairs.iter().map(|(k, v)| format!("{}: {}", self.lit_to_str(k), self.lit_to_str(v))).collect();
                let result = format!("{{{}}}", parts.join(", "));
                self.gen_str_constant(&result)
            }
            Expr::TupleLit(items) => {
                let parts: Vec<String> = items.iter().map(|e| self.lit_to_str(e)).collect();
                let result = format!("({})", parts.join(", "));
                self.gen_str_constant(&result)
            }
            _ => {
                let empty_ptr = self.fresh();
                self.e(&format!("{empty_ptr} = getelementptr i8, ptr %buf, i64 0"));
                (empty_ptr, "0".into())
            }
        }
    }

    fn lit_to_str(&self, expr: &ExprNode) -> String {
        match &expr.value {
            Expr::LitStr(s) => s.clone(),
            Expr::LitInt(n) => n.to_string(),
            Expr::LitBool(b) => (if *b { "true" } else { "false" }).to_string(),
            Expr::LitChar(c) => c.to_string(),
            Expr::LitReal(v) => format!("{}", v),
            Expr::LitHex(n) => format!("{}", n),
            _ => "?".to_string(),
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
