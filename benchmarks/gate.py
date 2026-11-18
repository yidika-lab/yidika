#!/usr/bin/env python3
"""Performance gate: every program in benchmarks/gate must run as fast as,
and use no more memory than, the same program written in C.

Usage: python3 benchmarks/gate.py [--yidi PATH] [--runs N]
Runs on Linux and macOS. Each program is started by benchmarks/measure.c,
a small C launcher, so its peak memory is not mixed with Python's.
Exit code 1 when a benchmark goes over a limit or prints a different result.
"""
import argparse
import os
import platform
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
GATE_DIR = os.path.join(HERE, "gate")
TIME_LIMIT = 1.15
MEMORY_LIMIT = 1.30
NOISE_FLOOR_S = 0.02


def native_cpu_flag():
    return "-mcpu=native" if platform.machine().lower() in ("arm64", "aarch64") else "-march=native"


def build_c(source, exe):
    subprocess.run(["clang", "-O3", native_cpu_flag(), source, "-o", exe], check=True)


def build_yidika(yidi, source):
    subprocess.run([yidi, "build", source], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return os.path.splitext(source)[0]


def pin_to_one_core():
    if hasattr(os, "sched_setaffinity"):
        os.sched_setaffinity(0, {sorted(os.sched_getaffinity(0))[-1]})


def run_once(launcher, exe, output_path):
    measured = subprocess.run([launcher, exe, output_path], check=True, capture_output=True, text=True).stdout.split()
    ns, rss_kb, code = (int(v) for v in measured)
    if code != 0:
        raise RuntimeError(f"{exe} exited with status {code}")
    with open(output_path, "rb") as f:
        return ns / 1e9, rss_kb, f.read()


def best_of(launcher, exe, runs, output_path):
    samples = [run_once(launcher, exe, output_path) for _ in range(runs)]
    return min(s[0] for s in samples), max(s[1] for s in samples), {s[2] for s in samples}


def over_limit(yidika, c, limit, floor=0.0):
    if c <= 0 or yidika <= floor:
        return False
    return yidika / c > limit


def check(name, yk, c):
    (yk_time, yk_mem, yk_out), (c_time, c_mem, c_out) = yk, c
    failures = []
    if yk_out != c_out or len(yk_out) != 1:
        failures.append(f"{name}: output differs from C")
    if over_limit(yk_time, c_time, TIME_LIMIT, NOISE_FLOOR_S):
        failures.append(f"{name}: {yk_time:.3f} s is more than {TIME_LIMIT}x C ({c_time:.3f} s)")
    if over_limit(yk_mem, c_mem, MEMORY_LIMIT):
        failures.append(f"{name}: {yk_mem} KB is more than {MEMORY_LIMIT}x C ({c_mem} KB)")
    return failures


def render(rows, failures):
    lines = [
        f"Performance gate: Yidika versus C (time limit {TIME_LIMIT}x, memory limit {MEMORY_LIMIT}x)",
        "",
        "| Benchmark | Yidika | C | Ratio | Yidika memory | C memory |",
        "|---|---|---|---|---|---|",
    ]
    for name, (yt, ym, _), (ct, cm, _) in rows:
        ratio = yt / ct if ct > 0 else 0
        lines.append(f"| {name} | {yt:.3f} s | {ct:.3f} s | {ratio:.2f} | {ym} KB | {cm} KB |")
    lines.append("")
    lines.extend(failures or ["All benchmarks are within their limits."])
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--yidi", default=shutil.which("yidi") or "yidi")
    parser.add_argument("--runs", type=int, default=5)
    args = parser.parse_args()
    work = tempfile.mkdtemp(prefix="yk_gate_")
    output_path = os.path.join(work, "stdout.txt")
    launcher = os.path.join(work, "measure")
    build_c(os.path.join(HERE, "measure.c"), launcher)
    rows, failures = [], []
    names = sorted(n[:-3] for n in os.listdir(GATE_DIR) if n.endswith(".yk"))
    for name in names:
        for ext in (".yk", ".c"):
            shutil.copy(os.path.join(GATE_DIR, name + ext), work)
        c_exe = os.path.join(work, name + "_c")
        build_c(os.path.join(work, name + ".c"), c_exe)
        yk_exe = build_yidika(args.yidi, os.path.join(work, name + ".yk"))
        pin_to_one_core()
        yk = best_of(launcher, yk_exe, args.runs, output_path)
        c = best_of(launcher, c_exe, args.runs, output_path)
        rows.append((name, yk, c))
        failures.extend(check(name, yk, c))
    shutil.rmtree(work, ignore_errors=True)
    report = render(rows, failures)
    print(report)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write(report + "\n")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
