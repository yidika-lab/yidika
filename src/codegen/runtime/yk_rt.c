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

// ── Platform Abstraction ────────────────────────────────────────
#ifdef _WIN32
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <winhttp.h>
#include <psapi.h>
#pragma comment(lib, "winhttp.lib")
#pragma comment(lib, "ws2_32.lib")
#include <winsock2.h>
#include <ws2tcpip.h>
#include <direct.h>
#include <process.h>
#include <io.h>
typedef SOCKET yk_socket_t;
#define YK_INVALID_SOCKET INVALID_SOCKET
#define yk_closesocket(fd) closesocket(fd)
#define YK_TLS __declspec(thread)
#define YK_GET_ERR() WSAGetLastError()
#else
#include <sys/socket.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/uio.h>
#include <sys/resource.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <netdb.h>
#include <arpa/inet.h>
#include <unistd.h>
#include <fcntl.h>
#include <pthread.h>
#include <sched.h>
#include <signal.h>
#include <errno.h>
#if defined(__linux__) && defined(YK_USE_URING)
#include <liburing.h>
#define YK_HAVE_URING 1
#endif
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

// ── Threads, sleep, process ─────────────────────────────────
#ifdef _WIN32
typedef HANDLE yk_thread_t;
typedef unsigned int yk_thread_ret;
#define YK_THREAD_CALL __stdcall
#define YK_THREAD_DONE 0
static int yk_thread_spawn(yk_thread_t* out, yk_thread_ret (YK_THREAD_CALL *fn)(void*), void* arg) {
    *out = (HANDLE)_beginthreadex(NULL, 0, fn, arg, 0, NULL);
    return *out != NULL;
}
static void yk_thread_join(yk_thread_t t) { WaitForSingleObject(t, INFINITE); CloseHandle(t); }
static void yk_thread_detach(yk_thread_t t) { CloseHandle(t); }
static void yk_sleep_ms(unsigned int ms) { Sleep(ms); }
static int64_t yk_process_id(void) { return (int64_t)GetCurrentProcessId(); }
#define yk_getcwd _getcwd
typedef SRWLOCK yk_mutex_t;
#define YK_MUTEX_INIT SRWLOCK_INIT
static void yk_mutex_lock(yk_mutex_t* m) { AcquireSRWLockExclusive(m); }
static void yk_mutex_unlock(yk_mutex_t* m) { ReleaseSRWLockExclusive(m); }
static void yk_localtime(const time_t* t, struct tm* out) { localtime_s(out, t); }
static void yk_gmtime(const time_t* t, struct tm* out) { gmtime_s(out, t); }
enum { YK_PATH_NONE = 0, YK_PATH_FILE = 1, YK_PATH_DIR = 2 };
static int yk_path_kind(const char* path) {
    DWORD attr = GetFileAttributesA(path);
    if (attr == INVALID_FILE_ATTRIBUTES) return YK_PATH_NONE;
    return (attr & FILE_ATTRIBUTE_DIRECTORY) ? YK_PATH_DIR : YK_PATH_FILE;
}
#else
typedef pthread_t yk_thread_t;
typedef void* yk_thread_ret;
#define YK_THREAD_CALL
#define YK_THREAD_DONE NULL
static int yk_thread_spawn(yk_thread_t* out, yk_thread_ret (*fn)(void*), void* arg) {
    return pthread_create(out, NULL, fn, arg) == 0;
}
static void yk_thread_join(yk_thread_t t) { pthread_join(t, NULL); }
static void yk_thread_detach(yk_thread_t t) { pthread_detach(t); }
static void yk_sleep_ms(unsigned int ms) {
    struct timespec ts;
    ts.tv_sec = ms / 1000;
    ts.tv_nsec = (long)(ms % 1000) * 1000000L;
    nanosleep(&ts, NULL);
}
static int64_t yk_process_id(void) { return (int64_t)getpid(); }
#define yk_getcwd getcwd
typedef pthread_mutex_t yk_mutex_t;
#define YK_MUTEX_INIT PTHREAD_MUTEX_INITIALIZER
static void yk_mutex_lock(yk_mutex_t* m) { pthread_mutex_lock(m); }
static void yk_mutex_unlock(yk_mutex_t* m) { pthread_mutex_unlock(m); }
static void yk_localtime(const time_t* t, struct tm* out) { localtime_r(t, out); }
static void yk_gmtime(const time_t* t, struct tm* out) { gmtime_r(t, out); }
enum { YK_PATH_NONE = 0, YK_PATH_FILE = 1, YK_PATH_DIR = 2 };
static int yk_path_kind(const char* path) {
    struct stat st;
    if (stat(path, &st) != 0) return YK_PATH_NONE;
    return S_ISDIR(st.st_mode) ? YK_PATH_DIR : YK_PATH_FILE;
}
#endif

#ifndef MSG_NOSIGNAL
#define MSG_NOSIGNAL 0
#endif
static int yk_send_all(yk_socket_t fd, const char* data, int64_t len) {
    while (len > 0) {
        int chunk = len > (1 << 30) ? (1 << 30) : (int)len;
        int n = (int)send(fd, data, chunk, MSG_NOSIGNAL);
#ifndef _WIN32
        if (n < 0 && errno == EINTR) continue;
#endif
        if (n <= 0) return -1;
        data += n;
        len -= n;
    }
    return 0;
}

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

static void yk_stdout_used(void);

typedef struct yk_vmem_slot { struct yk_vmem_slot* volatile next; } yk_vmem_slot;

typedef struct {
    char* base;
    char* commit_end;
    size_t total_size;
    size_t page_size;
    int slot_size;
    yk_vmem_slot* volatile free_head;  // Treiber stack head
} yk_vmem_pool;

#define YK_CONN_POOL_SIZE (512ULL * 1024 * 1024)  // 512 MB -> ~2M conns

static yk_vmem_pool yk_conn_pool = {0};
static yk_mutex_t yk_conn_pool_lock = YK_MUTEX_INIT;

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

static void* yk_vmem_pool_alloc_locked(yk_vmem_pool* pool) {
    int slot_size = pool->slot_size;
    yk_vmem_slot* head = pool->free_head;
    if (head) {
        pool->free_head = head->next;
        memset(head, 0, slot_size);
        return head;
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
            s->next = pool->free_head;
            pool->free_head = s;
        }
    }

    memset(page, 0, slot_size);
    return page;
}

static void* yk_vmem_pool_alloc(void) {
    yk_mutex_lock(&yk_conn_pool_lock);
    void* slot = yk_vmem_pool_alloc_locked(&yk_conn_pool);
    yk_mutex_unlock(&yk_conn_pool_lock);
    return slot;
}

