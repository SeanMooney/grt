# Build and Release Strategy

## Overview

This document describes the build, packaging, reproducibility, supply chain security, and
release automation strategy for `grt`. It covers the tools and workflows used to compile and
cross-compile release artifacts, enforce license and security compliance across the dependency
tree, generate software bills of materials, sign and publish releases, and support a
productive local development workflow.

Implementation decisions — crate selection, application architecture, and library design —
are out of scope here and are covered in `tech-stack.md`.

The primary audience for `grt` is Linux users. Apple Silicon macOS is a supported secondary
target, produced via cross-compilation on Linux build nodes without requiring a macOS host.
There is no Homebrew integration; macOS users install via the Nix flake or `cargo install`.

Two platforms are supported in the initial release: `x86_64-unknown-linux-musl` and
`aarch64-apple-darwin`. `aarch64-unknown-linux-musl` is a planned future addition;
`x86_64-apple-darwin` (Intel macOS) is not planned.

All build and release operations are automated through Ansible playbooks orchestrated by Zuul
CI. GitHub hosts the public repository and provides a target for release artifact publication,
but no GitHub Actions compute is used for any build, test, or release workload.

---

## Repository Structure

The layout below shows the files and directories directly relevant to building, releasing, and
compliance. The internal structure of individual crates is defined in `tech-stack.md`.

```
grt/
├── Cargo.toml              # workspace root
├── Cargo.lock              # committed — essential for reproducible builds
├── rust-toolchain.toml     # pins the exact Rust toolchain version
├── deny.toml               # cargo-deny policy: licenses, advisories, bans
├── release.toml            # cargo-release configuration
├── flake.nix               # Nix flake: dev shell and reproducible build
├── flake.lock              # committed — locks all Nix inputs
├── justfile                # local task runner (wraps common cargo/nix invocations)
├── CHANGELOG.md            # Keep-a-Changelog format; updated before each release
├── .cargo/
│   └── config.toml         # cross-compilation linker overrides and target defaults
├── crates/
│   └── */                  # workspace crate layout defined in tech-stack.md
├── ansible/
│   ├── inventory/
│   └── playbooks/
│       ├── setup-node.yaml
│       ├── lint.yaml
│       ├── test.yaml
│       ├── nix-build.yaml
│       ├── cross-compile.yaml
│       ├── sbom.yaml
│       ├── sign.yaml
│       ├── integration.yaml
│       └── release.yaml
└── zuul.d/
    ├── jobs.yaml
    └── project.yaml
```

### Cargo.lock

`Cargo.lock` is always committed. For an application binary this is standard practice and a
hard requirement for reproducibility — it guarantees that every CI run, every `nix build`, and
every `cargo install` from the repository resolves to the identical dependency tree. Changes
to `Cargo.lock` are reviewed the same as any other dependency change.

### flake.lock

`flake.lock` is committed alongside `Cargo.lock`. It pins every Nix input — Nixpkgs, the Rust
overlay, crane — to exact content-addressed revisions. Neither lock file is updated
automatically; both are updated deliberately and reviewed before merge.

---

## Rust Toolchain Pinning

The exact Rust toolchain version is declared in `rust-toolchain.toml` at the workspace root.
Cargo reads this file automatically when any `cargo` subcommand is invoked inside the
repository. The Nix flake consumes the same file via `rust-overlay`, ensuring the dev shell,
the Nix build, and the CI build all use the identical compiler.

```toml
# rust-toolchain.toml
[toolchain]
channel    = "1.82.0"
components = ["rustfmt", "clippy", "rust-src", "rust-analyzer"]
targets    = [
  "x86_64-unknown-linux-musl",
  "aarch64-apple-darwin",
  # "aarch64-unknown-linux-musl",  # planned — not yet supported
]
```

Pinning to a specific release version rather than a channel alias (`stable`, `beta`) means
that toolchain updates are intentional, reviewed changes — not silent drift caused by a
routine promotion or a Nixpkgs update.

---

## Reproducible Builds with Nix Flakes

