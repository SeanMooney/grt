{
  description = "grt — Gerrit and Git CLI/TUI";

  inputs = {
    nixpkgs.url     = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay    = { url = "github:oxalica/rust-overlay";
                        inputs.nixpkgs.follows = "nixpkgs"; };
    crane           = { url = "github:ipetkov/crane"; };
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
      in {
        packages = {
          default = grt;
          grt     = grt;
        };

        checks = {
          inherit grt;
          clippy  = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets -- -D warnings";
          });
          fmt     = craneLib.cargoFmt    { inherit src; };
          nextest = craneLib.cargoNextest (commonArgs // {
            inherit cargoArtifacts;
          });
        };

        devShells.default = pkgs.mkShell {
          inputsFrom = [ grt ];
          packages = with pkgs; [
            toolchain
            cargo-nextest
            just
          ];
        };
      });
}
