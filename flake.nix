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
      in
      {
        devShell = pkgs.mkShell {
          buildInputs = with pkgs; [
            pkg-config
            binutils
            gcc
            rust-analyzer
            # using a hardcoded rustfmt version to support nightly rustfmt features.
            rust-bin.nightly."2026-05-28".rustfmt
            rust-toolchain
            # Drives the feature-powerset compile check.
            cargo-hack
          ];

          # Silence nixpkgs cc-wrapper's target-mismatch warning emitted
          # when Rust's `cc` crate canonicalizes Apple triples before
          # invoking clang (e.g. `aarch64-apple-darwin` -> `arm64-apple-macosx`).
          NIX_CC_WRAPPER_SUPPRESS_TARGET_WARNING = "1";
        };
      }
    );
}