Nix provides the primary reproducibility guarantee. The `flake.nix` describes both the
development environment and the production build as a fully locked, content-addressed
derivation. Given the same `flake.lock`, any developer, any CI node, or any future auditor
can reproduce the exact same build output.

### Nix Installation

The upstream Nix installer from `nixos.org` is used on CI nodes and is the recommended
installer for developers. The Determinate Systems fork of the Nix installer is explicitly not
used; it diverges from upstream Nix in ways that can cause incompatibilities with the broader
Nix ecosystem and NixOS.

```bash
sh <(curl -sSf -L https://nixos.org/nix/install) --daemon --yes
```

### Why Crane

The flake uses the `crane` library rather than Nixpkgs's built-in `buildRustPackage`. Crane
separates the dependency compilation step from the workspace crate compilation step. The
compiled dependencies are stored as a Nix derivation and cached independently. Rebuilds —
whether local or on CI — only recompile workspace crates whose source has changed, not the
full dependency tree. This makes incremental builds substantially faster and is the primary
reason crane is preferred over `buildRustPackage` for Rust projects in Nix.

### Flake Structure

```nix
# flake.nix (abridged — the full file in the repository is authoritative)
{
  description = "grt — Gerrit and Git CLI/TUI";

  inputs = {
    nixpkgs.url     = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay    = { url = "github:oxalica/rust-overlay";
                        inputs.nixpkgs.follows = "nixpkgs"; };
    crane           = { url = "github:ipetkov/crane";
                        inputs.nixpkgs.follows = "nixpkgs"; };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, crane, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs      = import nixpkgs { inherit system;
                      overlays = [ rust-overlay.overlays.default ]; };
        toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
        craneLib  = (crane.mkLib pkgs).overrideToolchain toolchain;

        # Exclude non-Rust files from triggering unnecessary rebuilds
        src = craneLib.cleanCargoSource ./.;

        commonArgs = {
          inherit src;
          strictDeps = true;
          buildInputs = [];
        };

        # Dependencies compiled once and stored as a cached Nix derivation
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        grt = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          doCheck = true;
        });

        # Static musl build — x86_64 Linux is the sole Linux release target initially
        pkgsMusl   = pkgs.pkgsStatic;
        craneMusl  = (crane.mkLib pkgsMusl).overrideToolchain toolchain;
        grt-static = craneMusl.buildPackage (commonArgs // {
          cargoArtifacts     = craneMusl.buildDepsOnly commonArgs;
          CARGO_BUILD_TARGET = "x86_64-unknown-linux-musl";
        });
      in {
        packages = {
          default = grt;
          grt     = grt;
          static  = grt-static;
        };

        checks = {
          inherit grt;
          clippy  = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets -- -D warnings";
          });
          fmt     = craneLib.cargoFmt    { inherit src; };
          deny    = craneLib.cargoDeny   { inherit src; };
          nextest = craneLib.cargoNextest (commonArgs // {
            inherit cargoArtifacts;
          });
        };

        devShells.default = craneLib.devShell {
          checks   = self.checks.${system};
          packages = with pkgs; [
            toolchain
            rust-analyzer
            cargo-deny
            cargo-audit
            cargo-cyclonedx
            syft
            cargo-release
            cargo-nextest
            just
            cosign
          ];
        };
      });
}
```

The `checks` attribute wires `cargo fmt`, `cargo clippy`, `cargo deny`, and `cargo nextest`
into `nix flake check`, giving developers the same quality gate locally as CI provides.

---

## Static Linking and Cross-Compilation

### Linux: musl Static Binaries

Production Linux binaries are statically linked against musl libc. A statically linked musl
binary carries no runtime dependency on the host glibc version or any other shared library. It
runs on any Linux kernel without installation steps, making it the most portable distribution
format for Linux CLI tooling.

One Linux target is supported in the initial release:

- `x86_64-unknown-linux-musl` — primary release target; x86-64 Linux servers and workstations

`aarch64-unknown-linux-musl` (ARM64 Linux: servers, Raspberry Pi, Ampere-class hardware) is a
planned future target. When added it will follow the same static musl build approach and
require only the addition of the target to `rust-toolchain.toml` and the cross-compile
playbook.