static void yk_vmem_pool_free(void* ptr) {
    if (!ptr) return;
    yk_vmem_slot* s = (yk_vmem_slot*)ptr;
    yk_mutex_lock(&yk_conn_pool_lock);
    s->next = yk_conn_pool.free_head;
    yk_conn_pool.free_head = s;
    yk_mutex_unlock(&yk_conn_pool_lock);
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

typedef struct { char* data; int64_t len; int64_t cap; } yk_string;

#ifdef __cplusplus
extern "C" {
#endif

static yk_string* yk_string_header(void) {
    return (yk_string*)calloc(1, sizeof(yk_string));
}

void yk_string_init_ptr(yk_string* s, const char* data, int64_t len) {
    s->data = (char*)data;
    s->len = len;
    s->cap = 0;
}

void yk_string_free_header(yk_string* s) {
    free(s);
}

void yk_string_drop(yk_string* s) {
    if (s->cap > 0) free(s->data);
    s->data = NULL;
    s->len = 0;
    s->cap = 0;
}

yk_string* yk_string_clone(yk_string* src) {
    yk_string* s = yk_string_header();
    s->data = (char*)malloc((size_t)src->len + 1);
    memcpy(s->data, src->data, (size_t)src->len);
    s->data[src->len] = '\0';
    s->len = src->len;
    s->cap = src->len + 1;
    return s;
}

yk_string* yk_listen_addr_from_port(int64_t port) {
    char buf[32];
    int n = snprintf(buf, sizeof(buf), "0.0.0.0:%lld", (long long)port);
    yk_string* s = yk_string_header();
    s->data = (char*)malloc(n + 1);
    memcpy(s->data, buf, n + 1);
    s->len = n;
    s->cap = n + 1;
    return s;
}

#define YK_I64_TEXT_MAX 21

static const char yk_digit_pairs[201] = "00010203040506070809101112131415161718192021222324252627282930313233343536373839404142434445464748495051525354555657585960616263646566676869707172737475767778798081828384858687888990919293949596979899";

static int yk_format_i64(char out[YK_I64_TEXT_MAX], int64_t v) {
    char tmp[YK_I64_TEXT_MAX];
    int i = YK_I64_TEXT_MAX;
    uint64_t u = v < 0 ? (uint64_t)0 - (uint64_t)v : (uint64_t)v;
    while (u >= 100) {
        unsigned r = (unsigned)(u % 100);
        u /= 100;
        tmp[--i] = yk_digit_pairs[r * 2 + 1];
        tmp[--i] = yk_digit_pairs[r * 2];
    }
    if (u >= 10) {
        tmp[--i] = yk_digit_pairs[u * 2 + 1];
        tmp[--i] = yk_digit_pairs[u * 2];
    } else {
        tmp[--i] = (char)('0' + u);
    }
    if (v < 0) tmp[--i] = '-';
    int n = YK_I64_TEXT_MAX - i;
    memcpy(out, tmp + i, (size_t)n);
    return n;
}

static void yk_string_set_bytes(yk_string* out, const char* bytes, int64_t len) {
    out->data = (char*)malloc((size_t)len + 1);
    memcpy(out->data, bytes, (size_t)len);
    out->data[len] = '\0';
    out->len = len;
    out->cap = len + 1;
}

void yk_write_prompt(yk_string* prompt) {
    if (prompt && prompt->data && prompt->len > 0) fwrite(prompt->data, 1, (size_t)prompt->len, stdout);
    fflush(stdout);
}

void yk_string_from_int_into(yk_string* out, int64_t v) {
    char buf[YK_I64_TEXT_MAX];
    yk_string_set_bytes(out, buf, yk_format_i64(buf, v));
}

yk_string* yk_string_from_int(int64_t v) {
    yk_string* s = yk_string_header();
    yk_string_from_int_into(s, v);
    return s;
}

typedef struct { double real; double imag; } yk_complex;

static int yk_shortest_digits(double v, char* digits, int* exponent) {
    char sci[40];
    for (int precision = 1; precision <= 17; precision++) {
        snprintf(sci, sizeof(sci), "%.*e", precision - 1, v);
        if (strtod(sci, NULL) == v) break;
    }
    int count = 0;
    const char* c = sci[0] == '-' ? sci + 1 : sci;
    for (; *c && *c != 'e' && count < 17; c++) {
        if (*c != '.') digits[count++] = *c;
    }
    *exponent = *c == 'e' ? atoi(c + 1) : 0;
    while (count > 1 && digits[count - 1] == '0') count--;
    return count;
}

static int yk_format_real(double v, char* out, int cap) {
    if (isnan(v)) return snprintf(out, cap, "NaN");
    if (isinf(v)) return snprintf(out, cap, v < 0 ? "-inf" : "inf");
    char digits[17];
    int exponent = 0;
    int count = yk_shortest_digits(v, digits, &exponent);
    int point = exponent + 1;
    int n = 0;
#define YK_PUT(ch) do { if (n < cap - 1) out[n++] = (ch); } while (0)
    if (signbit(v)) YK_PUT('-');
    if (count == 1 && digits[0] == '0') {
        YK_PUT('0');
    } else if (point <= 0) {
        YK_PUT('0');
        YK_PUT('.');
        for (int i = 0; i < -point; i++) YK_PUT('0');
        for (int i = 0; i < count; i++) YK_PUT(digits[i]);
    } else {
        for (int i = 0; i < point; i++) YK_PUT(i < count ? digits[i] : '0');
        if (count > point) {
            YK_PUT('.');
            for (int i = point; i < count; i++) YK_PUT(digits[i]);
        }
    }
#undef YK_PUT
    out[n] = 0;
    return n;
}

static int yk_format_complex(const yk_complex* c, char* out, int cap) {
    char re[400];
    char im[400];
    yk_format_real(c->real, re, sizeof(re));
    yk_format_real(c->imag < 0 ? -c->imag : c->imag, im, sizeof(im));
    return snprintf(out, cap, "%s %c %si", re, c->imag < 0 ? '-' : '+', im);
}

static yk_string* yk_string_from_buffer(const char* buf, int n) {
    if (n < 0) n = 0;
    yk_string* s = yk_string_header();
    s->data = (char*)malloc(n + 1);
    memcpy(s->data, buf, n);
    s->data[n] = 0;
    s->len = n;
    s->cap = n + 1;
    return s;
}

static int yk_encode_utf8(int64_t cp, char* out) {
    if (cp < 0 || cp > 0x10FFFF || (cp >= 0xD800 && cp <= 0xDFFF)) cp = 0xFFFD;
    if (cp < 0x80) { out[0] = (char)cp; return 1; }
    if (cp < 0x800) { out[0] = (char)(0xC0 | (cp >> 6)); out[1] = (char)(0x80 | (cp & 0x3F)); return 2; }
    if (cp < 0x10000) { out[0] = (char)(0xE0 | (cp >> 12)); out[1] = (char)(0x80 | ((cp >> 6) & 0x3F)); out[2] = (char)(0x80 | (cp & 0x3F)); return 3; }
    out[0] = (char)(0xF0 | (cp >> 18)); out[1] = (char)(0x80 | ((cp >> 12) & 0x3F)); out[2] = (char)(0x80 | ((cp >> 6) & 0x3F)); out[3] = (char)(0x80 | (cp & 0x3F));
    return 4;
}

yk_string* yk_string_from_char(int64_t cp) {
    char buf[4];
    return yk_string_from_buffer(buf, yk_encode_utf8(cp, buf));
}

void yk_print_field(yk_string* s, bool last) {
    yk_stdout_used();
    fwrite(s->data, 1, (size_t)s->len, stdout);
    fputc(last ? '\n' : ' ', stdout);
}

yk_string* yk_string_from_real(double v) {
    char buf[400];
    return yk_string_from_buffer(buf, yk_format_real(v, buf, sizeof(buf)));
}

void yk_string_concat_into(yk_string* out, yk_string* a, yk_string* b) {
    int64_t len = a->len + b->len;
    char* data = (char*)malloc((size_t)len + 1);
    memcpy(data, a->data, (size_t)a->len);
    memcpy(data + a->len, b->data, (size_t)b->len);
    data[len] = '\0';
    out->data = data;
    out->len = len;
    out->cap = len + 1;
}

yk_string* yk_string_concat_ptr(yk_string* a, yk_string* b) {
    yk_string* s = yk_string_header();
    yk_string_concat_into(s, a, b);
    return s;
}

int64_t yk_string_len_ptr(yk_string* s) { return s->len; }

yk_string* yk_string_from_cstr(const char* s) {
    yk_string* result = yk_string_header();
    result->len = (int64_t)strlen(s);
    result->data = (char*)malloc(result->len + 1);
    memcpy(result->data, s, result->len + 1);
    result->cap = result->len + 1;
    return result;
}

static int64_t yk_peak_memory_kb(void) {
#ifdef _WIN32
    PROCESS_MEMORY_COUNTERS counters;
    if (!K32GetProcessMemoryInfo(GetCurrentProcess(), &counters, sizeof(counters))) return -1;
    return (int64_t)(counters.PeakWorkingSetSize / 1024);
#elif defined(__linux__)
    FILE* status = fopen("/proc/self/status", "r");
    if (!status) return -1;
    char line[256];
    long long kb = -1;
    while (fgets(line, sizeof(line), status)) {
        if (sscanf(line, "VmHWM: %lld kB", &kb) == 1) break;
    }
    fclose(status);
    return (int64_t)kb;
#else
    struct rusage usage;
    if (getrusage(RUSAGE_SELF, &usage) != 0) return -1;
    return (int64_t)(usage.ru_maxrss / 1024);
#endif
}

static void yk_report_peak_memory(void) {
    fflush(stdout);
    fprintf(stderr, "yk-peak-memory-kb %lld\n", (long long)yk_peak_memory_kb());
}

static int yk_stdout_is_terminal(void) {
#ifdef _WIN32
    return _isatty(_fileno(stdout));
#else
    return isatty(fileno(stdout));
#endif
}

#define YK_STDOUT_FLUSH_MS 50

static yk_thread_ret YK_THREAD_CALL yk_stdout_flusher(void* arg) {
    (void)arg;
    for (;;) {
        yk_sleep_ms(YK_STDOUT_FLUSH_MS);
        fflush(stdout);
    }
    return 0;
}

static void* volatile yk_stdout_flusher_pending = NULL;

static void yk_start_stdout_flusher(void) {
    yk_thread_t flusher;
    if (yk_thread_spawn(&flusher, yk_stdout_flusher, NULL)) {
        yk_thread_detach(flusher);
    } else {
        fflush(stdout);
        setvbuf(stdout, NULL, _IOLBF, BUFSIZ);
    }
}

static void yk_stdout_used(void) {
    void* pending = yk_stdout_flusher_pending;
    if (pending && YK_CAS_PTR(&yk_stdout_flusher_pending, pending, NULL) == pending) {
        yk_start_stdout_flusher();
    }
}

static void yk_buffer_piped_stdout(void) {
    setvbuf(stdout, NULL, _IOFBF, 1 << 16);
    yk_stdout_flusher_pending = (void*)1;
}

#if defined(__has_feature)
#if __has_feature(address_sanitizer)
#define YK_UNDER_ASAN 1
#endif
#endif
#if defined(__SANITIZE_ADDRESS__)
#define YK_UNDER_ASAN 1
#endif

static void yk_write_stack_overflow(void) {
    const char* message = yk_trap_messages[YK_TRAP_STACK_OVERFLOW];
#ifdef _WIN32
    DWORD written;
    HANDLE err = GetStdHandle(STD_ERROR_HANDLE);
    WriteFile(err, "error: ", 7, &written, NULL);
    WriteFile(err, message, (DWORD)strlen(message), &written, NULL);
    WriteFile(err, "\n", 1, &written, NULL);
#else
    ssize_t ignored = write(2, "error: ", 7);
    ignored = write(2, message, strlen(message));
    ignored = write(2, "\n", 1);
    (void)ignored;
#endif
}

#ifdef _WIN32
static LONG WINAPI yk_on_stack_overflow(EXCEPTION_POINTERS* info) {
    if (info->ExceptionRecord->ExceptionCode != EXCEPTION_STACK_OVERFLOW) return EXCEPTION_CONTINUE_SEARCH;
    yk_write_stack_overflow();
    ExitProcess(1);
    return EXCEPTION_CONTINUE_SEARCH;
}

static void yk_catch_stack_overflow(void) {
    ULONG reserve = 64 * 1024;
    SetThreadStackGuarantee(&reserve);
    AddVectoredExceptionHandler(1, yk_on_stack_overflow);
}
#else
#define YK_ALT_STACK_SIZE (64 * 1024)
#define YK_GUARD_SPAN (1024 * 1024)

static uintptr_t yk_main_stack_low = 0;
static char yk_alt_stack[YK_ALT_STACK_SIZE];

static void yk_record_main_stack(void) {
#if defined(__APPLE__)
    pthread_t self = pthread_self();
    yk_main_stack_low = (uintptr_t)pthread_get_stackaddr_np(self) - pthread_get_stacksize_np(self);
#else
    struct rlimit limit;
    if (getrlimit(RLIMIT_STACK, &limit) != 0 || limit.rlim_cur == RLIM_INFINITY) return;
    uintptr_t top = (uintptr_t)__builtin_frame_address(0);
    if (top > limit.rlim_cur) yk_main_stack_low = top - (uintptr_t)limit.rlim_cur;
#endif
}

static int yk_is_stack_guard(uintptr_t addr) {
    return yk_main_stack_low && addr + YK_GUARD_SPAN >= yk_main_stack_low && addr < yk_main_stack_low + YK_GUARD_SPAN;
}

static void yk_on_fault(int sig, siginfo_t* info, void* context) {
    (void)context;
    if (yk_is_stack_guard((uintptr_t)info->si_addr)) {
        yk_write_stack_overflow();
        _exit(1);
    }
    struct sigaction fallback;
    memset(&fallback, 0, sizeof(fallback));
    fallback.sa_handler = SIG_DFL;
    sigaction(sig, &fallback, NULL);
}

static void yk_catch_stack_overflow(void) {
    yk_record_main_stack();
    stack_t alt;
    alt.ss_sp = yk_alt_stack;
    alt.ss_size = sizeof(yk_alt_stack);
    alt.ss_flags = 0;
    if (sigaltstack(&alt, NULL) != 0) return;
    struct sigaction action;
    memset(&action, 0, sizeof(action));
    action.sa_sigaction = yk_on_fault;
    action.sa_flags = SA_SIGINFO | SA_ONSTACK;
    sigemptyset(&action.sa_mask);
    sigaction(SIGSEGV, &action, NULL);
    sigaction(SIGBUS, &action, NULL);
}
#endif

void yk_runtime_init(void) {
#ifndef YK_UNDER_ASAN
    yk_catch_stack_overflow();
#endif
    if (!yk_stdout_is_terminal()) {
        yk_buffer_piped_stdout();
    } else {
#ifdef _WIN32
        setvbuf(stdout, NULL, _IONBF, 0);
#else
        setvbuf(stdout, NULL, _IOLBF, BUFSIZ);
#endif
    }
    const char* report = getenv("YK_PEAK_MEMORY");
    if (report && report[0] == '1') atexit(yk_report_peak_memory);
}

void yk_print_int(int64_t v) {
    yk_stdout_used();
    char buf[YK_I64_TEXT_MAX + 1];
    int n = yk_format_i64(buf, v);
    buf[n] = '\n';
    fwrite(buf, 1, (size_t)n + 1, stdout);
}
void yk_print_real(double v) { yk_stdout_used(); char buf[400]; yk_format_real(v, buf, sizeof(buf)); puts(buf); }
void yk_print_bool(bool v) { yk_stdout_used(); printf("%s\n", v ? "true" : "false"); }
void yk_print_str_ptr(yk_string* s) { yk_stdout_used(); printf("%.*s\n", (int)s->len, s->data); }


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
void yk_print_complex(yk_complex* c) { yk_stdout_used(); char buf[810]; yk_format_complex(c, buf, sizeof(buf)); puts(buf); }

static void yk_fail(const char* message) {
    fflush(stdout);
    fprintf(stderr, "error: %s\n", message);
    exit(1);
}

void yk_trap(int64_t code) {
    yk_fail(code >= 0 && code < YK_TRAP_COUNT ? yk_trap_messages[code] : "runtime error");
}

int64_t yk_pow_int(int64_t base, int64_t exponent) {
    if (exponent < 0) yk_trap(YK_TRAP_NEGATIVE_EXPONENT);
    uint64_t result = 1, factor = (uint64_t)base, remaining = (uint64_t)exponent;
    while (remaining) {
        if (remaining & 1) result *= factor;
        factor *= factor;
        remaining >>= 1;
    }
    return (int64_t)result;
}
double yk_pow_real(double a, double b) { return pow(a, b); }

yk_string* yk_string_from_bool(bool v) {
    const char* s = v ? "true" : "false";
    yk_string* r = yk_string_header();
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
    yk_string* result = yk_string_header();
    result->data = out;
    result->len = j;
    return result;
}

yk_string* yk_string_from_complex(yk_complex* c) {
    char buf[810];
    return yk_string_from_buffer(buf, yk_format_complex(c, buf, sizeof(buf)));
}

typedef struct { void (*entry)(int64_t*); yk_thread_t thread; int64_t result; } yk_task;

enum { YK_KIND_INT = 0, YK_KIND_REAL = 1, YK_KIND_BOOL = 2, YK_KIND_STR = 3 };

typedef struct { int64_t* data; int64_t len; int64_t cap; int64_t kind; int64_t is_set; } yk_list;

yk_list* yk_list_new_of(int64_t kind, int64_t is_set) {
    yk_list* l = (yk_list*)malloc(sizeof(yk_list));
    l->data = NULL; l->len = 0; l->cap = 0; l->kind = kind; l->is_set = is_set;
    return l;
}

yk_list* yk_list_new(void) { return yk_list_new_of(YK_KIND_INT, 0); }

yk_string* yk_string_box(const yk_string* s) {
    yk_string* boxed = yk_string_header();
    *boxed = *s;
    return boxed;
}

static void yk_list_append(yk_list* l, int64_t val) {
    if (l->len >= l->cap) {
        l->cap = l->cap ? l->cap * 2 : 8;
        l->data = (int64_t*)realloc(l->data, l->cap * sizeof(int64_t));
    }
    l->data[l->len++] = val;
}

static bool yk_elem_eq(int64_t kind, int64_t a, int64_t b) {
    if (kind != YK_KIND_STR) return a == b;
    const yk_string* x = (const yk_string*)(intptr_t)a;
    const yk_string* y = (const yk_string*)(intptr_t)b;
    return x->len == y->len && memcmp(x->data, y->data, (size_t)x->len) == 0;
}

static bool yk_list_has(const yk_list* l, int64_t val) {
    for (int64_t i = 0; i < l->len; i++) {
        if (yk_elem_eq(l->kind, l->data[i], val)) return true;
    }
    return false;
}

void yk_list_push(yk_list* l, int64_t val) {
    if (l->is_set && yk_list_has(l, val)) return;
    yk_list_append(l, val);
}

void yk_list_push_kind(yk_list* l, int64_t val, int64_t kind) {
    if (l->len == 0) l->kind = kind;
    yk_list_push(l, val);
}

int64_t yk_list_get(yk_list* l, int64_t idx) {
    if (idx < 0 || idx >= l->len) {
        fflush(stdout);
        fprintf(stderr, "error: Index %lld out of bounds for list of length %lld\n", (long long)idx, (long long)l->len);
        exit(1);
    }
    return l->data[idx];
}

int64_t yk_list_len(yk_list* l) { return l->len; }

void yk_list_free(yk_list* l) { if (l) { free(l->data); free(l); } }

int64_t yk_list_pop(yk_list* l) {
    if (l->len == 0) yk_trap(YK_TRAP_POP_FROM_EMPTY_LIST);
    return l->data[--l->len];
}

void yk_list_clear(yk_list* l) { l->len = 0; }

typedef struct { char* data; int64_t len; int64_t cap; } yk_text;

static void yk_text_add(yk_text* t, const char* src, int64_t n) {
    if (n <= 0) return;
    if (t->len + n + 1 > t->cap) {
        int64_t cap = t->cap ? t->cap : 32;
        while (cap < t->len + n + 1) cap *= 2;
        t->data = (char*)realloc(t->data, (size_t)cap);
        t->cap = cap;
    }
    memcpy(t->data + t->len, src, (size_t)n);
    t->len += n;
    t->data[t->len] = 0;
}

static void yk_text_add_elem(yk_text* t, int64_t kind, int64_t v) {
    char buf[400];
    int n = 0;
    switch (kind) {
    case YK_KIND_REAL: { double d; memcpy(&d, &v, sizeof d); n = yk_format_real(d, buf, sizeof(buf)); break; }
    case YK_KIND_BOOL: n = snprintf(buf, sizeof(buf), "%s", v ? "true" : "false"); break;
    case YK_KIND_STR: { const yk_string* s = (const yk_string*)(intptr_t)v; yk_text_add(t, s->data, s->len); return; }
    default: n = snprintf(buf, sizeof(buf), "%lld", (long long)v); break;
    }
    if (n > (int)sizeof(buf) - 1) n = (int)sizeof(buf) - 1;
    yk_text_add(t, buf, n);
}

yk_string* yk_list_to_string(yk_list* l) {
    yk_text t = { NULL, 0, 0 };
    yk_text_add(&t, l->is_set ? "set{" : "[", l->is_set ? 4 : 1);
    for (int64_t i = 0; i < l->len; i++) {
        if (i > 0) yk_text_add(&t, ", ", 2);
        yk_text_add_elem(&t, l->kind, l->data[i]);
    }
    yk_text_add(&t, l->is_set ? "}" : "]", 1);
    yk_string* s = yk_string_header();
    s->data = t.data;
    s->len = t.len;
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
    yk_stdout_used();
    yk_string* s = yk_list_to_string(l);
    printf("%.*s\n", (int)s->len, s->data);
    free(s->data);
    free(s);
}

int64_t yk_result_str_new(int64_t data, int64_t len) {
    yk_string* s = yk_string_header();
    s->data = (char*)malloc(len + 1);
    memcpy(s->data, (void*)(intptr_t)data, len);
    s->data[len] = '\0';
    s->len = len;
    return (int64_t)(intptr_t)s;
}

void yk_print_result_val(int64_t val, int8_t ok) {
    yk_stdout_used();
    yk_string* s = (yk_string*)(intptr_t)val;
    int looks_like_str = (val != 0 && s->len >= 0 && s->len < 1048576 && s->data != NULL);
    if (looks_like_str) {
        printf("%s%.*s)\n", ok ? "Ok(" : "Error(", (int)s->len, s->data);
    } else {
        printf("%s%lld)\n", ok ? "Ok(" : "Error(", (long long)val);
    }
}

static yk_thread_ret YK_THREAD_CALL yk_task_start(void* arg) {
    yk_task* t = (yk_task*)arg;
    t->entry(&t->result);
    return YK_THREAD_DONE;
}

uint64_t yk_run_thread(void (*entry)(int64_t*)) {
    yk_task* t = (yk_task*)malloc(sizeof(yk_task));
    t->entry = entry;
    t->result = 0;
    if (!yk_thread_spawn(&t->thread, yk_task_start, t)) { free(t); return 0; }
    return (uint64_t)(intptr_t)t;
}

int64_t yk_join_thread(uint64_t handle) {
    yk_task* t = (yk_task*)(intptr_t)handle;
    yk_thread_join(t->thread);
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
    yk_string* s = yk_string_header();
    s->data = (char*)malloc(n + 1);
    memcpy(s->data, buf, n + 1);
    s->len = n;
    s->cap = n + 1;
    return s;
}

void yk_time_sleep(int64_t ms) { if (ms > 0) yk_sleep_ms((unsigned int)ms); }

int64_t yk_time_timestamp(void) { return (int64_t)time(NULL); }

int64_t yk_sys_pid(void) { return yk_process_id(); }
void yk_sys_exit(int64_t code) { exit((int)code); }
yk_string* yk_sys_cwd(void) {
    char* cwd = yk_getcwd(NULL, 0);
    if (!cwd) return yk_string_from_cstr("");
    yk_string* s = yk_string_header();
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
    yk_string* s = yk_string_header();
    s->data = strdup(p);
    s->len = (int64_t)strlen(p);
    return s;
}
yk_string* yk_sys_env(yk_string* name) {
    char* val = getenv(name->data);
    if (!val) { val = ""; }
    yk_string* s = yk_string_header();
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
    yk_string* s = yk_string_header();
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
            yk_string* r = yk_string_header();
            r->data = buf; r->len = i;
            return r;
        }
    }
    yk_string* r = yk_string_header();
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
            yk_string* r = yk_string_header();
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
            yk_string* r = yk_string_header();
            r->data = buf; r->len = nlen;
            return r;
        }
        if (s->data[i] == '/' || s->data[i] == '\\') break;
    }
    yk_string* r = yk_string_header();
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

