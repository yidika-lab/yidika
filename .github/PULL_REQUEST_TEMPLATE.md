## Description

What this change does, and which issue it closes.

Fixes #(issue)

## Research

What was checked before choosing this approach (specs, existing crates, how other projects solve it) and why this approach wins.

## Type of change

- [ ] Bug fix
- [ ] New feature
- [ ] Documentation update
- [ ] Refactor
- [ ] Test improvement
- [ ] CI / release

## Checklist

- [ ] Follows the design rules in [AGENTS.md](../AGENTS.md#design-rules-mandatory-for-every-change) (TDD, SOLID, KISS, no duplication, cross-platform)
- [ ] Tests were written first and pass (`cargo test --workspace --lib --bins -- --skip compat`)
- [ ] Language or std change: a `compat_*` test covers interpreter/AOT parity
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` is clean
- [ ] New or upgraded dependencies pass `cargo audit`
- [ ] Docs updated if behaviour changed
