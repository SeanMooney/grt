# grt task runner

# Build in debug mode
build:
    cargo build --workspace

# Build in release mode
build-release:
    cargo build --workspace --release

# Run tests
test:
    cargo test --workspace

# Run clippy, fmt check, and cargo-deny
lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo deny check

# Run cargo-deny license and advisory checks
deny:
    cargo deny check

# Check formatting
fmt:
    cargo fmt --all --check

# Run the application
run *ARGS:
    cargo run -- {{ARGS}}

# Pre-release checks (used by cargo-release)
pre-release-checks:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo deny check
    cargo test --workspace

# Remove build artifacts
clean:
    cargo clean
