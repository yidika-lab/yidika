@echo off
REM Yidika HTTP Server — Windows Benchmark Script
REM Prerequisites: curl for testing
REM
REM For RPS measurement, install:
REM   - bombardier: https://github.com/codesenberg/bombardier/releases
REM   - wrk2: build from https://github.com/giltene/wrk2 (requires MinGW)
REM
REM Usage: run_bench_windows.bat [port]

set PORT=8080
if not "%1"=="" set PORT=%1
set DURATION=10
set SERVER_BIN=bench_server.exe

echo === Yidika HTTP Benchmark (Windows) ===
echo Port: %PORT%  Duration: %DURATION%s
echo.

REM Build the benchmark server
echo Building benchmark server...
cargo run -- build benchmarks\bench_server.yk -o %SERVER_BIN%
if %ERRORLEVEL% neq 0 exit /b %ERRORLEVEL%

REM Start server
echo Starting server on port %PORT%...
start /B "" %SERVER_BIN%

REM Wait for startup
ping -n 3 127.0.0.1 > nul

REM Quick sanity check
curl -s http://127.0.0.1:%PORT%/ > nul
if %ERRORLEVEL% neq 0 (
    echo ERROR: Server not responding
    taskkill /f /im %SERVER_BIN% > nul 2>&1
    exit /b 1
)
echo Server is up on http://127.0.0.1:%PORT%/
echo.

REM Run bombardier benchmark if available
where bombardier > nul 2>&1
if %ERRORLEVEL% equ 0 (
    echo === 1. HTTP/1.1 Throughput (bombardier) ===
    bombardier -t %DURATION%s -c 256 http://127.0.0.1:%PORT%/
    echo.
) else (
    echo bombardier not found. Install from https://github.com/codesenberg/bombardier
    echo.
)

REM Basic curl timing test
echo === 2. HTTP/1.1 Timing (curl, 100 requests) ===
for /l %%i in (1,1,100) do (
    curl -s -o nul -w "%%{time_total}\n" http://127.0.0.1:%PORT%/
) 2>&1 | python -c "import sys; vals=[float(l.strip()) for l in sys.stdin if l.strip()]; print(f'Average: {sum(vals)/len(vals)*1000:.2f}ms  Min: {min(vals)*1000:.2f}ms  Max: {max(vals)*1000:.2f}ms')" 2>&1 || echo (python not available for statistics)

REM Cleanup
taskkill /f /im %SERVER_BIN% > nul 2>&1
echo.
echo === Benchmark complete ===
