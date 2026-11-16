#!/bin/sh
# Times fib(35) three ways: yidi native, rustc -O, yidi interpreter.
# Needs: yidi on PATH, rustc, and a C++ compiler for yidi build.
set -eu
cd "$(dirname "$0")"
yidi build fib.yk >/dev/null
rustc -O -o fib_rs fib.rs
best() {
    best=""
    for _ in 1 2 3; do
        start=$(date +%s.%N)
        "$@" >/dev/null
        end=$(date +%s.%N)
        t=$(echo "$end - $start" | bc)
        if [ -z "$best" ] || [ "$(echo "$t < $best" | bc)" = 1 ]; then best=$t; fi
    done
    printf '%.3f s\n' "$best"
}
printf 'yidi native       '; best ./fib
printf 'rustc -O          '; best ./fib_rs 35
printf 'yidi interpreter  '; best yidi run fib.yk --interp
rm -f fib fib_rs
