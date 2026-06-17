#!/bin/bash
# Yidika HTTP Server — Linux Benchmark Script
# Prerequisites: wrk2 (or wrk), h2load, perf
#
# Install:
#   sudo apt install wrk          # or build from https://github.com/giltene/wrk2
#   sudo apt install nghttp2-bin  # provides h2load
#   sudo apt install linux-tools-common linux-tools-$(uname -r) # perf
#
# Usage:
#   ./run_bench_linux.sh [duration_secs] [threads] [connections]

set -euo pipefail

DURATION=${1:-30}
THREADS=${2:-8}
CONNECTIONS=${3:-512}
SERVER_PORT=8080
SERVER_BIN="./bench_server.exe"
BASE_URL="http://127.0.0.1:${SERVER_PORT}"

echo "=== Yidika HTTP Benchmark ==="
echo "Duration: ${DURATION}s  Threads: ${THREADS}  Connections: ${CONNECTIONS}"
echo ""

# Build the benchmark server
echo "Building benchmark server..."
cargo run -- build "benchmarks/bench_server.yk" -o "${SERVER_BIN}"

# Start server in background
echo "Starting server..."
"${SERVER_BIN}" &
SERVER_PID=$!
sleep 1

# Verify server is up
if ! curl -s "${BASE_URL}/" > /dev/null 2>&1; then
    echo "ERROR: Server not responding"
    kill "${SERVER_PID}" 2>/dev/null || true
    exit 1
fi
echo "Server is up at ${BASE_URL}"

echo ""
echo "=== 1. HTTP/1.1 Throughput (wrk2) ==="
wrk2 -t"${THREADS}" -c"${CONNECTIONS}" -d"${DURATION}" \
    -R 2000000 --latency "${BASE_URL}/" 2>&1 || echo "(wrk2 not available)"
echo ""

echo "=== 2. HTTP/1.1 Throughput (wrk) ==="
wrk -t"${THREADS}" -c"${CONNECTIONS}" -d"${DURATION}" \
    --latency "${BASE_URL}/" 2>&1 || echo "(wrk not available)"
echo ""

echo "=== 3. HTTP/2 Throughput (h2load) ==="
h2load -t"${THREADS}" -c"${CONNECTIONS}" -m10 -w8 \
    -n$((CONNECTIONS * DURATION * 1000)) \
    --h2c "${BASE_URL}/" 2>&1 || echo "(h2load not available)"

echo ""
echo "=== 4. Perf flamegraph ==="
if command -v perf > /dev/null 2>&1; then
    perf record -g -p "${SERVER_PID}" -- sleep "${DURATION}" 2>/dev/null || true
    perf script 2>/dev/null | ./stackcollapse-perf.pl > out.perf-folded 2>/dev/null || true
    echo "Run: perf report or use FlameGraph to visualize"
else
    echo "(perf not available, install linux-tools)"
fi

# Cleanup
kill "${SERVER_PID}" 2>/dev/null || true
wait "${SERVER_PID}" 2>/dev/null || true
echo ""
echo "=== Benchmark complete ==="
