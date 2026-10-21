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
          version = "0.10.0";

          src = pkgs.fetchCrate {
            inherit pname version;
            hash = "sha256-lk13D/RqrskzhXwLNz+KEbWowllQqGxoGU47T4ZdBvk=";
          };

          cargoHash = "sha256-/uztLZLC8A7lgAin8YoIrhv3+mOvy7OKJWdfAbQay9g=";

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

          # `topcoat asset bundle` runs its own `cargo build`, which must
          # resolve the same target, offline mode, and strip setting as the
          # hook's build. Its `OUT_DIR`, and the asset ids hashed from it,
          # match only then. The config holds those settings whatever
          # environment the CLI forwards.
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
            rust-bin.nightly."2026-10-01".rustfmt
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
    // {
      # `eachDefaultSystem` yields per-system outputs, and a NixOS module is
      # not one of those, so the module attaches outside that call.
      nixosModules.torrss = import ./nix/module.nix self;
      nixosModules.default = self.nixosModules.torrss;
    };
}
