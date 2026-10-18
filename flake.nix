{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { nixpkgs, ... }@inputs:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forEachSystem = nixpkgs.lib.genAttrs systems;
    in
    {
      devShells = forEachSystem (system:
        let
          pkgs = import nixpkgs {
            inherit system;
            config.allowUnfree = true;
            overlays = [
              inputs.rust-overlay.overlays.default
              (final: prev: {
                rustToolchain = prev.rust-bin.stable.latest.default.override {
                  extensions = [ "rust-src" "llvm-tools-preview" ];
                  targets = [ "wasm32-unknown-unknown" ];
                };
              })
            ];
          };
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              (if stdenv.isDarwin then vlc-bin else vlc)
              just
              mediamtx
              ffmpeg
              tailwindcss_4
              dioxus-cli
              bacon
              cargo-llvm-cov
              rustToolchain
            ];

            DIRENV = "Leo";
          };
        });
    };
}
