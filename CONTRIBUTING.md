# Contributing to Composable Rust

Thank you for your interest in contributing to Composable Rust!

> **Note**: This project is currently in early development (Phase 0). Contribution guidelines will be finalized in Phase 1.

## Development Setup

### Prerequisites

- Rust 1.85.0 or later (required for edition 2024)
- Cargo

### Getting Started

1. Clone the repository
2. Run `cargo build --all-features`
3. Run `cargo test --all-features`
4. Run `./scripts/check.sh` to run all quality checks

## Code Quality Standards

### Formatting

All code must be formatted with `rustfmt`:

```bash
cargo fmt --all
```

Configuration is in `rustfmt.toml`.

### Linting

All code must pass clippy with strict lints:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Lint configuration is in the workspace `Cargo.toml`.

### Documentation

All public APIs must be documented:

- Use `///` for public API documentation
- Use `//!` for module-level documentation
- Include examples where appropriate
- Build docs with: `cargo doc --no-deps --all-features`

### Testing

- All business logic must have unit tests
- Use property-based testing where appropriate (proptest)
- Test coverage should be comprehensive
- Run tests with: `cargo test --all-features`

## Code Review Process

Coming in Phase 1.

## Architecture Decisions

Before making significant architectural changes:

1. Review the [Architecture Specification](specs/architecture.md)
2. Review the [Implementation Roadmap](plans/implementation-roadmap.md)
3. Discuss proposed changes in an issue first

## Development Workflow

1. Create a feature branch
2. Make your changes
3. Run `./scripts/check.sh` to ensure all checks pass
4. Commit with clear, descriptive messages
5. Submit a pull request

## Questions?

- Check the [Architecture Specification](specs/architecture.md)
- Review the [Implementation Roadmap](plans/implementation-roadmap.md)
- Open an issue for discussion

## License

By contributing, you agree that your contributions will be dual-licensed under MIT and Apache-2.0.