### macOS: cargo-zigbuild

Cross-compilation to macOS targets is handled by `cargo-zigbuild`, which uses the Zig
compiler as a cross-linker. This avoids the macOS SDK licensing constraints that make
conventional macOS cross-compilation impractical from a Linux host. The Zig toolchain is
available in Nixpkgs and is therefore pinned by `flake.lock` like every other build
dependency.

One macOS target is supported:

- `aarch64-apple-darwin` — Apple Silicon (M-series)

Intel macOS (`x86_64-apple-darwin`) is not planned. macOS binaries are produced on Linux
Zuul nodes and are not run or tested in CI against a macOS host. macOS users who require
higher confidence are directed to build from source using the Nix flake or `cargo build`
directly.

### .cargo/config.toml

```toml
[target.x86_64-unknown-linux-musl]
rustflags = ["-C", "target-feature=+crt-static"]

# aarch64-unknown-linux-musl will be added here when that target is introduced

[target.aarch64-apple-darwin]
linker = "aarch64-apple-darwin-gcc"    # provided by cargo-zigbuild via Zig
```

---

## License and Supply Chain Compliance

### License Policy

`grt` will be released under one of MIT, BSD-2-Clause, or Apache-2.0; the exact choice has
not yet been made. Regardless of which permissive license is selected, the entire dependency
tree must be composed of comparably permissive crates. Copyleft and viral licenses are
incompatible with this requirement and are treated as hard build errors.

`cargo-deny` enforces this policy on every CI run. GPL variants (all versions), LGPL, AGPL,
SSPL, and BUSL are in the deny list and cause the `grt-lint` job to fail immediately if any
dependency — direct or transitive — carries one of those licenses. There is no warning
threshold for these; they are always errors.

```toml
# deny.toml

[licenses]
allow = [
  "MIT",
  "Apache-2.0",
  "Apache-2.0 WITH LLVM-exception",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "ISC",
  "Unicode-DFL",
  "CC0-1.0",
]
deny = [
  "GPL-2.0",
  "GPL-3.0",
  "AGPL-3.0",
  "LGPL-2.0",
  "LGPL-2.1",
  "LGPL-3.0",
  "SSPL-1.0",
  "BUSL-1.1",
]
copyleft   = "deny"
unlicensed = "deny"

[advisories]
db-path  = "~/.cargo/advisory-db"
db-urls  = ["https://github.com/rustsec/advisory-db"]
ignore   = []     # advisories are never silently suppressed

[bans]
multiple-versions = "warn"
deny              = []

[sources]
unknown-registry = "deny"
unknown-git      = "deny"
allow-registry   = ["https://github.com/rust-lang/crates.io-index"]
```

### cargo-audit

`cargo-audit` checks `Cargo.lock` against the RustSec advisory database on every CI run,
independently of `cargo-deny`. Running both provides belt-and-suspenders coverage: `cargo
deny` is fast and policy-driven; `cargo-audit` provides a standalone advisory check that can
also be run locally with `just audit` without requiring the full deny configuration.

---

## Software Bill of Materials (SBOM)

An SBOM is generated for every release and attached as a release artifact. Two complementary
tools provide coverage at different levels of the supply chain.

### cargo-cyclonedx — Source-Level SBOM

`cargo-cyclonedx` reads `Cargo.lock` and emits a CycloneDX SBOM enumerating every crate in
the dependency tree — direct and transitive — with their versions, content hashes, and
declared SPDX license identifiers. This is the source-level view: it describes what the
software is built from.

Both JSON and XML CycloneDX variants are generated and attached to each release:

```bash
cargo cyclonedx --format json --output-cdx sbom.cargo.cdx.json
cargo cyclonedx --format xml  --output-cdx sbom.cargo.cdx.xml
```

### syft — Artifact-Level SBOM

