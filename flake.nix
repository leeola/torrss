{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/master";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      nixpkgs,
      rust-overlay,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        rust-toolchain = (pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml).override {
          extensions = [ "rust-analysis" ];
        };

        # `topcoat dev` ships in a separate crate from the framework, and the
        # CLI warns and refuses to drive a workspace whose resolved `topcoat`
        # version falls outside its own semver compatibility range. Keep this
        # version in step with `topcoat` in `[workspace.dependencies]`.
        topcoat-cli =
          let
            rustPlatform = pkgs.makeRustPlatform {
              cargo = rust-toolchain;
              rustc = rust-toolchain;
            };
          in
          rustPlatform.buildRustPackage rec {
            pname = "topcoat-cli";
            version = "0.6.2";

            src = pkgs.fetchCrate {
              inherit pname version;
              hash = "sha256-+6DKc2yjKwju64iokWV1EUXf62GGj1W32kp7E7WVKdM=";
            };

            cargoHash = "sha256-CDnvF0CMkpjIK0dDhuxIfpm0PnyEjFvbIjWhgSfFk1w=";

            doCheck = false;
          };
      in
      {
        devShell = pkgs.mkShell {
          buildInputs = with pkgs; [
            pkg-config
            binutils
            gcc
            rust-analyzer
            # using a hardcoded rustfmt version to support nightly rustfmt features.
            rust-bin.nightly."2026-08-20".rustfmt
            rust-toolchain
            # Drives the feature-powerset compile check.
            cargo-hack
            # Provides `topcoat dev`, the auto-rebuilding development server.
            topcoat-cli
            # Read by the `build.rs` Tailwind step, which uses a dev shell CLI
            # instead of downloading one that NixOS cannot run.
            tailwindcss_4
          ];

          # Silence nixpkgs cc-wrapper's target-mismatch warning emitted
          # when Rust's `cc` crate canonicalizes Apple triples before
          # invoking clang (e.g. `aarch64-apple-darwin` -> `arm64-apple-macosx`).
          NIX_CC_WRAPPER_SUPPRESS_TARGET_WARNING = "1";
        };
      }
    );
}