static yk_string* yk_string_take(char* buf, int64_t len) {
    yk_string* s = yk_string_header();
    s->data = buf;
    s->len = len;
    return s;
}

yk_string* yk_fs_read(yk_string* path) {
    FILE* f = fopen(path->data, "rb");
    if (!f) return yk_string_from_cstr("");
    if (fseek(f, 0, SEEK_END) != 0) { fclose(f); return yk_string_from_cstr(""); }
    long size = ftell(f);
    if (size < 0 || fseek(f, 0, SEEK_SET) != 0) { fclose(f); return yk_string_from_cstr(""); }
    char* buf = (char*)malloc((size_t)size + 1);
    if (!buf) { fclose(f); return yk_string_from_cstr(""); }
    size_t got = fread(buf, 1, (size_t)size, f);
    fclose(f);
    buf[got] = '\0';
    return yk_string_take(buf, (int64_t)got);
}

static void yk_fs_put(yk_string* path, yk_string* content, const char* mode) {
    FILE* f = fopen(path->data, mode);
    if (!f) return;
    fwrite(content->data, 1, (size_t)content->len, f);
    fclose(f);
}

void yk_fs_write(yk_string* path, yk_string* content) { yk_fs_put(path, content, "wb"); }

void yk_fs_append(yk_string* path, yk_string* content) { yk_fs_put(path, content, "ab"); }

