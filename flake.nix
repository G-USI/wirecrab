{
  description = "Wirecrab - AsyncAPI toolkit in Rust";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
    cross-rs.url = "github:cross-rs/cross";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" "clippy" ];
        };

        buildInputs = with pkgs;
          [ openssl pkg-config ] ++ lib.optionals stdenv.isDarwin [
            darwin.apple_sdk.frameworks.Security
            darwin.apple_sdk.frameworks.SystemConfiguration
          ];
      in {
      devShells.default = pkgs.mkShell {
          buildInputs = buildInputs
            ++ [ rustToolchain pkgs.reuse pkgs.commitlint ];

          RUST_BACKTRACE = 1;

          shellHook = ''
            # Configure git hooks on nix develop
            git config core.hooksPath .githooks
            echo "Git hooks configured: .githooks"
          '';
        };

      devShells.esp32 = pkgs.mkShell {
          buildInputs = with pkgs; [ cross.packages.x86_64-unknown-linux-gnu.build-std ];
          
          shellHook = ''
            # ESP32 cross-compilation via cross-rs
            echo "ESP32 build environment: xtensa-esp32-espidf"
          '';
        };
}
