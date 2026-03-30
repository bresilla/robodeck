{
  description = "robodeck development shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    { nixpkgs, rust-overlay, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];

        pkgs = import nixpkgs {
          inherit system overlays;
          config.allowUnfree = true;
        };
      in
      {
        devShells.default = pkgs.mkShell {
          packages = [
            (pkgs.rust-bin.stable.latest.default.override {
              extensions = [
                "rust-src"
                "rustfmt"
                "clippy"
                "rust-analyzer"
              ];
              targets = [ "wasm32-unknown-unknown" ];
            })
            pkgs.pkg-config
            pkgs.trunk
            pkgs.wasm-pack
            pkgs.binaryen
            pkgs.wasm-bindgen-cli
            pkgs.cacert
          ];

          RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";
          shellHook = ''
            echo "robodeck dev shell"
          '';
        };
      }
    );
}
