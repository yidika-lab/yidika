use criterion::{black_box, criterion_group, criterion_main, Criterion};
use yidi::interpret::Interpreter;
use yidi::syntax::ast;
use yidi::syntax::parser::Parser;

fn run_yk(source: &str) -> String {
    ast::reset_ids();
    let module = Parser::parse(source).unwrap();
    let mut interp = Interpreter::new();
    interp.load_module(&module);
    interp.run_main().unwrap()
}

fn bench_empty_main(c: &mut Criterion) {
    c.bench_function("empty_main", |b| {
        b.iter(|| {
            run_yk(black_box("fn main() { }"));
        })
    });
}

fn bench_arithmetic(c: &mut Criterion) {
    c.bench_function("arithmetic_100k", |b| {
        b.iter(|| {
            run_yk(black_box("fn main() { x: int = 0; for (i in 0..100000) { x = x + i; } print(x); }"));
        })
    });
}

fn bench_function_calls(c: &mut Criterion) {
    c.bench_function("function_calls_10k", |b| {
        b.iter(|| {
            run_yk(black_box("fn add(a: int, b: int) -> int { return a + b; } fn main() { x: int = 0; for (i in 0..10000) { x = add(x, i); } print(x); }"));
        })
    });
}

fn bench_fib_recursive(c: &mut Criterion) {
    c.bench_function("fib_recursive_30", |b| {
        b.iter(|| {
            run_yk(black_box("fn fib(n: int) -> int { if (n <= 1) { return n; } return fib(n - 1) + fib(n - 2); } fn main() { print(fib(30)); }"));
        })
    });
}

fn bench_list_push(c: &mut Criterion) {
    c.bench_function("list_push_10k", |b| {
        b.iter(|| {
            run_yk(black_box("fn main() { list: [int] = []; for (i in 0..10000) { list.push(i); } print(len(list)); }"));
        })
    });
}

fn bench_string_concat(c: &mut Criterion) {
    c.bench_function("string_concat_1k", |b| {
        b.iter(|| {
            run_yk(black_box("fn main() { s: str = \"\"; for (i in 0..1000) { s = s + \"a\"; } print(len(s)); }"));
        })
    });
}

fn bench_method_dispatch(c: &mut Criterion) {
    c.bench_function("method_dispatch_10k", |b| {
        b.iter(|| {
            run_yk(black_box("class Counter { val: int; fn inc(self) { self.val = self.val + 1; } fn get(self) -> int { return self.val; } } fn main() { c: Counter = Counter { val: 0 }; for (i in 0..10000) { c.inc(); } print(c.get()); }"));
        })
    });
}

criterion_group!(benches, bench_empty_main, bench_arithmetic, bench_function_calls, bench_fib_recursive, bench_list_push, bench_string_concat, bench_method_dispatch);
criterion_main!(benches);