void yk_fs_remove(yk_string* path) { remove(path->data); }

int64_t yk_fs_exists(yk_string* path) { return yk_path_kind(path->data) != YK_PATH_NONE; }

int64_t yk_fs_is_dir(yk_string* path) { return yk_path_kind(path->data) == YK_PATH_DIR; }

int64_t yk_fs_is_file(yk_string* path) { return yk_path_kind(path->data) == YK_PATH_FILE; }

static const char b64_enc[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

yk_string* yk_base64_encode(yk_string* input) {
    int64_t len = input->len;
    int64_t out_len = ((len + 2) / 3) * 4;
    char* out = (char*)malloc(out_len + 1);
    int64_t j = 0;
    for (int64_t i = 0; i < len; i += 3) {
        int64_t n = len - i;
        unsigned int a = (unsigned char)input->data[i];
        unsigned int b = n > 1 ? (unsigned char)input->data[i + 1] : 0;
        unsigned int c = n > 2 ? (unsigned char)input->data[i + 2] : 0;
        unsigned int triple = (a << 16) | (b << 8) | c;
        out[j++] = b64_enc[(triple >> 18) & 0x3F];
        out[j++] = b64_enc[(triple >> 12) & 0x3F];
        out[j++] = n > 1 ? b64_enc[(triple >> 6) & 0x3F] : '=';
        out[j++] = n > 2 ? b64_enc[triple & 0x3F] : '=';
    }
    out[out_len] = '\0';
    yk_string* s = yk_string_header();
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

static void yk_base64_invalid(void) {
    fflush(stdout);
    fprintf(stderr, "error: base64.decode: invalid input\n");
    exit(1);
}

static int b64_dec_slot(const char* quad, int k, bool last) {
    if (quad[k] == '=' && last && k >= 2) return -2;
    int v = b64_dec_char(quad[k]);
    if (v < 0) yk_base64_invalid();
    return v;
}

yk_string* yk_base64_decode(yk_string* input) {
    int64_t len = input->len;
    if (len % 4 != 0) yk_base64_invalid();
    char* out = (char*)malloc((size_t)(len / 4) * 3 + 1);
    int64_t j = 0;
    for (int64_t i = 0; i < len; i += 4) {
        const char* q = input->data + i;
        bool last = i + 4 == len;
        int a = b64_dec_slot(q, 0, last);
        int b = b64_dec_slot(q, 1, last);
        int c = b64_dec_slot(q, 2, last);
        int d = b64_dec_slot(q, 3, last);
        if (c == -2 && d != -2) yk_base64_invalid();
        unsigned int triple = ((unsigned)a << 18) | ((unsigned)b << 12) | ((unsigned)(c < 0 ? 0 : c) << 6) | (unsigned)(d < 0 ? 0 : d);
        out[j++] = (char)((triple >> 16) & 0xFF);
        if (c >= 0) out[j++] = (char)((triple >> 8) & 0xFF);
        if (d >= 0) out[j++] = (char)(triple & 0xFF);
    }
    out[j] = '\0';
    yk_string* s = yk_string_header();
    s->data = out; s->len = j;
    return s;
}

yk_string* yk_datetime_now(void) {
    time_t t = time(NULL);
    struct tm lt;
    yk_localtime(&t, &lt);
    char buf[64];
    strftime(buf, sizeof(buf), "%Y-%m-%d %H:%M:%S", &lt);
    return yk_string_from_cstr(buf);
}

yk_string* yk_datetime_utc(void) {
    time_t t = time(NULL);
    struct tm gt;
    yk_gmtime(&t, &gt);
    char buf[64];
    strftime(buf, sizeof(buf), "%Y-%m-%d %H:%M:%S", &gt);
    return yk_string_from_cstr(buf);
}

int64_t yk_datetime_year(int64_t ts) {
    struct tm lt; time_t t = (time_t)ts;
    yk_localtime(&t, &lt);
    return (int64_t)lt.tm_year + 1900;
}

int64_t yk_datetime_month(int64_t ts) {
    struct tm lt; time_t t = (time_t)ts;
    yk_localtime(&t, &lt);
    return (int64_t)lt.tm_mon + 1;
}

int64_t yk_datetime_day(int64_t ts) {
    struct tm lt; time_t t = (time_t)ts;
    yk_localtime(&t, &lt);
    return (int64_t)lt.tm_mday;
}

int64_t yk_datetime_hour(int64_t ts) {
    struct tm lt; time_t t = (time_t)ts;
    yk_localtime(&t, &lt);
    return (int64_t)lt.tm_hour;
}

int64_t yk_datetime_minute(int64_t ts) {
    struct tm lt; time_t t = (time_t)ts;
    yk_localtime(&t, &lt);
    return (int64_t)lt.tm_min;
}

int64_t yk_datetime_second(int64_t ts) {
    struct tm lt; time_t t = (time_t)ts;
    yk_localtime(&t, &lt);
    return (int64_t)lt.tm_sec;
}

yk_string* yk_datetime_format(int64_t ts, yk_string* fmt) {
    struct tm lt; time_t t = (time_t)ts;
    yk_localtime(&t, &lt);
    size_t sz = fmt->len * 2 + 64;
    char* buf = (char*)malloc(sz);
    strftime(buf, sz, fmt->data, &lt);
    return yk_string_from_cstr(buf);
}

extern "C" yk_string* yk_fetch(yk_string* url, yk_string* method, yk_string* body) {
    if (!url || url->len == 0) return yk_string_from_cstr("");
#ifdef _WIN32
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
    yk_string* s = yk_string_header();
    s->data = buf; s->len = total;
    return s;
#else
    (void)method; (void)body;
    yk_log("fetch: no HTTP client on this platform yet");
    return yk_string_from_cstr("");
#endif
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
    struct yk_route_node* next_method;
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
    while (cur->is_leaf && strncmp(cur->method, method, 7) != 0) {
        if (!cur->next_method) {
            yk_route_node* other = yk_route_node_new();
            other->segment = cur->segment;
            other->segment_len = cur->segment_len;
            other->is_param = cur->is_param;
            other->param_is_int = cur->param_is_int;
            cur->next_method = other;
        }
        cur = cur->next_method;
    }
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
    for (yk_route_node* leaf = cur; leaf && leaf->is_leaf; leaf = leaf->next_method) {
        if (strcmp(leaf->method, method) == 0 || strcmp(leaf->method, "*") == 0) return leaf;
    }
    for (yk_route_node* leaf = cur; leaf && leaf->is_leaf; leaf = leaf->next_method) {
        if (leaf->is_ws && strcmp(method, "GET") == 0) return leaf;
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
#ifndef _WIN32
    SOCKET socket;
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
#elif defined(YK_HAVE_URING)
    struct io_uring ring;
#endif
    void* handler_pool; // yk_handler_pool* for offloading handler execution
    void (*logger)(void* entry);
} yk_server;

#ifdef YK_HAVE_URING
static pthread_mutex_t yk_ring_mutex = PTHREAD_MUTEX_INITIALIZER;
#elif !defined(_WIN32)
static void yk_posix_finish_response(yk_conn* c);
#endif

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

typedef struct { yk_string status; yk_string body; } yk_log_response;
typedef struct { yk_string method; yk_string path; yk_string body; yk_log_response response; } yk_log_entry;

static yk_mutex_t yk_logger_mutex = YK_MUTEX_INIT;

void yk_server_set_logger(int64_t handle, void* fn_ptr) {
    yk_server* s = (yk_server*)(intptr_t)handle;
    s->logger = (void (*)(void*))fn_ptr;
}

static yk_string yk_borrowed_string(const char* data, int64_t len) {
    yk_string s;
    s.data = (char*)(data ? data : "");
    s.len = (data && len > 0) ? len : 0;
    s.cap = 0;
    return s;
}

static void yk_log_exchange(yk_server* s, const char* method, int64_t method_len, const char* path, int64_t path_len, int status, const char* body, int64_t body_len) {
    if (!s->logger) return;
    char status_text[64];
    int n = snprintf(status_text, sizeof(status_text), "%d %s", status, yk_status_reason(status));
    if (n < 0) n = 0;
    if (n >= (int)sizeof(status_text)) n = (int)sizeof(status_text) - 1;
    yk_log_entry entry;
    entry.method = yk_borrowed_string(method, method_len);
    entry.path = yk_borrowed_string(path, path_len);
    entry.body = yk_borrowed_string("", 0);
    entry.response.status = yk_borrowed_string(status_text, n);
    entry.response.body = yk_borrowed_string(body, body_len);
    yk_mutex_lock(&yk_logger_mutex);
    s->logger(&entry);
    yk_mutex_unlock(&yk_logger_mutex);
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
    yk_string* result = yk_string_header();
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
    yk_string* result = yk_string_header();
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
    yk_string* result = yk_string_header();
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
    yk_string* s = yk_string_header();
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

// TLS node cache - amortise malloc/free per-thread
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
#else
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
            // Retry pop - another thread might have stolen it
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
// NO blocking I/O - all I/O goes through IOCP (Win) or io_uring (Linux)
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

        // Execute handler (compute only - no I/O)
        char* handler_buf = yk_get_handler_buf(16384);
        item.handler(&item.resp, &item.req, handler_buf, yk_tls_handler_buf_size);

        int status = yk_valid_status(item.resp.status);
        char* body_ptr = (char*)item.resp.body;
        int64_t body_len = item.resp.body_len;
        yk_log_exchange(s, item._method, item.req.ml, item._path, item.req.pl, status, body_ptr, body_len);
        fflush(stdout);
        int keep_alive = (c->keep_alive && s->running);

        // Build response header into c->resp_buf
        const char* conn_hdr = keep_alive ? "keep-alive" : "close";
        if (yk_conn_grow_resp_buf(c, 4096) != 0) { yk_closesocket(c->socket); yk_conn_free(c); continue; }
        int n = snprintf(c->resp_buf, c->resp_buf_cap,
            "HTTP/1.1 %d %s\r\nContent-Length: %lld\r\nContent-Type: text/plain\r\nConnection: %s\r\n\r\n",
            status, yk_status_reason(status), (long long)(body_ptr ? body_len : 0), conn_hdr);
        c->header_len = n;
        c->body_ptr = body_ptr;
        c->body_len = (int)(body_ptr ? (body_len > 0 ? body_len : 0) : 0);
        c->resp_len = n + c->body_len;
        c->resp_sent = 0;

        // Post async send - IOCP/io_uring worker handles completion & keep-alive
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
#elif defined(YK_HAVE_URING)
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
#else
        yk_posix_finish_response(c);
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
static int yk_is_header_end_at(const char* buf, int buf_used, int i) {
    return i + 3 < buf_used && buf[i] == 0x0D && buf[i+1] == 0x0A && buf[i+2] == 0x0D && buf[i+3] == 0x0A;
}

static int yk_find_header_end(const char* buf, int buf_used) {
    int pos = 0;
#ifdef __AVX2__
    __m256i cr = _mm256_set1_epi8(0x0D);
    while (pos + 32 <= buf_used) {
        __m256i chunk = _mm256_loadu_si256((const __m256i*)(buf + pos));
        unsigned int m = (unsigned int)_mm256_movemask_epi8(_mm256_cmpeq_epi8(chunk, cr));
        while (m) {
            unsigned long bit;
#ifdef _MSC_VER
            _BitScanForward(&bit, m);
#else
            bit = (unsigned long)__builtin_ctz(m);
#endif
            m &= m - 1;
            if (yk_is_header_end_at(buf, buf_used, pos + (int)bit)) return pos + (int)bit;
        }
        pos += 32;
    }
#endif
    for (int i = pos; i + 3 < buf_used; i++) {
        if (yk_is_header_end_at(buf, buf_used, i)) return i;
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
    int headers_end = hdr_end + 2;
    while (i < hdr_end) {
        int line_start = i;
        // Find end of this header line (the last one ends inside the final CRLF CRLF)
        i = yk_find_char(buf, i, headers_end, '\n');
        if (i >= headers_end) break;
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

        status = yk_valid_status(resp.status);
        body_ptr = (char*)resp.body;
        body_len = resp.body_len;
    } else {
        static char not_found[] = "Not Found";
        status = 404;
        body_ptr = not_found;
        body_len = sizeof(not_found) - 1;
        yk_log("no route: %s %s (404)", c->method, c->path);
    }
    yk_log_exchange(s, c->method, c->method_len, c->path, path_len, status, body_ptr, body_len);
    fflush(stdout);

    // Build response: header goes into resp_buf, body is a separate pointer (global constant)
    const char* conn_hdr = keep_alive ? "keep-alive" : "close";
    // Ensure resp_buf is large enough for the header
    if (yk_conn_grow_resp_buf(c, 4096) != 0) return -1;
    int n = snprintf(c->resp_buf, c->resp_buf_cap,
        "HTTP/1.1 %d %s\r\nContent-Length: %lld\r\nContent-Type: text/plain\r\nConnection: %s\r\n\r\n",
        status, yk_status_reason(status), (long long)(body_ptr ? body_len : 0), conn_hdr);
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

static int yk_parse_listen_addr(yk_string* addr, char* host, int host_cap, int* port) {
    int len = (int)(addr->len < host_cap - 1 ? addr->len : host_cap - 1);
    memcpy(host, addr->data, len);
    host[len] = '\0';
    char* colon = strrchr(host, ':');
    if (!colon) return 0;
    *colon = '\0';
    int p = atoi(colon + 1);
    *port = (p > 0 && p < 65536) ? p : 8080;
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
        // Handler offloaded to dedicated thread pool - connection owned by handler worker
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
    yk_stdout_used();
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
    printf("Server listening on: %s:%d\n", host, port);
    yk_log("backend IOCP, %d threads", s->thread_count);
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
#elif defined(YK_HAVE_URING)

#include <sys/epoll.h>

// io_uring worker thread

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
                    // Handler offloaded to dedicated thread pool - connection owned by handler worker
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
    yk_stdout_used();
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
    printf("Server listening on: %s:%d\n", host, port);
    yk_log("backend io_uring, %d threads", s->thread_count);
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

#else

// ── Portable POSIX server (Linux without liburing, macOS, BSD) ──
// A fixed pool of blocking workers: the thread count bounds concurrency,
// and an idle timeout stops slow clients from holding a worker forever.
#include <signal.h>

#define YK_POSIX_QUEUE_CAP 1024
#define YK_POSIX_IDLE_TIMEOUT_SEC 10

typedef struct {
    yk_socket_t fds[YK_POSIX_QUEUE_CAP];
    int head;
    int len;
    pthread_mutex_t lock;
    pthread_cond_t ready;
    yk_server* server;
} yk_posix_queue;

static int yk_posix_queue_push(yk_posix_queue* q, yk_socket_t fd) {
    pthread_mutex_lock(&q->lock);
    int ok = q->len < YK_POSIX_QUEUE_CAP;
    if (ok) {
        q->fds[(q->head + q->len) % YK_POSIX_QUEUE_CAP] = fd;
        q->len++;
        pthread_cond_signal(&q->ready);
    }
    pthread_mutex_unlock(&q->lock);
    return ok;
}

static yk_socket_t yk_posix_queue_pop(yk_posix_queue* q) {
    pthread_mutex_lock(&q->lock);
    while (q->len == 0) pthread_cond_wait(&q->ready, &q->lock);
    yk_socket_t fd = q->fds[q->head];
    q->head = (q->head + 1) % YK_POSIX_QUEUE_CAP;
    q->len--;
    pthread_mutex_unlock(&q->lock);
    return fd;
}

static int yk_posix_send_response(yk_conn* c) {
    if (yk_send_all(c->socket, c->resp_buf, c->header_len) != 0) return -1;
    if (c->body_ptr && c->body_len > 0) return yk_send_all(c->socket, c->body_ptr, c->body_len);
    return 0;
}

static void yk_posix_close(yk_conn* c) {
    if (c->ws) { yk_ws_cleanup(c); return; }
    if (c->h2) { yk_h2_cleanup(c); return; }
    yk_closesocket(c->socket);
    yk_conn_free(c);
}

static void yk_posix_finish_response(yk_conn* c) {
    yk_posix_send_response(c);
    yk_posix_close(c);
}

static void yk_posix_serve_conn(yk_conn* c, yk_server* s) {
    for (;;) {
        if (yk_conn_grow_buf(c, c->buf_used + 4096) != 0) break;
        ssize_t n = recv(c->socket, c->buf + c->buf_used, (size_t)(c->buf_cap - c->buf_used - 1), 0);
        if (n < 0 && errno == EINTR) continue;
        if (n <= 0) break;
        c->buf_used += (int)n;
        c->buf[c->buf_used] = '\0';
        int ret = yk_process_request(c, s);
        if (c->ws || c->h2) { yk_log("posix server: WebSocket and HTTP/2 need the io_uring or IOCP server"); break; }
        if (ret < 0 || ret == 2) break;
        if (ret == 0) continue;
        if (yk_posix_send_response(c) != 0 || !c->keep_alive || !s->running) break;
        c->buf_used = 0;
    }
    yk_posix_close(c);
}

static void yk_posix_tune_socket(yk_socket_t fd) {
    struct timeval tv;
    tv.tv_sec = YK_POSIX_IDLE_TIMEOUT_SEC;
    tv.tv_usec = 0;
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));
    setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof(tv));
    int one = 1;
    setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &one, sizeof(one));
#ifdef SO_NOSIGPIPE
    setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &one, sizeof(one));
#endif
}

static yk_thread_ret YK_THREAD_CALL yk_posix_worker(void* arg) {
    yk_posix_queue* q = (yk_posix_queue*)arg;
    for (;;) {
        yk_socket_t fd = yk_posix_queue_pop(q);
        yk_conn* c = yk_conn_alloc();
        if (!c) { yk_closesocket(fd); continue; }
        c->socket = fd;
        yk_posix_serve_conn(c, q->server);
    }
    return YK_THREAD_DONE;
}

static yk_socket_t yk_posix_listen(const char* host, int port) {
    yk_socket_t fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) return YK_INVALID_SOCKET;
    int opt = 1;
    setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof(opt));
    struct sockaddr_in sa;
    memset(&sa, 0, sizeof(sa));
    sa.sin_family = AF_INET;
    sa.sin_port = htons((unsigned short)port);
    if (strcmp(host, "*") == 0 || strcmp(host, "0.0.0.0") == 0) sa.sin_addr.s_addr = INADDR_ANY;
    else if (inet_pton(AF_INET, host, &sa.sin_addr) != 1) { close(fd); return YK_INVALID_SOCKET; }
    if (bind(fd, (struct sockaddr*)&sa, sizeof(sa)) != 0 || listen(fd, SOMAXCONN) != 0) {
        close(fd);
        return YK_INVALID_SOCKET;
    }
    return fd;
}

