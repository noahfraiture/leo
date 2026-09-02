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
      system = "aarch64-darwin";
      pkgs = import nixpkgs {
        inherit system;
        config.allowUnfree = true;
        overlays = [
          # Overlay which add rust-bin to pkgs
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
      devShells.${system}.default = pkgs.mkShell {

        packages = with pkgs; [
          vlc-bin
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
    };
}