`syft` (by Anchore) scans the compiled release binary and produces an SBOM reflecting what is
present in the artifact itself. For a fully static musl binary this largely agrees with the
`cargo-cyclonedx` output, but `syft` additionally captures components that do not appear in
`Cargo.lock` — notably musl libc itself and the Zig runtime components used during macOS
cross-linking. Both SPDX and CycloneDX formats are produced from the primary Linux release
binary:

```bash
syft packages file:./grt-x86_64-unknown-linux-musl \
  -o spdx-json=sbom.syft.spdx.json                 \
  -o cyclonedx-json=sbom.syft.cdx.json
```

All four SBOM files are attached to the GitHub Release alongside the binaries.

---

## Binary Signing

Release binaries are signed using `cosign` in keyless mode. Keyless signing obtains a
short-lived signing certificate from Fulcio (the Sigstore certificate authority) by
authenticating with an OIDC token provided by the Zuul CI node's identity. No private key
material is stored or managed in the repository or in Zuul's secrets store. A transparency
log entry is written to Rekor automatically as part of every signing operation.

Each binary receives its own certificate and detached signature file:

```bash
cosign sign-blob                                  \
  --yes                                           \
  --oidc-issuer https://accounts.google.com       \
  --output-certificate grt-x86_64-linux-musl.crt  \
  --output-signature   grt-x86_64-linux-musl.sig  \
  grt-x86_64-linux-musl
```

The `.crt` and `.sig` files are attached to the release alongside the binary and can be
verified by any user with `cosign` installed:

```bash
cosign verify-blob                                      \
  --certificate grt-x86_64-linux-musl.crt               \
  --signature   grt-x86_64-linux-musl.sig               \
  --certificate-oidc-issuer https://accounts.google.com \
  grt-x86_64-linux-musl
```

---

## Release Versioning

### Semantic Versioning

`grt` follows Semantic Versioning. For a CLI/TUI tool the practical meaning is:

- **PATCH** — bug fixes, dependency updates, no behaviour or interface change
- **MINOR** — new commands or capabilities, backward-compatible changes
- **MAJOR** — breaking changes to the CLI interface, configuration format, or stored state

### cargo-release

`cargo-release` manages the mechanics of cutting a release: version bumps across the
workspace, `CHANGELOG.md` finalisation, Git commit, and tag push. It is configured in
`release.toml` at the workspace root.

```toml
# release.toml
[workspace]
shared-version   = true
tag-name         = "v{{version}}"
pre-release-hook = ["just", "pre-release-checks"]
```

The `pre-release-checks` just recipe runs `cargo fmt --check`, `cargo clippy`, `cargo deny
check`, and `cargo nextest run` before any version bump is committed. This ensures a broken or
non-compliant state can never be tagged.

### CHANGELOG

`CHANGELOG.md` follows the Keep-a-Changelog format. Content is written by hand. `cargo
release` automates only the mechanical parts: replacing the `[Unreleased]` header with the
version number and date, and opening a fresh `[Unreleased]` section for subsequent work.

---

## CI Architecture: Zuul

All CI work runs on Zuul. GitHub serves as the repository host and the target for release
artifact upload. The Zuul GitHub driver reports job status back to pull requests and commits
as commit statuses, giving contributors visibility into CI results. No GitHub Actions are
used.

### Pipelines

Three pipelines cover the full development and release lifecycle:

- **check** — runs on every pull request and every push to a feature branch; provides fast
  feedback on lint, formatting, and the test suite
- **gate** — runs on pull requests approved for merge; includes cross-compilation and
  integration testing; the commit cannot merge until this passes
- **release** — triggered by pushing a version tag (`v*`); produces all distributable
  artifacts and publishes the GitHub Release

### zuul.d/jobs.yaml

