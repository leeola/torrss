{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/master";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
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

        rustPlatform = pkgs.makeRustPlatform {
          cargo = rust-toolchain;
          rustc = rust-toolchain;
        };

        # `topcoat dev` ships in a separate crate from the framework, and the
        # CLI warns and refuses to drive a workspace whose resolved `topcoat`
        # version falls outside its own semver compatibility range. Keep this
        # version in step with `topcoat` in `[workspace.dependencies]`.
        topcoat-cli = rustPlatform.buildRustPackage rec {
          pname = "topcoat-cli";
          version = "0.6.2";

          src = pkgs.fetchCrate {
            inherit pname version;
            hash = "sha256-+6DKc2yjKwju64iokWV1EUXf62GGj1W32kp7E7WVKdM=";
          };

          cargoHash = "sha256-CDnvF0CMkpjIK0dDhuxIfpm0PnyEjFvbIjWhgSfFk1w=";

          doCheck = false;
        };

        torrss = rustPlatform.buildRustPackage {
          pname = "torrss";
          version = (pkgs.lib.importTOML ./bin/Cargo.toml).package.version;

          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              ./LICENSE
              ./bin
              ./lib
            ];
          };

          # The lock file resolves to crates.io alone, so there is no git
          # source needing `outputHashes` and no vendor hash to bump.
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = [
            # `lib/build.rs` runs the `tailwindcss` executable from PATH.
            pkgs.tailwindcss_4
            topcoat-cli
            pkgs.makeWrapper
          ];

          # `topcoat asset bundle` runs its own `cargo build` with every
          # `CARGO*` variable removed and no `--target`, so the hook's
          # env-only settings never reach it. Repeating them in config keeps
          # the inner build, and its `OUT_DIR`, identical to the hook's.
          postConfigure = ''
            mkdir -p .cargo
            cat >> .cargo/config.toml <<EOF

            [build]
            target = "${pkgs.stdenv.hostPlatform.rust.rustcTarget}"

            [net]
            offline = true

            [profile.release]
            strip = false
            EOF
          '';

          # `runHook postBuild` runs before the install hook copies the
          # release directory, so the bundle is scanned from the very binary
          # that gets installed.
          postBuild = "topcoat asset bundle --release --out assets";

          # `AssetBundle::load()` looks only beside the executable, and
          # `share/` keeps data out of `bin/`, so the wrapper points the
          # binary at the bundle. `--set-default` leaves a user's own
          # `--assets` in force.
          postInstall = ''
            mkdir -p $out/share/torrss
            cp -r assets $out/share/torrss/assets
            wrapProgram $out/bin/torrss \
              --set-default TORRSS_ASSETS $out/share/torrss/assets
          '';
        };
      in
      {
        packages = {
          default = torrss;
          inherit torrss;
        };

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
    )
    //
      {
        # `eachDefaultSystem` yields per-system outputs, and a NixOS module is
        # not one of those, so the module attaches outside that call.
        nixosModules.torrss = import ./nix/module.nix self;
        nixosModules.default = self.nixosModules.torrss;
      };
}