void yk_server_serve(int64_t handle, yk_string* addr) {
    yk_stdout_used();
    yk_server* s = (yk_server*)(intptr_t)handle;
    if (!s || s->count == 0) { printf("No routes registered\n"); return; }
    char host[256];
    int port = 0;
    if (!yk_parse_listen_addr(addr, host, sizeof(host), &port)) { printf("Invalid address format\n"); return; }
    signal(SIGPIPE, SIG_IGN);
    s->listen_fd = yk_posix_listen(host, port);
    if (s->listen_fd == YK_INVALID_SOCKET) { printf("bind failed\n"); return; }

    static yk_posix_queue queue;
    memset(&queue, 0, sizeof(queue));
    pthread_mutex_init(&queue.lock, NULL);
    pthread_cond_init(&queue.ready, NULL);
    queue.server = s;
    s->running = 1;
    int workers = s->thread_count > 0 ? s->thread_count : 8;
    for (int i = 0; i < workers; i++) {
        yk_thread_t t;
        if (yk_thread_spawn(&t, yk_posix_worker, &queue)) pthread_detach(t);
    }
    printf("Server listening on: %s:%d\n", host, port);
    yk_log("backend posix, %d threads", workers);
    fflush(stdout);
    yk_log("serve start %s:%d (posix, %d routes, %d threads)", host, port, s->count, workers);

    while (s->running) {
        yk_socket_t fd = accept(s->listen_fd, NULL, NULL);
        if (fd < 0) { if (errno == EINTR) continue; break; }
        yk_posix_tune_socket(fd);
        if (!yk_posix_queue_push(&queue, fd)) { yk_log("posix server: queue full, dropping connection"); close(fd); }
    }
    close(s->listen_fd);
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

// WebSocket accept key: SHA-1(key + magic GUID) -> base64
static void yk_ws_accept_key(const char* client_key, char* out, int out_len) {
    const char* magic = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    yk_sha1_ctx ctx; yk_sha1_init(&ctx);
    yk_sha1_update(&ctx, (const unsigned char*)client_key, strlen(client_key));
    yk_sha1_update(&ctx, (const unsigned char*)magic, 36);
    unsigned char hash[20]; yk_sha1_final(&ctx, hash);
    yk_ws_base64_encode(hash, 20, out);
}

// WebSocket frame send (unmasked server -> client, cross-platform)
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
                case 9: // ping -> pong
                    yk_ws_send(c->socket, 10, ws->payload, ws->payload_len);
                    break;
                case 1: // text frame -> echo back
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
    if (!key_start) { yk_closesocket(c->socket); yk_conn_free(c); return; }
    key_start += 18; while (*key_start == ' ') key_start++;
    char* key_end = strstr(key_start, "\r\n"); if (!key_end) { yk_closesocket(c->socket); yk_conn_free(c); return; }
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
        if (opcode == 9) { yk_ws_send(c->socket, 10, NULL, 0); free(payload); continue; } // ping -> pong
        if (opcode == 1) { // text frame -> echo back
            yk_ws_send(c->socket, 1, payload, payload_len);
        }
        free(payload);
    }
    done:
    yk_closesocket(c->socket);
    yk_conn_free(c);
}

// ═══════════════════════════════════════════════════════════════
// HTTP/2 - RFC 7540
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
    // EOS symbol (0xFF) padding check - skip remaining bits
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
    // HPACK decoder state (dynamic table - simplified: reuse static only)
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
    uint8_t payload[18]; // 3 settings x 6 bytes each
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
            // Literal with indexing - name indexed or literal
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