```yaml
- job:
    name: grt-base
    abstract: true
    nodeset:  ubuntu-jammy
    timeout:  1800
    pre-run:  ansible/playbooks/setup-node.yaml

- job:
    name: grt-lint
    parent: grt-base
    run: ansible/playbooks/lint.yaml
    description: >
      cargo fmt --check, cargo clippy -D warnings, cargo deny check.
      Denied licenses (GPL, LGPL, AGPL, SSPL, BUSL) are fatal errors here.

- job:
    name: grt-test
    parent: grt-base
    run: ansible/playbooks/test.yaml
    description: >
      Full workspace test suite via cargo-nextest.
      Produces JUnit XML consumed by Zuul for per-test result reporting.

- job:
    name: grt-nix-build
    parent: grt-base
    run: ansible/playbooks/nix-build.yaml
    description: >
      nix build .#grt — verifies the Nix flake builds cleanly.
      Runs nix flake check to validate all checks defined in the flake.

- job:
    name: grt-cross-compile
    parent: grt-base
    timeout: 3600
    run: ansible/playbooks/cross-compile.yaml
    description: >
      Produces release binaries for the two initial supported targets:
        x86_64-unknown-linux-musl
        aarch64-apple-darwin
      Generates SHA256SUMS. Passes artifacts to dependent jobs.
      (aarch64-unknown-linux-musl will be added as a future target.)

- job:
    name: grt-sbom
    parent: grt-base
    run: ansible/playbooks/sbom.yaml
    dependencies:
      - grt-cross-compile
    description: >
      cargo-cyclonedx: source-level CycloneDX SBOM (JSON + XML).
      syft: artifact-level SPDX and CycloneDX SBOM from the primary binary.

- job:
    name: grt-sign
    parent: grt-base
    run: ansible/playbooks/sign.yaml
    dependencies:
      - grt-cross-compile
    description: >
      cosign keyless signing of all four release binaries.
      Produces a .crt and .sig file per binary.

- job:
    name: grt-integration
    parent: grt-base
    timeout: 2400
    run: ansible/playbooks/integration.yaml
    description: >
      Starts a Gerrit instance in Docker.
      Runs end-to-end tests against the live REST API and SSH interface.

- job:
    name: grt-publish-release
    parent: grt-base
    run: ansible/playbooks/release.yaml
    dependencies:
      - grt-cross-compile
      - grt-sbom
      - grt-sign
    description: >
      Uploads all artifacts (binaries, checksums, SBOMs, signatures) to the
      GitHub Release. Transitions the release from draft to published.
      Publishes workspace crates to crates.io.
```

### zuul.d/project.yaml

```yaml
- project:
    name: your-org/grt

    check:
      jobs:
        - grt-lint
        - grt-test
        - grt-nix-build

    gate:
      jobs:
        - grt-lint
        - grt-test
        - grt-nix-build
        - grt-cross-compile
        - grt-integration

    release:
      jobs:
        - grt-lint
        - grt-test
        - grt-cross-compile
        - grt-sbom
        - grt-sign
        - grt-publish-release
```

`grt-lint` and `grt-test` are required in the release pipeline so that no artifact can be
published from a state that would have failed gate.

---

## Ansible Playbooks

Each Zuul job is backed by a single Ansible playbook. Playbooks are kept intentionally narrow
— each does one thing. Complex logic lives in the tools themselves (`cargo`, `syft`, `cosign`,
`gh`), not in Ansible task files.

### setup-node.yaml — CI Node Preparation

Installs Nix, the Rust toolchain from `rust-toolchain.toml`, and all auxiliary tools on a
fresh Zuul executor node. This runs as the `pre-run` for every job via `grt-base`.

