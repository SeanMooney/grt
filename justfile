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

# Install pre-commit hooks (run once after clone)
setup-hooks:
    pre-commit install
    pre-commit install --hook-type commit-msg

# Cut a release: tag, publish to crates.io, and create GitHub release
release:
    #!/usr/bin/env bash
    set -euo pipefail
    VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*= *"//' | sed 's/".*//')
    echo "Releasing v${VERSION}..."
    cargo release "${VERSION}" --execute
    just build-release
    sha256sum target/release/grt > target/release/SHA256SUMS
    NOTES=$(awk "/## \[${VERSION}\]/{found=1; next} found && /^## \[/{exit} found{print}" CHANGELOG.md)
    gh release create "v${VERSION}" \
      --title "grt v${VERSION}" \
      --notes "${NOTES}" \
      target/release/grt \
      target/release/SHA256SUMS
    echo "Released v${VERSION}"

# Remove build artifacts
clean:
    cargo clean
