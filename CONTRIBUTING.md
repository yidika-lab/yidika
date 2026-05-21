# Contributing

We welcome contributions! Here's how to get started.

## Prerequisites

- Rust 1.70+ (`rustup`)
- Familiarity with the [project architecture](docs/dev/ARCHITECTURE.md)

## Getting Started

1. Fork the repository
2. Create a feature branch: `git checkout -b feat/my-feature`
3. Make your changes
4. Ensure tests pass: `cargo test`
5. Run clippy: `cargo clippy`
6. Submit a Pull Request

## Code Style

- **Rust**: follow `cargo fmt` (we use standard Rust formatting)
- **No unsafe code** unless absolutely necessary and documented
- **No comments in code** — prefer self-documenting code with clear variable/function names
- Keep functions small and focused (single responsibility)
- Write tests for every new feature (see existing `#[test]` patterns)

## Pull Request Guidelines

- Use a clear, descriptive title
- Reference any related issues
- Keep PRs focused — one feature/fix per PR
- Add tests for new functionality
- Update docs if the API changes

## Testing

```sh
cargo test                    # Run all tests
cargo test --lib              # Run lib tests
cargo clippy                  # Lint
cargo check --tests           # Check tests compile
```

## Commit Messages

Use conventional commits:
```
feat: add matrix multiplication support
fix: correct span reporting in if-else nodes
docs: update type system reference
test: add test for recursive function calls
```

## Code of Conduct

All contributors must follow our [Code of Conduct](CODE_OF_CONDUCT.md).