```yaml
# ansible/playbooks/setup-node.yaml
- hosts: all
  tasks:
    - name: Install Nix (upstream installer, daemon mode)
      shell: |
        sh <(curl -sSf -L https://nixos.org/nix/install) --daemon --yes
      args:
        creates: /nix/store
      become: true

    - name: Configure Nix
      copy:
        dest: /etc/nix/nix.conf
        content: |
          experimental-features = nix-command flakes
          max-jobs = auto
          substituters = https://cache.nixos.org
          trusted-public-keys = cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY=
      become: true

    - name: Restart Nix daemon
      systemd:
        name: nix-daemon
        state: restarted
      become: true

    - name: Install rustup
      shell: |
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
          | sh -s -- -y --no-modify-path
      args:
        creates: "{{ ansible_env.HOME }}/.cargo/bin/rustup"

    - name: Activate toolchain declared in rust-toolchain.toml
      shell: "{{ ansible_env.HOME }}/.cargo/bin/rustup show"
      args:
        chdir: "{{ zuul.project.src_dir }}"

    - name: Install cargo tooling
      shell: |
        cargo install          \
          cargo-nextest        \
          cargo-deny           \
          cargo-cyclonedx      \
          cargo-release        \
          cargo-zigbuild
      environment:
        PATH: "{{ ansible_env.HOME }}/.cargo/bin:{{ ansible_env.PATH }}"

    - name: Install syft
      shell: |
        curl -sSfL https://raw.githubusercontent.com/anchore/syft/main/install.sh \
          | sh -s -- -b /usr/local/bin
      args:
        creates: /usr/local/bin/syft
      become: true

    - name: Install cosign
      shell: |
        LATEST=$(curl -sSf https://api.github.com/repos/sigstore/cosign/releases/latest \
          | grep tag_name | cut -d '"' -f4)
        curl -sSfLo /usr/local/bin/cosign \
          "https://github.com/sigstore/cosign/releases/download/${LATEST}/cosign-linux-amd64"
        chmod 755 /usr/local/bin/cosign
      args:
        creates: /usr/local/bin/cosign
      become: true
```

### lint.yaml — Format, Clippy, Deny

```yaml
# ansible/playbooks/lint.yaml
- hosts: all
  vars:
    src: "{{ zuul.project.src_dir }}"
  tasks:
    - name: Check formatting
      shell: cargo fmt --all --check
      args:
        chdir: "{{ src }}"
      environment:
        PATH: "{{ ansible_env.HOME }}/.cargo/bin:{{ ansible_env.PATH }}"

    - name: Run clippy (warnings as errors)
      shell: cargo clippy --workspace --all-targets -- -D warnings
      args:
        chdir: "{{ src }}"
      environment:
        PATH: "{{ ansible_env.HOME }}/.cargo/bin:{{ ansible_env.PATH }}"

    - name: Run cargo-deny (license and advisory policy)
      shell: cargo deny check
      args:
        chdir: "{{ src }}"
      environment:
        PATH: "{{ ansible_env.HOME }}/.cargo/bin:{{ ansible_env.PATH }}"
```

### test.yaml — Test Suite

```yaml
# ansible/playbooks/test.yaml
- hosts: all
  vars:
    src: "{{ zuul.project.src_dir }}"
  tasks:
    - name: Run tests with cargo-nextest
      shell: |
        cargo nextest run  \
          --workspace      \
          --profile ci     \
          --no-fail-fast
      args:
        chdir: "{{ src }}"
      environment:
        PATH: "{{ ansible_env.HOME }}/.cargo/bin:{{ ansible_env.PATH }}"
        RUST_BACKTRACE: "1"

    - name: Fetch JUnit test results
      synchronize:
        src:  "{{ src }}/target/nextest/ci/junit.xml"
        dest: "{{ zuul.executor.log_root }}/junit.xml"
        mode: pull
```

A `[profile.ci]` section in `.config/nextest.toml` configures `junit { path = "junit.xml" }`
so Zuul can parse test results and surface per-test failures directly in the review interface.

### cross-compile.yaml — Release Binaries

```yaml
# ansible/playbooks/cross-compile.yaml
- hosts: all
  vars:
    src:     "{{ zuul.project.src_dir }}"
    out_dir: "{{ zuul.executor.work_root }}/artifacts"
    linux_targets:
      - x86_64-unknown-linux-musl
      # aarch64-unknown-linux-musl will be added here when that target is introduced
    macos_targets:
      - aarch64-apple-darwin
  tasks:
    - name: Create artifact directory
      file:
        path:  "{{ out_dir }}"
        state: directory

    - name: Build Linux musl targets
      shell: |
        cargo build --release --target {{ item }} --locked
        cp target/{{ item }}/release/grt \
           {{ out_dir }}/grt-{{ item }}
      args:
        chdir: "{{ src }}"
      loop: "{{ linux_targets }}"
      environment:
        PATH: "{{ ansible_env.HOME }}/.cargo/bin:{{ ansible_env.PATH }}"

    - name: Build macOS targets via cargo-zigbuild
      shell: |
        cargo zigbuild --release --target {{ item }} --locked
        cp target/{{ item }}/release/grt \
           {{ out_dir }}/grt-{{ item }}
      args:
        chdir: "{{ src }}"
      loop: "{{ macos_targets }}"
      environment:
        PATH: "{{ ansible_env.HOME }}/.cargo/bin:{{ ansible_env.PATH }}"

    - name: Generate SHA256 checksums
      shell: sha256sum grt-* > SHA256SUMS
      args:
        chdir: "{{ out_dir }}"
```

### sbom.yaml — SBOM Generation

```yaml
# ansible/playbooks/sbom.yaml
- hosts: all
  vars:
    src:     "{{ zuul.project.src_dir }}"
    out_dir: "{{ zuul.executor.work_root }}/artifacts"
  tasks:
    - name: Generate CycloneDX SBOM from Cargo.lock (JSON)
      shell: |
        cargo cyclonedx \
          --format json \
          --output-cdx {{ out_dir }}/sbom.cargo.cdx.json
      args:
        chdir: "{{ src }}"
      environment:
        PATH: "{{ ansible_env.HOME }}/.cargo/bin:{{ ansible_env.PATH }}"

    - name: Generate CycloneDX SBOM from Cargo.lock (XML)
      shell: |
        cargo cyclonedx \
          --format xml \
          --output-cdx {{ out_dir }}/sbom.cargo.cdx.xml
      args:
        chdir: "{{ src }}"
      environment:
        PATH: "{{ ansible_env.HOME }}/.cargo/bin:{{ ansible_env.PATH }}"

    - name: Generate syft SBOM from primary release binary
      shell: |
        syft packages \
          file:{{ out_dir }}/grt-x86_64-unknown-linux-musl \
          -o spdx-json={{ out_dir }}/sbom.syft.spdx.json   \
          -o cyclonedx-json={{ out_dir }}/sbom.syft.cdx.json
```

### sign.yaml — Cosign Keyless Signing

```yaml
# ansible/playbooks/sign.yaml
- hosts: all
  vars:
    out_dir: "{{ zuul.executor.work_root }}/artifacts"
    binaries:
      - grt-x86_64-unknown-linux-musl
      - grt-aarch64-apple-darwin
      # grt-aarch64-unknown-linux-musl will be added when that target is introduced
  tasks:
    - name: Sign release binaries with cosign (keyless)
      shell: |
        cosign sign-blob                                       \
          --yes                                                \
          --oidc-issuer https://accounts.google.com            \
          --output-certificate {{ out_dir }}/{{ item }}.crt   \
          --output-signature   {{ out_dir }}/{{ item }}.sig   \
          {{ out_dir }}/{{ item }}
      loop: "{{ binaries }}"
```

### release.yaml — GitHub Release Publishing

```yaml
# ansible/playbooks/release.yaml
- hosts: all
  vars:
    out_dir: "{{ zuul.executor.work_root }}/artifacts"
    tag:     "{{ zuul.tag }}"
    repo:    "{{ zuul.project.canonical_name }}"
  tasks:
    - name: Install GitHub CLI
      shell: |
        curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
          | sudo dd of=/usr/share/keyrings/githubcli-archive-keyring.gpg
        echo "deb [arch=$(dpkg --print-architecture) \
          signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] \
          https://cli.github.com/packages stable main" \
          | sudo tee /etc/apt/sources.list.d/github-cli.list
        sudo apt-get update && sudo apt-get install -y gh
      args:
        creates: /usr/bin/gh
      become: true

    - name: Create draft GitHub Release
      shell: |
        gh release view {{ tag }} --repo {{ repo }} 2>/dev/null || \
        gh release create {{ tag }}  \
          --repo  {{ repo }}         \
          --draft                    \
          --title "{{ tag }}"        \
          --generate-notes
      environment:
        GH_TOKEN: "{{ github_token }}"

    - name: Upload all artifacts to release
      shell: |
        gh release upload {{ tag }} \
          --repo {{ repo }}         \
          --clobber                 \
          {{ out_dir }}/*
      environment:
        GH_TOKEN: "{{ github_token }}"

    - name: Publish release (draft → published)
      shell: |
        gh release edit {{ tag }} \
          --repo {{ repo }}       \
          --draft=false
      environment:
        GH_TOKEN: "{{ github_token }}"

    - name: Publish crates to crates.io
      shell: cargo publish --workspace --locked
      args:
        chdir: "{{ zuul.project.src_dir }}"
      environment:
        PATH:        "{{ ansible_env.HOME }}/.cargo/bin:{{ ansible_env.PATH }}"
        CARGO_TOKEN: "{{ crates_io_token }}"
```

Both `github_token` and `crates_io_token` are stored in Zuul's secrets store and are never
committed to the repository. `github_token` requires only `contents: write` scope on the
`grt` repository.

---

## Distribution and Installation

### Nix Flake

Users with Nix installed can build and run `grt` directly from the flake without cloning the
repository:

```bash
nix run github:your-org/grt
```

Or install it permanently into a Nix profile:

```bash
nix profile install github:your-org/grt
```

Submission of a package to the `nixpkgs` collection is a future consideration once the CLI
interface and configuration format are stable.

### crates.io and cargo install

The binary crate is published to crates.io to support installation by users who have a Rust
toolchain:

```bash
cargo install grt
```

Publishing is handled by the `release.yaml` playbook and is gated on the full release
pipeline succeeding.

### Direct Binary Download

Pre-built binaries for both supported targets are attached to every GitHub Release, alongside
`SHA256SUMS`, the SBOM files, and per-binary cosign signatures. Users without Nix or a Rust
toolchain can download the appropriate binary, verify the checksum and signature, and place it
in their `PATH`.

A minimal shell script (`install.sh`) at the repository root automates this: it detects
whether the host is Linux x86-64 or macOS ARM64, downloads the matching binary from the
latest GitHub Release, verifies the SHA256 checksum, and installs to `~/.local/bin`. The only
dependencies are `curl` and `sha256sum`.

---

## Developer Workflow

### Local Development

The Nix dev shell provides the pinned Rust toolchain and all build and compliance tools
without requiring any global installation. Entering the shell is the only required setup step
on a machine with Nix:

```bash
nix develop
```

The `justfile` provides short recipes for common local operations:

```
just build          # cargo build for the host target
just build-static   # cargo build --target x86_64-unknown-linux-musl
just test           # cargo nextest run --workspace
just lint           # fmt check + clippy + cargo deny check
just audit          # cargo audit (standalone RustSec advisory check)
just sbom           # generate SBOMs locally into ./target/sbom/
just release-dry    # cargo release --dry-run (preview version bump and changelog)
```

Ansible playbooks invoke `cargo` and tools directly. The `justfile` is for developer
convenience only and is not used by CI.

### Updating Dependencies

Dependency updates are deliberate rather than automated:

1. `cargo update` produces a revised `Cargo.lock`
2. `just lint` and `just audit` are run locally to validate the updated tree against license
   policy and the advisory database
3. The updated `Cargo.lock` is committed in a dedicated pull request describing what changed
4. The Zuul `check` pipeline validates the update before it can merge

For Nix inputs, `nix flake update` is run, the `flake.lock` diff is reviewed, and `nix flake
check` is run locally before committing. Toolchain bumps (changes to `rust-toolchain.toml`)
follow the same process and are always a separate commit from dependency updates.

### Cutting a Release

1. Ensure `CHANGELOG.md` has a complete entry under `[Unreleased]`
2. Run `cargo release minor` (or `patch` or `major` as appropriate)
3. `cargo-release` runs the pre-release checks, bumps all workspace crate versions, finalises
   the changelog, commits, and pushes the version tag
4. The Zuul `release` pipeline fires automatically on the pushed tag and handles all artifact
   production and publication
